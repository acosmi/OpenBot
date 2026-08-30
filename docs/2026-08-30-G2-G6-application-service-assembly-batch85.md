# Batch85：G2/G6 ApplicationService 生产组装单源化

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G2-G6-application-service-assembly`
>
> base：`b2229b688a55fd08f53ebd535d250fad83211d47`（PR #67 merge commit）
>
> implementation：`5ba28673d6d03340095d74b2511d9438f5dcfb5f`
>
> 第一真源：v4 §5.1–§5.2、§6.1、§6.4、§8.6、§9.2、§13.1–§13.3、
> §16.2–§16.4、§24 G2/G6、§28.1 R157–R159。

## 1. 结论

Batch84 已把 Desktop 所需 credential Vault、audit、run assertion 与 MCP OAuth 材料收成持锁
data-plane 的 typed 输出，但全部 production PostgreSQL adapter 仍只在 Server `main.rs` 手工组装。
若 Tauri 再复制同一段 wiring，Server/Desktop 会拥有两个可漂移的业务入口；反向把环境解析、Axum
或 native window 搬进 Infra，又会破坏 §5.1–§5.2 的 transport 边界。

本批只关闭共享生产组装前置：

1. Infra 新增唯一 `assemble_postgres_application` composition root；
2. 输入全部是 host 已解析、已验证的 typed 值，函数不读环境、不起 listener、不创建 window；
3. 所有 PostgreSQL application adapters 与唯一 `Arc<dyn ApplicationService>` 只在该模块构造；
4. Server 删除重复 wiring，精确调用共享组装一次；
5. 输出显式保留 run runtime、MCP catalogue/connections/reconciler、component 与 callback lifecycle owner；
6. dedicated LISTEN 配置改为 Debug 脱敏的 `ThreadListenerDatabase`，Desktop 直接从持锁 sidecar 的
   borrowed SCRAM bytes 构造，不产生 password `String`，不重建 Server `DatabaseConfig`；
7. Desktop 本批仍未消费完整共享组装，也未创建真实 Tauri window。

## 2. 组装所有权

`openbot-infra::application_assembly` 负责且只负责 adapter wiring：People/Audit/Policy、Thread/Run、
Memory、Agent admin/directory/callback、tool control/journal/approval、MCP catalogue/credential/connection、
Drive、Channel routing、Components/Sandbox 与 UI preferences。host 必须显式提供 deployment、tenant、
policy snapshot、Vault/HMAC material、provider endpoint/egress 和 remote probe。

模块构造后只返回 typed application boundary 与必须跟随 host 生命周期保留的 adapter handles。Server
继续拥有配置解析、built-in Agent host、Axum router/listener 与 shutdown 顺序；未来 Tauri 继续拥有
app-data、sidecar、native window 与 background task。共享模块没有 `std::env`、Axum、Tauri、Webview、
`TcpListener` 或 `ServerBuilder`。

`tools/check-application-assembly.sh` 机械要求：

- production `OpenBotApplication::new` owner 恰为共享 Infra 模块；
- Server consumer 恰一次；
- typed application Arc 恰一次；
- shared assembly 零 process configuration/transport/window ownership；
- Desktop listener 只从 running sidecar 构造一次，且共享 listener 生产源码无 `String` secret storage。

## 3. Dedicated LISTEN secret 边界

原 Server `DatabaseConfig` 可从环境字符串形成，不能成为 Desktop 的共享凭据类型。新
`ThreadListenerDatabase` 只保存 `tokio_postgres::Config`，Debug 恒为
`ThreadListenerDatabase(<redacted>)`。Desktop constructor 固定：

- host=`127.0.0.1`；
- database=`openbot`；
- user=`desktop_admin`（contracts 单源）；
- port 非零；
- password 恰 64-byte lowercase hex；
- application name=`openbot-thread-events`；
- connect timeout=10s。

Server 仍可从既有 `DatabaseConfig` 转入同一 wrapper。只有 Infra `server-runtime` 能打开底层 config；
Desktop 的 `desktop-local` 最小 feature 只持 opaque hand-off value，不因此拉入 SAML/OpenSSL 图。

## 4. 真实 PostgreSQL 证据

自包含 PostgreSQL 17.11 临时集簇施加正式 baseline/native，写入 canonical user/admin role，加载
`PolicyStore`，再调用共享生产组装。通过返回的唯一 application 执行 `GetCurrentUser` 得到 typed
`CurrentUser`，随后显式停止 MCP reconciler。最终 `1/0/0`，临时集簇停机并删除。

另以 Batch83 的 Homebrew PostgreSQL sidecar 宿主纵向验证 running data-plane 可从 exact child 的
borrowed SCRAM bytes 构造 listener wrapper，Debug 不泄密，再完成 clean shutdown。最终 `1/0/0`。

## 5. 首跑失败与修正

- 沙箱内 `initdb` 因 SysV shared memory `EPERM` 失败，不计通过；同一命令获准在宿主临时目录重跑后绿；
- Desktop 宿主测试首编译发现 listener 类型误放在 `server-runtime` gated 的 thread module；没有让
  Desktop 拉整套 Server 图，而是拆成 `server-runtime|desktop-local` 共享的 opaque module，随后
  Desktop 全特性与宿主纵向均绿；
- 并行 Server tests 等待同一 Cargo build lock；保留原进程继续等待并取得两个成功终态，没有重启冒充。

以上失败均不计成功；下表只记最终实跑。

## 6. 本轮亲跑证据

| 证据 | 最终结果 |
| --- | --- |
| format / diff / assembly guard | 全绿；owner=1、Server consumer=1、Desktop listener=1、forbidden=0 |
| Infra lib | `323 passed / 0 failed` |
| Server lib / bin | `216/0/0` + `7/0/0` |
| Desktop all-feature | `123 passed / 0 failed / 2 ignored`；另 1 doctest ignored |
| 真实 PG shared assembly | `1/0/0` |
| 真实 Desktop sidecar/bootstrap/listener-config/shutdown | `1/0/0` |
| macOS Clippy | Desktop、Infra、Server all-target/all-feature `-D warnings` 全绿 |
| Windows target | Desktop + 唯一 unsafe crate all-target/all-feature Clippy 全绿；compile-only |
| Linux tier-2 | `desktop-vault` feature compile 绿；仅既有 budget/transport dead-code 2 warning |
| Cargo | lock package `825→825`；Cargo manifest/lock 零变化 |
| parity | `813/881/1694`、0 violation；overlay=`1289/397/2/6` |
| recount | 本仓 `71 passed / 0 mismatch`；未设 `OPENBOT_UPSTREAM_DIR`，上游 88 条诚实 skipped；strict 未跑 |
| Grok/package/npm | Git tree=`86f5a85f…`，inventory 2,110；非 Grok `package.json` 恰 1；npm=0 |

本批无 API/UI/T-ID/CSS/locale/parity ledger 变化，没有重跑 Trunk、Browser、Engine、golden、Cargo
Vet 或完整 workspace。没有运行 `cargo xtask ci`，没有派发 GitHub Actions；R63 manual-only 不变。

## 7. 未闭合边界

- Desktop 尚未把 Batch83 pool/auth、Batch84 key material 与本批 shared assembly 接成一个 background owner；
- 真实 `AppHandle::path().app_data_dir()`、Tauri setup、最后窗口退出、reconciler/runtime/sidecar 有序停机未接；
- built-in Agent host、Desktop Local installed-app OAuth 与 Desktop Remote session 尚未进入 native owner；
- release PostgreSQL binary build/sign/relocatable 依赖、Windows process/key-store 真机、Linux Secret
  Service、backup/upgrade、actual `tauri.conf.json`/capability/identity、真实 Wry/golden 仍未闭合；
- 不关闭 Desktop 全 Vault/KMS、G2/G6 整关，不修改 `grok-bot/`，不新增 Grok 产品能力或 npm。

下一批应让真实 Tauri background setup 从 `AppHandle::path().app_data_dir()` 开始，顺序持有
authority→verified sidecar/data-plane→application key material→shared application assembly，最后才创建
window；任何中段失败必须逆序清理，最后一个 window 退出必须停止所有 background owner 后再释锁。
