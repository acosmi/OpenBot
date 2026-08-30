# Batch79：Desktop Local instance-bound PostgreSQL 17 bootstrap

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G2-G6-desktop-sidecar-bootstrap`
>
> base：`dd794ff82c18cdf2a644933d09e069dd4e3c82ce`（PR #61 merge commit）
>
> implementation：`9e1600dca21a815f7c4bfd71ec7635cb711c5a0c`
>
> 第一真源：v4 §5.1–§5.3、§6.1、§6.5、§13.1–§13.3、§14.1、§15.3、§16.2、§24 G2/G6、§28.1 R151–R153；GUI v2 §15.1。

## 1. 结论

本批关闭 R152 留下的“任意 PostgreSQL pool 可能跨 app instance”缺口，并为后续 Tauri setup 建立不可绕过的 pre-window bootstrap：

- host 已断言的 current-user app-data root 只派生一个直接子目录 `postgresql-17-<instance-id>`；
- 新目录从创建时即为 Unix 0700，既有 symlink、非目录或 group/other 可见目录 fail-closed；
- 连接后、任何 schema/principal/package 写入前，向活 PostgreSQL 反查并验证 actual data directory、major、监听面、当前连接地址、password mode 与 HBA；
- Server 原有 fresh/legacy/Rust-managed 初始化从 transport 上收到 infra，Server 与 Desktop 使用同一实现；
- bootstrap 顺序固定为 attestation → shared database initialization → canonical principal → exact-tenant single-user package synchronization；
- 自包含 PostgreSQL 17.11 真测试证明 first Fresh、restart RustManaged、membership，以及第二 app instance 连接同库在写前被拒。

本批没有伪造 sidecar supervisor 或 Tauri binary。真实 `AppHandle::path().app_data_dir()`、进程启动锁、随机 SCRAM secret 的 OS key-store、ready/shutdown/backup/upgrade、reviewed Desktop package/identity 与 native window setup 仍未闭合。

## 2. 为什么不能直接写 Tauri setup

R152 已有 principal 与 package membership，但方法接收 caller 提供的任意 `Pool`。固定 actor `desktop-local-user` 若接到另一 app instance 的数据库，会获得另一实例的 thread、memory 与 membership；窗口即使绑定了正确 `AuthContext`，数据库真源仍然错位。

审计还发现：

- fresh/legacy/native 数据库初始化实现住在 `openbot-server::database`；Desktop 复制它会形成第二 migration 真源；
- 仓内尚无 PostgreSQL sidecar 发行 pin/supervisor，也无 reviewed Desktop package、产品 identity 或 binary；
- 只验证“将传给 `initdb` 的计划路径”不等于证明活库实际使用该路径；
- 只读 `password_encryption` 只说明新密码如何写，不证明 TCP HBA 真的要求 SCRAM；
- `listen_addresses=localhost` 会把暴露判据交给 hosts 解析，不满足机械 closed set。

因此下一依赖不是一个接受任意 pool 的假 `.setup`，而是先让 pool 自证“我就是这个 instance 的本地 PG17 sidecar”。

## 3. 实现

### 3.1 Instance-bound data directory

`DesktopLocalAuthorityStore::load_or_create_installation` 复用 Batch77 的 CSPRNG instance，并在同一 asserted app-data root 下派生：

```text
postgresql-17-<64 lowercase hex instance id>
```

路径是 root 的直接子项，major 与 instance 都进入名称。目录新建时 Unix mode=0700；已存在时必须是非 symlink 普通目录且 group/other bits=0。canonical parent 必须精确等于 canonical root。`Debug` 只显示 non-secret instance id，目录路径固定 `<redacted>`。

普通 `load_or_create()` 仍只读取/铸造 authority，不因一次身份读取额外创建数据库目录；只有 installation setup 入口创建 sidecar path。

### 3.2 活库 attestation

`bootstrap_postgres` 在任何业务写入前从 PostgreSQL 本身取得并判定：

- `data_directory` 必须是绝对路径，canonical 后逐字等于 installation 目录；
- `server_version_num / 10000 == 17`；
- `listen_addresses` 每项只允许空、`127.0.0.1`、`::1`，不接受 `localhost` / `*` / 其它地址；
- `host(inet_server_addr())` 必须为 loopback，Unix socket 则为 `NULL`；
- `password_encryption` 必须为 `scram-sha-256`；
- `pg_hba_file_rules` 必须零解析错误，且所有 `host%` 规则的 `auth_method` 都是 `scram-sha-256`。

查询/解码/权限不足统一为 stable attestation unavailable；mismatch、major、exposure 与 SCRAM 分别有稳定无载荷错误。错误与 `Debug` 都不保存 setting 原值、path 或 secret。

### 3.3 唯一数据库初始化与启动顺序

Server 原 `DatabaseOrigin`、错误分层与 `initialize` 实现原样上收到 `openbot_infra::db::initialization`；`openbot_server::database` 只 re-export，既有调用与测试 API 不变。

Desktop installation 的完整 pre-window 顺序为：

1. package tenant 与 authority tenant exact 相等，否则数据库调用 0；
2. attestation 活库 scope/version/exposure/SCRAM；
3. 共用 fresh/legacy/Rust-managed 初始化；
4. Batch78 canonical principal/sole admin provision；
5. 同一 authority 的 single-user Tenant Package synchronization。

后续 Tauri setup 只能在这五步成功后创建 Batch76 verified window；本批没有提前创建窗口。

## 4. 自包含 PostgreSQL 17.11 纵向

ignored integration test 不依赖预先运行的共享测试库：

1. production authority store 在唯一临时 app-data root 铸 instance 与 sidecar data-dir；
2. 本机 Homebrew PostgreSQL 17.11 `initdb` 就在该精确 data-dir；
3. 配置只监听数值 `127.0.0.1`，host auth 全 SCRAM，runtime Unix socket 使用短 `/tmp/obpg-*`；
4. 测试账号经 TCP password 连接并创建空 `openbot` 库；
5. 首次 bootstrap 实得 `DatabaseOrigin::Fresh`、membership grant=1、single-user groups ignored；
6. 数据库实查 canonical user、admin role、`desktop-home` membership 都存在；
7. 同一 Fresh 库逐字段复核 T-TEST-0912 的 nullable callback hash/time 两列；
8. 第二次 bootstrap 实得 `RustManaged`、grant=0；
9. 第二 app instance 生成自己的 exact-tenant package，却连接第一实例 pool，得到 `PostgresDataDirectoryMismatch`，写入前拒绝；
10. pool close、PG fast-stop，app-data 与 socket 临时目录均为 0。

测试 password 是仓内明示的 test-only fixture，不是随机生产 secret，更不是 Keychain/Credential Manager 证据。

## 5. 首跑失败与修正

- integration test 首次 no-run 编译因局部变量遮蔽同名 package helper 失败；机械改名后编译绿；
- sandbox `initdb` 因 SysV `shmget` EPERM 失败，尚未进入产品 SQL，不计通过；
- 宿主首次 `pg_ctl` 因 macOS Unix socket 103-byte 上限拒绝长临时路径；只把 runtime socket 改为短 `/tmp/obpg-*`，data-dir 仍在 instance root；
- 下一次 attestation 把 `inet_server_addr()::text` 的 `127.0.0.1/32` 当非 IP 拒绝；改用 PostgreSQL `host(inet_server_addr())`，没有放宽非-loopback；
- HBA 证明加入后 Clippy 首跑以 `too_many_arguments` 判红；改成 closed `PostgresSidecarAttestation` 记录，未加 allow。

上述失败均不记为通过；最终结果以下表为准。

## 6. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| data-dir/filesystem/attestation targeted | `8/0/0` |
| self-contained PG17.11 bootstrap | `1/0/0` |
| Infra完整lib | `319/0/0` |
| Server完整lib | `216/0/0` |
| Infra + Server all-target/all-feature Clippy `-D warnings` | HBA后首跑too-many-arguments红；closed record修复后通过 |
| format/diff | `cargo fmt --all -- --check`、`git diff --check`通过 |
| parity | 首跑要求T-TEST-0912 revalidate；真PG复核两列并加exception后最终`813/881/1694`、0 violation、required revalidate=1且已满足；fixtures `17/22/39`、overlay `1444/242/2/6` |
| strict recount | clean pinned upstream `891df72f…`，`159/0/0` |
| Grok | Git tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`，diff0；inventory 2,110 files |
| invariants | Cargo.lock/workflow/dependency与parity ledger/T-ID集合diff0；overlay仅新增1条机械要求的revalidate；非Grok恰一个`package.json`；新增npm/package=0 |

