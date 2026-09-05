# G4 AG-UI Reasoning Terminal Retention（Batch93）

> 日期：2026-09-03（America/Los_Angeles）
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §7.5、§14.3、§16.5、§17.2、§24、§25、§28.1
> 仓库纪律：`CLAUDE.md`
> 基线：Batch92 / PR #75 head，`957fc1da331da4b080810d4cb95349be657b0622`
> implementation：`d39e6a4bf2c18414bdd6803cb1ca0d321a9c3d48`

## 1. 本批关闭的真实缺口

AG-UI reasoning 的 decoder、provider-neutral visible delta、PostgreSQL semantic chunk 与 UI 隐藏投影都已存在，但 `T-EVT-0008` 仍明确要求单独裁决内容保留期。Batch92 后的实际行为是：reasoning 不物化进 assistant transcript，却仍永久留在 `run_events.payload`；UI 不展示并不等于数据已经清除。

本批固定以下边界：

- active run 为 expected-sequence crash recovery 保留可重放 visible reasoning；
- 任一 completed / failed / cancelled / reconciliation_required terminal 必须在同一事务、terminal 对外可见前，把全部 reasoning chunk 收敛为固定无内容 marker；
- marker 为 `{"channel":"reasoning","delta":"","retained":false}`，事件行、run/thread sequence、cursor 与 terminal 事实保持；
- `REASONING_ENCRYPTED_VALUE.encryptedValue` 继续在 decoder 边界直接丢弃，不能进入 provider-neutral event；
- 升级前已经终态的数据由 native0027 做同一历史回填；active run 与 text chunk 不得改变。

这里关闭的是 PostgreSQL 当前可见值的逻辑保留期。PostgreSQL WAL、备份、只读副本、快照与灾备介质上的物理留存/擦除仍属于 G8 retention/runbook，本批没有也不会冒充该证据。

## 2. 原子运行时边界

所有正常完成、provider failure、用户取消、排队期取消、dead-letter 与 stale-lease reconciliation 最终都进入 `finish_run_in_transaction`。该统一入口现在按以下顺序提交：

1. 从 text channel 聚合最终 assistant transcript；
2. 在同一事务把该 run 的 reasoning payload 覆盖成固定 marker；
3. 物化 text-only assistant message；
4. 写唯一 terminal event，推进 run/thread sequence，释放 lease 并通知。

任一步失败都会回滚整个事务，不存在 terminal 已提交而 reasoning 仍可读的中间窗口。terminal exact replay只核对既有 terminal/text materialization，不会恢复被清除的 reasoning。event row 不删除，因此 durable cursor 和重连 sequence 不产生洞。

## 3. native0027 历史回填与 schema fixture

`native_0027_terminal_reasoning_retention` 是 native migration 框架中唯一具名的数据覆写例外。SQL 只允许一次 `UPDATE public.run_events`，并同时要求：

- `event_type='semantic_chunk'`；
- `payload.channel='reasoning'`；
- 关联 run 已处于四种 terminal status 之一；
- payload 尚不是固定 marker。

机械单测继续对 0013–0026 执行 expand-only schema 禁令，并对 0027 单独锁死一条 UPDATE、固定 SET/WHERE、零 DDL、零 DELETE、零其它 DML、零 `IF NOT EXISTS`。0027 只改数据，post schema fixture 与 0026 逐字节相同：46 表、455 列、326 NOT NULL、253 约束、92 索引，5,341 行，SHA-256 `ad5375da9abc5d03f1fa9587f5efda3e76e2cb89edf470e3bc4650a58670ba2c`；native ledger 由 14 变 15。

fixture 台账新增 `T-FIX-0043`，因此 fixtures 从 `20/22/42` 变为 `21/22/43`。

## 4. 真实 production vertical

同一 production remote Agent 继续经过 `SafeRemoteAguiTransport → SafeDialer → SSE → AguiDecoder → RemoteAguiProvider → BuiltInAgentRuntime → PostgresRunRuntime`。服务端发送：

- visible reasoning summary `checked evidence`；
- encrypted reasoning canary `ENCRYPTED_REASONING_CANARY`；
- text `remote answer`；
- completed terminal。

