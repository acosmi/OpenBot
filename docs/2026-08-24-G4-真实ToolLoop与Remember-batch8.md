# G4 Batch 8：真实 Tool Loop 与 Remember

> 日期：2026-08-24（America/Los_Angeles）
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §4.3、§7.2–§7.4、§8.1–§8.6、§24、§28.1 R71
> 堆叠基线：PR #24 head `27585571339dfc4f678098011a5538529a9e7c32`
> 当前分支：`feat/2026-08-24-G4-tool-loop-remember`

## 1. 本批闭合边界

本批把此前“能解析 tool call，但固定失败”的 Rust Agent 接成第一条真实 production tool loop：

```text
provider complete tool batch
→ stable output-index ordering
→ fresh DB AuthContext
→ metadata/schema + authoritative target
→ CEL policy
→ decision + attempt
→ single-use capability CAS
→ origin=remember_tool PostgreSQL memory effect
→ outcome + memory audit
→ assistant/tool messages + checkpoint
→ context reload
→ next provider sampling
→ terminal
```

内建 `remember` 是第一个真实 executor。它不代表 RMCP、Drive、browser、file、shell 已完成，也不代表 live vendor trace。

## 2. 关键裁决

### 2.1 两种 call identity 必须分开

- provider call id 只用于 assistant call 与 tool result 配对；
- Rust gateway 另铸 UUIDv7 + per-run sequence，作为 decision/attempt/capability 的唯一 control-plane identity；
- provider/model 没有 actor、owner、target、source、policy、effect、generation 或 capability 的参数位。

### 2.2 完整 sampling 才执行 tool

host 不在第一条 `ToolCallCompleted` 到达时立即执行。它先等当前 provider stream 正常 `Usage + Completed`，收齐完整 batch，再按稳定 output index 排序。重复 index/call id、缺 usage、半截 JSON、断流均在 effect 前 fail-closed。

首个 `remember` 声明 `parallel_safe=false`，因此严格串行；一般规则仍是只有 metadata 明示 parallel-safe 且资源锁不冲突才可并行。跨 sampling 的调用总数由 pure reducer 累计，超过 8 时新 effect 数为 0。

### 2.3 Durable tool pair

每个确定 outcome 在一个 PostgreSQL transaction 里写：

- assistant message（前一 sampling text + structured tool call）；
- tool message（redacted result + callId/name/stable error code）；
- `run_events.checkpoint(kind=tool_exchange)`，只含 control id、provider id hash、args/result hash 与稳定 metadata；
- run/thread/message sequences 与 NOTIFY。

terminal 只聚合最后一个 tool checkpoint 之后的 text chunks，避免前一 sampling 文本重复。相同 expected sequence + 同 exchange 精确 replay；参数/结果改变必须 conflict。

### 2.4 Remember catalog 与 provenance

provider schema 与 `ToolMetadata.schema_hash` 来自同一 JSON object。OpenAI 固定 `strict:true`，所以五个 properties 全在 required：

- `memoryKind = preference | fact`；
- `scope = user | bot | thread`（只有 scope class，没有 ID）；
- `content`；
- `tags`；
- `sensitivity = normal | sensitive`。

owner、Bot/thread ID、Fact source message/thread 与 `origin=remember_tool` 全由当前 run/数据库构造。Fact source 固定取当前 run 的 durable user message；模型不能自报 provenance。没有后台抽取 job。

### 2.5 撤权竞态

异步 Agent tool effect 不复用旧 session bytes：每次 effect 前，`PostgresAgentAuthorizationSource` 以 active run/lease 重新读取 user role、revocation 与 auth generation，构造不可序列化 `AuthContext`。

前置读取仍不足以堵住 TOCTOU，因此 AuthGeneration 随 tenant/run/thread/actor 一起封入 `AuthorizedToolCall`。memory writer 在真正 INSERT 的同一 transaction 再比较 generation、revoked 与 role：

- 相同 generation → 可继续；
- capability mint 后 generation 改变 → definite `not_committed`，memory 不新增；
- commit unknown / tool future 在执行中被外层取消 → run reconciliation，不伪装 failed/cancelled success。

### 2.6 三种 memory audit

- `memory.remember_refused`：policy/approval 在执行前拒绝；无 decision/attempt/executor；
- `memory.remember_succeeded`：effect committed；
- `memory.remember_failed`：已有 decision/attempt/capability，但得到确定非成功 outcome；
- unknown commit 不进 failed，而进 run reconciliation。

audit payload 不含 memory content、provider args 或 model-visible full result。

## 3. 生产落点

