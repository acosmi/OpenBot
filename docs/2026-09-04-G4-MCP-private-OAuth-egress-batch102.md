# Batch102：custom private MCP user-OAuth 全生命周期出口绑定

日期：2026-09-04

implementation：`3745c4f6f10395d7ced7c214f5f74e97ca49a36b`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4
§2.4、§6.4、§8.6、§9.1–§9.4、§13.2–§13.3、§15.3、§17.2、§24 G4、§28.1 R176

固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`

## 1. 本批结论

Batch99 只把 custom server 的 canonical numeric CIDR 绑定到 catalog `tools/list` 与每次
`tools/call`。Batch102 将同一 PostgreSQL authority 贯穿 generic MCP actor OAuth 的全部网络阶段：

1. admin OAuth-client registration discovery；
2. connect authorization 的 resource probe、PRM 与 AS metadata discovery；
3. callback authorization-code token exchange；
4. 每次 runtime refresh-token rotation；
5. local-first disconnect 后的 immediate RFC 7009 revoke；
6. `SKIP LOCKED` pending-revocation reconciliation。

基础 `McpOAuthClient` 只贡献 resolver、TLS roots 与 scheme policy；每次操作用 current server 的
exact CIDR 替换 destination policy。每个 redirect 仍重新 DNS 解析与过滤。空/NULL 继续 public-only，
不是“允许所有内网”。Google Drive 是 compile-time reviewed public REST/OAuth adapter，显式拒绝任何
MCP private-egress override。

本批不关闭完整 G4：Desktop Local installed-app OAuth、管理员删除 custom server 后的 vendor revoke
runbook/持久化补偿、RMCP protocol cancel、三家 recorded/live provider trace、Browser/file/shell、
computer runtime budget、完整 thread/computer approval integration 与 Plugins/Skills UI 仍未完成。

## 2. 固定上游与缺口复核

固定上游 generic MCP OAuth 已有 PRM、AS metadata、code/refresh/revoke，但没有本项目 v4 的 numeric
private-egress authority；它不能作为“内网 OAuth 已安全实现”的证据。Rust 在 Batch99 前也使用单一
global public-only `SafeDialer`：tools 可因 server CIDR 访问内网，但 OAuth discovery/token/revoke 仍
拒绝同一目的地，形成配置可保存、catalog 可刷新、用户却永远连不上的断裂。

反向若给 OAuth client 一个全局 private allowlist，又会让一个 server 的管理员授权扩大到所有 server，
并允许 PRM redirect/issuer/token endpoint 借另一个 connector 的网络权限。正确边界只能是每次从
当前 server 行读取 exact CIDR，并把同一集合用于整个 discovery redirect chain。

## 3. 单一 canonical egress 解析

新增私有 `mcp_egress` 模块，catalog、admin connection 与 credential selection 共用：

- 最多 32 项；
- PostgreSQL array 以逗号连接后的总字节数 ≤2,048；
- 严格 lexical sorted、unique；
- 只接受 numeric canonical CIDR，拒绝 hostname、裸 IP、host bits、空白；
- parse 后条目数必须与 stored list 相等。

这修正了原先 catalog/connection 两份近似校验的漂移面；数据库 CHECK 继续是持久化底线，Rust reader
对 legacy/tampered row 仍 fail-closed。

## 4. state v3 与 connect/callback 线性化

OAuth attempt 从 v2 升为 v3，在 HMAC identifier + AEAD payload 中新增 exact CIDR list。旧 attempt
升级后 fail-closed，不按新网络权限重新解释。begin 在 discovery 后、state 落库前，以
`users + mcp_servers FOR SHARE` 同时锁 current AuthGeneration、role/revoke、endpoint、client pointer、
transport 与 CIDR；配置变化要么先完成使 begin 失败，要么等待 state commit。

callback 仍先 `DELETE RETURNING` 烧 state，再验 code/issuer/actor；随后逐字比较 current client、resource、
transport、CIDR，任何漂移均在 token effect 前失败。code exchange 使用 attempt/current 共同确认的
allowlist；refresh credential 持久化事务再次 `FOR SHARE` user/server 并复核同一字段，避免 token
返回后配置抢跑。

## 5. runtime refresh rotation 与 revoke

`PluginUserCredentialStore` 的 selection envelope 新增 closed transport、canonical CIDR/parsed allowlist。
`OAuthRefreshExchange` 只借出这一已验证 authority；generic MCP exchanger 只接受 `transport=mcp`，
curated Drive exchanger只接受`google_drive_rest + empty allowlist`。

token endpoint 返回 access + rotated refresh 后，store 在 access token 离开 broker 前：

- 锁 current `mcp_servers` 与 deployment OAuth-client credential；
- 逐字复核 server id/endpoint/transport/CIDR/client pointer；
- 再 CAS 原 user encrypted refresh、更新 connection scope、写 `credential.rotated` hash-chain audit；
- 任一漂移返回 Conflict，不提交新 refresh，不写成功 audit，也不释放 access token。

disconnect 先提交本地 tombstone/join delete/audit，再重新加载 current server/client/CIDR尝试 vendor
revoke；不再使用 tombstone 前的旧网络快照。pending reconciler每轮同样重新读取 current authority；
CIDR清空时保持 pending且网络调用数不增加，恢复 exact CIDR 后才重试。

## 6. 真实 PostgreSQL 17.11 证据

一次性 PostgreSQL 17.11 host-SCRAM + loopback OAuth/RMCP server 实得：

- OAuth lifecycle `2/0/0`：空 CIDR registration/refresh 先拒绝且 credential/audit/token-call=0；配置
  `127.0.0.1/32` 后 registration discovery、authorization、code exchange、四次 refresh rotation、
  controlled 401 retry、disconnect、pending retry、reconnect 全通过；
- begin 后把 CIDR 从 `127.0.0.1/32` 改成 `127.0.0.0/8`，旧 state 被烧毁且 code-call=0；
- pending revoke 时清空 CIDR，sweep=`1 attempted / 0 revoked / 1 pending`且 revoke-call 不增加；恢复
  `127.0.0.1/32` 后下一轮成功；
- credential matrix `12/0/0`：原 11 条 actor/server/secret/retirement 隔离全回归，新增 exchanger
  在 token response 后、rotation commit 前修改 CIDR，最终 Conflict、旧 ciphertext 不变、rotation audit=0；
- RMCP/catalog/private-egress matrix `6/0/0`；Infra unit `327/0/0`。

两次有效红灯均保留：Batch100 收紧 `grantedTo` 后，Batch99 的旧 RMCP 夹具只有裸 `agents`、没有
authoritative `agent_profiles`，本轮全矩阵正确暴露；补真实 public profile 后 6/6，未放宽权限查询。
新增 drift 测试首跑又因同一 PostgreSQL 参数同时被推断 UUID/text 报 operator mismatch；拆成独立 text
参数后精确用例与完整 12 条矩阵均重跑成功。

## 7. 守门与台账

- Infra `327/0/0`；OAuth PG `2/0/0`；credential PG `12/0/0`；RMCP PG/loopback `6/0/0`；
- Infra/Server/Desktop all-target/all-feature Clippy `-D warnings`，fmt；
- SafeDialer、RMCP、Application assembly、增强后的 MCP OAuth guard 全绿；
- `cargo xtask parity-check`：`848 done / 862 todo / 1710`、0 violation；
- fixed upstream strict recount：`160 passed / 0 mismatch / 0 skipped`。

本批不新增/关闭 T-ID，只重验证 T-API-0083、0087、0088、0157 的 done evidence；API 仍为
`94/80/174`，fixtures `23/22/45`，overlay `1299/403/2/6`。native latest/schema 仍为
`0029`、`47表/478列/342 NOT NULL/269约束/97索引`。

本批没有 schema/fixture、UI/CSS/locale/bundle、依赖或 Cargo.lock 变化；没有运行 npm、被 R63 禁止的
`cargo xtask ci` 或 GitHub Actions，没有改 `grok-bot`。三个外部工具 worktree 与 OpenAI recorded
trace worktree均未触碰。全部一次性 PG cluster 已停止并删除；固定上游临时克隆在最终 strict 后删除。
