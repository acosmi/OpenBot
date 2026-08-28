# G4 Batch 7：三 Provider、Retry、Token Budget 与生命周期审计

> 日期：2026-08-24（America/Los_Angeles）
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §6.4、§7.2–§7.5、§8.6、§15.4、§24、§28.1 R70
> 堆叠基线：`feat/2026-08-24-G4-agent-openai-provider` 的文档提交 `aad5dbdbaa082d32769c889b7a5966e390183f0c`
> 当前分支：`feat/2026-08-24-G4-anthropic-google-retry`

## 1. 本批闭合边界

本批只闭合以下可由本机生产代码、真实 loopback HTTP/SSE 与 PostgreSQL 17 证明的子面：

1. managed Anthropic Messages 与 Google `streamGenerateContent` 两个独立 Rust adapter；
2. package / managed 两层权威路由，managed 缺失时绝不回落 package provider 或 package key；
3. pre-stream unavailable、首事件 429/明确 5xx 的有界 retry，以及 `Retry-After`；
4. 每次 sampling 的 output token cap、三家 usage 归一化与 host 二次校验；
5. `agent.invoked`、`agent.stream_stalled`、新增 `agent.run_deadline_exceeded` 的 production PostgreSQL hash-chain audit；
6. 三家 provider key/header、redirect 与 SafeDialer 边界。

没有 vendor 凭据的 synthetic loopback stream 只能证明 wire parser、HTTP 请求与 production assembly，不能冒充 live vendor/recorded fixture。`fixtures/MANIFEST.yaml` 的三家 provider fixture 因此仍是 **0/3**。

## 2. 固定协议与第一性裁决

### 2.1 Anthropic

- endpoint 固定为 Messages `/v1/messages`；
- key 只进入 `x-api-key`，并发送 `anthropic-version: 2023-06-01`；
- system 与 messages 分离；连续同角色消息按 vendor 形状合并；tool use/result 不伪装成普通文本；
- 接受 thinking/text、content block skeleton、partial tool JSON、usage、ping 与未知扩展；顺序、字段或终态 usage 损坏即 fail-closed；
- 未显式给 output cap 时，默认表按锁定 `@langchain/anthropic@1.5.6`，不是按当前网页猜测。

### 2.2 Google

- endpoint 固定为 `v1beta/models/{model}:streamGenerateContent?alt=sse`；
- key 只进入 `x-goog-api-key`，禁止出现在 URL/query；
- system、contents、function declarations、function response 分域；
- 锁定 `@google/generative-ai@0.24.1` 的 stream DTO 没有稳定 response id，因此对首个规范 chunk 做 SHA-256，生成只用于 trace/correlation 的确定 id；该 id 不参与授权、ACL 或幂等绑定；
- usage 必须单调且 total 不小于已知 input+output；多 candidate、usage 回退、坏 finish/error shape 均 fail-closed。

### 2.3 Retry 与 unknown commit

锁定 `@langchain/core@1.2.8` 的默认：首次请求后最多 6 次 retry、1s×2、`[1,2)` jitter、指数项最多 64s；`Retry-After` 与指数值取较大者，外层 absolute run deadline 始终是总上限。

允许 retry 的只有：

- provider session 建立前的 `Unavailable`；
- session 首事件就是规范化 429 或明确 5xx。

认证、schema、policy、已发送未确认的 `CommitUnknown`、任何 response identity/text/reasoning/tool delta 之后的断流均不重放。这样不能把一次可能已被 vendor 接受的 sampling 偷偷执行两次。

### 2.4 Token 与 audit

新增 `OPENBOT_PROVIDER_MAX_OUTPUT_TOKENS`：缺省 16384，只接受 `1..=1000000`，`0`、负数、小数和越界值均启动报错；三家 request 都携 cap。adapter 必须在 `Completed` 前给 usage，host 再拒绝缺失、重复、不自洽与超限 usage。

这只是每次 sampling 的 output cap。真实 tool loop 落地后仍须累计整个 run 的多步 input/output token 与费用，不能把本批写成“完整 budget”。

生命周期 audit 的顺序固定为：

```text
durable activate → agent.invoked → context/provider
real body read gap → drop session → agent.stream_stalled → failed terminal
absolute deadline → drop child/session → agent.run_deadline_exceeded → Cancelling → Cancelled
```

audit payload 只有权威 actor/run 与 allowlisted stable code；prompt、provider body、tool arguments、secret 不进入 payload。audit 写失败进入 reconciliation，不继续 sampling 或提交普通终态。

## 3. 生产实现落点

| 子面 | Rust 落点 |
| --- | --- |
| Anthropic | `crates/openbot-infra/src/provider/anthropic.rs` |
| Google | `crates/openbot-infra/src/provider/google.rs` |
| OpenAI usage 收紧 | `crates/openbot-infra/src/provider/openai.rs` |
| provider 共用验证/Retry-After | `crates/openbot-infra/src/provider/common.rs` |
| SafeDialer provider auth/redirect | `crates/openbot-infra/src/net/safe_http.rs` |
| package/managed route | `crates/openbot-agent/src/provider_router.rs`、`crates/openbot-infra/src/provider/context.rs` |
| retry | `crates/openbot-agent/src/retry.rs` |
| host budget/deadline/stall | `crates/openbot-agent/src/runtime.rs` |
| lifecycle audit port/writer | `crates/openbot-application/src/provider.rs`、`crates/openbot-infra/src/agent_audit.rs` |
| env 与 production factory | `crates/openbot-server/src/config/agent.rs`、`crates/openbot-server/src/main.rs` |
| managed tenant projection | `crates/openbot-application/src/tenant/package.rs`、`crates/openbot-infra/src/tenant/postgres.rs` |

