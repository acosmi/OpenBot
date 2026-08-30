# Batch75：单 Webview structured aggregate budget

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G6-webview-aggregate-budget`
>
> base：`6e14e420c7636e5ea08f8c465c6cfff0fbc4140e`（PR #57 merge commit）
>
> implementation：`24d9a5b1e051386b3ae4ed581e5054824da3a18f`
>
> 第一真源：v4 §13.1–§13.3、§15.3、§24 G6、§28.1 R146–R149；GUI v2 §15.1。

## 1. 结论

本批关闭 Batch72–74 明示保留的 Rust structured-event 聚合预算缺口：

- 同一 actual Webview 的全部 internal broker route 共享一个 256-slot queued event-ref budget，不再每条 subscription 各自获得 256；
- ordinary event 在进入 mpsc 前原子取得 permit；`DesktopSession::next_frame` 出队即释放；raw receiver 绕过 helper 时最迟随 frame drop 释放；send failure、receiver drop 与 frame clone 均不会泄漏 permit；
- terminal frame 不消耗 256 event permit，各 route 已有的 terminal-only reserve 保留，队列满仍能显式断开；
- 同一 Webview 的 live + pending structured subscription 总数同样上限 256；第 257 条在调用 `ApplicationService::subscribe` 前 fail-closed；
- pending open 在 await 前预留 slot，window close 先摘 aggregate 并取消全部 pending subscribe，再关闭 registered route；
- aggregate token 与 host binding cancellation 同时阻断旧 label open 附着到同名新 window；
- 不同 actual Webview 拥有独立 budget，一个窗口不能饿死另一个窗口。

本批关闭的是 §13.2 Rust queued event-ref 与 subscription task 乘法，不是完整 native queue saturation/window lifecycle gate。

## 2. 根因

Batch146 为允许同一 actual window 并行 roster、approval、thread stream，给每条 subscription 铸了 injective internal label。该做法正确解决了 duplicate label 与 cross-stream delivery，但每条 internal label 都调用原有 `EventBroker::open_window`，而后者会创建 `256 + terminal reserve` 的独立 mpsc。

因此三条普通产品流可同时持有 768 个 queued event ref；compromised renderer 还能直接重复 invoke open，继续乘上 task、mpsc 与 terminal reserve。严格 CSP 与 closed request 不能代替资源边界。

简单把 256 除以当前 stream 数不可行：开关 stream 会改变既有队列容量，并发 open 仍能同时通过检查；process-global 256 又会造成跨窗口饥饿。正确边界必须按 host-observed actual window 共享、在 await 与入队之前预留，并在所有退出路径可机械回收。

## 3. 实现面

### 3.1 Shared event-ref permit

`EventQueueBudget` 用 checked atomic CAS 管理 256 permit。普通 broker window 仍各自持有一个 budget；structured bridge 为同一 actual window 的全部 internal route 传入同一个 `Arc<EventQueueBudget>`。

`AppEventRef` 私有持有 permit guard，但 permit 不参与 frame equality/wire identity。成功出队经 `DesktopSession::next_frame` 立即释放；若调用方直接使用公开 receiver，permit 会更保守地保留到 frame drop。发送失败或 receiver 整体 drop 时，queued frame 的 guard 自动回收，不靠猜 receiver 长度。

aggregate budget 满时沿既有 delivery class 语义处理：critical/coalescable 显式 `queue_overflow` terminal，latest-value 仍保留有 gap 的最新值，screen 仍构造性拒绝。

### 3.2 Live/pending subscription bound

`WindowAggregate` 同时持有：

- actual-window generation token；
- shared event budget；
- window-close cancellation；
- pending 计数；
- host subscription ID → internal route 登记。

open 在调用 ApplicationService 前把 pending 加一；live + pending 达 256 时第 257 条直接返回 stable `structured_subscription_window_budget_exhausted`。RAII pending guard覆盖counter/application/transport错误；commit只接受同一个 aggregate token。

window close 移除整个 aggregate 后先 cancel pending，再 exact close registered routes。`WindowAuthority` 另持 binding cancellation，覆盖“已读到旧 authority、unbind 已完成、之后才开始 bridge open”的竞态；对 renderer 仍统一投影既有 `desktop_window_unbound`。

## 4. 压力与负向证明

- 同窗 ChannelActivity 与 ToolApprovalActivity 各生产 200 条 critical；不消费时两个 internal route 合计只入队 256，remaining capacity 恰 0；
- 第 257 条 aggregate critical 不能静默消失，至少一条 stream 收到 `queue_overflow` terminal；两流收到的 ordinary event 总和恰 256；
- 消费全部 ordinary frame 后 shared permit 恰回到 256；drop 两条 subscription 后 aggregate 不存在；
- 同窗前 256 条 live subscription 可开，第 257 条拒绝；另一 actual window 仍可独立打开且容量 256；
- 256 条并发 pending open 全部进入测试 gate，第 257 条在 ApplicationService 前拒绝；window close 不释放 gate也能在 deadline 内取消全部 256 条，最终 broker route/registry/aggregate 均为 0；
- 既有 stale binding、exact cross-window close、terminal/gap、latest-value 与metrics测试全部回归。

## 5. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` + `git diff --check` | 通过 |
| Desktop all-feature 首次 | `101/4/0`；暴露 permit 释放点与 stale-window error projection，不记为通过 |
| Desktop all-feature 最终 | `106/0/0`；doc-test 1 ignored 显式可见 |
| Clippy | Desktop all-target/all-feature，`-D warnings` 通过 |
| macOS host | all-feature unit与all-target Clippy通过 |
| Windows target | `x86_64-pc-windows-msvc` all-target/all-feature check 通过；compile-only，不冒充 runtime |
| Tauri dependency guard | Linux host graph absent；13 build scripts；9 WebView2 payload；既有 MPL/UNIC/Vet blockers 不变 |
| parity | `813/881/1694`，0 violation；fixtures `17/22/39`；overlay `1445/241/2/6`；diff required revalidate=0 |
| strict recount | clean pinned upstream `891df72f…`，`159/0/0` |
| Grok | tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`，diff 0；inventory 2,110 files |
| invariants | Cargo.lock/workflow diff 0；非 Grok 恰一个 `package.json`；本批 package/lock/npm diff 0 |

本批无 contracts/UI/T-ID/CSS/locale/Cargo dependency 变化，因此没有重跑 Trunk、Browser、Engine 或 golden；不把 Batch74 的普通 Web 浏览器与bundle证据冒充本批新运行。没有运行 `cargo xtask ci`，没有派发 GitHub Actions。

## 6. 未闭合边界

- actual `tauri.conf.json`/capability/binary、verified local session → `bind_window` 与 destroyed event → `unbind_window` assembly 仍不存在；
- 尚无真实 macOS/Windows native Webview 的 open→pressure→terminal→close journey；Windows仍只有cross compile；
- 本批证明的是 Rust mpsc queued event-ref aggregate。Tauri/Wry 到 WebView callback scheduler 的真实运行时背压、XSS、多窗口 shutdown 仍需 native journey，不能由 unit test外推；
- Screen 继续走独立 loopback binary plane，不进入该 256 structured budget；
- formal Desktop golden、T-UI-0126、供应链 blocker与完整 G6 仍 todo；
- reviewed 外部产品名/bundle ID/deep-link 尚无裁决，本批没有擅自创建发行 identity。

下一批只能在不虚构品牌/发行身份的前提下继续 native lifecycle assembly；若可发布 binary 必须先取得 reviewed identity，否则先实现可复用、可真实测试的 host lifecycle primitive并明确不冒充发行构建。
