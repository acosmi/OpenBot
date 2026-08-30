# Batch74：WASM structured transport host selection

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G6-wasm-structured-transport`
>
> base：`91f5231a49d284ce2a2fefa98036b7ae5a2f7246`（PR #56 merge commit）
>
> implementation：`b6520820f71a3cc59b0d134da0d7f95be5e218ae`
>
> 第一真源：v4 §3.1 条4–5、§5.1–§5.3、§13.1–§13.4、§15.3、§21.1 条2、§24 G4/G6、§28.1 R146–R148；GUI v2 §15.1。

## 1. 结论

本批把 Batch72/73 的 Desktop structured-event host bridge 与 actual commands 接到同一份 release WASM：

- 只在 Tauri 2.11.5 注入的 `window.isTauri=true` 时选择 Desktop transport；普通 Web 的 WebSocket/EventSource 原样保留；
- 不引入 `@tauri-apps/api`、第二个 `package.json` 或任何 npm 依赖，也不开启宽泛的 global Tauri API；
- WASM 只读取 `__TAURI_INTERNALS__.invoke/transformCallback/unregisterCallback` 与 `__CHANNEL__` exact protocol；
- Tauri Channel 每帧发送 closed frame 的 JSON string，Rust/WASM 直接 serde，避免任意 `u64` 先经 JavaScript Number 丢精度；
- host subscription ID 在 JavaScript safe-integer 上界前 fail-closed；
- callback 按 Tauri `index` 在 257 项上限内重排，并要求 closed terminal 后出现 exact Channel end；
- open receipt 与首帧可任意先后，receipt 前 frame 也以 257 项为上限；
- receipt、stream family、sequence、gap、terminal/end 任一不一致均 fail-closed；
- invoke reject、local integrity failure、组件 Drop 发生在 receipt 前或后，最终都 unregister callback 并调用 host exact close；
- AppSidebar roster、Approval activity、Conversation thread events 三个 owner 已接入；generation 切换先 Drop 旧 connection，再建立新 connection。

本批关闭的是 WASM host selection、callback ordering/cleanup 与 exact close 代码闭环。它没有启动真实 Tauri binary/Webview，因此不关闭 native-window journey。

## 2. 根因与裁决

Batch73 已有可 invoke 的 open/close command，但 UI 仍无条件构造 Web realtime URL。Desktop custom scheme 没有合法 WebSocket/EventSource endpoint，继续使用 Web transport 只会永久重连不存在的 socket。不能按 URL scheme 猜 host：Windows WebView2 的 custom-scheme 映射可能呈 HTTP 形态；钉版 Tauri 的构造性标志是注入的 `window.isTauri`。

零 npm 约束又排除了 JavaScript package。为避免把整个 Tauri API 暴露给主 Webview，本批只实现钉版 Channel/invoke 所需的 exact internals。该选择被 R148 固化；如果 Tauri pin 升级导致注入协议变化，必须同批重验，不允许静默兼容猜测。

Tauri Channel 的大 payload 会先按 chunk/index 回调，最后再发 `{end,index}`。按 callback 到达顺序消费会制造假 sequence gap；只 unregister callback 也不会可靠终止 host pump。因此 adapter 必须同时拥有有界重排、terminal/end 判据与 host exact close。

## 3. 实现面

### 3.1 Shared wire 与 host 精度边界

`openbot-contracts::desktop` 增加只读 frame getters 与 `DESKTOP_STRUCTURED_SUBSCRIPTION_ID_EXCLUSIVE_LIMIT`。host counter 到达该排他上界时拒绝继续铸 ID。Desktop 的 `Channel<String>` 发送原始 frame JSON；测试覆盖 frame 内大于 `2^53` 的业务 `u64` 仍逐字保留。

### 3.2 WASM adapter

`openbot-ui/src/desktop_transport.rs` 只由 `api` 模块私有拥有，避免把整个 `openbot_ui` root 变成 overlay 粗前缀。纯 Rust `OrderedChannel<T>` 与 `WireSequenceTracker` 分别验证：

- duplicate/stale/pressure/end-before-gap；
- first sequence、checked increment、reported gap、terminal-last；
- pending receipt 前的 bounded frame queue；
- frame JSON 1 MiB 上限；
- exact subscription ID、stream family 与 thread scope。

WASM invoke 的 resolve/reject closure 转交 JavaScript GC；connection 的 `finished` 只在 terminal+end、显式失败或 Drop 时完成。Drop-before-receipt 由 settlement callback 在 receipt 到达后补 exact close，避免 race 泄漏。

### 3.3 三个 realtime owner

- AppSidebar：Tauri 使用 `ChannelActivity`，Web 保持原 WebSocket；
- Approval：Tauri 使用 `ToolApprovalActivity`，Web 保持原 WebSocket；
- Conversation：Tauri 使用 `ThreadEvents`，Web 保持原 EventSource。

三者继续执行既有 hint→权威 refetch、backoff 与 generation fencing；没有把 IPC event 当成权威状态。

## 4. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| `cargo fmt --all` + `git diff --check` | 通过 |
| Contracts tests | `99/0/0` |
| Desktop all-feature tests | `102/0/0`；doc-test 1 ignored 显式可见 |
| UI all-feature tests | `175/0/0` |
| WASM | Contracts/UI `wasm32-unknown-unknown` check 通过 |
| Clippy | Contracts/Desktop/UI all-target/all-feature，`-D warnings` 通过 |
| macOS host | Desktop all-target/all-feature check 通过 |
| Windows target | `x86_64-pc-windows-msvc` all-target/all-feature check 通过；compile-only，不冒充 runtime |
| Tauri dependency guard | Linux host graph absent；13 build scripts；9 WebView2 payload；既有 MPL/UNIC/Vet blockers 不变 |
| tools | clean 后首次 verify 因本地工具文件不存在而失败；随后 `tools fetch` 安装钉版 Tailwind 4.3.3、Trunk 0.21.14、wasm-opt 132、wasm-bindgen 0.2.127，最终 verify 通过 |
| release bundle | `NO_COLOR=true` 后 offline/locked A/B 各 8 文件逐字相同；首次 ambient `NO_COLOR=1` 在构建前被 Trunk 拒绝，未冒充通过 |
| bundle gates | i18n 782 leaf keys；design 104 Rust files/74 icons；CSS 361 classes；WASM gzip `1,849,013/3,670,016`、CSS `114,965/131,072`、fonts `740,216/819,200`、external/inline script `1/0` |
| WASM literals | open command、close command、`__TAURI_INTERNALS__`、`__CHANNEL__`、`isTauri` 各恰 1 |
| ordinary Web regression | release in-app Browser 三页均 `isTauri=undefined`：roster generation 2、approval card 1、thread transcript 7；main/nav/h1 唯一，visible alert/overflow/duplicate/log 均 0 |
| parity | 最终 `813/881/1694`，0 violation；fixtures `17/22/39`；overlay `1445/241/2/6` |
| strict recount | 更新后首次因`OPENBOT_UPSTREAM_DIR`未恢复而得到`71/0/88`并按`--require-upstream`失败；定位clean pinned upstream `891df72f…`后以显式路径重跑，最终`159/0/0` |
| Grok | tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`，diff 0；inventory 2,110 files |
| invariants | Cargo.lock/workflow diff 0；非 Grok 恰一个 `package.json`；本批 package/lock/npm diff 0 |