`Cargo.lock` 仍为 **428 packages**。本批只增加已经存在于锁图的 `httpdate`、`getrandom`、`metrics` 直接依赖边，没有下载或升级 package。

## 4. 本机机械证据

本批遵守 R63：没有运行 `cargo xtask ci`，没有派发 GitHub Actions。

| 验收 | 结果 |
| --- | --- |
| `cargo check`（agent/infra/server/testkit，all-targets/all-features/locked） | exit 0 |
| `cargo test -p openbot-agent --all-features --locked` | **16/0/0** |
| domain Agent reducer | **3/0/0** |
| infra provider filter | **44/0/0**；其中 Anthropic 5、Google 4、OpenAI 6 |
| SafeDialer 定向 | **14/0/0** |
| Server agent config | **6/0/0** |
| Server production main factory | **7/0/0** |
| application tenant package | **19/0/0** |
| fintech fixture | **1/0/0** |
| PostgreSQL 17.11 Agent/provider/audit | **4/0/0** |
| PostgreSQL 17.11 tenant sync | **8/0/0** |
| 六 crate all-targets/all-features Clippy `-D warnings` | exit 0 |
| `cargo fmt --all -- --check` / `git diff --check` | exit 0 / exit 0 |
| `bash tools/check-safe-dialer-dependencies.sh` | exit 0 |
| `cargo xtask parity-check --json` | **320 done / 1337 todo / 1657 total**，0 violations/warnings |
| strict recount（固定上游 commit） | **153/153/0** |

PG 用例使用本机临时启动的 PostgreSQL **17.11 Homebrew trust** 实例与随机临时库，结束后实例已关闭。这不是新增 SCRAM 证据；既有 SCRAM 证据仍按 R62–R69 保留。

PG Agent 四条分别证明：

1. package provider delta → durable journal/history/terminal + invoked audit；
2. real OpenAI loopback SSE + fresh Vault credential + reasoning/text 分流；
3. 同一 managed Agent row 依次走 Anthropic、Google，package provider 调用数精确为 0；
4. HoldingContext 的 absolute deadline 与真实 stalling SSE，得到 `invoked → deadline → invoked → stalled` 四行 hash chain，audit 在 terminal 前。

## 5. 台账变化

| 台账 | Batch 6 | Batch 7 |
| --- | ---: | ---: |
| env | 37 done / 36 todo / 73 | **49 / 25 / 74** |
| events | 4 done / 69 todo / 73 | **7 / 67 / 74** |
| tests | 184 / 863 / 1047 | **不变** |
| fixtures | 10 / 22 / 32 | **不变** |
| parity 总计 | 305 / 1350 / 1655 | **320 / 1337 / 1657** |

env 的 12 个 done 增量包括既有但此前未勾的 stall/deadline、三家 key/base、BOT provider/model/protocol，以及新增 output cap；events 的 3 个 done 增量是 invoked、stall 与新增 deadline。没有 synthetic vendor stream 被记成 upstream test 或 recorded fixture。

## 6. 明确未完成

- 三家 recorded/live vendor trace：**0/3**；
- run-wide input/output token、费用、并发 tool、computer runtime 完整 budget；
- retry 的真实 vendor outage/配额演练；
- 真实 tool loop、8-step 回注、`remember`；
- remote AG-UI、RMCP 3.1.4、Drive；
- browser/file/shell production executor；
- G5–G8 与 GUI。

因此只勾 §24.1 G4 的对应子项，**G4 整关保持未通过**。

## 7. 一手来源

- [Anthropic Messages streaming](https://platform.claude.com/docs/en/build-with-claude/streaming)
- [Google generateContent / streamGenerateContent](https://ai.google.dev/api/generate-content)
- [`@langchain/anthropic@1.5.6` 固定发布物](https://unpkg.com/@langchain/anthropic@1.5.6/)
- [`@langchain/google-genai@2.2.0` 固定发布物](https://unpkg.com/@langchain/google-genai@2.2.0/)
- [`@google/generative-ai@0.24.1` 固定发布物](https://unpkg.com/@google/generative-ai@0.24.1/)
- [`@langchain/core@1.2.8` 固定发布物](https://unpkg.com/@langchain/core@1.2.8/)

## 8. 恢复点

- implementation commit：`520f0b02ecfc87a6c8be795ec6a308e1aa9fa0cf`；
- 分支：`feat/2026-08-24-G4-anthropic-google-retry`；
- PR：[#24](https://github.com/acosmi/OpenBot/pull/24)；
- base：`feat/2026-08-24-G4-agent-openai-provider`（PR #23 head）；
- head：`feat/2026-08-24-G4-anthropic-google-retry`；
- 创建后机器实得：`OPEN / CLEAN / MERGEABLE`，`statusCheckRollup=[]`；
- implementation head 的 GitHub Actions run 数：**0**；
- 父 PR #23 同时复核为 `OPEN / CLEAN / MERGEABLE`。

堆叠链尚未进入 `main`。合并必须继续按 baseRefName 依赖顺序使用 merge commit，不能直接假设 `main` 已含本批。
