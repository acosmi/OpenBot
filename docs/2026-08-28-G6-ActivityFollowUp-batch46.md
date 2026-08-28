# Batch 46：Activity Follow-up Ask

> 日期：2026-08-28。分支 `codex/2026-08-28-G6-activity-follow-up`；base `8855e35`；
> WIP `0a01fde`；implementation `3a277568fa8706300ac7ba6d0c93b1d255adec22`；
> 固定上游 `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批补齐 Activity 两种 report 的 conversation follow-up `ask`，并修复 native conversation 首次
begin 失败后 `resumable` 把 Retry 自己锁死的问题。未运行 `cargo xtask ci` 或 Actions，未触碰
`docs/assets/`，未 push/建 PR。

## 第一真源裁决

- Activity follow-up 是新的普通 user turn，不是当前 tool call 的 HITL `respond`；不能与
  `askApproval/askChoice` 共用暂停/恢复实现；
- 发给模型的两条 prompt 是协议文本，逐字保留上游英文，不随界面 locale 翻译；按钮标签本地化；
- 新 turn 必须绑定 durable component message 的 Server-derived Agent，不能猜频道 roster 第一项；
- 本仓每 thread 只有一个 foreground run。Activity data 可能早于当前 run terminal 读完，因此 busy、
  resumable、submitting 时按钮明确 disabled，idle 后启用；不静默吞点击、不并发造 run；
- composer 与 component 共用唯一 `begin_channel_run`。`PendingTurn` 保存 Agent；失败后的 Retry 必须
  重放同一 thread/run/Agent/message，不能重新 mint 幂等键；
- action 只在真实非空 data 与 conversation callback 同时存在时显示；reading/refused/failed/empty
  均不画假操作。

## 实施

- `PendingTurn` 增加 `agent_id`；统一 send callback 接收 `(BotId, message)`，composer queue 继续绑定
  当前 channel Agent，component follow-up 使用配对 history 的 Agent；
- 拆分编辑锁与 Send disable 判定：resumable 时 textarea 锁定，但 Retry 可点击；callback 允许读取
  existing attempt 并按原 run id 重放；
- conversation 向 paired component 传 follow-up callback 与 reactive disabled signal；
- Activity 两个真实 data view 在 `GalleryFrame` action 区复用 tokenized Button。busiest prompt 只插入
  已显示的 Bot id；refusal prompt 不拼任何拒绝正文；
- test-only fixture 为两种 report 返回 typed data、投影两组 durable component exchange，并让 busiest
  首次 begin 固定 503，以真实浏览器证明 retry；动态 user/assistant message 都保留 run 的 Agent。

## 证据

| 面 | 本轮亲自运行结果 |
| --- | --- |
| UI tests | conversation **12 / 0 / 0**；Activity **1 / 0 / 0** |
| compile / Clippy / WASM | UI+Server all-targets/all-features check；UI+Server及最终fixture Clippy `-D warnings`；contracts/UI WASM绿 |
| tools / i18n / design / CSS | pins全绿；**517** leaf；**85 Rust / 74 icons**；**271** class literals |
| production bundle | WASM gzip **1,254,517 B**；CSS **93,646 B**；fonts **740,216 B**；external/inline **1/0** |
| parity / recount | components **6/16/22**；总计 **664/1014/1678**；strict **157/157/0**；0 violation/warning |

Release 浏览器先实得 Activity/Refusals 两个 report、两个 action 各 1。busiest 第一次 POST 被 fixture
固定为 503；此时 Retry enabled、两个 Activity action 与 textarea disabled、prompt 尚未出现、alert 1。
点击 Retry 后第二次 POST=201，前后两份 body 的 `runId`、`botId=bot-0`、channel anchor 与 exact prompt
逐字相同；durable user/assistant 都是 `agentId=bot-0`。Refusals action 另发一个 201，第二条 prompt 逐字
等于上游。正常成功路径 application console error=0；一次 retry 场景只多出预期注入的 503 network
record，另有 Chromium preload-SRI 支持警告 1，未削弱 SRI。

两 prompt、两 reply、两 button、两 report 在 hard reload 后各自只出现一次；1440×900 与 600×800
`scrollWidth=clientWidth`，duplicate id/nested article 0，被拒 Notice 的正文仍不泄漏。CSS 字节不变，
剩余预算仍为 **4,658 B**。

关闭 `T-CMP-0001`。`T-CMP-0004` Decisions/HITL、`T-CMP-0008` Refused sandbox 共用、component
admin/sandbox/Desktop renderer 与 formal golden 继续 todo；`parity/ui.yaml::biz-gallery-activity` 因正式
golden 口径未闭保持 todo。
