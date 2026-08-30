# Batch70：Agent callback token 与正式 Web lifecycle journey

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G4-G6-agent-callback-panel`
>
> base：`f38ea98cd91c470b3f106f5598d0f25f734d1e3c`（PR #52 merge commit）
>
> implementation：`2311aa8a62ecc77e8ce0a11495b4e372dced3e32`
>
> 第一真源：v4 §3.1条3、§5.1–§5.3、§6.4、§13.1–§13.3、§15.1–§15.3、§21.1、§24 G4/G6、§28.1 R142–R144；GUI形态与128 KiB CSS预算服从v2 §6.2、§10.5。

## 1. 结论

本批闭合了固定上游 Agent callback-token panel 的 Server/Desktop/Web 交接，并补齐此前被 Browser URL policy
阻断的 `/agents` 正式 Web lifecycle journey：

- Desktop custom protocol 新增 `POST/DELETE /api/agents/{id}/callback-token`；
- 同一 Leptos bundle 新增一次明文 callback panel、typed API、中英文案与 token-only CSS；
- testkit fixture 注入同一 `AgentCallbackTokenAdministration` typed port，使 release Browser 不走 503 捷径；
- release Browser 实跑 create/edit/duplicate/hide/recover/delete/start、connection probe、write-only
  Authorization 与 callback issue/rotate/revoke；
- Browser 发现并修复 Agent 空提交后，字段已纠正但总警报不消失的响应式缺陷；
- 关闭 T-ROUTE-0007、T-UI-0031–0034。

本批没有关闭 T-UI-0126 formal page golden，也没有把 Web Browser 证据外推成 Tauri native-window、
Windows runtime 或 Desktop channel create/run。P1 Windows/runsc 仍红，P2/P3/P4 未进入。

## 2. Callback authority、framing 与明文生命周期

### 2.1 Desktop

- callback route 只接受一个 percent-decoded Agent path segment；POST=Issue、DELETE=Revoke；
- window label 先取得 host-bound `AuthContext`，写请求在读取业务输入前检查 fresh；
- callback body 必须为空，unsupported method、non-empty body、坏 path 与 stale request 都覆零原始 `Vec<u8>`；
- 业务只经既有 `InProcessTransport -> ApplicationService`，不在 Desktop 新建 store/权限规则；
- issue 只接受 `AppReply::AgentCallbackToken` 并返回 201 typed DTO；revoke 只接受
  `AgentCallbackTokenRevoked` 并返回 204 空体；全部 no-store；
- Desktop 测试实得 stale secret 401、两次 issue token 不同、旧值不在第二响应、profile flag true、
  non-empty body 405、revoke 204/profile flag false，response secret canary 0。

### 2.2 Web

- API 固定 same-origin credentials、`RequestCache::NoStore`、redirect error；POST只认201 typed JSON，
  DELETE复用exact 204 empty mutation；Agent ID先验证再编码为一个segment；
- `CallbackTokenIssued` 不 Clone/Display、Debug redacted、drop zeroize；组件只在
  `endpoint.is_some() && can_manage` 时构造；
- 明文只进入 component-local `StoredValue<Option<CallbackTokenIssued>>`，不进入query cache；
- issue/rotate前先清旧值，成功后才显示；Dismiss、revoke、unmount、profile切换与navigation均清槽；
- 明文位可键盘聚焦、可选择、自动换行，不依赖宿主之间共享clipboard。

## 3. Browser 发现的表单缺陷

空表单提交后，三项必填字段分别正确得到 `aria-invalid=true`；但旧实现的总错误：

```text
attempted.get() && build().is_err()
```

只追踪 `attempted`，`build()` 对各字段使用 `get_untracked()`。因此后续输入已经合法、三个
`aria-invalid` 已归零、连接探针已成功时，总警报仍保持旧值。

修复后以独立 `Signal<bool>` 追踪全部字段，并用 `RwSignal::with` 借用字符串，避免每次验证额外复制
password。新增单测锁定：未attempt不报错 → 空提交报错 → 三字段合法后清除 → auth无endpoint再次报错 →
补endpoint再次清除。

最终 release Browser 实得空提交 `invalid/alert=3/4`，逐项纠正后 `0/0`。

## 4. 正式 release Browser journey

固定上游只读 clone 为 `891df72f1827454d8b353d108fe5dd2313b7e30d`，clean；只核页面/组件结构，
没有复制 Grok 文本或新增 Grok 产品能力。Browser 使用 release/offline/locked bundle 与仅监听
`127.0.0.1:39070` 的 `required-features=testkit` host。

### 4.1 Callback

- remote + manageable 面板恰1；managed built-in与unmanaged remote均0；
- 初始有credential时显示Rotate/Revoke；rotate两次值均为合法 `obot_agt_` 形态且逐字不同；
- Dismiss后code/明文DOM均0；明文显示时 `tabindex=0`、`user-select:text`、
  `overflow-wrap:anywhere`；
- 明文显示状态hard reload后code=0且旧值不重放，权威profile仍显示Rotate/Revoke；
- Revoke后只显示Generate，hard reload仍保持；重新Generate得到第三个不同值；
- 中英即时切换后标题、状态与动作全部本地化；alert/console error=0。

### 4.2 Agent lifecycle

- `/agents` 的mine/hidden/explore三组均取Server projection；create/detail由query拥有且hard reload稳定；
- connection probe得到`RUN_STARTED`；带测试Authorization创建后profile/roster均不回显明文；
- edit空auth路径成功，credential保留本身沿用R142真实PG证据，不拿fixture UI冒充；
- duplicate固定private/managed，endpoint/auth/callback均不复制；
- Hide把copy从mine移到hidden；hidden profile显示badge与Unhide；恢复后回mine；
- Delete先显示命名确认组，最终删除后visible/hidden均0；
- Start channel精确进入`/channel/new?agent=<id>`并选中该Agent；
- cancel前填测试password，重开后不重填创建的权威profile `hasAuth=false`；另一次显式重填创建得
  `hasAuth=true`，两次response/DOM明文均0。

### 4.3 布局与AX

| 视口 | 结果 |
| --- | --- |
| 1024×640 | body=viewport；horizontal overflow=0；main/nav/h1=1/1/1；callback=1；duplicate ID=0 |
| 600×900 | body=viewport；horizontal overflow=0；token code overflow=0；main/nav/h1=1/1/1；duplicate ID=0 |

600×900 截图只作本轮视觉QA。仓内 `fixtures/ui/golden/` 仍不存在，因此 T-UI-0126 保持 todo。

## 5. 本轮亲跑证据

### 5.1 Rust、WASM 与供应链

| 证据 | 最终结果 |
| --- | --- |
| Contracts / Application / Desktop | `95/153/83`，0 fail；Desktop callback矩阵在同一83条中 |
| UI / fixture | `169/9`，0 fail；新增一次明文槽、响应式表单、fixture authority用例 |
| Clippy | Contracts/Application/Desktop all-target/all-feature、UI all-target/all-feature、fixture bin，全部 `-D warnings` |
| UI WASM | `wasm32-unknown-unknown --locked` 绿 |
| Windows target | `x86_64-pc-windows-msvc` Desktop all-feature check绿；compile-only，不冒充runtime |
| Tauri guard | Linux host graph absent；13 build scripts；9 WebView2 payload；既有policy blockers原样保留 |
| fmt/diff | `cargo fmt --check`、`git diff --check` 绿 |

### 5.2 GUI、Engine 与机械台账

| 闸门 | 最终结果 |
| --- | --- |
| release UI | Trunk 0.21.14，`--release --offline --locked` 绿 |
| i18n / design / CSS | 782 leaf keys；103 Rust files/74 icons；361 class literals |
| bundle | wasm gzip `1,815,510/3,670,016`；CSS `114,965/131,072`；fonts `740,216/819,200`；scripts `1/0` |
| tools | Tailwind 4.3.3、Trunk 0.21.14、Binaryen 132、wasm-bindgen 0.2.127 verify绿 |
| Engine | Electron 43.3.0 sha `ee939d…`；ASAR 17,306 B；release epoch/protocol 1/1；host verify绿 |
| shim/Grok | shim 3 files、405/600 LOC；Grok inventory 2,110；tree `86f5a85f…`；零改动 |
| parity | `813/881/1694`，0 violation/0 warning；routes `24/8/32`；UI `91/61/152` |
| overlay | carry/revalidate/split/superseded=`1445/241/2/6`；新增本批5条revalidate |
| strict recount | fixed upstream `891df72f…`，`159/0/0` |
| package/npm | 非Grok恰一个engine shim `package.json`；零package lock；未运行npm |

## 6. 失败与环境边界（如实）

- 初次工具/Engine fetch在文件沙箱内DNS失败；获准宿主网络后按pin下载并verify绿；
- `engine bundle`成功后，沙箱内第一次`engine verify`得到`--version exit None`；同一bundle在宿主权限
  重跑得`v43.3.0`并通过SHA/ASAR/integrity校验；失败不计绿；
- 首次Trunk命令被继承的`NO_COLOR=1`拒绝，显式`NO_COLOR=true`后原命令绿；
- fixture首次在文件沙箱绑定loopback得到`Operation not permitted`，宿主仅监听127.0.0.1后绿；
- Browser backend不支持`networkidle`，改用documented `load`；编辑面一个offscreen locator出现CDP deadline，
  用同一Browser fresh visible DOM node完成语义输入与保存，不换浏览器表面；
- 中途磁盘只余125 MiB，UI编译真实报`No space left on device`。按用户授权精确删除已完成取证的
  `target/debug` 18 GiB与Windows target 320 MiB，空闲恢复18 GiB；失败命令从头重跑后才计绿；
- 未运行`cargo xtask ci`，未派发GitHub Actions（R63 manual-only）；未运行live vendor credential、
  Windows runtime、runsc/Xvfb或真实Tauri window。

## 7. 台账变化

- routes：`23/9 -> 24/8`，关闭T-ROUTE-0007；
- UI：`87/65 -> 91/61`，关闭T-UI-0031/0032/0033/0034；
- parity：`808/886 -> 813/881`，总数保持1694；
- overlay：5条carry转revalidate，`1450/236/2/6 -> 1445/241/2/6`；
- API/tests/fixtures无状态变化：`80/90/170`、`457/590/1047`、`17/22/39`；
- T-UI-0126 Agent page formal golden保持todo。

## 8. 明确未做与下一步

- Desktop Agent callback已有typed framing，但未在真实Tauri native window运行；
- Desktop `/channel/new` create/run custom protocol仍缺，Start channel只证明Web journey；
- Windows target compile不代表WebView2 runtime，P1 Windows与Ubuntu runsc/Xvfb仍无对应机器证据；
- AG-UI interrupt/resume、其余事件完整durable/UI projection、MCP private egress/admin完整面、
  browser/file/shell与协议级cancel仍缺；
- T-UI-0126及其余formal golden、Tauri binary/window lifecycle、AppSidebar/完整Composer与G6整关不勾。

下一批继续选择不依赖P1外部机器的独立G4/G6缺口；若补Desktop channel transport，仍只能关闭typed
framing，必须等真实native-window journey后才能声明完整Desktop Agent journey。
