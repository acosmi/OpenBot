# OpenBot Rust 代码质量与逻辑审计复核报告

> 复核日期：2026-09-03（America/Los_Angeles）
> 复核基线：`main@2accbbe4364618bec5c0d323124f9b828c570448`
> 输入材料：`docs/2026-09-01-OpenBot-Rust代码质量与逻辑审计报告.md`
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §24、§28.1 R1–R164 及 `CLAUDE.md`
> 修复实现：`d97f232bdd497057a028203c63e7394ff9667833`

## 结论

输入报告的八项问题中，只有两项形成当前代码回归并需要立即修复：

1. 已激活但仍在等待 Agent 并发许可的 run 收到取消时，没有提交 durable `Cancelled` terminal；
2. Batch89 新增 `GetRunCostBudget` / `ReplaceRunCostBudget` 后，`transport_parity.rs` 的无通配穷举未同步，真实触发 `E0004`。

两项均已修复并由先红后绿或编译失败后绿的定向证据闭合。其余结论属于四类：第一真源已明确登记的未完成面、发布前故意 fail-closed 的门禁、已被现有实现消除的旧风险、或被报告误判为缺陷的已披露边界。不得为了让报告“变绿”而移除安全门禁、改写 CEL 迁移裁决或绕开单链审计不变量。

## 逐项复核

| 输入项 | 事实判定 | 处理 |
| --- | --- | --- |
| Agent 并发排队取消黑洞 | **属实，已修复**。`BuiltInAgentConsumer::revoke` 对已激活 reservation 返回 `ChildSignalled`；旧 `execute_activation` 在取得 semaphore 前只调用 `cleanup`，没有经过 reducer 与 `DurableTextRun::finish`。新增回归测试在修复前稳定 1 秒超时。 | 新增 `cancel_before_execution`，从 `Queued → Cancelling → CommittingResults` 提交 `RunTerminal::Cancelled`，随后释放 per-run tool sequence 并清 reservation。 |
| `openbot-computer` 没有生产反向依赖 | **事实为真，但不是已完成功能的回归**。§24 G5、R114、R118–R129 已明确把 CDP/ScreenHub、browser/file/shell、Server runsc 与 Desktop renderer 列为未完成；当前不得宣称 Computer 已装配。 | 保持 todo。P1 Windows/runsc 真机证据未齐，不能以直接接线绕过阶段门。 |
| 多用户 readiness 固定 `computer_isolation=NotReady` | **代码事实为真，缺陷判定错误**。§24 G5 与发布级不变量要求多用户 Server 的 runsc isolation 起不来就 readiness 失败。当前隔离未装配时返回 503 是 fail-closed，而不是应删除的生产阻断。 | 不修改。改成 Ready 或删除 probe 会构成安全回归。 |
| 全局 audit advisory lock 与时间戳交叉误杀 | **锁存在；报告描述的生产时钟竞态不成立**。单一 hash chain 必须串行取得前驱。生产调用先在同一事务中调用 `next_event_coordinates`；该函数先取得同一 transaction advisory lock，再用 PostgreSQL `clock_timestamp()` 铸造严格晚于表尾的坐标，随后 `append_event_in_transaction` 重入同一锁并提交。报告建议的权威数据库时钟已在实现中。 | 不修改协议。全局单链可能是未来需基准测试的吞吐风险，但本轮没有测量证据证明 SLA 失败，不能把性能假设当 correctness 缺陷。 |
| 工具仅 remember/MCP、Approval reducer 死分支、多工具串行 | **混合误判**。browser/file/shell 是已登记 todo；acting tool approval 已在 `openbot-application::tool` 调用 `ToolControlPlane::approval`，生产实现通过 `PostgresToolApprovalCoordinator::request_and_wait` durable 挂起，并非无法审批。当前 production tool metadata 全部 `parallel_safe=false`，按 §7.4 必须稳定顺序执行。 | 不改现有串行语义。通用 `parallel_safe=true` 调度、resource-lock 冲突检测和 computer runtime budget 继续是明确 todo；`AgentEvent::ApprovalRequired` 未被 runtime 使用可记为状态模型清理项，但不能据此否定现有 durable approval 管线。 |
| CEL 六条 `error → non-error` | **差异属实，“静默”判定错误**。六条由封闭 `DIVERGENCE_LEDGER` 双向钉死；迁移 preflight 会标方向、`requires_operator_confirmation=true`，多一条或少一条都判红。§8.3/R28 明确允许标准 CEL 方法超集，同时禁止未经确认切换 writer。 | 不做语义补丁。定向 corpus 测试 6/0/0，证明差异集合与 operator-confirmation 机制均在执行。 |
| `HeaderValue` 不会 zeroize | **底层副本事实属实，不构成已承诺保证的破坏**。`SecretBytes` 文档只保证自己当前 allocation 的 length/capacity，明确不保证类型边界外调用方副本；网络发送必然把明文交给协议栈。`HeaderValue::set_sensitive(true)` 负责 Debug/trace 脱敏，不宣称物理擦除通用分配器。 | 不引入 unsafe 内存篡改或虚假的“零驻留”承诺。后续文档应继续区分 owned secret zeroization、协议栈副本与 OS/root 对手模型。 |
| transport `E0004` 与 Desktop dead-code Clippy | **前半属实并已修复；后半本轮不可复现**。基线上的 transport test 确实因两个预算命令未覆盖而编译失败；`cargo clippy -p openbot-desktop --all-targets --all-features -- -D warnings` 本轮通过。 | 在无 wildcard 的专项排除臂中显式登记两个预算命令；保留穷举棘轮。未对 Desktop 方法做无依据删除/allow。 |