Windows完整infra/sidecar未跑，不能从macOS推定Windows PostgreSQL service、DACL、Credential Manager或进程行为。本批无API/UI/T-ID/CSS/locale变化，没有重跑Trunk、Browser、Engine或golden。没有运行 `cargo xtask ci`，没有派发GitHub Actions。

## 7. 未闭合边界

- Tauri binary 尚未真实调用 `AppHandle::path().app_data_dir()`；本批 root 是 host assertion 与测试临时路径；
- PostgreSQL sidecar release pin、manifest/digest/release epoch、进程 supervisor、启动锁、ready probe、graceful shutdown、orphan recovery、backup/upgrade 均未实现；
- 随机 SCRAM secret 尚未进入 macOS Keychain / Windows Credential Manager；test-only password 不得外推；
- reviewed Desktop package 尚未解决随机 instance tenant 的安全构造/绑定，测试 package 只承担纵向 fixture；
- authority 尚未通过实际 Tauri setup 交给 Batch76 lifecycle，未创建真实Wry/WebView2窗口；
- Desktop Remote、capability、`tauri.conf`、发行identity/binary、Windows真机与golden仍todo；
- 本批不新增 Grok 产品能力，不修改 `grok-bot/`，不新增 npm/package/dependency。

下一批必须先实现 sidecar supervisor 的窄契约（pin/digest、single-instance start lock、OS key-store secret、ready/shutdown）或在这些依赖具备后接真实 Tauri setup；不得让 setup 接受未通过本批 attestation 的任意 pool。
