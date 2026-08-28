# Batch 48：Component Decisions Runtime

> 日期：2026-08-28。分支 `codex/2026-08-28-G6-component-decisions-runtime`；base `470d9e1`；
> WIP `e9f4a40`；implementation `b7652c4af39e905a1a65adeb9f5c1072a3d0e2e8`；
> 固定上游 `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批把 Batch47 的 durable 后端接到真实 provider/runtime/conversation，完成 `askApproval` 与
`askChoice` 两个 compiled HITL renderer。未运行 `cargo xtask ci` 或 Actions，未触碰
`docs/assets/`，未 push/建 PR。

## 第一真源裁决

- 固定上游 `kind: decision` 唯一走 `useHumanInTheLoop`；人的 answer 就是当前 provider call 的
  tool result。pending 与 complete 必须共用同一 renderer，complete 从 recorded result 回读；
- provider call id 只负责 provider pairing；runtime 原样传给 gateway，Rust 另铸 UUIDv7 作为
  decision/internal call identity，不能把 vendor id 升格为 authority；
- `HumanReleased` 不能直接 `Sampling + StartProvider`，否则会跳过 durable tool exchange。本批改为
  `AwaitingHuman → ExecutingTools` 且零 effect；exchange checkpoint 提交后再由
  `ToolResultCommitted → Preparing` 进入下一次 sampling；
- decision 被拒/参数坏也必须形成一个 tool result；只画 RefusedCard 不回答会永久挂起 run；
- 普通 effect tool 在 cancel 时仍 drop-first 并进入 reconciliation。human wait 没有外部 acting effect，
  但有待退休的 durable row；其 JoinHandle 在 cancel 时 detach，继续到 PostgreSQL 观察 terminal 后
  原子写 cancelled audit，再结束；
- manifest/provider/schema/renderer 同批从 ordinary 11 项扩为总计 13 项，禁止广告不可恢复半成品；
- pending UI 只消费 actor-scoped typed API。provider call id 可进入 pending projection，因为完成后
  durable history本来就公开同一 pairing id，且 answer endpoint 只认 decision id + 当前 AuthContext；
- 默认 UI 文案 reactive i18n；模型 description/schema/answer 协议不翻译。Choice 只接受 stored
  id+label，Approval note 按 ECMAScript trim 且上限 4 KiB UTF-8。

## 实施

- contracts manifest 加入两个 `Decision`，统一 parameter schema/title/answer pairing；pending DTO 增加
  非authority provider pairing id；
- gateway 用 fresh DB AuthContext 调 typed `AwaitComponentHumanDecision`，把 normalized answer 序列化为
  exact provider result；runtime 在阻塞前后驱动 HumanRequired/Released，并把 provider call id 写入
  `RunToolExchange`；
- PostgreSQL pending decode/list 投影 pairing id，answer pairing validator 上收到 contracts；catalogue
  首次同步/审计从11扩13，existing管理员治理仍零覆盖；
- conversation 1秒读取actor pending，只画当前active run；answer in-flight防重复。checkpoint触发
  authoritative snapshot，assistant/tool pair替换临时pending；terminal、跨tab answer或撤权后诚实收口；
- `HumanDecisionCard` 同一实现覆盖 Approval/Choice pending+complete。Approval保留details/custom labels/
  optional note；Choice保留ordered options/description并以文字+Check表达选中。坏result/错label走共享
  RefusedCard；Settings两个preview都是无enabled control的complete样本；
- 浏览器即时切换语言发现 Input placeholder与默认Decline初次locale固化；Input升级为reactive TextProp，
  两默认按钮也改为调用时i18n。Choice显式处理Enter/Space并prevent default，避免双激活。

## 证据

| 面 | 本轮亲自运行结果 |
| --- | --- |
| core / UI tests | Agent **33**；contracts **84**；domain unit **369**；application components **6**；UI **128**；Server components **4**；Desktop all-features **78**；transport parity **8**，均0失败 |
| PostgreSQL 17.11 / SCRAM | component catalogue / durable human / provider context 各 **1/0/0**；新增完整 Agent decision vertical slice **1/0/0** |
| 完整 PG vertical slice | answer前 `run=running` 且tool result 0；跨Application answer后decision=answered、requested/answered audit各1；第二次provider context含exact assistant/tool pair，之后terminal completed |
| compile / Clippy / WASM | 9 crate all-targets/all-features Clippy `-D warnings`；contracts/UI wasm32 check；fmt/diff均绿 |
| tools / i18n / design / CSS | tools钉版全绿；**529** leaf；**86 Rust / 74 icons**；**281** class literals |
| production bundle | WASM gzip **1,322,323 B**；CSS **96,050 B**；fonts **740,216 B**；external/inline **1/0**；CSS预算余 **2,254 B** |
| parity / recount | components **7/15/22**；UI **87/65/152**；总计 **674/1012/1686**；fixtures **17/22/39**；strict passed/mismatch/skipped **158/0/0**；0 violation/warning |

已提交实现的精确 release bundle 实得：Approval approve 与 Choice `Enter` 各提交一次，随后 hard reload
仍分别显示 Approved/Answered；Choice 只选 recorded Production 且全部option disabled。另测 Decline hard
reload、4097-byte note使两action disabled、中英即时双向切换、1440×900 / 1024×640 / 900×800 /
600×800 四视口 `scrollWidth=clientWidth`；Gallery 14 published tile中恰2个Decision、两个preview enabled
control=0。duplicate id、nested interactive、visible alert、external resource、console warn/error均0。

关闭 `T-CMP-0004` 与 `T-UI-0056`。`T-CMP-0008` Refused sandbox共用、component admin/sandbox/
Desktop sandbox renderer、formal golden/完整AX矩阵继续 todo；这两个 renderer 是native Leptos DOM，不属于
Desktop sandbox a11y豁免。G4/G6整关继续不勾。
