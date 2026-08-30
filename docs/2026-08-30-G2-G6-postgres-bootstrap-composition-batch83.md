# Batch83：PostgreSQL sidecar → Desktop Local bootstrap composition

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G2-G6-postgres-bootstrap-composition`
>
> base：`9a23a992b9fbe42c34c54f8cd95b1eee4d2fb585`（PR #65 merge commit）
>
> implementation：`798cdbe59c0f952c182cacae14002b03bfc3e10b`
>
> 第一真源：v4 §5.1–§5.2、§6.1、§6.4–§6.5、§13.1–§13.3、§14.1、§15.3、§16.2–§16.4、§24 G2/G6、§28.1 R153–R157；GUI v2 §15.1。

## 1. 结论

本批把 Batch82 的 SCRAM-ready child 与 Batch79 的共享数据库 bootstrap 接成一个 pre-window owner：

1. running child 的 data-dir 必须与同一 `DesktopLocalInstallation` exact 相等；
2. Tenant Package tenant 必须先等于 app-instance tenant，失败时数据库写入为 0；
3. 只以 numeric `127.0.0.1`、sidecar 固定 admin role 与一个 startup-only `SecretBytes` 建 admin pool；
4. admin pool 先执行 R153 live attestation；unattested typestate 没有建库 API；
5. attestation 后才创建/核验固定 `openbot` 业务库；
6. 复用 Batch79 的 migration → canonical principal → exact-tenant package；
7. `RunningDesktopLocalDataPlane` 同时持 pool、installation、bootstrap report 与 exact sidecar；
8. clean shutdown 先关闭 pool，再经 verified `pg_ctl` 等 exact child 后释放 start lock；失败/普通 Drop 保留 stale lock。

这不是完整 Tauri setup：没有从真实 `AppHandle::path().app_data_dir()` 取路径，没有 production `ApplicationService` assembly，也没有创建窗口。

## 2. Admin role 单一真源

首次宿主纵向中，supervisor ready 与直接查询均通过，但 infra pool 稳定返回 SQLSTATE 28P01。逐项对拍后确认根因不是 password：Batch82 `initdb` 创建的 role 是 `desktop_admin`，新 infra 代码误写成 `postgres`；此前正向对照使用 `connection.user()`，所以成功。

修复把非秘密 role 上收到 `openbot_contracts::desktop::DESKTOP_LOCAL_POSTGRES_ADMIN_USER`。`initdb --username`、ready probe、borrowed connection、infra admin pool、业务库 owner 与纯 join 测试全部消费同一常量；不再保留第二个 role 字面量。

## 3. Attestation-before-write typestate

`connect_for_attestation` 只返回 `UnattestedDesktopLocalAdmin`。它能借 admin pool 给 `DesktopLocalInstallation::attest_postgres_admin`，但没有 `CREATE DATABASE` 能力。只有 exact data-directory、PG major 17、numeric loopback/current address、password encryption 与 all-host HBA SCRAM 全通过后，才能得到 `AttestedDesktopLocalAdmin` 并调用 `connect_application`。

因此顺序不是注释约定，而是 API 可达性：

```text
SCRAM ready child
  → Unattested admin pool
  → R153 live attestation
  → Attested admin
  → fixed database
  → shared init/principal/package
  → RunningDesktopLocalDataPlane
