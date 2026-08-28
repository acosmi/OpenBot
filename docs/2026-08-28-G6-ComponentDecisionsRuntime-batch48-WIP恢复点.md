# Batch 48 WIP：Component Decisions Runtime

> 日期：2026-08-28。分支 `codex/2026-08-28-G6-component-decisions-runtime`；base 为 Batch47
> 正式 head `470d9e1`。固定上游
> `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 本机定向测试；不运行 `cargo xtask ci`，不派发 Actions，不触碰 `docs/assets/`。

## 已核实边界

- 固定上游 `kind: decision` 唯一走 `useHumanInTheLoop`；人的 answer 是当前 provider call 的
  tool result。pending 与 complete 共用同一 renderer，complete 必须从 recorded result 回读；
- 被撤权/拒绝的 decision 也必须产生一个 tool result；只画 RefusedCard 而不回答会永久挂起 run；
- provider call id 是 pairing，不是 control identity，但必须由 runtime 原样传到 gateway，再与
  Rust UUIDv7 decision/internal call id 分开绑定；
- reducer 当前 `HumanReleased -> Sampling + StartProvider` 会跳过 durable tool exchange，必须改为
  `HumanReleased -> ExecutingTools + no effect`，由 exchange checkpoint 后的 `ToolResultCommitted`
  唯一进入下一次 context load；
- human wait 不能像普通工具一样在 cancel 时简单 drop。等待任务必须脱离被取消的 join handle，直到
  PostgreSQL观察 terminal/scope失效并原子写 cancelled audit；外部 effect工具仍保持drop-first；
- manifest/provider/schema/renderer必须同批从11扩13，不能广告不可渲染或不可恢复的半成品；
- pending UI只消费actor-scoped typed API；provider call id可进入pending projection，因为完成后的
  durable history本来就公开同一pairing id，且它从不作为answer authority；
- answer成功后以checkpoint/snapshot切换到durable complete card；Choice必须逐字回读stored id+label，
  Approval必须回读closed decision和optional note。所有默认UI文案i18n，模型协议文本不翻译。

## 实施计划

1. contracts/manifest/provider context从11扩13；补provider call pairing与answer/arguments纯校验；
2. gateway调用typed AwaitComponentHumanDecision，runtime接HumanRequired/Released与取消后durable cleanup；
3. Leptos Approval/Choice pending+complete renderer、typed list/answer API、conversation poll/checkpoint合并；
4. fixture提供独立Approval/Choice active threads，浏览器实测answer、hard reload与completed回读；
5. PG/Agent/contracts/UI/Server回归、Clippy/WASM/bundle五闸门、parity/recount；满足全部证据才关闭
   `T-CMP-0004`与`T-UI-0056`。
