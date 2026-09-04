# Batch109：Engine protocol v2 与真实双 role CDP 输入

日期：2026-09-04

implementation：`71e7feb7005100accf190e16b4bb3f4c79209149`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4
§10.3、§11.1–§11.3、§12.2、§12.5–§12.6、§19.1 P1/P2、§24 G5/G7、§28.1 R184

固定产品上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`

## 1. 结论与边界

Batch109把Batch108的pure `CdpInputPlan`接入真实Rust-owned `EngineProcess`和clean-room Electron shim。
这是不兼容wire扩展，因此没有在protocol 1中偷加字段，而是把descriptor、generated module、bundle
manifest与runtime handshake同步升级为protocol 2 / release epoch 2。

macOS arm64上，`BrowserComputer`和`SandboxedComponent`两个独立Seatbelt Electron role都经fresh
HumanLease receipt执行了mouse move/down/up、wheel、keyDown/rawKeyDown/keyUp与insertText。固定HTML/CSS
surface使hover、active、文本、Backspace与scroll产生可观察JPEG SHA变化；F1按固定上游以virtual code 0
成功。T-BROP-0037–0043据此done。

仍未闭合：

- `SecretInsert`只证明普通input wire在写pipe前拒绝，独立typed secret effect未实现，T-BROP-0044保持todo；
- input ACK与frame channel已解耦，但当前conformance仍在每次accepted input后用
  `Page.captureScreenshot`产证；正式`Page.startScreencast`、ACK/backpressure/ScreenHub/viewer ticket未实现；
- `BrowserRuntimeManager→EngineProcess`的Server/Desktop production assembly仍缺；
- Windows只完成target check/Clippy，未运行protocol-v2 bundle/Named Pipe/renderer；Linux runsc/Xvfb未运行；
- P1入口仍红，所以本批只能算G5A预入场证据，不能宣称P2已进入、G5/G7通过或Computer产品面完成。

## 2. 版本化wire与authority

`engine-protocol-v2.json`固定：

- commands=`start/input/stop/shutdown`；events新增`input_applied`；
- input kinds恰为`mouse_move/mouse_down/mouse_up/wheel/key_down/raw_key_down/key_up/insert_text`；
- control frame仍≤64 KiB，超限在Rust pipe write前返回`engine_control_frame_too_large`；
- wire不含自由CDP method、actor、role、policy、intent、lease epoch或secret value。

`ControlService::authorize_human_input_receipt`只在fresh actor/AuthGeneration/computer/tab/generation/epoch/
expiry全匹配时铸造字段私有、不可Clone、不可serde的单次receipt。`EngineProcess::apply_human_input`消费它，
再核process computer/generation/active tab和执行时`now < expires_at`；cross-scope与expiry exact边界均在
operation ID和pipe write前拒绝。普通`SecretInsert`在pure plan处拒绝，同样不产生operation/frame。

input command只等待exact operation/tab/input-kind ACK；`next_frame()`独立读取绑定role/computer/
generation/tab且sequence单调的binary frame。这避免把当前conformance capture耦合成最终Screen API，后续可
换成`Page.startScreencast`而不改变authority入口。

## 3. shim与CDP闭集

shim仍恰3文件、唯一非Grok `package.json`、零npm，非空LOC=`483/600`。`electron-shim-check`新增机械
规则：所有`sendCommand`首参必须是literal，集合必须恰为：

- `Page.enable`、`Runtime.evaluate`、`Page.captureScreenshot`；
- `Input.dispatchMouseEvent`、`Input.dispatchKeyEvent`、`Input.insertText`。

动态method或`Network.enable`等额外CDP均判红。input payload逐variant exact-key校验；mouse/button/click/
finite/modifier、wheel delta、key/code/text、u32 virtual code与native=windows全部在shim二次收紧。错误只回
stable code，command结构和用户文本不进Debug/console。

debugger在session成功后保持attach，stop/shutdown/error/fatal统一先detach再destroy；renderer与主进程仍
零TCP listener，退出后全部PID与profile lock为0。

## 4. bundle与真实证据

官方macOS arm64 Electron 43.3.0重新下载并校验：

- archive=`122102881 B`；SHA-256=`ee939d1564d83d61032b3b3cb23af4e46005a4900c91f0695f7ed793f0ce6e83`；
- `--version=v43.3.0`；
- protocol-v2 bundle `app.asar=22819 B`；ASAR header SHA-256=
  `cb289d04fc42ba59622590c0f699933d3b55c4361a39ae92670a056c105a5063`；
- fuse wire=`000011001`，ad-hoc signature、rebrand、embedded integrity与manifest verify全绿；
- generated protocol module SHA-256=
  `ef213bb4d8f9f66b0854ef4feb9a7718de5bae139348320ed3fe8f6641b9bdf6`。

`fixtures/computer/engine-input-wire-v2.json`=`1764 B`，SHA-256=
`f8529f5950f0482e2cb3388899bcab86143dac50475499fe3bcf20808d8a9810`；其中Windows/runsc、production
assembly、secret typed effect、ScreenHub与Page.startScreencast均固定false。

## 5. 实跑矩阵

| 检查 | 结果 |
|---|---|
| `cargo test -p openbot-contracts --all-features --locked` | `104/0/0` |
| `cargo test -p openbot-computer --all-features --locked` | lib=`56/0/0`，fixture=`1/0/0`，host=`0/0/2 ignored` |
| `cargo test ... engine_conformance ... --include-ignored --test-threads=1` | `3/0/0`；其中真实role=`2/0/0` |
| `cargo test -p openbot-testkit --bin xtask --features xtask --locked` | `103/0/0` |
| Contracts+Computer、testkit all-target/all-feature Clippy | 通过 |
| Windows `cargo check`与Computer all-target/all-feature Clippy | 通过；runtime未跑 |
| `cargo xtask engine protocol --check` | version=2、generated hash match |
| `cargo xtask electron-shim-check` | 3 files、483/600 LOC、literal CDP allowlist、hash match |
| `cargo xtask engine bundle` / `verify` | protocol/epoch2、ASAR/fuses/integrity/signature/manifest通过 |
| `cargo xtask parity-check --json` | parity=`868/842/1710`，browser=`14/36/50`，fixtures=`28/22/50`，overlay=`1276/426/2/6`，0 violation/warning |
| `cargo xtask recount` | `71/0/89 skipped`；无固定上游目录，strict未跑 |

首次普通沙箱下载因DNS失败，获准网络后复用同一钉版URL成功；首次protocol-v2 Computer单测因两份旧测试
帧硬编码version1而`52/2`，改读`ENGINE_PROTOCOL_VERSION`后完整重跑通过。上述失败均未计通过。

本批无schema/native/API/route/UI/bundle-Web/dependency/Cargo.lock/env变化；Cargo package仍829，
`grok-bot`不变，未运行R63禁止的`cargo xtask ci`，未派发Actions。

## 6. 下一步

优先把独立frame面换成正式`Page.startScreencast`+frame ACK/背压，再接ScreenIngress/ScreenHub/ticket；同时
把`BrowserRuntimeManager`、Engine bundle与ControlService装入Server/Desktop production composition。
SecretInsert需独立target/document-generation command并在实际字段上证明填入、正文零日志/零frame。
Windows与runsc仍须各自真机重建protocol-v2 bundle并重跑相同matrix，不能由本次macOS证据外推。