```

## 4. 固定业务库与 unknown-commit

业务库只允许固定 `openbot`：owner=`desktop_admin`、UTF8、`C` collate/ctype、非 template、允许连接、connection limit=-1。既有数据库逐字段不等即 fail-closed。

首次不存在时执行一条固定 `CREATE DATABASE`；若命令返回错误，立即重读 `pg_database`：exact row 存在则显式记为 `ReconciledAfterCreateError`，不存在则 `Create` 失败，错误 row 则 `ShapeInvalid`。这兑现 §15.3 unknown-commit reconciliation，不把未知结果伪装成普通 success/500。

## 5. Secret 与 feature graph

composition 从 sidecar 明文只复制一次到 startup-only `SecretBytes`，贯穿 admin typestate 与 application pool 构造，随后 Drop 擦除；没有第二个 public `DatabaseConfig.password String`、环境变量或 URL。deadpool/tokio-postgres 为后续连接保留的 driver-owned password copy 属协议必要内存，不冒充可擦除。

`openbot-infra` 拆为：

- `desktop-local`：db、canonical principal/audit helper、Tenant Package PostgreSQL adapter；
- `server-runtime`：SafeDialer/provider/MCP等 Server 网络图；
- `server-sso`：在 `server-runtime` 上追加 SAML/xmlsec/OpenSSL，仍是默认 Server 图。

workspace 默认关闭 infra features；Server/testkit 显式开 `server-sso`，Desktop 只开 `desktop-local`。Windows Desktop tree 的 `ring/rustls/samael/openssl-sys` 均为 0；Server tree仍必须含 rustls、samael 0.0.22 与 openssl-sys。没有新 crate/package。

## 6. 真实 PostgreSQL 17.11 纵向

ignored host fixture继续复制本机 Homebrew 17.11 的 `postgres/initdb/pg_ctl` 到测试 manifest；它只证明 production state machine，动态 dylib/share 仍来自 Homebrew，不是发行 bundle。

同一 app instance 三段执行：

- 第一次启动把 wrong-tenant package 交给 composition：稳定 `PackageScope`，verified clean shutdown，start lock 0；
- 第二次启动从 admin `postgres` 库查询 `openbot` 不存在后 clean shutdown，机械证明上一段数据库写 0；
- 第三次启动：attestation→`openbot` Created→shared Fresh→canonical local user/sole admin→membership 1，查询 current database/user role/channel membership 全真，clean shutdown；
- 第四次启动：database Existing→shared RustManaged→membership grant 0，再 clean shutdown；
- app/bundle/data/lock测试目录最终 0。

最终宿主单跑=`1/0/0`。

## 7. 完整 revalidation 与首跑失败

feature-gating命中 `openbot-infra` 粗 owner 前缀，parity首跑要求155条 done T-ID revalidate。没有直接写 overlay：先用一次性 PostgreSQL 17.11 集簇跑真矩阵。

首轮被磁盘 ENOSPC 阻断，未开始测试；清理本仓 target 后释放19.8 GiB。第二轮 trust HBA 被错口令负向测试正确拒绝；最终改为test-only SCRAM集簇。期间真实发现并修复三个既有 ignored-test缺陷：

- component manifest已13项，排除3项应为10、admin排除2项应为11，旧断言仍是8/9；
- native 0021历史fixture错误调用apply-latest，实际跑到0023；改为apply-through 0021；
- native 0022同形态，改为apply-through 0022。

最终 `openbot-infra --tests --include-ignored` 由 `--list` 机械计514条，全部`514/0/0`；`agent_runtime_postgres=8/0/0`。随后才把精确155 T-ID加入exception-only overlay。

## 8. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| Desktop完整all-feature | `120/0/2`；新增composition pure test，两个ignored分别为Batch82与Batch83 host fixture |
| Batch83 host PG17.11 | `1/0/0` |
| infra完整lib | `321/0/0` |
| infra全部tests+ignored | `514/0/0`，SCRAM临时集簇 |
| agent runtime真库 | `8/0/0` |
| Server完整lib | `216/0/0`，Server SSO保持 |
| macOS Clippy | Desktop最小图、infra/Server完整图 all-target/all-feature `-D warnings`通过 |
| Windows | Desktop+唯一unsafe crate all-target/all-feature target Clippy通过；runtime未跑 |
| Linux | `desktop-local-bootstrap` feature-only compile通过；既有budget/transport dead-code 2 warning |
| dependency guards | key-store/feature/Tauri/六target cargo-deny通过 |
| Cargo | package `825→825`；只给Desktop增加既有workspace `openbot-infra` direct edge |
| parity | `813/881/1694`、0 violation；overlay=`1289/397/2/6`，155条本批机械revalidate |
| strict recount | pinned upstream `159/0/0` |
| Grok/package/npm | Git tree `86f5a85f…`、inventory 2,110；非Grok一个package.json；npm=0 |

本批无API/UI/T-ID/CSS/locale变化，没有重跑Trunk/Browser/Engine/golden。没有运行 `cargo xtask ci`，没有派发GitHub Actions。

## 9. 未闭合边界

- release PostgreSQL binary仍未reproducibly build/sign/manifest；Homebrew fixture非发行证据；
- Windows process与Credential Manager真机未跑；Linux Secret Service未实现；
- 真实Tauri `AppHandle::path().app_data_dir()`、setup background task、production `ApplicationService` assembly、verified window-last顺序未接；
- `tauri.conf.json`、capability、产品identity、真实Wry journey/golden未落；
- crash residue production repair/orphan identity、backup/restore/upgrade/release epoch assembly未实现；
- Desktop Local installed-app OAuth、完整Vault/KMS与G2/G6整关不关闭；
- 不修改grok-bot，不新增Grok产品能力/npm。

下一批应先抽取/复用 production `ApplicationService` assembly，再以Tauri实际path resolver在background runtime执行 authority→bundle/process→本批data plane；只有全部成功后才绑定并创建主窗口，任何失败窗口数必须为0。
