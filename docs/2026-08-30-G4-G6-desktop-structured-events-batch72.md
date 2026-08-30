# Batch72：Desktop structured-event host bridge

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G6-desktop-structured-events`
>
> base：`7e101de1db64544fc2a53afc94f5d4db933bfe22`（PR #54 merge commit）
>
> implementation：`c055bcbf749d8d0a37cc6c42cb03d8b2a61bab88`
>
> 第一真源：v4 §3.1 条4–5、§5.1–§5.3、§13.1–§13.4、§15.1–§15.3、§21.1、§24 G4/G6、§28.1 R145/R146。

## 1. 结论

本批完成 `DesktopSession` 到真实 `tauri::ipc::Channel` 的 **host bridge primitive**：

- application stream 自然结束不再静默 EOF，而是先发 `upstream_ended` terminal；
- host 关闭单个 subscription 时同步摘除 exact broker route，并发 `subscription_closed` terminal；
- 同一实际 native window 可持有多个 host-minted subscription；renderer 不能选择内部 label、subscription ID、actor、tenant 或 auth generation；
- wire frame 只投影 closed stream、sequence、gap、typed `AppEvent` 与 closed terminal；
- sequence、stream family 与 exact thread 在 IPC 前检查；错误 family/thread 不到 renderer；
- `DesktopTauriProtocol` 只使用已绑定的 `AuthContext`，window unbind 关闭该实际 window 的全部已登记 stream；
- window 在 `subscribe().await` 期间关闭并以同 label 重建时，checked binding nonce 的 await 后复核拒绝旧 binding；
- Tauri sink 消失立即结束 pump 并取消 subscription；Screen 仍不进入该桥。

本批没有把 custom-protocol `Response<Vec<u8>>` 冒充 SSE/WebSocket，也没有把 Tauri event 用作 Screen 通道。

## 2. 根因与裁决

R145 正确地让 Desktop events/ws 保持 404，但留下三条必须先闭合的 transport 缺口：

1. `DesktopSession` 文档规定 receiver 只在 terminal 后结束，旧 pump 在上游自然 EOF 时却直接退出并 drop sender；
2. 一个实际 window 同时需要 thread、channel activity、approval 等 closed stream，直接复用真实 window label 会被 broker 的 duplicate-label 边界拒绝；
3. 只在 `subscribe().await` 前读取 authority 无法防止同 label window 在 await 期间被销毁、重建。

据此落 R146：

- `EventBroker::close_window` 是 exact、idempotent、同步 route removal；
- `InProcessTransport` 为每个 session 保存独立 cancellation，主动关闭和自然 EOF 都先形成 terminal；
- bridge 用 checked `u64` subscription counter 与 length-prefixed internal label，counter exhaustion fail-closed；
- `DesktopTauriProtocol` 的 window binding 另有 checked nonce，await 后必须仍是同一 binding；
- wire 不重新引入任何 authority 字段，且在 Tauri Channel 前执行 sequence/gap 与 closed stream/thread 校验。

## 3. 实现面

### 3.1 Broker 与 transport

- 新增 `DisconnectReason::{UpstreamEnded, SubscriptionClosed}` 与稳定低基数标签；
- 新增 exact `close_window`，先 flush latest-value carrier，再投 terminal、合并 shed/disconnect metric并摘 route；
- `PumpHandle` 保存 label、per-session cancellation 与 task；`close_session` 同步关闭 route，并取消所有同 label 的残留 pump handle；
- upstream `None` 显式关闭为 `UpstreamEnded`，全局 shutdown 仍由原有 `close_all` 统一发 `Shutdown`。

### 3.2 Closed structured wire

`DesktopStructuredEventFrame` 只有两类：

- `event`：`subscriptionId`、closed `stream`、`sequence`、可选 `skipped`、typed `AppEvent`；
- `terminal`：同一身份/序号/gap，加 closed reason 与仅 queue overflow 时存在的 delivery class。

投影中没有 window、actor、tenant、role、auth generation 或任意自由错误文本。非 queue-overflow terminal 机械省略 `overflowClass`。

### 3.3 Host authority 与 lifecycle

- `DesktopStructuredEventBridge` 只接受 host 已取得的 `AuthContext`，铸造内部 route 与 subscription ID；
- actual window label 只留在 host registry，不发给 renderer；
- `DesktopTauriProtocol::open_structured_subscription` 从既有 window authority registry 取身份；
- `pump_structured_events` 把 typed subscription 直接抽入真实 Tauri Channel；
- `unbind_window` 先移除 authority，再同步关闭该 actual window 的全部登记 route；
- checked window binding nonce 防止旧 async open 附着到新同名 window。

## 4. 测试矩阵

新增证据覆盖：

- broker exact close：只终止目标 route、另一 window 继续收帧、重复 close 为 false；
- transport 自然 EOF：event → `upstream_ended` terminal → `None`；
- transport 主动 close：exact route 立即为 0、`subscription_closed` terminal、pump 在 deadline 内 join；
- 同一实际 window 的 thread + channel activity 两个真实 Tauri Channel 各自得到 event + terminal；
- drop 单 subscription 只关自身 route，window close 一次关闭它的全部 subscription且不影响另一 window；
- wrong stream family 与 wrong thread ID 在 IPC projection 前返回 integrity violation；
- Tauri callback sink error 后 active registry/broker route 均为 0；
- subscription counter 与 window binding counter 到顶均 fail-closed且零 authority/route；
- unbound window 不能开 stream；bound window unbind 可唤醒 pending Channel pump；
- subscribe await 中 unbind + 同 label rebind 后，旧 open 返回 `desktop_window_unbound`，零残留 route。

## 5. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| `cargo fmt --all` + `git diff --check` | 通过 |
| `cargo test -p openbot-desktop --locked` | `80/0/0`；doc-test 1 ignored 显式可见 |
| `cargo test -p openbot-desktop --all-features --locked` | `98/0/0`；doc-test 1 ignored 显式可见 |
| Desktop Clippy | all-target/all-feature，`-D warnings` 通过 |
| macOS host check | all-target/all-feature 通过 |
| Windows target | `x86_64-pc-windows-msvc` all-target/all-feature check 通过；compile-only，不冒充 runtime |
| Tauri dependency guard | Linux host graph absent；13 build scripts；9 WebView2 payload；既有 MPL/UNIC/Vet blockers 不变 |
| parity | `813/881/1694`，0 violation；fixtures `17/22/39`；overlay `1445/241/2/6` |
| strict recount | 仓外 clean upstream=`891df72f…`；`159/0/0`，skip 0 |
| Grok | git tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`，diff 0；inventory check 2,110 files |
| invariants | Cargo.lock/workflow diff 0；非 Grok 恰一个 `package.json`；本批 package/lock/npm 文件 diff 0 |

