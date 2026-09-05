# MCP server 删除与 vendor 撤权 runbook

适用版本：v4 §9.4 / §24 G4，Batch103 起。

## 目标与不变量

管理员删除 MCP server 后，OpenBot 必须先在一个 PostgreSQL 事务中停止全部本地访问，再异步执行
vendor compensation。vendor 不可达、同名 server 被重新添加或 retained material 损坏，都不得恢复
本地连接、grant 或 credential 可用性。

本 runbook 只处理已经通过 typed `DELETE /api/plugins/servers/{server_id}` 发起的删除。不要直接
删除数据库行，也不要直接改 `credentials.metadata`：那会绕过 fresh admin authority 与 hash-chain
audit。

## 删除后的预期状态

API 成功返回时，下列本地结果已经同事务提交：

- `mcp_servers`、`mcp_tools`、`mcp_user_credentials` 中该 server 的行均为零；
- 所有 `plugin_grants(kind='mcp')` 的该 server 前缀均删除，包括已经 stale/orphan 的 grant；
- 当前 deployment credential 与全部 actor refresh credential 均已有 `revoked_at`；
- 每个原本 active 的 actor 先写一条 `mcp.account_disconnected`、
  `revocation_reason=mcp_server_removed`、`vendor_revoked=false` audit；
- 管理员操作写一条 `configuration.changed/change=mcp_server_removed` audit；
- 上述任一步失败则事务整体回滚，不返回成功。

自动 reconciler 每 30 秒扫描一次。`pending` 超过 30 秒可 claim；进程在 claim 后崩溃留下的
`revoking` 超过 2 分钟可由另一副本重新 claim。单轮最多 32 条。

## 只读核查

以下示例使用 `psql` 变量进行引用；`server_id` 不是 secret，但仍不要把私有 endpoint 或 metadata
全文复制到工单。

```sql
\set server_id 'replace-with-server-id'

SELECT
  (SELECT count(*) FROM public.mcp_servers WHERE id = :'server_id') AS servers,
  (SELECT count(*) FROM public.mcp_tools WHERE server_id = :'server_id') AS tools,
  (SELECT count(*) FROM public.mcp_user_credentials WHERE server_id = :'server_id') AS joins,
  (SELECT count(*) FROM public.plugin_grants
    WHERE kind = 'mcp' AND split_part(ref, '/', 1) = :'server_id') AS grants;

SELECT kind::text,
       metadata->>'revocation_status' AS revocation_status,
       count(*)
  FROM public.credentials
 WHERE provider = :'server_id'
   AND revoked_at IS NOT NULL
   AND metadata->>'revocation_reason' = 'mcp_server_removed'
 GROUP BY kind::text, metadata->>'revocation_status'
 ORDER BY kind::text, revocation_status;
```

第一条查询必须全部为零。第二条只读状态与数量，禁止在诊断命令中选择或导出
`encrypted_value`、完整 `metadata`、refresh token、client secret 或私有 endpoint。

## 状态解释

| kind | 状态 | 含义与动作 |
|---|---|---|
| `mcp_user_token` | `pending` | 本地已撤权，等待自动 RFC 7009 revoke；无需恢复 server |
| `mcp_user_token` | `revoking` | 某副本正在处理；两分钟内不要并行人工重放 |
| `mcp_user_token` | `revoked` | vendor 已确认；该 token 的 server-removal 网络上下文已经从 metadata 擦除 |
| `mcp_user_token` | `operator_required` | retained token/client/context 无法安全使用，自动重试已永久停止；按下节人工处置 |
| `mcp_oauth_client` | `retained_for_user_token_revocation` | 只为尚未完成的 user-token revoke 保留加密 client；不得重新挂回 server |
| `mcp_oauth_client` | `operator_required` | user-token 自动阶段已完成或无法继续；仍需在 vendor 控制台删除/撤销 OAuth client registration |
| `mcp` | `operator_required` | deployment bearer 已在 OpenBot 本地撤权；仍需在 vendor 侧轮换或撤销 bearer |

