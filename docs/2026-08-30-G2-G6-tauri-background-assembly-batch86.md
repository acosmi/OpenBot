# Batch86：G2/G6 Tauri background assembly 与 window-last shutdown

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G2-G6-tauri-background-assembly`
>
> base：`ab7ce33c53fca0dfa29b0ef675d7bd79f8e7916d`（PR #68 merge commit）
>
> implementation：`35ab92b12b7ae85ec8bce0c02366ea0a1c0c8b76`
>
> 第一真源：v4 §5.1–§5.2、§6.1、§6.4–§6.5、§7.1–§7.5、§8.6、§9.2、
> §13.1–§13.4、§14.1、§16.2–§16.4、§24 G2/G4/G6、§28.1 R150–R160。

## 1. 结论

Batch85 已把 PostgreSQL application adapter wiring 收成唯一 Infra composition root，但真实 Tauri
仍没有一个 owner 从 `AppHandle::path().app_data_dir()` 按序接 authority、sidecar、database、keys、
ApplicationService、Agent host 与 native window。直接在 `tauri.conf` 预建窗口会让 renderer 早于
数据库/密钥获得执行机会；只接 ApplicationService 不接 `RunRelay/BuiltInAgentRuntime`，又会产生
“UI 可提交 run、但 durable outbox 无 consumer”的假完成。

本批关闭以下前置：

1. 新增显式 `desktop-local-runtime` feature；最小 typed/bootstrap/Vault 图不变；
2. Tauri event loop 以零窗口启动，setup 唯一读取实际 `app_data_dir()`；
3. Tauri context 只允许 `create=false` window template，任何静态预建窗口在 `build` 前拒绝；
4. background worker 顺序固定为 authority/installation→verified PG sidecar→fixed `openbot` DB/
   migration/principal/package→OS-store application keys→policy→shared ApplicationService→
   BuiltInAgentRuntime/RunRelay→protocol slot→首窗；
5. pending custom protocol 固定 503，protocol 只允许单次 install，不能用旧 owner 替换；
6. Desktop UI preferences 继续使用同一 app-data 下的 closed 原子文件；Server 继续显式注入
   PostgreSQL preference adapter，Batch85 的 shared assembly 不再硬编码任一 host；
7. Desktop package provider只接受 HTTPS base URL、exact numeric CIDR、stored Vault credential，零
   env key/HTTP fallback；remote AG-UI 使用同一 SafeDialer；Server-only managed provider在首窗前拒绝；
8. 最后一个真实 `Destroyed` 后顺序固定为 authority→transport→RunRelay/Agent→MCP reconciler/
   adapters→data-plane/sidecar；shutdown 完成前 `ExitRequested` 被阻止，完成后才允许退出。

## 2. Tauri 延迟就绪边界

custom scheme 与 command handler 必须在 Builder 阶段登记，但 app-data authority 只有 setup 阶段
可取得。`DesktopTauriProtocolSlot` 因此只有三态：pending、ready once、replacement rejected。首窗
创建前 renderer 不存在；即便内部 scheme 被提前请求，也只返回 503，不能触发未绑定业务调用。

`DesktopLocalTauriBuilder` 隐藏原 Builder，只暴露带 exit fence 的 `run(Context<Wry>)`，调用方不能
绕开本批 `ExitRequested` 处理。外部 context 如含任意 `window.create=true`，在 Tauri 自己先建窗前
直接返回稳定配置错误；正式 capability/identity 仍由后续 reviewed release 提供，本批不伪造。

## 3. Application、偏好与 Agent host

共享 `PostgresApplicationAssemblyInput` 新增 host-owned `UiPreferenceAdministration`：Server 传
`PostgresUiPreferenceAdministration`，Desktop 传 `DesktopUiPreferenceStore(app_data/ui-preferences-v1)`。
其余 People/Policy/Thread/Memory/Tool/MCP/Drive/Components adapters 仍只有一个 constructor owner。

Desktop Agent host 使用现有 `openbot-agent` 与 Infra primitives，不读 Server env：

- package OpenAI-compatible provider固定 Responses；channel routing固定 Chat Completions；
- base URL仅 HTTPS且无userinfo/query/fragment；response 64 MiB、connect 30s；
- output tokens 与 Server 同为 `1..=1,000,000` closed range，deadline/stall 由 typed config给出；
- active credential每 run 从 PostgreSQL + Desktop Vault读取，环境 fallback 构造性不存在；
- `AuthorizedAgentToolGateway`、Postgres authorization/sequence/audit/context、MCP/components与
  remote assertion全部复用现有生产实现；
- package若要求 managed provider，Desktop 没有合法managed credential source，首窗前明确失败；
- `RunRelay` 的 dedicated LISTEN 也消费同一脱敏 `ThreadListenerDatabase`，不重建password String。

## 4. 真实 PostgreSQL 纵向

宿主把 Homebrew PostgreSQL 17.11 三程序复制进 test manifest；这只证明 process/runtime composition，
不是 release binary build/sign。测试从私有临时 app-data 开始，依次完成：

- CSPRNG instance authority与private PG data-dir；
- verified version/initdb/SCRAM ready、fixed `openbot`、正式baseline/native、canonical user/admin与
  Tenant Package membership；
- 同一 keyed memory OS store分别写 PostgreSQL SCRAM 与 application master，写次数精确为 2；
- shared ApplicationService `GetCurrentUser`成功；
- `UpdateUiPreferences(theme=light)`写入 app-data closed file；
- mint native thread→BeginRun，真实 RunRelay/BuiltInAgentRuntime消费dispatch；由于测试没有模型
  credential，run稳定到达`failed/provider_authentication`，证明不是空 relay且没有网络fallback；
- 显式 shutdown 后 start lock消失，所有临时目录删除。

最终 `1 passed / 0 failed`。

## 5. 首跑失败与修正

- 首版 background 只接 ApplicationService，复核发现 run outbox 无 consumer；补 Desktop Agent host+
  RunRelay并把其停机置于MCP/sidecar之前；
- 首版 shared assembly硬编码 PostgreSQL UI preferences，与Batch16 Desktop local-file真源冲突；改成
  host adapter输入并以真实 app-data 文件回归；
- 首版初始化查询误调用“设置exit requested”，会首窗前无条件停机；拆query/mutation并补相位测试；
- 完整测试沙箱首跑两条private Keychain报macOS `-50`、一条process分类偏移；宿主完整重跑最终绿；
- Infra沙箱15条、Server沙箱9条loopback fixture均因`EPERM`失败；宿主完整重跑分别全绿；
- run terminal首断言猜成`provider_unavailable`，源码与实值均证明缺credential应为
  `provider_authentication`，只修测试预期，不改生产分类；
- Windows full-runtime cross Clippy在`ring`构建脚本因macOS缺MSVC CRT `assert.h`失败，未到仓内代码；
  不计绿、不以最小图替代完整图。

## 6. 本轮亲跑证据

| 证据 | 最终结果 |
| --- | --- |
| format / diff / background guard | 绿；app-data/sidecar/shared-app/Agent+relay/window-last各1，ordered shutdown、SSO=0 |
| Desktop all-feature | `130 passed / 0 failed / 3 ignored`；另1 doctest ignored |
| Desktop real background PG | `1/0/0`；含local preferences、真实Agent consumer terminal与clean lock |
| Infra lib | 宿主 `323/0/0` |
| Server lib / bin | 宿主 `216/0/0` + `7/0/0` |
| shared assembly real PG | `1/0/0` |
| macOS Clippy | Desktop、Infra、Server all-target/all-feature `-D warnings`绿 |
| Windows | `tauri-host+desktop-vault`最小图与唯一unsafe crate Clippy绿；full-runtime cross在ring/MSVC CRT前置失败，未跑真机 |
| Linux tier-2 | `desktop-vault` compile绿，仅既有budget/transport dead-code 2 warning；full runtime未跑 |
| dependency guards | Tauri、PG/OS-store、application assembly、background assembly全绿；bootstrap TLS/SSO=0，full Desktop rustls且SSO/OpenSSL=0 |
| Cargo | package `825→825`；只新增既有`openbot-agent`/`url` Desktop direct edge |
| parity | `813/881/1694`、0 violation；overlay=`1289/397/2/6` |
| recount | 本仓 `71 passed / 0 mismatch`；未设上游目录，88 skipped；strict未跑 |
| Grok/package/npm | tree=`86f5a85f…`，inventory 2,110；非Grok `package.json`恰1；npm=0 |

本批无 API/UI/T-ID/CSS/locale/parity ledger 变化，没有重跑 Trunk、Browser、Engine、golden、Cargo
Vet 或完整 workspace。没有运行 `cargo xtask ci`，没有派发 GitHub Actions；R63 manual-only 不变。

## 7. 未闭合边界

- 没有正式 `tauri.conf.json`、capability allowlist、产品名/bundle ID/deep-link、签名或可发布binary；
- MockRuntime 与源码顺序守卫不等于真实 macOS/Windows Wry event-loop/window journey或formal golden/AX；
- release PostgreSQL binary仍未reproducibly build/sign/验证relocatable依赖；host fixture不是发行证据；
- Windows full-runtime/Keychain counterpart/进程行为未在Windows真机运行；Linux Secret Service未实现；
- Desktop Local installed-app OAuth、Desktop Remote session、managed provider source仍未实现；
- model credential缺失纵向只证明fail-closed consumer，不是live vendor trace或成功回复；
- tray/reopen、多窗口产品策略、backup/upgrade/recovery、真实window crash/restart仍未闭合；
- 不关闭G2/G4/G6整关，不修改`grok-bot/`，不新增Grok产品能力或npm。

下一批应在reviewed外部产品identity可得前，继续关闭不依赖品牌的剩余项；若进入正式Tauri发行，必须
同批提交exact `tauri.conf`/capability、无预建窗证明、macOS/Windows真实Wry journey与签名/安装证据，
不能把本批MockRuntime/host PostgreSQL证据外推为可发布Desktop。