输入报告另称 SafeDialer 缺少“单 IP 超时”。当前 `SafeDialer::execute` / `execute_stream` 已以 `SafeHttpBudget::timeout` 包住包括 DNS、逐 IP connect、TLS、响应头在内的整个请求；首个黑洞地址最多消耗本次请求的总预算，不会越过 run/request deadline。因此它是是否要细分预算的优化议题，不是无界等待缺陷。

## 修复后的机械证据

- 修复前：`cargo test -p openbot-agent revoke_activated_run_waiting_for_concurrency_commits_cancelled --locked` 为 `0 passed / 1 failed`，失败原因为等待 durable terminal 超时；修复后同一测试 `1/0/0`，完整 `openbot-agent` 为 `39/0/0`。
- 修复前：`cargo test -p openbot-testkit --test transport_parity --locked` 以 `E0004` 拒绝编译；修复后 `8/0/0`，其中 `every_command_variant_is_accounted_for` 保持无通配穷举。
- `cargo clippy -p openbot-agent -p openbot-testkit --all-targets --all-features -- -D warnings`：通过。
- `cargo clippy -p openbot-desktop --all-targets --all-features -- -D warnings`：通过。
- `cargo test -p openbot-domain --test cel_corpus_parity --locked`：`6/0/0`。
- `cargo test -p openbot-infra net::safe_http::tests::total_deadline_includes_waiting_for_response_headers --lib --locked`：`1/0/0`。
- `cargo xtask parity-check`：parity `823/881/1704`、fixtures `20/22/42`、overlay `1293/403/2/6`，0 violation；本批未改变任何 T-ID 状态。
- `grok-bot` tracked files=`2110`、tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`；非 Grok `package.json` 恰一份且零 lockfile；本批没有修改 `grok-bot`、Cargo.lock 或 workflow。

## 仍然成立的发布判断

本次复核不改变 §24 的总判断：G2/G3/G4/G5/G6/G7/G8 仍未整关闭合，项目仍不可宣称多用户生产就绪。尤其不得把本批的 run 排队取消修复冒充“并发 tool/computer runtime budget 已完成”；该完整预算、Computer 生产装配、Windows/runsc 真机证据与 browser/file/shell executor 仍是后续工作。
