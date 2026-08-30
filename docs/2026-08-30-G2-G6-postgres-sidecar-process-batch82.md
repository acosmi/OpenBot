# Batch82：PostgreSQL sidecar verified process lifecycle

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G2-G6-postgres-sidecar-process`
>
> base：`84fe4f11e935eb7e9c21b618d8f6ec6ad4dceab1`（PR #64 merge commit）
>
> implementation：`cd66ed46c0255137bac2de0c91f5077503b48461`
>
> 第一真源：v4 §5.1–§5.2、§6.4、§13.1、§14.1、§15.3、§16.2–§16.4、§24 G2/G6、§28.1 R154–R156；GUI v2 §15.1。

## 1. 结论

本批把 Batch79–81 的 path/bundle/lock/secret 原语收成一个唯一 PostgreSQL 进程 owner：

1. app-data root、instance data-dir 必须 exact direct-child、绝对、非symlink、Unix私有；
2. acquire instance+manifest start lock；
3. 三个 manifest-verified program 在独立5秒deadline内语义验证 PostgreSQL 17.11；
4. 持锁从OS key-store load/create Batch81 secret；
5. data-dir空才执行 `initdb`，secret只经stdin prompt两次输入，整个write+wait受30秒deadline；
6. 写closed loopback/SCRAM runtime config与HBA；
7. 直接spawn verified `postgres`并持有exact Child；
8. 只有10秒内TCP SCRAM连接成功并执行`SELECT 1`才返回ready；
9. clean shutdown只经verified `pg_ctl -m fast -w`，再等待exact Child成功退出后释放lock；
10. unclean Drop先把lock改为stale-preserve，再kill child；显式repair前下一次start稳定held。

本批闭合“进程到ready/shutdown”而不是完整Desktop启动。它没有依赖 `openbot-infra`，因此没有执行 Batch79 shared schema initialization、canonical principal 或 Tenant Package；也没有创建Tauri window。

## 2. 为什么不直接塞进 Tauri setup

若 setup 自己拼 `Command`：

- 可能回退到PATH/Homebrew而绕过Batch80 manifest；
- 可能把secret放进argv/env/password file；
- 可能看到子进程存在就加载window，但HBA/口令/port尚不可用；
- Drop时若先释放lock再结束进程，会产生短暂双owner；
- 在Windows上若为连infra而直接拉全SAML/OpenSSL图，会重现已知原生构建阻塞。

因此本批 `postgres-supervisor` feature只新增既有 `tokio-postgres` direct edge，直接做到SCRAM ready。后续assembly再用borrowed connection material构造infra pool并调用Batch79；这样Windows process代码仍可独立编译，且schema/business依旧只在infra/application。

## 3. Version 与 data-dir前置

`verify_program_versions` 对 `postgres` / `initdb` / `pg_ctl` 逐个：

- program path已由Batch80全文件hash与execute-bit校验；
- `env_clear`，只设`LC_ALL=C`/`LANG=C`，current-dir为bundle root；
- `--version` command `kill_on_drop=true`，每个5秒；
- stdout/stderr必须恰一边非空，总量≤4 KiB；
- exact前缀`<program> (PostgreSQL) 17.11`；只允许一个bounded printable括号vendor suffix（host fixture的`(Homebrew)`）；
- version失败发生在key-store read/write之前。

data-dir只接受：

- 完全空目录→Fresh；
- 非symlink普通`PG_VERSION`≤16 bytes且trim后恰`17`→Existing；
- 无PG_VERSION但非空、PG16/其它值、wrong parent/symlink/宽权限均fail-closed，不猜partial recovery。

## 4. Secret不进argv/env/file

Fresh `initdb`固定参数含 `--pwprompt`、host SCRAM、local reject、UTF8、no-locale。password不构造combined String：对同一`PostgresScramSecret::expose()`先后两次`write_all`，各补单个newline，再shutdown stdin。取stdin、两次write、shutdown与wait全部在同一个30秒timeout；任一步失败/timeout都kill+wait。

生产源码机械反向检查：无`PGPASSWORD`、无`--pwfile`、无`std::env::{var,...}`，有`stdin(Stdio::piped())`。真实fixture再扫描`postgresql.auto.conf`、`pg_hba.conf`、`postmaster.opts`，raw 64-byte secret命中0。

tokio-postgres ready与未来pool内部会持有driver-owned password副本，这是协议连接不可避免的依赖内存；本仓不额外创建`DatabaseConfig.password String`，connection material以borrowed redacted view暴露。

## 5. Closed runtime config 与ready

每次启动重写私有 `postgresql.auto.conf` 与 `pg_hba.conf`，existing symlink/非普通文件拒绝，Unix mode=0600、file+directory fsync：

- `listen_addresses='127.0.0.1'`，随机probe port；
- `ssl=off`（同机loopback，跨网能力不存在）；
- `password_encryption='scram-sha-256'`；
- max connections 32、logging collector off；
- Unix socket disabled、POSIX dynamic shared memory；
- Unix local reject；IPv4与IPv6 host规则全部SCRAM（只监听IPv4）。

port由先bind `127.0.0.1:0`取得再释放，存在不可消除的TOCTOU；冲突时postgres early-exit/ready失败并安全清理，不自动改远端地址、hostname或放宽listener。本批不做retry。

ready循环每100ms先检查owned Child是否已退出，再以2秒内tokio-postgres NoTls连接numeric loopback/`postgres`库，使用Batch81 secret执行1秒`SELECT 1::int4`。只有值=1才返回 `RunningPostgresSidecar`。

## 6. Shutdown / Drop

clean：verified `pg_ctl -D … -m fast -w -t 5 stop`，command deadline6秒；随后另在5秒内wait exact Child，exit success后drop lock。pg_ctl失败、timeout、child异常时best-effort kill并保留stale lock。

unclean `Drop`不能async wait：先 `preserve_on_drop()`，再 `start_kill()`；Child自身`kill_on_drop=true`。真实测试等待port拒绝后确认stale lock存在，第四次start得到`postgres_sidecar_start_lock_held`；只在确认进程已死的测试repair里删除exact lock。

## 7. 真实 PostgreSQL 17.11 fixture

ignored host test将本机Homebrew 17.11的三个program复制到唯一测试bundle，按Batch80 manifest全文件hash后运行上述production state machine：

- first Fresh：三version→stdin initdb→direct postgres→SCRAM ready；
- 真查询：server major17、canonical data-dir、listen 127、password mode SCRAM、全部host HBA SCRAM；
- config/postmaster opts raw secret canary=0；
- verified pg_ctl fast shutdown，lock删除；
- Existing restart：同一MemorySecretStore值、再次ready/clean shutdown；
- 第三次Existing后直接Drop：port关闭、stale lock存在；第四次start held；确认死后repair；
- bundle/app/data/lock临时目录最终0。

该fixture只证明state machine：复制的Homebrew program仍链接/读取Homebrew外部dylib/share，manifest中的source/signing字段是测试contract。它不证明可发布bundle的relocatable依赖、codesign或source reproducibility。

## 8. Fault matrix

- wrong/不可执行version fixture：`VersionMismatch`，secret writes=0，lock删除；
- version通过但initdb exit17：`InitdbFailed`，secret已持久1（供安全重试），process0，lock删除，data仍空；
- scripted initdb必须从stdin读两次相同secret并写PG_VERSION，随后postgres exit17：`ExitedBeforeReady`，secret1，confirmed-dead lock删除；
- empty/partial/PG16/PG17状态、wrong direct child、Unix权限与closed config均有纯测试；
- start-lock stale preserve与replacement不误删证据保持。

## 9. 首跑失败与修正

- 首编译暴露 `Drop` 同时借child与self lock的双mutable borrow；先preserve再借child；
- sandbox真实fixture在`initdb`以稳定`InitdbFailed`停止，不计通过；宿主同命令继续；
- 宿主首次已到SCRAM ready，但测试发现IPv6 host rule写`reject`使Batch79“all host HBA SCRAM”判据为false；改为SCRAM（listen仍仅IPv4）后通过；
- macOS Clippy首跑指出closed-file type check可collapse；修后绿；
- Windows Clippy首跑指出Unix-onlysettings append导致target `unused_mut`；改cfg immutable shadow后绿；
- version timeout复核后补`kill_on_drop=true`；initdb deadline从只包wait扩为完整stdin write+wait；均在最终真纵向重跑后绿；
- parity第一次调用因Batch81 merge后已清理`target-xtask`而exit127；重建后正式parity绿；
- 多次fmt check只指出机械换行，最终rustfmt/check通过。

上述失败均不计成功；最终证据如下。

## 10. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| Desktop完整all-feature | `119/0/1`；doc-test`0/0/1 ignored` |
| sidecar bundle/lock/key-store/supervisor子集 | `10/0/1` |
| host PG17.11 supervisor纵向 | sandbox InitdbFailed不计；宿主最终`1/0/0` |
| macOS Clippy | Desktop all-target/all-feature `-D warnings`通过 |
| Windows | Desktop+唯一unsafe crate all-target/all-feature target Clippy通过；runtime未跑 |
| Linux | postgres-supervisor feature-only compile通过；既有budget/transport dead-code2warning，不冒充Clippy/runtime |
| dependency guards | key-store guard、Tauri guard、六target cargo-deny全部通过 |
| Cargo | package`825→825`，仅给Desktop加既有tokio-postgres direct edge |
| parity | `813/881/1694`、0 violation、required revalidate=0；fixtures`17/22/39`、overlay`1444/242/2/6` |
| strict recount | clean pinned upstream `891df72f…`，`159/0/0` |
| Grok | Git tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`，diff0；inventory2,110 files |
| package/npm | 非Grok恰一个`package.json`；npm=0 |

本批无API/UI/T-ID/CSS/locale变化，没有重跑Trunk、Browser、Engine/golden。没有运行 `cargo xtask ci`，没有派发GitHub Actions。

## 11. 未闭合边界

- macOS/Windows release PostgreSQL binary尚未reproducibly build/sign/manifest，host fixture非发行证据；
- Windows process与Credential Manager真机未跑；Linux Secret Service未实现；
- reserve-port有fail-closed TOCTOU且无retry；
- crash residue只fail-closed，production repair/orphan identity流程未实现；
- Batch79 shared schema/canonical principal/package sync尚未消费 `PostgresSidecarConnection`；
- 没有创建`openbot`业务库，也没有ApplicationService；ready只证`postgres`库TCP SCRAM；
- backup/restore/upgrade/release epoch assembly未实现；
- 真实Tauri `app_data_dir()` setup/window仍未接；
- 不关闭Desktop全Vault/KMS、G2/G6整关，不修改grok-bot，不新增Grok产品能力/npm。

下一批应把`RunningPostgresSidecar` connection通过无String长期副本的infra pool构造器接Batch79 bootstrap，再以真实Tauri path resolver执行“authority→bundle/lock/secret/process ready→schema/principal/package→verified window”的setup；window必须最后创建。
