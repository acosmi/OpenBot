# Batch101：退役无调用者的 legacy MCP browser call

日期：2026-09-04

implementation：`00b3ae92e46e0ff332d59df68183c5d0a2d73623`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4
§2.4、§5.2、§7.2、§8.1、§8.6、§9.1、§9.6、§15.2–§15.3、§17.2、§24 G4、§28.1 R175

固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`

## 1. 本批结论

固定上游 legacy `POST /api/plugins/call` 不在 Rust 版挂载，T-API-0094 从错误的
`parity/preserve/todo` 更正为 `替代/remove/done`。这不是删掉 MCP 工具能力：旧 browser tool loop
已在固定上游自身迁到 server-owned runtime；Rust 的 Agent runtime 也已通过唯一 typed
`AgentToolGateway → AppCommand::InvokeTool` 执行真实 RMCP/Drive 工具。

legacy 请求只有 `ref`、`args`、`agentId`。它不能证明或携带 Rust 工具管线要求的 run/thread、
call id/sequence、lease/fencing、AuthGeneration、budget 与 durable attempt identity。保留同形 handler
只能在多个 active run 中猜一个，或另造不属于 run 的旁路；两者都会违反 v4 §5.2/§8.1，不能用
“兼容”名义实现。

本批不关闭 G4：三家 recorded/live trace、完整 thread/cancel/computer integration、computer runtime
budget、Desktop Local OAuth、custom private user-OAuth、vendor revoke runbook、RMCP protocol cancel、
Browser/file/shell 与 Plugins/Skills UI 仍未完成。

## 2. 固定上游证据

独立临时克隆固定 commit 后读取：

- `server/src/plugins/routes.ts::createPluginRoutes` 在 `/call` 上方逐字声明
  `NOTHING IN THIS REPOSITORY CALLS IT`；
- 同一注释说明它是旧 browser client-side tool loop 的遗留入口；
- `server/src/plugins/tools.ts::grantedTools` 说明旧 `useFrontendTool` handler 已迁到 server-owned
  tool definitions，执行闭包直接调用 `store.callTool`；
- 生产装配在 `server/src/index.ts` / `server/src/app.ts` 把 granted tools 加入真实 Agent run；
- `git grep '/api/plugins/call'` 在生产源码只命中迁移历史说明，没有 fetch/client；其余命中是
  `server/tests/bot-access.test.ts` 对遗留 route 的测试。

固定上游 `store.callTool` 自己仍有另一个缺陷：decision/attempt 没有在 vendor effect 前 durable
commit，成功/失败 audit 发生在 effect 后。Rust 已按 v4 §2.4 使用 decision+attempt transaction、
single-use capability、outcome+commit state，不照译该实现。

## 3. Rust 退役边界

Server 与 Desktop 都增加正向认证请求的负向回归：携合法 JSON 调用 `/api/plugins/call`，结果必须
是 unmatched 404。Server 同时断言 `McpConnectionAdministration` 调用记录为空，证明请求没有落入
catalog、policy、vault 或 vendor。刻意不挂 410 handler：无仓内 caller 时，404 不暴露旧能力存在，
也不会留下一个未来可能被填成旁路的 transport 入口。

20,000 Unicode scalar result normalization、empty/non-text/isError handling 并未删除；它仍由真实
RMCP/Drive executor 使用。远程 Agent callback `/api/agent-tools/call` 也不是 legacy user-session route
的重命名：它要求 Agent token + deployment-signed run assertion，并最终进入同一 typed tool pipeline。

## 4. 替代路径的本轮证据

本轮重新运行：

- Agent gateway Rust-authoritative call id/sequence `2/0/0`；
- Application happy path、decision/capability write failure、outcome reconciliation `3/0/0`；
- PostgreSQL 17.11 host-SCRAM + loopback TLS RMCP
  `server_side_tools_cover_no_grant_vendor_schema_real_rmcp_audit_and_policy_refusal` `1/0/0`。

真实纵向包含：no-grant、current vendor schema、provider tool projection、Rust run/call sequence、CEL、
content-secret refusal、decision+attempt、capability、一次 vendor effect、success/isError outcome、
reconciliation 与 hash-chain audit。它证明替代路径能执行能力且保留 v4 authority，并非只用单元 fake
为删除旧 API 辩护。

## 5. 守门结果

- Server plugin routes `7/0/0`，其中 legacy route 404/calls=0；
- Desktop plugin protocol `1/0/0`，包含 legacy route 404；
- Agent `2/0/0`、Application `3/0/0`、真实 PG+TLS RMCP `1/0/0`；
- Server/Desktop all-target/all-feature Clippy `-D warnings`，fmt；
- `cargo xtask parity-check`：0 violation；
- fixed upstream strict recount：`160 passed / 0 mismatch / 0 skipped`。

一次性 PostgreSQL 17.11 cluster 已停止并删除。固定上游临时克隆只保留到文档后的最终 strict recount，
随后删除。

## 6. 台账变化与明确剩余

- API：`93/81/174 → 94/80/174`；关闭 T-API-0094；
- API label：`parity/替代/新增 = 139/7/28 → 138/8/28`；
- parity：`847/863/1710 → 848/862/1710`；
- fixtures：仍为 `23/22/45`；overlay 仍为 `1299/403/2/6`；0 violation；
- native latest/schema 仍为 `0029`、`47表/478列/342 NOT NULL/269约束/97索引`。

MCP backend API 台账至此无 todo，但这不等于 MCP 产品面完成：`/admin/plugins` index/detail/tool、
`/admin/skills`、AppSidebar/AdminSidebar、正式 browser journey/golden/AX、Desktop Local OAuth、custom
private user-OAuth、admin delete vendor revoke runbook 与 protocol cancel 仍保持 todo。

本批没有生产 handler、schema/fixture、UI/CSS/locale/bundle、依赖或 Cargo.lock 变化；没有运行 npm、
被 R63 禁止的 `cargo xtask ci` 或 GitHub Actions，没有改 `grok-bot`。Golden comparator、Google Drive
brand/provenance、BrowserInput→CDP mapper 与 OpenAI recorded trace 均在独立 worktree，本批未触碰。
