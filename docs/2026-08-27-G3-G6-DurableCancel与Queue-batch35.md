# Batch 35：Durable Run Cancel 与 Production Queue

> 日期：2026-08-27。分支：`codex/2026-08-26-G3-durable-cancel`。
> 基线：Batch34正式head `33f341fc2181fbc3c1992be75ad31f429519418d`。
> 实施提交：`86370626cebf3f78e87ac5b5a87e223377ff69ff`。
> 固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 本批只运行本机定向测试；未运行`cargo xtask ci`，未派发Actions，未处理`grok-bot`，
> 未修改/暂存/提交既有`docs/assets/`。
> 远端：Batch34 base ref与Batch35分支已push；PR等待用户明确授权，尚未创建。

## 1. 第一真源裁决

- v3 §7.2/§7.4要求cancel沿run→provider/tool/computer/process tree传播；GUI先显示
  `Cancelling`，只有children-stopped事实后才显示`Cancelled`；
- HTTP若只调用当前Server进程的consumer，落到另一副本时会静默无效。因此Stop必须先写
  PostgreSQL durable control，再由当前lease owner消费；NOTIFY只作低延迟wake，poll负责漏通知；
- `public.outbox`已经是replay-safe internal delivery真源。本批不新增native migration，也不把
  cancellation请求伪造成新的`runs.status`终态；
- consumer必须区分`ChildSignalled`与`NoLocalChild`：前者等待真正runtime child写terminal，后者才
  能同事务写Cancelled；`ReconciliationRequired`仍占foreground，不能被Cancel覆盖；
- GUI第一真源§6.5的queue只属于当前mount，busy期间park、可移除，任一确定terminal后合成一个
  follow-up；它不是durable outbox，hard reload必须丢弃。

## 2. 已闭合的生产路径

- [x] closed `CancelThreadRun` command/reply，request结果只分`requested`、
  `already_requested`、`already_terminal`；ack不声称child已经停止；
- [x] `ThreadConversationSnapshot`增加closed foreground state与`activeRunCancellable`：
  queued/running/cancelling/reconciliation四态与active run形状逐字段校验；
- [x] typed ApplicationService只从`AuthContext`注入deployment/tenant/actor。PostgreSQL在锁定run后
  同时校deployment/tenant、当前direct/thread或channel membership、run owner；另一channel member
  不能停止不是自己发起的run；
- [x] cancellation request与`agent_run_cancel` internal outbox同事务提交，payload只含绑定后的
  run/thread/requesting actor；exact replay不重复写，terminal race成功返回closed observation；
- [x] `POST /api/threads/{thread_id}/runs/{run_id}/cancel`要求已认证+trusted Origin，响应
  `no-store`；202只表示durable requested/already-requested，200表示已terminal；
- [x] production RunRelay以`LISTEN openbot_run_control`唤醒并保留100ms durable poll；claim只允许
  当前lease owner或expired fencing takeover，取消优先于dispatch与stale recovery；
- [x] built-in Agent reservation的watch token收到cancel后先drop context/provider/tool child，再由
  既有journal写唯一Cancelled terminal；relay在terminal事实可见后才deliver cancel outbox；
- [x] dispatch-before-start/no-local-child路径在同一事务写Cancelled，并同时收口cancel与原dispatch
  outbox，陈旧dispatch不再有再次claim窗口；
- [x] Leptos Stop只在空draft且snapshot证明当前actor可首次请求时可点；请求中或durable Cancelling
  时保持可见但disabled，terminal只由SSE/snapshot推进；
- [x] Batch33 `reduce_queue`接入真实conversation：busy submit park、逐条remove、busy→idle边沿只
  settle一次，多条文本以换行合并成一个run；stop后的terminal走同一drain；hard reload清空queue。

## 3. 真库发现并修复的缺口

第一次运行poll-only PostgreSQL测试得到：run=`cancelled`、cancel outbox=`delivered`、dispatch
outbox=`pending`、terminal=`1`。这证明“无本地child”的事务虽写了终态，却留下一个可被下一轮
relay重新claim的陈旧dispatch。

