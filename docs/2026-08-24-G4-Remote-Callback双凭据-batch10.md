# G4 Batch 10：Remote Callback 双凭据与空工具集拒绝链

> 日期：2026-08-24（America/Los_Angeles）
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §3.4、§6.4、§7.1、§7.5、§8.6、§24、§28.1 R73
> 堆叠基线：PR #26 head `2c14c5a1bd912cb61dd4e288bec80159e2b0051e`
> 当前分支：`feat/2026-08-24-G4-remote-callback-assertion`

## 1. 本批闭合边界

本批把 Batch 9 的 `run_assertion=None` 推进成真实生产双凭据边界：

```text
fresh same-origin owner/admin session
→ ApplicationService typed issue/rotate/revoke
→ OS CSPRNG 32 bytes + obot_agt_ wire
→ PostgreSQL only SHA-256 + issued_at
→ bot.callback_token_* audit in same transaction

active remote run
→ DB clock mint 10-minute signed assertion
→ deployment + tenant + Bot + actor + run + canonical tool-set hash
→ RunAgentInput.forwardedProps.openbotRun

POST /api/agent-tools/call
→ per-Agent token hash + assertion HMAC
→ token owner == signed Bot
→ active run/thread/member/lease + current role/revoke
→ signed tool-set == current authoritative tool-set
→ grant check
→ refusal audit or governed executor
```

当前 production remote tool-set 仍固定为空，因为 RMCP/Drive executor 与跨副本 call sequence 尚未
落地。故身份正确的任意工具 callback 会诚实返回 404 并写 refusal audit；没有 fake executor 成功路径。

## 2. Callback token

### 2.1 精确 wire 与 hash-only

- prefix 固定 `obot_agt_`；
- entropy 固定 OS CSPRNG 32 bytes；
- suffix 固定 base64url-no-pad 43 字符，总长 52；
- cheap shape gate 必须能精确解回 32 bytes；
- PostgreSQL 只存 64 个小写 hex 的 SHA-256，lookup 后再次 constant-time 比较；
- issue 每次替换旧 hash，旧 token 立即失效；revoke 同时清 hash/issued_at。

一次性 response 类型没有 `Clone`/`Display`，`Debug` 固定 redacted，当前 allocation drop 时 zeroize；
serde 只为一次 HTTP/Tauri 响应存在。明文不进 audit、数据库、普通日志或 trace。

### 2.2 权威管理范围

- owner 可管理自己 active `remote_ag_ui`；
- admin 可管理当前 tenant 的 package-backed remote Agent；
- built-in、deleted、cross-tenant package、别人的 private remote、stale AuthGeneration 统一 404；
- HTTP 要求 trusted Origin + fresh live session；资源 owner/admin 判据在 PostgreSQL transaction 内重新读取；
- role/access/generation 变更与 token 管理共用 people advisory transaction lock，堵住检查后撤权竞态；
- hash update/clear 与 lifecycle audit 同事务，audit 失败时 mutation 回滚；issue commit unknown 不返回 token。

这比固定上游的 route 后置 audit 更强：不存在“token 已生效但 issued audit 失败”的提交窗口。

## 3. Signed run assertion

### 3.1 兼容机制与新增绑定

HMAC 机制逐字兼容固定上游：

```text
run_key = HMAC-SHA256(master, "openbot:agent-run")
signature = HMAC-SHA256(run_key, base64url(payload_json))
wire = base64url(payload_json) + "." + base64url(signature)
```

固定 Rust/Bun 向量逐字符相等；Rust 产物又由固定上游
`server/src/agents/callback-token.ts::readRunAssertion` 实读回相同 bot/actor/run。

在上游 `botId/actorId/runId/exp` 之上，第一真源要求的安全绑定新增：

- `version=openbot.remote-run.v1`；
- `deploymentId` / `tenantId`；
- canonical `toolSetHash`；
- `iat`；
- `exp=iat+600000`，不接受调用方自定寿命。

tool set 先去重、按字节序排序，再以 domain label + count + 每项 length-frame 求 SHA-256；因此顺序不影响，
`a+bc` 与 `ab+c` 不会拼接碰撞。iat/exp 都取 PostgreSQL clock，跨 replica 不依赖各进程墙钟。

### 3.2 生产发送

package-backed remote Agent 每 run 重读 scope 后签 assertion，并放入
`RunAgentInput.forwardedProps.openbotRun`。Batch 9 的 E2E 已加强为远端用同 signer 验证：

- deployment/tenant/Bot/actor/run 全匹配；
- 当前 tool-set hash 等于 canonical empty set；
- `tools=[]` 与 assertion 同源。

## 4. Callback verifier 与审计

`POST /api/agent-tools/call` 已正式注册，但还不是完成的执行 API：

- unknown/empty/revoked token；
- missing/null/non-string/wrong-key/expired assertion；
- token owner 与 signed Bot 不同；
- run/thread/member/lease 不 active；
- signed/current tool set 不同或 requested tool 未授予；

全部在任何 executor 前停止。token 与 assertion 哪一半错共用 401；两份有效身份却 Bot 不同保留 upstream
403；未授予工具统一 404。credential refusal 先写 `mcp.callback_refused`，audit 只含 stable error code，
actor/Bot/tool/token/run/args 一律不信任、不落行；audit 失败返回 503，仍不执行。

固定上游仍接受 deployment-wide `AGENT_TOOL_TOKEN`。第一真源 §3.4/R9 已明确删除；生产回调没有该
配置槽或分支，遗留变量只在 migration preflight 中作为启动拒绝项存在。

