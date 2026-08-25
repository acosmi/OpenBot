# G4 Rust Agent + OpenAI Provider batch 6

> 第一真源修订：v3 §28.1 R69；R63 继续有效，只运行本机定向测试，不派发 Actions，不运行
> `cargo xtask ci`。

## 1. 完成项

- [x] `openbot-domain` pure Agent reducer：固定 phase、event、effect 与 terminal；
- [x] tool step cap 固定 8；cancel 等待 child-stopped fact；lease/journal unknown 只进 reconciliation；
- [x] bounded built-in runtime：reserve → durable outbox ack → activate，ack 失败 revoke；
- [x] activation 起算 absolute deadline 与 lease heartbeat，覆盖 context load、provider connect 与 body stream；
- [x] provider-neutral request/event/failure/session ports；vendor DTO 不穿 application boundary；
- [x] OpenAI Responses 与 Chat Completions 两条 streaming adapter；
- [x] SSE UTF-8 分片、多行 data、skeleton、延迟字段、partial JSON、交错 tool arguments、空 delta、未知扩展；
- [x] SafeDialer JSON POST、redirect 逐跳重验、真实 body read-gap stall、response/request size cap；
- [x] normalized text/reasoning 共用 expected-sequence journal；reasoning 不进入 assistant materialization；
- [x] package Bot 固定读取 `model.yaml::default_model/credential_secret_ref`；默认协议固定 Responses；
- [x] package Agent `systemPrompt` 每 run 重读，并始终前置 MIT-attributed provenance guidance；
- [x] model credential 每 sampling fresh 查询；active exact match、stored-first、environment fallback、corrupt-no-fallback；
- [x] Server main 生产装配、缺失面 fail-closed readiness；
- [x] private/self-hosted provider 精确 CIDR 与显式 HTTP 双开关；HTTP 不绕过 destination policy。

## 2. 关键裁决

1. Package Bot 与 managed 插槽不能共用一份“全局 model”。Package Bot 的 model/credential ref 只来自
   五 YAML 中已验证的 `model.yaml`；managed 的 `BOT_PROVIDER/BOT_MODEL/BOT_RESPONSES_API` 仍是另一层。
2. 固定上游锁定的 `@ai-sdk/openai@3.0.99` 发布产物中，`createLanguageModel` 直接调用
   `createResponsesModel`。因此 package Bot 固定 Responses；Chat adapter 为 openai-compatible/reference
   与 managed 后续面保留，不靠模型名字猜协议。精确发布物：
   `https://unpkg.com/@ai-sdk/openai@3.0.99/dist/index.mjs`。
3. 模型 key 不在启动时塞进长期 Agent 对象。每次 sampling 重新查询 PostgreSQL：
   `kind=model/provider=openai/key_id=package ref/revoked_at IS NULL`，按 `created_at DESC,id DESC`；
   只有“没有 matching row”才能回落 trim 后的 `OPENAI_API_KEY`。存在但损坏时回落会掩盖 vault tamper，故拒绝。
4. Context 查询同时绑定 run/thread/Bot/actor/fencing/deployment/tenant/membership/package/profile；
   renderer 或 provider 不能自报 scope。超 4096 messages/6MiB 明确失败，不用假摘要冒充 §7.4 压缩。
5. Provider raw event/body 默认不持久化。Text 与 reasoning 只以 normalized semantic chunk 落 journal；
   terminal 聚合时只认 channel=text，避免 reasoning 泄漏进用户 transcript。
6. Heartbeat/deadline 从 activation 开始，而不是 provider headers 到达后。Context/provider-start future 先 drop，
   runtime 才回送 children-stopped 并提交 Cancelled，不能先显示 cancelled、后台 child 仍活着。
7. `OPENBOT_PROVIDER_ALLOW_HTTP=true` 只放宽 scheme；loopback/private/reserved 仍须
   `OPENBOT_PROVIDER_EGRESS_ALLOW_CIDRS` 的规范数值 CIDR。两变量均为新增，已进 env ledger 与 R69。

## 3. 本机证据

- domain Agent reducer：**3/0/0**；
- application run runtime：**4/0/0**；
- built-in Agent/Gateway：**7/0/0**；
- OpenAI adapter：**6/0/0**；
- SafeDialer streaming：**2/0/0**；
- PostgreSQL context prompt：**1/0/0**；
- Server config：**65/0/0**；Server main assembly helpers：**4/0/0**；
- PostgreSQL 17.11 **trust Unix socket** + 真实 loopback HTTP/SSE：**2/0/0**；这不是新增 SCRAM 证据；
- PG 竖切覆盖 BeginRun→relay→context→provider→reasoning/text journal→assistant/terminal，以及
  stored/env/corrupt/missing、newest active exact match、timestamp tie-breaker、Agent role 热更新；
- 六 crate all-targets/all-features Clippy `-D warnings`：通过；
- `bash tools/check-safe-dialer-dependencies.sh`：通过；socket/DNS/TLS/HTTP production caller 仍唯一；
- strict recount：**151/151/0**；parity-check violations/warnings **0/0**；
- Cargo.lock package：新增 **0**，仍 **428**；
- env ledger：**37 done / 36 todo / 73**；tests：**184/863/1047**；API：**26/130/156**；
- 总 parity：**305 done / 1350 todo / 1655**；fixtures：**10/22/32**；
- implementation head 的 GitHub Actions run 数：**0**。

## 4. 明确未完成边界

- Anthropic / Google Generative AI adapter；
- 三家真实 provider recorded trace 与 live credential smoke（当前 fixtures 仍 0/3）；
- 429 / retryable 5xx / connect-before-send retry、Retry-After、退避与完整 retry budget；
- token/cost/concurrent-tool/computer budget、deadline/stall 的 allowlisted audit 与 metrics；
- 完整 context compression/provenance source range；
- 真实 tool loop、结果回注、parallel-safe resource lock、8-step 终局与 `remember` tool；
- managed slot production runtime、remote AG-UI、callback assertion、RMCP、Drive；
- browser/file/shell executor 与 G5/G7 isolation/handover。

OpenAI adapter 已能解析完整 tool call，但 runtime 在唯一 application tool loop 尚未接通时明确写
`tool_loop_unavailable` terminal；这不是 tool runtime 完成。G4 整关保持未勾。

## 5. Git 恢复点

- 实施提交：`c7a9e8af6e595293719b123ca93eb49d22e60522`；
- 分支：`feat/2026-08-24-G4-agent-openai-provider`；
- 堆叠 PR：**#23**，base=`feat/2026-08-24-G3-intelligence-importer`；创建后
  `OPEN/CLEAN/MERGEABLE`；
- PR #23 与 implementation head 的 GitHub Actions run 数：**0**。
