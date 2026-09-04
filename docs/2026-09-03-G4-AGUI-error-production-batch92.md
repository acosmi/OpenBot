# G4 AG-UI Error Production Vertical（Batch92）

> 日期：2026-09-03（America/Los_Angeles）
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §2.4、§7.5、§24、§28.1
> 基线：Batch91 / PR #74，`ad22e76b3f55fa8d1f7258b488d1af4bd9ca066d`
> implementation：`1f684126e38c5ed81ca954e59c2d65c4af4128e7`

## 1. 本批关闭的边界

`AguiDecoder` 已能解析 `RUN_ERROR` 并拒绝 malformed message，但 production adapter 与 PostgreSQL 终态此前没有一条证据证明远端错误正文不会进入本地 journal/audit/GUI；`parity/events.yaml::T-EVT-0011` 因此仍是 todo。

本批只关闭 AG-UI error 族：

- 合法 `RUN_ERROR` 映射为本地 `ProviderFailure::GenerationFailed`；
- malformed/unknown/sequence-invalid event 映射为本地 `ProviderFailure::InvalidResponse`；
- remote `message` / `code` 只在 decoder 的短生命周期 untrusted value 中存在，不进入 `ProviderEvent`；
- durable terminal、API 与 GUI 只消费本地 stable code；
- 既有 SafeDialer body read-gap stall→`agent.stream_stalled` 路径不改变。

本批不把其它 AG-UI state/messages/activity/step/raw/custom/interrupt/tool-result 族标为完成。

## 2. 单元与表现层边界

`RemoteAguiSession::accept` 在 `RUN_ERROR` 边界直接丢弃远端 prose/code，只发 `ProviderEvent::Failed(GenerationFailed)`。decoder 拒绝 object-shaped assistant `message.content` 后，session 清空 pending 并只发 `InvalidResponse`。

新增 Agent 测试同时携带 `REMOTE_ERROR_SECRET_CANARY` 与伪 vendor code，证明两种输入分别变成本地两个封闭 failure，之后 session EOF。UI 既有 `TerminalNotice` 只有 Failed/Cancelled/ReconciliationRequired 三个无载荷枚举，定向测试证明不能承载远端错误字词。

## 3. 真实 PostgreSQL + SafeDialer/SSE 纵向

同一个 production remote Agent、同一个 RunRelay 与同一个 `RemoteAguiProvider → SafeRemoteAguiTransport → SafeDialer` 依次运行三条真实 run：

1. 完整 lifecycle/state/messages/activity/raw/custom/reasoning/text → `completed`；
2. `RUN_STARTED → RUN_ERROR(message=canary, code=vendor-secret-code)` → `failed/provider_generation_failed`；
3. `RUN_STARTED → malformed MESSAGES_SNAPSHOT` → `failed/provider_invalid_response`。

三条 run 均先写 `agent.invoked`；错误 run 没有 provider retry。最终联合扫描 `messages.content`、`run_events.payload`、`audit_events.payload`，canary 命中为 0。临时 PostgreSQL 17.11 完整 Agent 套件 `9/0/0`，实例 fast stop 后 data/socket 目录清零。

## 4. 验证

- `cargo test -p openbot-agent --locked`：`47/0/0`。
- UI closed terminal notice：`1/0/0`。
- `cargo test -p openbot-testkit --locked`：`17/0/9 ignored`。
- PostgreSQL 17.11 `agent_runtime_postgres`：`9/0/0`。
- Agent/Testkit all-target/all-feature Clippy `-D warnings`：通过。
- `cargo fmt --check`：通过。
- `cargo xtask parity-check`：parity=`824/880/1704`、events=`36/52/88`、fixtures=`20/22/42`、overlay=`1293/403/2/6`、0 violation。

## 5. 未声称完成

- 仅 `T-EVT-0011 agui-error` 从 todo 变 done；其它 AG-UI todo 不变。
- 没有 schema、migration、API、依赖、Cargo.lock、UI bundle 或环境变量变化。
- native latest 仍为 `0026`，schema 仍为 46 表/455 列/253 约束/92 索引。
- strict recount 未配置固定上游目录；按 R63 未运行 `cargo xtask ci`，未派发 Actions。
- `grok-bot`、零 npm、单一非 Grok `package.json` 与 manual-only workflow 不变。

## 6. R168 机器台账身份纠正

Batch94 复核发现：本批 docs commit `a46438f2d2f338d3e9473d72f639f8b674b6fcc3` 把上面的 error `done_evidence` 错挂到了相邻 `T-EVT-0003 agui-tool-call-result`，并把它置为 done；真正的 `T-EVT-0011 agui-error` 仍留 todo。实现、真 PostgreSQL 证据和本文件的结论没有漂移，但机器台账身份是错的，不能因为 done 总数仍增加 1 就忽略。

R168 / correction `2050fab0369bbc1537f94347eb1f74b75ffb5820` 已把同一 evidence 移到 `T-EVT-0011` 并恢复 `T-EVT-0003` 为 todo。修前与修后 events 总数都为相应阶段的 36 done；变化的是证据归属，不是计数。独立记录见 `docs/2026-09-03-G4-AGUI-error-ledger-identity-fix-batch94.md`。