同一 `server_id` 后来重新添加时，旧 tombstone 只使用删除事务冻结的 versioned
resource/transport/client/CIDR 上下文；绝不回落到新 server。不要通过重新添加同名 server 尝试“修复”
旧撤权任务。

## 人工 vendor 处置

1. 对 `mcp_user_token/operator_required`，在 vendor 管理面撤销对应用户对该 OAuth application 的
   authorization/session。只在受控终端本地查看 actor/credential ID；不要把 token 密文或 metadata
   全文复制出去。
2. 对 `mcp_oauth_client/operator_required`，使用该 credential 保留的非 secret `clientId`/`issuer`
   定位 vendor registration，并在 vendor 控制台删除或禁用它。不要解封 `clientSecret` 来做人工核查。
3. 对 `mcp/operator_required`，在 vendor secret 管理面轮换或撤销该 bearer；不要把新 token 写回已经
   删除的 server。
4. 把 vendor 控制台的操作人、时间、vendor ticket/receipt 记录在组织的变更或 incident 系统中。
   当前版本没有允许绕过 audit 的“直接改 metadata 为完成”入口，因此不要手工清除
   `operator_required`。OpenBot 内的 typed operator-attestation/最终 secret retirement 属后续 G8
   retention/release 工作，不在本 runbook 中伪造。

## 异常处理

- `pending` 长时间不下降：先检查 reconciler 是否运行以及稳定错误码；确认网络、DNS/TLS、vendor
  revocation endpoint 和删除时冻结的精确 CIDR authority。不要放宽全局 egress。
- `operator_required`：这是终态告警，不会自动重试；按上节处理。
- API 返回 commit unknown：先用第一条只读查询判断本地删除是否已提交，再查 audit/status；不要因
  HTTP 结果未知而重新创建 server 或 credential。
- vendor revoke 已成功但本地确认提交失败：RFC 7009 revoke 必须按幂等方式重试；状态仍为
  `pending/revoking` 时不要改成本地成功。
- 发现 server credential 错绑到另一 provider/kind：删除 server 仍应完成，但 OpenBot 不会撤销那条
  无关 credential；单独按数据完整性 incident 调查。

## Audit 核查

```sql
SELECT event_type, actor_user_id,
       payload->>'revocation_reason' AS reason,
       payload->>'vendor_revoked' AS vendor_revoked,
       payload->>'change' AS change,
       row_hash IS NOT NULL AS chained
  FROM public.audit_events
 WHERE target_type = 'mcp_server'
   AND target_id = :'server_id'
   AND event_type IN ('mcp.account_disconnected', 'configuration.changed')
 ORDER BY created_at, id;
```

期望顺序是本地 `vendor_revoked=false` 与 configuration removal 已提交，随后每个自动成功的 token
各有 `vendor_revoke_confirmed/vendor_revoked=true`。本地材料在 claim 后被判定不可恢复时，另有
`vendor_revoke_operator_required/vendor_revoked=false`。所有新行都必须 `chained=true`。

## 可重复演练证据

- `mcp_oauth_runtime`：真实 PostgreSQL 17.11/SCRAM + loopback OAuth，覆盖本地原子删除、same-ID
  replacement 不劫持旧上下文、成功 revoke 后 token context 擦除、零 user-token client 直接转人工、
  删除前/删除后 client 损坏不无限重试、错绑 credential 不被误撤销以及 hash-chain audit。
- `mcp_protocol`：真实 PostgreSQL 17.11/SCRAM + TLS RMCP，覆盖 bearer server 的 grant/tool/server
  本地闭包及 bearer `operator_required`。

这些演练不等于真实第三方控制台已完成 client/bearer 删除，也不替代 Desktop Local OAuth、RMCP
protocol cancel、外部安全审计或 G8 全量 runbook/发行闸门。