最终 PostgreSQL 结果为：assistant text 与 completed terminal 保持；reasoning marker 恰 1；`checked evidence` 与 encrypted canary 在 `messages.content`、`run_events.payload`、`audit_events.payload` 联合扫描命中均为 0。package provider call 仍为 0。

独立 run-runtime 真库用例先在 terminal 前逐字段读出 active reasoning canary，再提交 terminal，随后只读到固定 marker；terminal exact replay不复活内容，text 仍为 `hello world`。

## 5. 验证结果

- PostgreSQL 17.11 native0027 regeneration 开 / 关：各 `1/0/0`；四种历史终态 marker=4，active 原值、text、event identity/sequence/terminal/time 不变，ledger=15。
- PostgreSQL 17.11 `run_runtime` 完整矩阵：`5/0/0`。
- remote AG-UI + SafeDialer/SSE + Agent + PostgreSQL 定向 production vertical：`1/0/0`。
- `openbot-infra` lib：宿主权限 `324/0/0`。
- `openbot-agent`：`47/0/0`。
- `openbot-testkit`：`17/0/9 ignored`；ignored 的 remote PG 定向用例已按上一条单独真跑。
- UI live text/reasoning-hidden/terminal-reload 定向：`1/0/0`。
- Infra/Testkit/Server/Desktop all-target/all-feature Clippy `-D warnings`：通过。
- `cargo fmt --all -- --check`：通过。
- `cargo xtask parity-check`：parity=`825/879/1704`、events=`37/51/88`、fixtures=`21/22/43`、overlay=`1293/403/2/6`、0 violation。
- `cargo xtask recount`：`71/0/89 skipped`；89 条全部因为未设置固定 `OPENBOT_UPSTREAM_DIR`，因此 strict 未冒充通过。
- `cargo xtask grok-inventory --check`：2,110 files，reference tree 未变；Git tree仍为 `86f5a85f560f721677fa7e587a67ac0ffc036cb5`。
- 非 Grok `package.json` 恰 1；本批无 npm、无 Cargo.lock/依赖、无 API/env/视觉变更；Actions 仍只有 `workflow_dispatch`。
- 临时 PostgreSQL 已 fast stop，精确 data/socket 根目录已删除，55493 无 listener。

## 6. 首跑与环境事实

- 新 native guard 最初把 SQL 注释中的分号也计入 statement 数，得到 `2 != 1`；改为只统计去注释后的 statement lines 后，完整 Infra 单测重跑通过。
- 沙箱内完整 Infra lib 的 15 条 loopback bind 用例均以 `Operation not permitted` 失败；在宿主权限下原命令重跑为 `324/0/0`，没有把沙箱失败算作产品失败或成功。
- 沙箱内 `initdb` 因 shared memory `Operation not permitted` 失败；同一精确 `/tmp` 目录在宿主权限初始化后完成真实测试并清理。
- 第一次误把 `--locked` 传给 `cargo xtask parity-check` 子命令，被参数解析明确拒绝；改用仓库规定的 `cargo xtask parity-check` 后通过。
- 第一次 non-strict recount 正确发现 fixtures 顶部四个自复算期望仍是 42/20/42/32，得到 `67/4/89 skipped`；同步为43/21/43/33后重跑为`71/0/89 skipped`。
- UI 第一次用 `--exact` 但未带模块全名，匹配 0 条；移除错误过滤后实跑 1 条通过。

## 7. 本批没有声称完成

- 只把 `T-EVT-0008 agui-reasoning` 与 `T-FIX-0043 db-schema-0027` 标为 done；AG-UI state/messages/activity/step/raw-custom/interrupt-resume/tool-result 等仍按机器台账保持 todo。
- 没有关闭完整 G3、G4 或 G8；computer runtime budget、三家 recorded/live provider trace、Browser/file/shell、Desktop OAuth、MCP private egress/admin、外审/KMS、发行/golden 等仍未完成。
- strict recount 因没有配置固定上游目录未跑；按 R63 未运行 `cargo xtask ci`，未派发 GitHub Actions。
- 本批不证明 WAL/backup/replica 的物理擦除，不改变现有 backup retention，也不把一次 Batch 或 PR 当成 v4 全量完成。
