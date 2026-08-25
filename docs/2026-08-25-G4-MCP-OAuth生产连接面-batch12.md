# G4 MCP OAuth 生产连接面（Batch 12）

日期：2026-08-25
分支：`feat/2026-08-25-G4-mcp-oauth-runtime`
基线：Batch 11 文档 head `5fb87e36ee492e69a5267699bbb7a94c1325bd66`
第一真源：后端 §2.4、§6.4、§9.2–§9.4、§14.3、§24；前端方案只作后续 GUI 契约，本批没有冒充 G6 页面完成。
固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
实现提交：`98a378cb94a6e2dfdf5da4bea76ad0cb5749bfb2`。
堆叠 PR：[#29](https://github.com/acosmi/OpenBot/pull/29)，base 为 Batch 11 分支
`feat/2026-08-25-G4-rmcp-runtime`；创建时 `OPEN/CLEAN/MERGEABLE`、checks 空、Actions=0。

## 1. 本批关闭的真实缺口

Batch 11 只允许匿名 HTTPS 与 deployment bearer；user OAuth 必须 `AuthRequired`，这是正确
fail-closed，但不构成 credential-backed MCP。Batch 12 补的是 Server / Desktop Remote 的完整
生产边界：

- 管理员 fresh-session + trusted-Origin 登记/轮换 deployment OAuth client；
- authenticated actor 列连接、发起授权、公开 callback、local-first disconnect；
- MCP 2026-07-28 401 PRM、AS discovery、issuer、PKCE S256、RFC 8707 resource；
- authorization-code 换 refresh、每 operation refresh rotation、actor-specific bearer；
- resource 401 只做一次受控 refresh/retry；insufficient scope 不循环；
- client/transport credential generation 进入 grant identity，旧 grant 不自动复活；
- vendor revoke 失败进入 durable pending，由多副本安全 reconciliation 重试。

这不是用预置 token 冒充 OAuth：本批真用 loopback protected RMCP、PRM、AS metadata、token、
revocation endpoint 与 PostgreSQL/Vault 走完 wire + transaction。

## 2. OAuth / secret / SSRF 边界

唯一出网仍是 `SafeDialer`。`McpOAuthClient` 不引入 reqwest：

1. 对 MCP endpoint 发无 Authorization 的 bounded initialize probe，优先解析 401 Bearer
   `resource_metadata`；
2. 再支持 RFC 9728 endpoint-path 与 root well-known fallback；
3. PRM `resource` 必须等于 PostgreSQL 中 exact MCP endpoint，登记 issuer 必须出现在
   `authorization_servers`；
4. 按 MCP 2026-07-28 规定的 RFC 8414 / OIDC 三路径顺序发现 AS metadata，metadata issuer
   必须逐字等于用于构造 well-known 的 issuer；
5. authorization/token/revocation endpoint 由 SafeDialer 逐跳 DNS/IP/TLS/redirect 校验；生产
   只许 HTTPS；HTTP 例外只给显式 CIDR allowlist 的本机 conformance；
6. authorization 与 token request 都带 exact RFC 8707 `resource`；access token 只进同一 MCP
   的 Bearer header，不进 URL/query/GUI/model/audit；
7. admin client secret 用共享 `Arc<Zeroizing<String>>` wire 类型；Clone 不复制 allocation，Debug
   永久 redacted。解封 client/refresh/access 都是 `SecretBytes`，token response 原始 body 也在
   drop 时 zeroize。

规范核对来源：

- [MCP 2026-07-28 Authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)
- [Authorization Server Discovery](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/authorization-server-discovery)
- [Authorization Security Considerations](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/security-considerations)

## 3. state / callback / exact redirect

Server callback 只由 configured `OPENBOT_PUBLIC_URL` 构造
`/api/plugins/oauth/callback`；生产非 HTTPS 时 begin 明确不可用，不从 incoming Host 推导。
`OPENBOT_APP_URL` 只作配置输入，先拒 userinfo/query/fragment；return destination 是
`settings|admin` enum，不接收 URL。

每个 begin 生成独立 32-byte state 与 48-byte PKCE verifier：

- DB identifier = deployment+tenant-domain HMAC(state)，不存 state；
- value = AES-256-GCM(actor/server/AuthGeneration/client credential id/resource/verifier/exact
  redirect/issuer/scope/return enum/DB expiry)，key 由独立 HKDF label 派生；
- callback `DELETE ... RETURNING` commit 后才解密/校验；code 缺失、iss mismatch、expired、坏
  verifier、actor generation/client/resource 漂移都已经烧掉 state；
- metadata 宣告 RFC 9207 `authorization_response_iss_parameter_supported=true` 时，缺 iss 也拒；
  未宣告时若返回 iss，仍逐字验证；
- 成功与全部失败都是 bodyless 302、`Cache-Control: no-store`、
  `Referrer-Policy: no-referrer`；code/state/vendor error 不进错误页。

## 4. Refresh rotation 与 actor runtime

code response 没有 refresh token即失败，不存短寿 access 冒充连接。新 v2 user credential、active
pointer、scope、connected_at 与 `mcp.account_connected` 同 transaction；若已有旧 pointer，旧值
同批 tombstone，外部 revoke 独立 reconciliation。

每次 MCP operation 都重新按 `(server, actor)` 查询/解封：

- token form 固定 refresh grant + exact resource + current scope +登记 client auth；
- provider 必须返回非空、与旧 token 不同且与 access token 不同的新 refresh token；
- 新 ciphertext CAS、scope、metadata 与 `credential.rotated` hash-chain audit 同 transaction；
- commit unknown/conflict 不释放 access；跨 replica CAS loser 只重读 winning ciphertext 一次；
- RMCP 401 时仅 user OAuth 再 refresh + 原调用 retry 一次；第二次 401=`AuthRequired`；403
  insufficient scope 直接进入重新授权，不无限 retry；
- callback 产出的 refresh 真实进入同一个 broker/RMCP runtime，不是 test-only executor。

## 5. Native 0018 与 stale grant

`native_0018.sql` expand-only 给 `mcp_servers` / `plugin_grants` 各加 nullable nonnegative
`credential_generation`。legacy NULL 在读取时等价 0，不回填历史事实。

client 登记/轮换 transaction：

1. 先把所有既有 grant 的 NULL 固定为旧 generation；
2. server generation +1；已有 catalog generation 同时 +1，使在飞旧 capability 立即 stale；
3. 退役旧 deployment client 与所有旧 user connection；
4. catalog 置可诊断的 `credential_changed_requires_regrant`；
5. 后续真实 refresh 对 generation mismatch 只能 `suspended_missing`，不会自动 active；
6. 管理员未来显式 regrant 才能把 grant 绑定新 generation。

机器 fixture：`schema-0018.json` 为 41 表 / 368 列 / 268 NOT NULL / 199 约束 / 80 索引 /
4 触发器 / 4 enum / 1 public function / 0 extension；4364 行，SHA-256
`3226eefb20d536c206b5d75522a77f6f82981f820fd5a414086871c21be75ebe`。

## 6. Disconnect 与 reconciliation

`DELETE /api/plugins/connections/{server_id}` 的顺序固定：

1. current actor/AuthGeneration/role 与 exact credential 复核；
2. transaction 内 `revoked_at` + metadata tombstone、删 user join、写
   `mcp.account_disconnected(vendor_revoked=false)`；
3. commit 后才 RFC 7009 revoke；vendor 失败绝不恢复 join/secret 可用性；
4. pending row 由 `FOR UPDATE SKIP LOCKED` bounded claim，状态 pending→revoking；进程崩溃超过
   2 分钟可重领；每 30 秒最多 32 条；
5. success 后 metadata=revoked 并写 `vendor_revoked=true`；failure 回 pending；
6. People offboarding 也把 token 标 pending，避免只本地退役后永远不再尝试 vendor revoke。

返回 DTO 只说 `revoked|pending`，不会在 vendor 503 时谎称已撤权。

## 7. 本机证据（未运行 CI）

| 验收 | 结果 |
| --- | ---: |
| contracts MCP secret/serde | 2 / 0 / 0 |
| application MCP admin gate + operation ledger | 2 / 0 / 0 |
| infra OAuth discovery/parser | 3 / 0 / 0 |
| infra redirect/expiry pure boundary | 3 / 0 / 0 |
| Server plugins framing/fresh-before-body/public callback | 3 / 0 / 0 |
| Server main assembly | 7 / 0 / 0 |
| PG17/SCRAM OAuth runtime | 2 / 0 / 0 |
| PG17/SCRAM native 0017 + 0018 | 4 / 0 / 0 |
| PG17/SCRAM existing MCP / user credential / callback | 5 / 0 / 0；11 / 0 / 0；2 / 0 / 0 |
| six-crate all-targets/all-features Clippy `-D warnings` | 通过 |
| contracts/UI `wasm32-unknown-unknown` | 通过 |
| RMCP / SafeDialer / RustSec / SAML / WS / importer guards | 通过 |
| test-inventory fixed-upstream replay | 105 files / 229 describes / 1047 tests；overlay 全保留 |
| strict recount | 154 / 0 / 0 |
| parity-check | 394 done / 1267 todo / 1661 total；0 violations/warnings |
| fixtures | 12 done / 22 todo / 34 total |

明确没有运行 `cargo xtask ci`，没有 dispatch GitHub Actions。未新增 Cargo package，Cargo.lock
仍为 Batch 11 的 460 packages；既有依赖审计结论不冒充本轮重新完成一次全 CI。

## 8. 仍未完成（不打勾）

- Desktop Local installed-app OAuth client、system browser、随机 `127.0.0.1` 短期 listener；
- MCP 专用 private/reserved CIDR 管理配置与 Server egress gateway；
- plugin server add/delete、manual runtime refresh、effect classifier、grant/regrant 完整 admin/UI；
- run/user cancellation 向 RMCP cancellation notification 传播；
- human proof-of-intent/approval GUI；
- Google Drive REST（不是 MCP）；browser/file/shell 与 G5–G8；
- 三家 live/recorded provider trace仍 0/3；外部安全审计/KMS/HSM/Windows native。

因此只勾 Server/Desktop Remote MCP OAuth 子面；G2、G4 与整项目均保持未通过。
