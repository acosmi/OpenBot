# Batch 47：Durable Component Human Decisions

> 日期：2026-08-28。分支 `codex/2026-08-28-G6-component-human-decisions`；base `43cc6e1`；
> WIP `feb689b`；implementation `b0e7e7f28d103e226d1e1c0a8ee543d4954b0cc1`；
> 固定上游 `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批建立 `askApproval` / `askChoice` 的 durable request/list/answer/wait 控制面，但刻意不把两个
component 加入 production manifest/provider，也不接 Agent suspend/resume 或 Leptos renderer。未运行
`cargo xtask ci` 或 Actions，未触碰 `docs/assets/`，未 push/建 PR。

## 第一真源裁决

- Decisions 是 surface/HITL tool：人的回答最终成为当前 provider tool call 的 result；它不是 §8.5
  的 acting `tool_approvals`。后者授权外部 effect，绑定 target/effect/policy/generation，不能复用为
  自然语言问答；
- 浏览器内存 `respond` 无法跨刷新/副本，也无法证明 exactly-once。独立状态必须绑定 deployment、
  tenant、thread、run、actor、AuthGeneration、Agent、provider call、component 与 canonical args hash；
- native 兼容期只做 expand：新增 0023 表，不改 run status，不借半成品修改 Agent/runtime 语义；
- request 重新验证 running lease、fresh generation、membership、Agent policy、current component build与
  grant；answer 只允许原 actor 在同一 fresh scope 内提交 closed Approval 或 stored Choice；
- 同 `(run_id, provider_call_id)` 请求 exactly-once；同 answer 幂等，异 answer 冲突。跨副本等待以
  PostgreSQL 为真源、1 秒 bounded poll；scope失效、过期或取消不得伪造 answer；
- bounded args/answer 保留在 durable row，供下一批形成 tool exchange；Rust `Debug` 脱敏，四类 audit
  payload 只记录权威标识、component、canonical hash与输入字节数，不记录问题、note、choice或label。

## 实施

- native `0023` 新增 `component_human_decisions`；Approval/Choice 的 arguments、answer、state、时间与
  resolution 关系由数据库 CHECK 封闭，唯一键固定 `(run_id, provider_call_id)`；
- contracts 新增 closed pending/answer/resolved DTO 与三个 typed command；`askApproval`、`askChoice`
  schema/description/手写 closed validator 已建立，但 build manifest 仍保持 ordinary 11 项；
- application 新增 authority-only scope、30 分钟且不越过 run deadline 的 TTL、64 KiB arguments、
  100 choices、16 KiB string、4 KiB note 边界，以及 request/list/resolve/wait port；
- PostgreSQL request/list/resolve/wait 均复核当前 authority。request/answer 与 hash-chain audit 同事务；
  audit 强制失败时分别证明 row 0 或 pending 不变；Choice 回答按数据库保存的 id+label 校验；
- Axum 与 Tauri 接入 `GET /api/components/human-decisions` 和
  `POST /api/components/human-decisions/{decision_id}/answer`；Axum 写面 Origin-before-body，成功响应
  no-store，外部请求不能自报 actor/tenant/role；
- tables 账本同时补回 Batch36 已存在但历史漏记的 `user_memory_controls`，并新增本批表；不是本批
  重建旧表或篡改历史完成度。

## 证据

| 面 | 本轮亲自运行结果 |
| --- | --- |
| PostgreSQL 17.11 / SCRAM | native 0023 fixture regeneration 开/关各 **1 / 0 / 0**；component decisions **1 / 0 / 0** |
| unit / transport | contracts components **6**；application components **6**；domain audit **3**；native **3**；tables **23**；Server components **4**；Desktop Tauri all-features **1**，均 0 失败 |
| compile / Clippy / WASM | affected all-target compile；7 crate Clippy `-D warnings`；contracts/UI `wasm32-unknown-unknown` check，均绿 |
| schema fixture | **45 tables / 428 columns / 316 NOT NULL / 243 constraints / 91 indexes / 4 triggers / 4 enums / 1 function / 0 extensions**；SHA-256 `489c0ac781baf4efc12e4a23bd28a1d37a716b54a996fcbe7737dc7acb376e5b` |
| parity / recount | API **60/109/169**；events **33/53/86**；tables **58/0/58**；components **6/16/22**；总计 **672/1014/1686**；fixtures **17/22/39**；strict passed/mismatch/skipped **158/0/0**；0 violation/warning |

关闭 `T-API-0168/0169`、`T-TBL-0057/0058`、`T-EVT-0083–0086` 与 `T-FIX-0039`。
`T-CMP-0004` 继续 todo：当前 production provider 无法广告 Decisions，Agent 尚未进入/退出
`AwaitingHuman`，conversation 也没有 pending/completed Approval/Choice renderer。由于本批没有生产 UI、
CSS、manifest 或 renderer 变化，没有重建 bundle、跑浏览器或冒充 visual/golden 证据。
