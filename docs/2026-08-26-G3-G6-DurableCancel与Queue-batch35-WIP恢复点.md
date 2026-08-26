# Batch 35 WIP：Durable Run Cancel 与 Production Queue

> 日期：2026-08-26。分支 `codex/2026-08-26-G3-durable-cancel`；base = Batch34正式head
> `33f341fc2181fbc3c1992be75ad31f429519418d`。固定上游
> `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 只跑本地定向测试；不运行`cargo xtask ci`，不派发Actions，不处理`grok-bot`，不修改/
> 暂存/提交`docs/assets/`。

## 第一真源裁决

- v3 §3.1条4、§7.2、§7.4要求每个thread一个foreground actor，cancel沿run→provider/tool/
  computer/process tree传播；UI先显示Cancelling，只有children-stopped事实后才显示Cancelled；
- GUI第一真源§6.5要求运行中有真实stop，并允许把输入park成当前mount内的queue；turn不论completed/
  failed/cancelled结束都settle成一个follow-up，queue不是durable outbox；
- Batch34已经证明snapshot/SSE/idle send，但stop若只调用当前Server进程里的consumer，HTTP落到另一副本
  会静默无效，因此不能直接画按钮；
- 取消请求属于现有`public.outbox`允许的replay-safe internal delivery，不需要为同一事实新增`runs`
  状态列或native 0022。request outbox绑定run/thread/requesting actor；lease owner消费。PostgreSQL
  NOTIFY只作低延迟wake，周期poll负责漏通知/重连；
- consumer必须区分“本机child已收到cancel”与“本机从未持有child”。前者等待runtime写terminal，后者
  才能在同事务写Cancelled+deliver cancel outbox。tool已发出且commit未知时继续
  ReconciliationRequired，不能用Cancelled抹掉不确定性。

## 本批实施范围

1. closed cancel command/reply与conversation foreground state/cancellable projection；
2. typed ApplicationService + membership/run-owner授权的PostgreSQL cancellation request；
3. `POST /api/threads/{thread_id}/runs/{run_id}/cancel`，trusted Origin先于任何业务调用；
4. multi-replica run-control relay：owner/fencing、NOTIFY wake、missed-notify poll、dispatch-before-start race；
5. Leptos Stop→Cancelling→Cancelled/reconciliation，并把Batch33 queue reducer接到真实conversation：
   busy输入可park、逐条可移除、turn结束一次合并发送、stop后同一路settle；
6. PG17.11 SCRAM、Agent child-drop、Server、UI/WASM与真实release浏览器定向验收；机器证据成立后才
   更新API/parity、两份第一真源、CLAUDE、移交指南与正式Batch35文档。

## 明确不在本批冒充

- `@coworker`、`/skill`真实sources、附件与per-channel draft persistence；
- markdown/syntect、完整tool boundary、Screen/computer detail；
- RMCP/computer/file/shell各自的协议级cancel notification/process-tree验收；本批只把统一run token与
  built-in/remote Agent host的durable入口接通，未落的executor不能据此打勾；
- 完整ChannelChat/ConversationView/Composer与G3/G4/G6整关仍需其余判据。