修订后`finish_unstarted_cancellation_in_transaction`先逐字段复核dispatch outbox的aggregate、
destination、delivery class、run/thread/event sequence绑定，再与Cancelled terminal、cancel outbox
同事务收口；缺行、dead-letter或绑定漂移均fail-closed。

随后新增“tampered dispatch→零terminal→恢复→进程重启重放”负向路径，又发现claim SQL用
`NOT(last_error_code = 'child_signalled' AND …)`：当`last_error_code IS NULL`时，SQL三值逻辑令
`NOT(NULL)`仍为NULL，已claim但尚未写error code的崩溃记录会被WHERE静默过滤。修订为
`last_error_code IS NOT DISTINCT FROM $4`，只抑制已确认child-signalled且lease仍活跃的记录；NULL
owned claim可精确重放。相同PG测试最终重跑为1/0/0。

## 4. 本机机械证据

| 面 | 结果 |
| --- | --- |
| contracts / application / Agent / Server / UI | **76 / 137 / 28 / 202 / 105**，均0失败 |
| infra host tests | **306 / 0 / 0** |
| Axum/in-process transport parity | **8 / 0 / 0** |
| PostgreSQL **17.11 host SCRAM** | poll-only durable cancel **1 / 0 / 0**；cross-replica active child-drop **1 / 0 / 0** |
| six-crate all-targets/all-features Clippy `-D warnings` | 通过 |
| contracts/UI WASM | 通过 |
| i18n / design / CSS | **417** leaf；**71 Rust / 74 icons**；**206** class literals |
| release bundle | WASM gzip **801,086 B**；CSS **73,154 B**；fonts **740,216 B**；external/inline **1/0** |
| parity | API **50/115/165**；tests **372/675/1047**；UI **85/67/152**；总计 **639/1037/1676**；fixtures **15/22/37** |
| strict fixed-upstream recount | **157 / 157 / 0** |
| parity violations / warnings | **0 / 0** |

PG证据一：同actor首次request=`requested`、exact replay=`already_requested`、另一个channel member
得到NotVisible；active run与delivered cancel row并存被判corrupt。snapshot为Cancelling且不可再次Stop；
tampered dispatch时run保持running、cancel不delivered、terminal=0；逐字恢复原payload并模拟claim后崩溃，
新relay仍由poll精确重放，写唯一Cancelled且cancel/dispatch均delivered；terminal replay=
`already_terminal`且control row总数1。

PG证据二：另一Server副本提交request，lease owner的真实built-in Agent正阻塞在context child；
dispatch delivered与`agent.invoked`先成立，随后cancel使child drop，再出现唯一Cancelled terminal；cancel
outbox最终delivered，event_type=cancelled与terminal总数均1。

release WASM浏览器：

- Stop点击后按钮文本`正在停止…`、disabled、Cancelling status=1；模拟child停止后Cancel notice=1、
  Stop/Cancelling=0、alerts=0；
- queue row 1→remove→0；两条park消息只生成一个换行合并的user turn，两个standalone turn=0；
- Stop时queue保持1，Cancelled后queue=0且follow-up user turn恰1；
- hard reload前queue=1、reload后queue=0，但durable foreground从snapshot恢复Stop=1；
- 1440×900、1024×640、900×640、600×640四档X/Y overflow=0、composer可见、
  main/nav/h1各1、duplicate IDs=0、alerts=0、console error/warn=0。

临时PostgreSQL只监听127.0.0.1，测试后已停止并删除；fixture与浏览器tab均已关闭。

## 5. 台账变化与未完成边界

- 新增`T-API-0165`，API由49/115/164变为50/115/165；其余固定上游test/UI/route条目不倒算；
- `T-UI-0043`/`T-UI-0123`与`T-ROUTE-0009`继续todo：`@coworker`、`/skill` sources、附件、
  per-channel draft persistence、steer尚未闭合；
- markdown/syntect、完整tool boundary、Screen/computer detail仍未闭合，ChannelChat/
  ChatTranscript/ConversationView/Composer整项不勾；
- RMCP notification、computer/file/shell process tree各自的协议级cancel仍独立todo；本批只闭合统一
  durable入口与built-in Agent host，不把尚未存在的executor写成完成；
- G3/G4/G6整关继续不勾；Cargo.lock/package/native migration均无新增。