首次 parity 因 adapter 被声明为 crate-root module，使路径前缀粗化为 `openbot_ui`，机械报 134 个 done target 需要 revalidate。本批没有批量加 overlay 规避，而是把 adapter 改由 `api` 模块私有拥有；最终 parity 0 违反、`diff required revalidate=9`。

九条为 T-TEST-0166、T-TEST-0217、T-TEST-0218、T-TEST-0219、T-TEST-0240、T-TEST-0241、T-TEST-0242、T-TEST-0243、T-TEST-0247。它们分别由 API module ownership 与 Conversation 文件 diff 命中；full UI 175/0/0 已重跑。本轮没有重跑这些条目历史证据中的 PostgreSQL suite，故不冒充 PG 重跑。

没有运行 `cargo xtask ci`，没有派发 GitHub Actions。

## 5. 未闭合边界

- actual Tauri binary/tauri.conf/capability、verified session → `bind_window`、window destroyed → `unbind_window` assembly 仍不存在；
- 尚无真实 native Webview 中的 open→frame→terminal/end→close journey，ordinary Web 浏览器回归不能替代该证据；
- 当前每个 host subscription 仍有独立 internal broker route；单 Webview aggregate queue/并发上限尚未证明满足 §13.2 的 256 预算；
- Windows 只有 cross compile，没有真实 WebView2 command/channel/window runtime；
- Screen 仍走独立 binary plane，不进入 structured-event bridge；
- formal Desktop golden、T-UI-0126 与完整 G4/G6 仍 todo；
- 无新增/关闭 T-ID，无视觉/CSS/locale语义变化。

下一批必须先按 v4 §24 机械核对 native-window assembly 与单 Webview 256 预算的依赖顺序；在真实 window/session lifecycle 证据前，不得把本批写成 Desktop journey 已完成。
