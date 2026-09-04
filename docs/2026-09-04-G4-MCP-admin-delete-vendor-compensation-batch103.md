# Batch103：MCP admin 删除后的 vendor compensation 与 runbook

日期：2026-09-04

implementation：`caea34d88504cbbd567a79a8c8fb2e99c06fe872`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4
§2.4、§6.4、§8.6、§9.2–§9.4、§15.3、§17.2、§24 G4/G8、§28.1 R177

固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`

## 1. 本批结论

Batch99 的 admin delete 已在本地事务撤销 deployment/actor credential、grant 与 server，但 server 行
提交后消失，既有 pending reconciler 只能重新按 `server_id` 读取 current endpoint/client/CIDR：没有
同名 server 时永久 pending；同名 server 后来重建时若直接回落 current row，又会把旧 refresh token
交给新 resource/client。

Batch103 将删除时的 vendor compensation authority 冻结为 existing credential metadata 中的
`server_removal_revocation` v1：

- resource URL；
- closed vendor transport；
- retained OAuth client credential UUID；
- canonical exact numeric CIDR list。

上下文不复制 refresh token、client secret 或 bearer；secret 仍只在原 Vault v2 ciphertext 中。Debug
固定隐藏 resource，只显示transport、是否有client与CIDR条数。删除后同 ID server 不参与旧任务。

正式 runbook：`docs/runbooks/mcp-server-removal-vendor-revocation.md`。

本批不关闭完整 G4/G8：Desktop Local installed-app OAuth、RMCP/computer/file/shell protocol cancel、
四个 Plugins/Skills UI route、三家 recorded/live trace、acting Approval/computer、typed
operator-attestation、vendor receipt 入库与最终 secret physical retirement 仍未完成。

## 2. 删除事务与本地线性化

`remove_server` 在已有 preflight 后再次于事务中验证 current admin/AuthGeneration/revocation，并锁 exact
server row。automatic compensation 只有同时满足以下条件才成立：

1. resource 是有 host/port、无 userinfo/query/fragment 的 HTTP(S) URL；正式产品仍由 OAuth
   `SchemePolicy::HttpsOnly` 限制，HTTP 只服务 allowlisted loopback conformance；
2. transport 是 `mcp`，或完全匹配 compile-time Google Drive identity 且 Drive OAuth runtime 已装配；
3. stored CIDR 可由共享 canonical parser逐项重建；
4. server credential 的 kind/provider 与 server 精确绑定，尚未撤销；
5. Vault 能按 deployment/service AAD 解封，且 retained OAuth client 通过 closed local parser；本步零网络。

任一项不满足时，本地 server 删除仍可完成，但 user token 与 owned server credential直接进入
`operator_required`，不制造永远 pending 的任务。若 server 的 credential pointer 错绑到另一
kind/provider，删除不会撤销无关 credential。

同一事务随后：

- 锁定所有 active/pending/revoking user token并设置 `revoked_at`；
- active actor各写一条 `mcp.account_disconnected`、`mcp_server_removed`、`vendor_revoked=false`
  hash-chain audit；既有 pending token不重复记一次本地断开；
- 删除 `kind=mcp` 且 `split_part(ref,'/',1)=server_id` 的全部 grant，不依赖 current tool join，因此
  stale/orphan ref 同样撤销；
- 删除 server；FK级联清 tools/user joins；
- 写 admin `configuration.changed/change=mcp_server_removed`；
- 任一步失败全事务回滚，commit unknown仍按 reconciliation错误返回。

OAuth client在存在automatic user token时标
`retained_for_user_token_revocation`；零 user token时立即进入`operator_required`并记
`user_token_revocations_completed_at`。deployment bearer只完成OpenBot本地撤权，始终进入人工vendor轮换。

## 3. Reconciler 与永久/暂时错误分型

claim仍是 `FOR UPDATE SKIP LOCKED`，pending 30秒、crash遗留revoking 2分钟后可重领，单批32条。

对普通 disconnect/rotation tombstone，Batch102 的“每轮读取 current server authority”不变。只有
`revocation_reason=mcp_server_removed` 强制走 retained context：

- exact removed resource/transport/CIDR；
- exact revoked OAuth client row、provider与reason；
- retained client Vault AAD与closed parse；
- 从不调用后来同 ID server/client。

vendor/TLS/DNS/HTTP failure 是暂时错误，回到 pending。retained refresh/client/context 的本地损坏或
缺失是永久错误：该 token 一次 claim 后改 `operator_required`，client同步标
`automatic_user_token_revocation_failed_at`，写
`vendor_revoke_operator_required/vendor_revoked=false` audit，下一轮不再 claim。

RFC7009成功后在一个事务中：

- token改 `revoked`并写`vendor_revoked_at`；
- 写`vendor_revoke_confirmed/vendor_revoked=true` hash-chain audit；
- 从该 user token metadata 擦除 `server_removal_revocation`，避免完成后继续保留私有网络上下文；
- 若同一retained client已无pending/revoking token，client改`operator_required`并记
  `user_token_revocations_completed_at`，交给runbook完成vendor registration删除。

## 4. 真实 PostgreSQL 17.11 与网络证据

独立 PostgreSQL 17.11 host-SCRAM 集簇与 loopback OAuth/RMCP fixture 实得：

- OAuth lifecycle `2/0/0`；
- admin delete先得到server/join/grant=`0/0/0`与一条versioned pending token；
- 删除后插入同 ID replacement server+另一client+不可达endpoint，旧token仍只命中原OAuth fixture；
- 首轮强制vendor failure为`1 attempted / 0 revoked / 1 pending / 0 operator_required`；恢复后下一轮
  `1/1/0/0`，证明retained context可重试；
- 成功后user-token context已擦除，旧client为operator_required且有完成时间，新replacement client
  保持active；
- valid OAuth client但零user token时不再永久retained，直接进入operator_required；
- 删除前client plaintext shape损坏时token/client立即operator_required，下一轮attempted=0；
- 删除提交后才破坏retained ciphertext时，第一轮`1/0/0/1`、第二轮attempted=0且vendor调用数不增；
- mcp_server指向unrelated model credential时，server删除但unrelated credential保持active；
- local false、operator-required false、vendor confirmed true与configuration audit全部进同一hash chain，
  credential ciphertext无明文命中。

真实 TLS RMCP `6/0/0` 保持catalog/call/grant纵向，并新增证明当前deployment bearer在server删除后
`revoked_at`非空、状态operator_required、runbook context存在；server/tool/grant仍`0/0/0`。

## 5. 本轮机械证据

- Infra unit：`327/0/0`；
- Server lib：`222/0/0`；
- Desktop all-feature lib：`131/0/3 ignored`；
- PG OAuth：`2/0/0`；PG+TLS RMCP：`6/0/0`；
- Infra/Server/Desktop all-target/all-feature Clippy `-D warnings`；
- `cargo fmt --all -- --check`与`git diff --check`；
- MCP OAuth、RMCP、SafeDialer、Application assembly、Tauri background/dependency、WebSocket guards；
- `cargo xtask parity-check`：`848/862/1710`、fixtures `23/22/45`、0 violation；
- fixed-upstream strict recount：`160 passed / 0 mismatch / 0 skipped`；
- `cargo xtask tools verify`、`electron-shim-check`（3 files / 405 LOC / 单一非Grok package）、
  `grok-inventory --check`（2,110 files）；
- `grok-bot` Git tree仍为`86f5a85f560f721677fa7e587a67ac0ffc036cb5`；非Grok
  `package.json`恰一且无npm lockfile。

按R63不运行`cargo xtask ci`，不派发GitHub Actions。

本批专用 PostgreSQL 集簇已 fast-stop 并删除；固定上游临时克隆在 strict 通过后删除。两者均为
可重新生成的 `/tmp` 证据环境，不包含产品数据。

## 6. 台账口径与明确剩余

本批只重验证既有 `T-API-0084`，无新T-ID、schema、fixture、route/UI、event、依赖或Cargo变化：

- API=`94 done / 80 todo / 174`；
- parity=`848 done / 862 todo / 1710`；
- fixtures=`23 done / 22 todo / 45`；
- overlay carry/revalidate/split/superseded=`1299/403/2/6`；
- native latest=`0029`，schema=`47表/478列/342 NOT NULL/269约束/97索引`。

仍未闭合：三家recorded/live provider trace、acting Approval完整computer/thread/cancel集成、computer
runtime budget、Desktop Local OAuth、RMCP/computer/file/shell protocol cancel、四个Plugins/Skills UI
route、Browser/file/shell、ScreenHub/viewer ticket、Desktop真实Wry/正式发行与golden、P1 Windows/runsc
真机、G2外审/KMS/Windows、G8 typed operator-attestation/最终secret retirement/迁移发行及其它v4余项。
