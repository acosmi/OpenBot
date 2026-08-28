# Batch 46 WIP：Activity Follow-up Ask

> 日期：2026-08-28。分支 `codex/2026-08-28-G6-activity-follow-up`；base 为 Batch45 最终 head
> `8855e35`。固定上游 `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 本机定向测试；不运行 `cargo xtask ci`，不派发 Actions，不触碰 `docs/assets/`。

## 已核实边界

- Activity 的 follow-up 不是 HITL `respond`：它提交一个新的普通 user turn；`askApproval/askChoice`
  才会暂停当前 run，必须另批 durable resume；
- 上游两条模型可见 ask 文本逐字固定：busiest Bot 问句插入已显示的 Bot id；recent refusal 问句不
  插入拒绝正文。按钮标签本地化，发给模型的协议文本不翻译；
- 新 turn 必须绑定该 durable component message 的 Server-derived Agent，不能退回频道 roster 第一项；
- 本仓每 thread 只允许一个 foreground run。Activity read 可能早于当前 run terminal 完成，因此 action
  在 busy/resumable/submitting 时明确 disabled，idle 后启用；不静默吞点击、不并发造第二 foreground run；
- 复用唯一 `begin_channel_run` 发送面。`PendingTurn` 必须记录 Agent，使首次 begin 失败后的 Retry 原样
  重放 thread/run/Agent/message；当前 `resumable` 同时锁死 send callback 与按钮的缺口同批修复；
- 本批只在真实 data rows 非空且存在 conversation callback 时显示 action；empty/refused/failed/reading
  均不画假按钮。

## 实施计划

1. `PendingTurn` 增加 Agent；统一 send callback 接收 Agent+message，composer 与 component 共用；
2. 修复 resumable retry：编辑锁定但 Retry 可点击，重放 exact durable attempt；
3. conversation 将 bounded follow-up callback/idle signal传给 paired Activity renderer；
4. Activity 两种 report 只在真实非空数据上画 tokenized Button，调用上游逐字 prompt helper；
5. 单测 Agent/Retry/prompt/empty 边界，release fixture 注入 Activity 数据并用真实浏览器点击→新 user turn；
6. 完整证据前 `T-CMP-0001` 保持 todo；Decisions/Refused sandbox/admin/golden 不进入本批。
