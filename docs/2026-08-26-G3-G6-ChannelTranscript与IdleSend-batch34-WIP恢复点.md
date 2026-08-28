# Batch 34 WIP：Channel Transcript、SSE Realtime 与 Idle Send

> 日期：2026-08-26。分支 `codex/2026-08-26-G3-channel-conversation`；base = Batch33正式head
> `8e7da1d96fb14a87368f17440fc4c75d29787890`。固定上游
> `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 只跑本地定向测试；不运行`cargo xtask ci`，不派发Actions，不处理`grok-bot`，不修改/
> 暂存/提交`docs/assets/`。

## 接续结果（2026-08-26）

本恢复点已由实施提交 `6013072529f22054599264296552fd474a9e6bf1` 完整接续。正式证据见
[Channel Transcript、Realtime 与 Idle Send](2026-08-26-G3-G6-ChannelTranscript与IdleSend-batch34.md)。
实际收口采用原子`GET /conversation` snapshot（messages+foreground+active tail+cursor），而不是
WIP最初写的单独history GET；它能从snapshot cursor无缝接SSE durable replay/live并诚实判断busy。

contracts/Server/UI/PG=`1 + (2+1) + 101 + 1`均0失败；i18n410/design71/CSS204，bundle
`781045/71409/740216/1/0`；API49/115/164、tests372/675/1047、parity638/1037/1675，
strict recount157/157/0。未运行CI/Actions，`docs/assets/`未动。

Stop/cancel/queue/markdown/screen仍未完成；下文不冒充边界继续有效。以下保留为开工历史快照。

## 本批生产闭环

1. 新增native `GET /api/threads/{thread_id}/history`，复用唯一typed GetThreadHistory；不让最终
   Leptos调用CopilotKit compatibility URL或自报agentId；
2. Leptos channel detail在拿到native thread后先接SSE durable replay/live，再加载history；按
   event_sequence去重，Started/semantic text/terminal驱动busy与streaming projection；terminal后
   refetch PostgreSQL history，NOTIFY/SSE不作真源；
3. user/assistant durable history以现有Message/Bubble/MessageScroller呈现；system/tool暂不伪装成
  普通对话，完整tool boundary/markdown仍独立todo；
4. idle send使用既有mint/begin API：有thread直接begin，无thread先mint再以channel anchor begin；
   成功receipt后等待SSE/history，不画optimistic fake assistant；HTTP失败恢复原草稿；
5. Textarea补第一真源Enter send / Shift+Enter newline / composition期间Enter不发送；
6. 真PG证明history→subscribe-before-send→BeginRun→started/chunk/terminal→history materialize；真实
   browser fixture只证明GUI，不冒充production provider/PG。

## 不冒充

- 本批不实现stop/cancel：跨副本正确Stop需要durable cancel request+notification+missed-notify poll+
  child-stopped terminal，不能把某一进程内`RunDispatchConsumer::revoke`直接挂HTTP；
- busy时Textarea可保留草稿但Send禁用；queue/queued rows/remove/settle/steer不渲染，等下一批
  durable cancellation落地后再消费Batch33 queue；
- 不勾完整`ChannelChat`/`ChatTranscript`/`ConversationView`/`Composer`与`T-ROUTE-0009`；
- 不引markdown/syntect，不把plain text slice冒充§6.4完整transcript renderer；
- 不显示raw provider/database error；terminal只投影closed本地化原因。

## 预期精确台账

- 新增native history API（下一连续API ID）；
- `T-TEST-0240–0248`以durable native history替代module stash后关闭；
- route/UI业务条目继续todo，直到完整控制/renderer判据成立。
