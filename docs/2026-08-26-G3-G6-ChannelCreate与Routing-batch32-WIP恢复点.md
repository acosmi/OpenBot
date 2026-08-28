# Batch 32 WIP：Channel Create、Routing 与 `/channel/new`

> 日期：2026-08-26。分支 `codex/2026-08-26-G3-channel-create-routing`；
> base = Batch31正式head `00ad915da6a1844551c943d866f170f0d40e41f8`。
> 只跑本地定向测试；不运行`cargo xtask ci`，不派发Actions，不处理`grok-bot`，
> 不修改/暂存/提交`docs/assets/`。

## 接续结果（2026-08-26）

本恢复点已由实施提交 `f9eb1594a5aad634be992c3f24f6dc1d21e2f806` 完整接续。正式、
可复核的 Batch32 证据在
[Channel Create、Routing 与 `/channel/new`](2026-08-26-G3-G6-ChannelCreate与Routing-batch32.md)。

下文六项生产闭环均已执行：纯routing11条、typed `/api/route`、原子channel create+native
thread、native BeginThreadRun HTTP framing、真实`/channel/new`首发、PG/Axum/WASM/浏览器/ledger。
PostgreSQL17.11 SCRAM=`2+1/0/0`，Server=`3+6+2/0/0`，UI=`68/0/0`，strict
recount=`157/157/0`；parity=`599/1075/1674`，0 violation/warning。`cargo xtask ci`未运行，
Actions未派发，`docs/assets/`未动。

恢复点“明确不冒充”的各项仍然成立：完整channel/home Composer、golden、Agent lifecycle、G3/G4/G6
整关继续todo。以下保留为开工时的历史设计快照，不再作为当前进度口径。

## 本批生产闭环

1. `openbot-domain::routing`逐条移植固定上游11条prompt/parse/fallback判据；模型失败、坏JSON、
   roster外ID、低置信度均回权威default，router不向用户抛模型错误；
2. typed `/api/route`经唯一ApplicationService：显式recipient优先且零模型调用，未选择才route；
   roster来自当前tenant/actor/admin，audit只记candidate IDs/choice/reason，不记原消息；
3. typed `POST /api/channels`：canonical unique Agent IDs、同事务锁profile并复核domain access，
   创建channel/creator membership/channel_agents/native channel thread；不写Intelligence mapping；
4. 新增native begin-run HTTP framing，使最终Leptos不用CopilotKit/React facade也能提交首条消息；
5. `/channel/new?agent=`保留URL recipient，hidden但可见的direct detail可用；无recipient禁发，
   首发顺序=create channel→begin durable run→navigate，失败显式且不画假成功；
6. 只在PG/Axum/Application/WASM/浏览器证据成立后关闭精确API/test/route/UI条目。

## 第一性原理边界

- channel create的native替代物是`threads(anchor_kind='channel')`，不是新增
  `intelligence_channel_mappings`；thread id必须带deployment fingerprint；
- profile在创建事务内按canonical Agent ID顺序`FOR UPDATE`，未来soft-delete/package同步与
  create不会TOCTOU或反序死锁；另一tenant package Agent与deleted/invisible统一404；
- client不得自报channel id/name/thread id/active/actor/tenant/admin；返回只用closed DTO；
- routing默认项按权威visible roster的确定性顺序取first public、无public再first；不凭猜测
  新造tenant package字段。以后若第一真源新增显式default字段，再做expand migration；
- route模型复用deployment package OpenAI adapter/credential/safe dialer；无credential或任何
  provider失败都必须落fallback并成功记录，不把routing变成第二套模型配置；
- state-changing POST要求authenticated same-origin，但普通会话无需fresh/admin。

## 明确不冒充

- 首页T-ROUTE-0006的完整Composer、`@`触发、skills命令、自动routing UI仍按完整journey独立验收；
- T-ROUTE-0009完整transcript/queue/stop/steer/screen仍todo；
- create/edit/duplicate/hide/unhide/delete Agent lifecycle仍todo；
- 多Agent同时会话只保留API parity；本批UI仍按固定上游`channel/new`的一位recipient路径；
- brand favicon、golden、G3/G4/G6整关不因本批局部闭环而打勾。
