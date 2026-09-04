# G4 并发 Tool Runtime Budget（Batch91）

> 日期：2026-09-03（America/Los_Angeles）
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §7.4、§24、§28.1
> 基线：Batch90 / PR #73，`985836725fe625b3f57e2558c9ebff782e5c3866`
> implementation：`f71087fc0aad923a0c94a55578fcb529930587a7`

## 1. 本批关闭的边界

Batch89 已闭合 token/cost/user cap，Batch90 又修复 run 在 provider concurrency semaphore 前取消不落 terminal；但一次 sampling 的多个 tool call 仍无条件逐个串行，`ToolMetadata.parallel_safe/resource_locks` 没有 runtime 消费者，也没有跨 run 的 tool in-flight 上限。

本批只关闭 built-in Agent 的并发 tool runtime budget，不接入或冒充 Computer/browser/file/shell：

1. 只有 Rust build-owned scheduling metadata 明确 `parallel_safe=true` 的调用可并行；
2. 同一 wave 的 resource lock key 必须两两不冲突；
3. 全部 run 共享一个 process-wide tool semaphore；
4. 结果仍按 provider stable output index 持久化并回注；
5. budget 等待期、effect 已启动后的 cancel/deadline/lease/unknown commit 分型保持 fail-closed。

## 2. 权威调度元数据

`AgentToolScheduling` 是 host-only、non-serde 的 first-party contract。默认值永远是 serial；MCP annotation、模型自述、数据库 description、sandboxed component 名字与 renderer 输入都不能把它改成 parallel。

当前 production allowlist 只有 11 个 ordinary compiled component。`askApproval` / `askChoice`、`remember`、全部 MCP/Drive、`custom_*` sandboxed component 和 unknown name 均保持 serial。allowlist 与 13 项 build manifest/11 项 ordinary confirmation 双向测试：新增 renderer 若未明确审查 scheduling，测试当场判红。

一次 parallel declaration 的 resource locks 使用领域层既有 `ResourceLockKey`，先排序、去重；超过 32 个不截断、不放宽，而是保守降为 serial。真正的 policy/approval/capability/action authority 没有搬进 Agent：每个 invocation 仍重新加载 `AuthContext` 并穿过原有 typed `ApplicationService`。

## 3. 有界调度与资源锁

`BuiltInAgentConfig.max_tool_concurrency` 是独立于 provider run concurrency 的 process-wide cap：默认 8，只接受 `1..=256`。本批不新增环境变量，Server/Desktop 继续使用同一个默认配置。

同一 sampling 先按 provider output index 收齐 complete calls，再形成稳定 waves：

- serial/human call 单独成 wave；
- parallel-safe 且 lock set 不相交的相邻调用进入同一 wave；
- 遇到 lock 冲突先完成已有 wave，再开始下一 wave；
- 跨 run 相同 lock 由 runtime-owned keyed semaphore 串行，互不冲突的 key 可继续并行；
- keyed semaphore 只在活跃 owner 存在时保留 strong reference，避免长期积累动态 lock owner。

每个实际调用必须同时取得 resource permits 与全局 tool permit。单元测试以四个并行调用和 cap=2 证明峰值恰为 2；另以相同/不同 resource key 的正反向测试证明只串行冲突集合。

## 4. 取消、期限与持久顺序

- 等待 tool permit 时收到 cancel：调用尚未开始，零 tool effect，走正常 `Cancelling → Cancelled`。
- parallel wave 中任一调用已取得 permit 后再 cancel/deadline：先 abort 并 drain 全部 child；由于 effect commit 可能未知，进入 `ReconciliationRequired/JournalCommitUnknown`，不得伪造 Cancelled。
- human decision 先取得 tool permit再 spawn；取消后 detached durable waiter 仍占 permit，直到 PostgreSQL 观察 terminal 并完成 retirement/audit，避免预算外孤儿。
- parallel child panic、错 position 或 join 丢失统一先停其余 child，再进入 reconciliation。
- 全部成功结果先按原 output index 重排，然后逐条写 assistant/tool exchange；下一次 sampling 只能在完整 batch durable 后开始。

## 5. 验证

- `cargo test -p openbot-contracts --locked`：`101/0/0`。
- `cargo test -p openbot-agent --locked`：`46/0/0`。
- `cargo test -p openbot-testkit --locked`：`17/0/9 ignored`。
- 临时 PostgreSQL 17.11 宿主回归 `agent_runtime_postgres`：`9/0/0`，包括新增真实 production `AuthorizedAgentToolGateway → ApplicationService → PostgreSQL` 双 component 并发纵向；峰值 `DecideComponent=2`，provider context 与数据库 tool message 顺序均为 quote→notice。
- Contracts/Agent/Testkit/Server/Desktop 五 crate `--all-targets --all-features -D warnings`：通过。
- Contracts `wasm32-unknown-unknown`：通过。
- `cargo fmt --check`：通过。
- `cargo xtask parity-check`：parity=`823/881/1704`、fixtures=`20/22/42`、overlay=`1293/403/2/6`、0 violation。

临时 PostgreSQL 在 sandbox 首次因本机 socket 权限不可连接，该次不计通过；宿主重跑全绿。实例已 fast stop，data/socket 临时目录已删除。

## 6. 明确未声称完成

- Computer runtime budget、browser/file/shell executor及其协议级 cancel 仍 todo。
- 当前 production parallel allowlist 不包含 MCP/Drive；它们的动态 metadata/resource identity 尚未形成可跨 prepare→execute 防漂移的权威 proof，因此继续 serial。
- 没有新增 API、schema、migration、T-ID、依赖、Cargo.lock、UI、bundle 或环境变量。
- native latest 仍为 `0026`，schema 仍为 46 表/455 列/253 约束/92 索引。
- 未配置固定上游目录，strict recount 未跑；按 R63 未运行 `cargo xtask ci`，未派发 GitHub Actions。
- `grok-bot`、零 npm、单一非 Grok `package.json` 与 manual-only workflow 均保持不变。