第一次未设置 `OPENBOT_UPSTREAM_DIR` 的 recount 如实得到 `71/0/88`，没有记成 strict 通过；随后先核仓外 clone clean 且 HEAD 精确为 `891df72f1827454d8b353d108fe5dd2313b7e30d`，再以 `--require-upstream` 重跑得到 `159/0/0`。

本批没有 UI/CSS/locale/Cargo dependency 变化，因此没有重跑 Trunk bundle、Browser、Engine fetch/verify 或 golden；不引用 Batch70 的 bundle 作为本批新证据。没有运行 `cargo xtask ci`，没有派发 GitHub Actions。

## 6. 台账与未闭合边界

- 无新增或关闭 T-ID；parity、fixtures 与 overlay 计数不变；
- `/api/channels/events`、thread `/events` 与 `/ws` 在 custom protocol 继续 404；WASM 尚未实现 Desktop host selection，不能因 host API 存在就改成 done；
- 本批没有注册实际 `#[tauri::command]`，没有可发布 Tauri binary/tauri.conf/capability/window assembly，也没有真实 WebView window；
- 当前 bridge 为每个 host subscription 建内部 broker route；正式 native assembly 仍须证明一个 Webview 的并发订阅与 aggregate queue 满足 §13.2 的“每窗口 256”预算，必要时先收敛为窗口级 multiplex，再做 journey；
- Screen loopback binary WebSocket、viewer ticket、正式 Desktop sandbox renderer均未实现；
- Windows 只完成 cross compile，没有 WebView2 runtime、window close、multi-window 或 golden 证据；
- T-UI-0126 与 G4/G6 整关不勾。

下一批若继续 Desktop，应先把 command registration、WASM host transport selection 与 native window lifecycle 接到本桥，并用真实窗口证明 unbind/reconnect/multi-stream；在这之前不得把 host unit test称为 Desktop realtime journey。
