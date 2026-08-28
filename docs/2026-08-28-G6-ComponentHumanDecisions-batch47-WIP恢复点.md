# Batch 47 WIP：Durable Component Human Decisions

> 日期：2026-08-28。分支 `codex/2026-08-28-G6-component-human-decisions`；base 为 Batch46 正式
> head `43cc6e1`。固定上游 `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 本机定向测试；不运行 `cargo xtask ci`，不派发 Actions，不触碰 `docs/assets/`。

## 已核实边界

- `askApproval/askChoice` 是 surface/HITL tool：当前 run 必须等待人的 answer 成为该 provider tool result；
  它不是 §8.5 acting `tool_approvals`。后者证明“允许执行外部 effect”，绑定 target/effect/policy/
  computer generation；把自然语言问题塞进去会篡改安全语义；
- 只用 React/Leptos 内存 `respond` 无法跨刷新/副本，也无法审计 exactly-once answer；必须有独立 durable
  state，绑定 deployment/tenant/thread/run/actor/auth generation/Agent/provider call/component/args hash；
- native schema 只能 expand。新增 0023 表，不改 runs status CHECK；本批只建立 request/list/answer/wait
  控制面，manifest/provider 暂不加入 Decisions，因此 production 模型仍不可能调用半成品；
- pending 保存 bounded renderer args；answer 只接受 closed decision enum/note 或 option id/label，并保存供
  waiter 回注。resolved/cancelled 后 args 与 answer 的保留/清理必须由后续 durable tool exchange 消费边界
  裁决，本批不假装已接 Agent resume；
- request 必须重复 Agent/component/current-build/published/withholding 检查并与 requested audit 同事务；
  answer 只允许原 actor、fresh AuthGeneration、当前 channel/thread membership、running run 与同 Agent；
- 同 `(run, provider_call_id)` exactly once；重复相同 answer 幂等，异值 conflict。跨副本 waiter 用
  bounded poll，不依赖单进程通知作为真源；cancel/expiry/run失效不得返回伪 answer。

## 实施计划

1. native 0023 + exact schema fixture/table row/ledger parity，secret/debug 分类；
2. contracts：closed pending projection、Approval/Choice answer、request/list/resolve reply；
3. application：internal request/wait 与 authenticated list/resolve ports，bounded validator；
4. PostgreSQL serializable request/list/resolve/wait，hash-chain requested/answered/cancelled audit同事务；
5. Axum/Tauri typed list/answer API，Origin-before-body、no-store、actor scope；
6. PG17 SCRAM 覆盖 scope、revocation、duplicate/conflict、audit rollback、cross-replica wait；
7. 本批不改 `T-CMP-0004`、manifest/provider/UI；完整 Agent AwaitingHuman 与 Decisions renderer 留 Batch48。