## 5. 生产落点

| 子面 | Rust 落点 |
| --- | --- |
| token shape/hash/tool-set/assertion HMAC | `crates/openbot-domain/src/remote_callback.rs` |
| one-time redacted/zeroized response DTO | `crates/openbot-contracts/src/agent.rs` |
| typed admin/callback auth ports | `crates/openbot-application/src/agent_admin.rs` |
| token PostgreSQL mutation/audit + callback verifier | `crates/openbot-infra/src/agent_callback.rs` |
| assertion injection | `crates/openbot-infra/src/provider/context.rs` |
| issue/revoke HTTP | `crates/openbot-server/src/http/agents.rs` |
| callback HTTP refusal boundary | `crates/openbot-server/src/http/agent_tools.rs` |
| production assembly | `crates/openbot-server/src/main.rs` |
| PG/Axum evidence | `crates/openbot-infra/tests/agent_callback.rs`、`crates/openbot-server/tests/agent_callback_postgres.rs` |

## 6. 本机证据

本批遵守 R63：没有运行 `cargo xtask ci`，没有派发 GitHub Actions。

| 验收 | 结果 |
| --- | --- |
| fixed Node/Bun HMAC vector + upstream `readRunAssertion` | **逐字符相等 / bot-actor-run 回读相等** |
| contracts | **67/0/0**；one-time token serde + Debug redaction |
| domain 全包 | **345/0/0** + CEL **6/0/0** + upstream policy **28/0/0**；callback 5 条 |
| application 全包 | **119/0/0**；两命令进入 exhaustive dispatch/operation ledger |
| Agent 全包 | **28/0/0** |
| Server lib | **168/0/0**；callback HTTP 5 条，含回环 WebSocket 既有回归 |
| Server main | **7/0/0** |
| transport parity | **7/0/0** |
| PG17.11 TCP SCRAM callback infra | **2/0/0** |
| PG17.11 TCP SCRAM callback HTTP | **1/0/0** |
| PG17.11 TCP SCRAM Agent runtime | **6/0/0**；remote RunAgentInput assertion 实验收 |
| 7 crate all-targets/all-features Clippy `-D warnings` | exit 0 |
| contracts WASM compile / fmt / diff / SafeDialer guard | exit 0 / exit 0 / exit 0 / exit 0 |
| Cargo.lock package 数 | **428**；只给 contracts 增加已有 subtle/zeroize direct edge |
| strict recount | **154/154/0** |
| parity | **352 done / 1308 todo / 1660 total**，0 violations/warnings |

真库矩阵额外证明：

1. owner issue→rotate→revoke，hash/time/audit 精确变化；
2. package remote 只有 admin 可签发；built-in/deleted/cross-tenant/other-owner/stale generation 均零写；
3. forced audit trigger 失败使 token hash/issued_at 全回滚；
4. valid token+assertion+active run 只因 empty grant set 得 404，证明双凭据已通过但 executor 未伪造；
5. unknown/expired/missing/mismatched/revoked 六类写六条 refusal，actor/target NULL，secret canary 0；
6. real session cookie→Axum→ApplicationService→PG issue/callback refusal/revoke 全程状态 201→404/401→204。

## 7. 台账变化

| 台账 | Batch 9 | Batch 10 |
| --- | ---: | ---: |
| API | 26 / 130 / 156 | **28 / 128 / 156** |
| events | 12 / 65 / 77 | **15 / 62 / 77** |
| tests | 184 / 863 / 1047 | **206 / 841 / 1047** |
| env | 49 / 25 / 74 | 不变 |
| fixtures | 10 / 22 / 32 | 不变 |
| parity 总计 | 325 / 1335 / 1660 | **352 / 1308 / 1660** |

关闭项：callback token POST/DELETE 两 API、20 条 fixed callback-token 判据、callback issued/revoked/refused
三 event、callback lifecycle audit 一条、callback schema 列一条。G2 机器子队列因 lifecycle audit 所在文件
增加一条，现为 **150/84/234**。

## 8. 明确未完成

- `POST /api/agent-tools/call` 的真实 success/outcome；该 API ledger 仍 todo；
- RMCP 3.1.4 client/conformance、MCP grant/catalog generation、per-user credential 与 vendor call；
- Drive/browser/file/shell executor；
- 跨 replica durable tool call sequence（不能复用 built-in host 的进程内 sequence）；
- remote tools 非空时的 callback→唯一 tool pipeline→outcome/result 回传；
- customer endpoint outbound authorization header；
- user-created `package_id IS NULL` remote Agent 完整 CRUD/connection test；
- interrupt/resume 与其余 AG-UI durable/UI projection；
- callback flood rate limit、独立安全外审/KMS/HSM。

因此只勾 §24.1 的双凭据、token lifecycle 与 empty-set refusal 子项，**G4 整关保持未通过**。

## 9. 恢复点

- implementation commit：`2fd1c6ff04d9bb544f4765d6fb67291f982178e4`；
- 分支：`feat/2026-08-24-G4-remote-callback-assertion`；
- PR：[#27](https://github.com/acosmi/OpenBot/pull/27)；
- base：`feat/2026-08-24-G4-remote-agui-protocol`（PR #26 head）；
- 创建后机器实得：`OPEN / CLEAN / MERGEABLE`，`statusCheckRollup=[]`；
- implementation head Actions run 数：**0**；
- 父 PR #26 同轮复核仍为 `OPEN / CLEAN / MERGEABLE`。

堆叠链尚未进入 `main`；合并必须继续按 `baseRefName` 依赖顺序使用 merge commit。