| 子面 | Rust 落点 |
| --- | --- |
| remember catalog/schema/port | `crates/openbot-application/src/builtin_tools.rs` |
| authoritative execution scope | `crates/openbot-application/src/tool.rs` |
| host loop/batch/order/8-step | `crates/openbot-domain/src/agent.rs`、`crates/openbot-agent/src/runtime.rs` |
| fresh ACL + control plane | `crates/openbot-infra/src/agent_tools.rs` |
| gateway UUIDv7/sequence | `crates/openbot-agent/src/gateway.rs` |
| memory effect/provenance/generation CAS | `crates/openbot-infra/src/memory_admin.rs` |
| tool outcome audit | `crates/openbot-infra/src/repo/tools.rs` |
| durable tool messages/checkpoint | `crates/openbot-application/src/run_runtime.rs`、`crates/openbot-infra/src/run_runtime.rs` |
| context pair validation | `crates/openbot-infra/src/provider/context.rs`、`provider/common.rs` |
| three-provider reinjection | `provider/openai.rs`、`provider/anthropic.rs`、`provider/google.rs` |
| production assembly | `crates/openbot-server/src/main.rs` |

Production main 不再使用 `NoToolControlPlane/NoToolJournal`；它注入与管理 API 相同的 `PolicyStore`、真实 `PostgresToolJournal`、memory store 与同一个 `ApplicationService` Arc。

## 4. 本机证据

本批遵守 R63：没有运行 `cargo xtask ci`，没有派发 GitHub Actions。

| 验收 | 结果 |
| --- | --- |
| `cargo test -p openbot-agent --all-features --locked` | **18/0/0** |
| `cargo test -p openbot-application --all-features --locked` | **119/0/0** |
| domain Agent / audit catalog | **3/0/0** / **3/0/0** |
| infra provider filter | **45/0/0**，含 closed tool-pair 与三家 request shape/loopback |
| Server main / agent config | **7/0/0** / **6/0/0** |
| PG17.11 Agent/provider/tool-loop | **5/0/0** |
| PG17.11 generic tool journal | **5/0/0** |
| PG17.11 explicit memory | **2/0/0** |
| 六 crate all-targets/all-features Clippy `-D warnings` | exit 0 |
| fmt / diff / SafeDialer guard | exit 0 / exit 0 / exit 0 |
| Cargo.lock package 数 | **428**（只新增已有 `thiserror` direct edge） |
| parity | **323 done / 1337 todo / 1660 total**，0 violations/warnings |
| strict recount | **154/154/0** |

PG remember 用例在同一个真实 run 里连续证明：

1. allow policy：committed memory 1 条，Fact source/Thread/owner/Bot 全由 DB 绑定，audit=`memory.remember_succeeded`；
2. capability mint 后推进 auth generation：writer 同事务拒绝，memory 总数仍 1，attempt=`not_committed`，audit=`memory.remember_failed`；
3. 同一 PolicyStore 切换 deny：只写 `memory.remember_refused`，decision/attempt 总数仍 2，memory writer call 总数仍 2；
4. 三个结果按顺序成为三组 assistant/tool messages 与 checkpoints，第四次 sampling 正常 completed；
5. 第一个 exchange exact replay=`true`，tampered result=`Conflict`。

测试 provider 是 deterministic external test double，只替代不可在本机无凭据调用的模型服务；Agent runtime、ApplicationService、CEL、tool journal、capability、memory executor、PostgreSQL 与 audit 都是 production 实现。它不能被记为 live vendor fixture。

构建缓存再次接近磁盘上限时执行了 `cargo clean`，只删除 7.0 GiB 可重建 `target/` 产物；源码、数据库与用户文件未删除。

## 5. 台账变化

| 台账 | Batch 7 | Batch 8 |
| --- | ---: | ---: |
| events | 7 done / 67 todo / 74 | **10 / 67 / 77** |
| env | 49 / 25 / 74 | 不变 |
| tests | 184 / 863 / 1047 | 不变 |
| fixtures | 10 / 22 / 32 | 不变 |
| parity 总计 | 320 / 1337 / 1657 | **323 / 1337 / 1660** |

新增三条都是本项目 explicit memory audit；没有 synthetic provider 测试被登记为 upstream test/fixture。

## 6. 明确未完成

- 三家 recorded/live vendor trace：**0/3**；
- human proof-of-intent / approval GUI；当前 backend 仍以 default-deny policy 为安全边界，catalog description 不是授权；
- run-wide input/output token、费用、并发 tool 与 computer runtime budget；
- remote AG-UI；
- RMCP 3.1.4 / server-side-tools 五条；
- Drive、browser、file、shell executor；
- G5–G8 / Memory GUI / migrations/releases。

因此只勾 §24.1 的 tool host 与 remember backend 子项，**G4 整关保持未通过**。

## 7. 恢复点

- implementation commit：`647350caab234487e4cc2508c8a628d955e4746d`；
- 分支：`feat/2026-08-24-G4-tool-loop-remember`；
- PR：[#25](https://github.com/acosmi/OpenBot/pull/25)；
- base：`feat/2026-08-24-G4-anthropic-google-retry`（PR #24 head）；
- 创建后机器实得：`OPEN / CLEAN / MERGEABLE`，`statusCheckRollup=[]`；
- implementation head Actions run 数：**0**；
- 父 PR #24 同轮复核仍为 `OPEN / CLEAN / MERGEABLE`。

堆叠链尚未进入 `main`；合并必须继续按 baseRefName 依赖顺序使用 merge commit。
