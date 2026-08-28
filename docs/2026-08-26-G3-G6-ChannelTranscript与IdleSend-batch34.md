# Batch 34：Channel Transcript、Realtime 与 Idle Send

> 日期：2026-08-26。分支：`codex/2026-08-26-G3-channel-conversation`。
> 基线：Batch33正式head `8e7da1d96fb14a87368f17440fc4c75d29787890`。
> 实施提交：`6013072529f22054599264296552fd474a9e6bf1`。
> 未运行`cargo xtask ci`，未派发Actions，未处理`grok-bot`，未修改/暂存/提交`docs/assets/`。

## 1. 已闭合的生产路径

- [x] typed `GetThreadConversation`与新增`GET /api/threads/{thread_id}/conversation`：只取path+
  AuthContext，响应`no-store`，closed DTO同时含ordered messages、current foreground run、该run尚未
  materialize的text tail与last event cursor；unknown/invisible/deleted统一default-empty；
- [x] `PostgresThreadDirectory::thread_conversation`以**单条SQL statement**取得上述四面，避免history/
  busy/cursor跨查询漂移；active run超过1或array/text不一致fail-closed；active tail只聚合最后一个
  tool-exchange checkpoint后的text chunks，避免已物化assistant文字重复；
- [x] SSE `/events?cursor=`增加一次性bootstrap cursor；标准`Last-Event-ID`在reconnect时优先。客户端
  从snapshot cursor后接durable replay/live，查询到连接间隙不会丢event；
- [x] Leptos `ChannelConversation`：snapshot→EventSource typed AppEvent→event_sequence去重/缺口refetch；
  started/semantic text/reasoning ignore/checkpoint/四terminal闭集驱动busy、streaming、localized notice；
  terminal后重新读取PG history，SSE/NOTIFY永不作真源；
- [x] durable user/assistant/tool-call/tool-result投影到既有Message/Bubble/MessageScroller；system prompt
  不进入transcript，tool result复用既有安全projection，DOM message id为SHA-256而非数据库自由文本；
- [x] idle send复用既有mint+BeginThreadRun：有thread直接begin，无thread先铸deployment UUIDv8再以
  channel anchor begin；response未知/失败复用同run-id，成功后由snapshot/SSE/history收敛，不画假assistant；
- [x] Textarea补Enter send、Shift+Enter newline与composition期间Enter不发送；busy仍可写草稿，
  Send禁用，terminal后草稿保留；
- [x] Chat PageShell固定为`100dvh - topbar`内部flex滚动。真实浏览器先发现仅min-height会把
  transcript算成约96万px；修后四视口高度为578/318px，页面X/Y overflow均0。

## 2. 构造性边界

- raw provider/watchdog/JavaScript Error不穿contracts；terminal只用fieldless closed code映射en/zh-CN，
  不把不可信“own words”直接显示；
- EventSource只消费`AppEvent::ThreadRunEvent/ThreadStreamError`；wrong thread、terminal bit漂移、未知
  channel、缺delta、event gap均不拼进可见文字，而是重新取snapshot或关闭流；
- active tail随snapshot返回，解决“run已产生chunk但cursor从最新开始会漏partial text”；
- queue与stop均未渲染。busy草稿只是保留在Textarea，不承诺自动运行；
- 多Agent channel仍按固定上游当前第一位recipient；per-message mention/skills尚未接；
- 当前是安全plain-text transcript。§6.4 markdown/syntect/remote image/link规则、完整ToolBoundary视觉、
  Screen detail尚未完成，因此ChannelChat/ChatTranscript/ConversationView/Composer与route整项不勾。

## 3. 本机证据

| 面 | 结果 |
| --- | --- |
| contracts snapshot closed wire | **1 / 0 / 0** |
| Server conversation / SSE cursor precedence | **2 + 1 / 0 / 0** |
| UI全包 | **101 / 0 / 0** |
| PostgreSQL **17.11 host SCRAM** | **1 / 0 / 0** |
| six-crate all-targets/all-features Clippy `-D warnings` | 通过 |
| UI WASM all-targets/all-features | 通过 |
| i18n / design / CSS | **410** leaf；**71 Rust / 74 icons**；**204** class literals |
| release bundle | WASM gzip **781,045 B**；CSS **71,409 B**；fonts **740,216 B**；external/inline **1/0** |
| parity | API **49/115/164**；tests **372/675/1047**；UI **85/67/152**；总计 **638/1037/1675**；fixtures **15/22/37** |
| strict upstream recount | **157 / 157 / 0** |
| parity violations / warnings | **0 / 0** |

PG真库顺序：BeginRun→snapshot(user/active/cursor0)→claim/ack→text`hel`→snapshot(active tail/
cursor1)→先订阅after1→text`lo`/completed→只收到event2/3→snapshot(user+assistant`hello`/
active null/cursor3)；outsider得到default-empty。实例只监听127.0.0.1，测试后停止并删除。

release WASM浏览器：

- existing channel初始durable user+assistant exact，named log/article，avatar AX重复0；
- Shift+Enter得到`First line\nSecond line`且article仍2；Enter后article3、thinking、send disabled，
  stop/queue DOM均0；SSE阶段`data-streaming-message=1`且文字`Fixture reply`；
- terminal后streaming marker0、history变4篇、busy期间输入的草稿保留、alerts0；
- hard reload前后`Durable after reload`与assistant逐字相同，证明不依赖module seed；
- 原本threadId=null的`channel-01`完成mint→begin→reply→hard reload，详情投影真实native thread；
- 1440×900 transcript578px；1024/900/600×640均318px，四档composer可见、X/Y overflow0、
  1 main/nav/h1、duplicate ID0、console error/warn0。

fixture只证明GUI交互；production snapshot/replay/materialization由上述PG测试承担。浏览器tab、fixture、
PG实例均已关闭。Cargo.lock/package delta0；Cargo.toml只给既有web-sys增加EventSource/MessageEvent feature。

## 4. 台账变化

- 新增API：`T-API-0164`；
- fixed upstream tests：`T-TEST-0217–0219`（raw stopped reason→closed localized terminal替代）、
  `T-TEST-0240–0248`（module seed/stash→durable begin+snapshot替代），共12条。

## 5. 明确仍未完成

- [ ] durable跨副本stop/cancel：DB request、NOTIFY、missed-notify poll、child-stopped terminal；
- [ ] Batch33 queue接production busy/settle/remove/steer visual；
- [ ] `@coworker`、`/skill` sources与真实skills；附件；per-channel draft persistence；
- [ ] pulldown-cmark/syntect、完整tool boundary、Screen/Computer detail；
- [ ] `T-UI-0041–0044/0122/0123`、`T-ROUTE-0009`与golden；
- [ ] G3/G4/G6整关继续不勾。
