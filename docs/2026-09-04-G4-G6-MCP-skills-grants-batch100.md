# Batch100：MCP skills、grants 与 actor-specific discovery

日期：2026-09-04

implementation：`62d26501233b843ab9b34092b8c16b8a71d0f68c`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4
§5.2、§6.2、§8.6、§9.1、§9.3、§9.6、§13.2–§13.3、§15.3、§17.2、§24 G4/G6、§28.1 R174

固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`

## 1. 本批结论

本批闭合固定上游五条 API 的 Rust-owned 后端纵向：

- `POST /api/plugins/skills`；
- `DELETE /api/plugins/skills/{slug}`；
- `POST /api/plugins/grants`；
- `DELETE /api/plugins/grants`；
- `GET /api/plugins/for/{agent_id}`。

Server 与 Desktop custom protocol 只负责认证、freshness、framing、大小限制和错误映射，五条路径
都进入同一个 typed `ApplicationService`。skill 是显式调用时注入的 instruction，不是 capability；
personal skill 归当前 actor，deployment skill 只允许 current admin。MCP grant 始终 admin-only；普通
用户只能把自己的 personal skill 给自己拥有的 active Bot，单有 skill 或单有 Bot 都不够。

本批不是完整 MCP Admin/G4/G6 完成。固定上游 legacy `POST /api/plugins/call` 没有仓内调用者，
Rust 版也不能绕过既有 run/decision/attempt/capability/outcome 管线，因此仍单独保持 todo；
`/admin/plugins` index/detail/tool、`/admin/skills`、AppSidebar/AdminSidebar、browser journey/golden/AX、
Desktop Local OAuth、custom private user-OAuth、管理员删除后的 vendor revoke/runbook 也未闭合。

## 2. 固定上游复核

在独立临时目录浅克隆固定 commit，逐文件读取：

- `server/src/plugins/routes.ts`；
- `server/src/plugins/store.ts`；
- `app/src/routes/_authed/admin/skills.tsx`。

复核结论：personal skill 可由本人写；`global=true` 只有 admin；更新已有 slug 时 owner 不随输入
改变；MCP grant 是 admin 的；personal skill 只有同时满足 skill owner 与 Bot owner 才能授权；
for-Agent 必须先验证 Bot 可见性。固定上游的裸 MCP grant 只写 kind/ref/agent，不能直接照译到
native0017 之后的 Rust runtime；本批在授权事务中写入完整 current catalog binding。

固定上游管理页还会把 `grantedTo` 的全部 Bot id 返回给每个登录用户，这是跨 private/tenant
枚举面；Rust 版按 v4 P0 不变量收紧为当前 tenant 内、当前 actor 可见且未删除的 Agent。admin
仍可看本 tenant 全部，不能跨 tenant。

## 3. closed contract 与 transport

contracts 新增 closed DTO：skill mutation/list、grant kind/mutation/receipt、for-Agent tool/skill
projection。请求不能携 actor、owner、role、effect、schema hash、catalog/credential generation 或
transport fingerprint；DELETE query 的重复、未知、缺失键均拒绝。skill 的 title/summary/instructions
虽然必须按业务返回，但其 `Debug` 只显示 slug、scope、长度与计数，不打印内容、owner 或 Bot 列表。

五个 typed command/reply 同批加入 exhaustive application dispatch、operation kind、Agent gateway
拒绝表、channel reply 拒绝表与 testkit transport registry。Server 写面使用 fresh trusted Origin；
Desktop 使用 host-owned freshness，成功响应统一 `no-store`，写入后请求 body 被清零。

## 4. skill 事务语义

skill slug 精确匹配固定上游 lower-kebab grammar（2–40 bytes）；title ≤256 bytes、summary ≤4 KiB、
instructions 1–64 KiB。每个 slug 先取得 transaction advisory lock，再锁 existing row：

- 新 personal row 的 owner 来自 `AuthContext.actor`；
- 新 deployment row 的 owner 为 NULL，且 current DB role 必须仍是 admin；
- 更新已有 row 只改 title/summary/instructions，绝不改 owner；
- 非 admin 对他人、deployment 或不存在 skill 的更新/删除统一 NotVisible；
- admin 可幂等清理未知 slug。

save mutation、closed `skill_saved` hash-chain audit、返回给该 actor 的 visible skill list 在同一事务；
不存在“写已提交、随后 refetch 失败却回 503”的模糊结果。delete 同事务清 exact skill grants、skill row
并写 `skill_removed` audit；audit 失败则业务 mutation 一并回滚。

## 5. grant 与 discovery 事务语义

每次 mutation 都在事务内重新读取并 `FOR SHARE` 锁住 current `users.auth_generation` 行；角色/撤权
变化必须在本操作提交后才能完成，授权与撤权有明确线性化点。Agent 从 `agents + agent_profiles +
deployment_packages` 读取 tenant、owner、visibility、deleted、system facts，并复用 domain
`can_access_agent`。

MCP grant 只有 enable 时要求 current available tool，并在同一事务写入：

- `catalog_generation`；
- `schema_hash`；
- `effect`；
- `transport_fingerprint`；
- `credential_generation`；
- `state=active`。

revoke 不要求 tool 仍在 current catalog，因此 admin 能清掉 missing/stale grant；对 soft-deleted Agent
也允许同 tenant admin 做清理。skill grant 的 catalog binding 六列保持 NULL，普通用户仍必须同时是
skill owner 与 active Bot owner。每次 grant/revoke 都在同一事务写 `configuration.changed`，payload
只允许 closed `change` 与 Bot id；重复 revoke 幂等但仍留下可追责 audit。

for-Agent 使用 Repeatable Read 单事务完成 current actor 检查、Agent tenant/visibility/deleted 检查、
credential-aware current MCP intersection 与 granted skill projection，不能把多个 READ COMMITTED
statement 的不同时间点拼成一个从未存在的结果。catalog、credential 或 transport 任一 generation/
fingerprint 漂移，旧工具立即不再返回；只有重新授权才恢复。

## 6. 真实 PostgreSQL 17.11 证据

一次性 PostgreSQL 17.11 cluster 使用 host `scram-sha-256`。新增 ignored integration
`plugin_admin_runtime` 在正式 baseline0012 + native0013..0029 上证明：

1. personal/global skill、owner-preserving admin edit、他人 overwrite/delete、普通用户 unknown delete；
2. personal skill→own Bot 正向，personal skill→他人/public Bot 负向，admin deployment skill 正向；
3. 非 admin MCP 拒绝，admin grant 的六个 current binding 字段逐值相等；
4. 普通用户管理页只见 own/public `grantedTo`，看不到他人 private 与 cross-tenant Agent；admin 只见
   本 tenant 全量；
5. private/deleted/missing/cross-tenant for-Agent 均 NotVisible，admin 也不能跨 tenant；
6. catalog generation、credential generation、endpoint/transport fingerprint 三次漂移均令 tool 消失，
   三次 re-grant 后才恢复；
7. tool unavailable + Agent soft-delete 后 admin 仍可 revoke，第二次 revoke 幂等；skill delete 无残 grant；
8. 15 条 plugin config audit 的 `prev_hash/row_hash` 连续且 payload 只有 `change`/`bot`，instruction canary
   为 0；注入 audit INSERT trigger 失败后 skill row 为 0。

最终实跑 `1 passed / 0 failed / 0 ignored`。测试夹具首轮把 custom vendor 写成展示名而非 URL host，
被 production `server_identity` 校验正确拒绝；修正夹具后才记为绿。

## 7. 审计时修复的既有问题

本批没有只实现 happy path；人工复核另外修复：

- 管理页跨 private/cross-tenant `grantedTo` id 泄漏；
- 普通用户对未知 skill 的 delete oracle/audit spam；
- current tool 消失后 admin 无法 revoke stale MCP grant；
- soft-deleted Agent grant 无治理路径；
- READ COMMITTED 多语句拼接与 current actor 检查无行锁的撤权竞态；
- skill instructions 通过派生 Debug 泄入普通日志；
- `configuration.changed` 空 payload 无法区分 save/remove/grant/revoke。

另有一条与 MCP 无关但在本批守门中真实暴露的历史 guard 漂移：WebSocket dependency guard 仍宣称
server 只能在 `threads.rs` 使用 WebSocket，而已有 Channel Activity 与 Tool Approval 两条 typed caller。
guard 已改为三文件精确白名单，并为后两条补 1 KiB inbound、trusted Origin、read-only 1008 close
反向约束；重跑为绿。这不计入五条 MCP API 的完成度。

## 8. 本机守门结果

- Contracts `104/0/0`；Domain `372/0/0`；Application `165/0/0`；Infra `326/0/0`；
  Server `221/0/0`；Desktop `131/0/3 ignored`；transport parity `8/0/0`；
- Batch100 PG17/SCRAM integration `1/0/0`；
- 八个相关 crate all-target/all-feature Clippy `-D warnings`；Contracts/UI WASM check；
- SafeDialer、RMCP、MCP OAuth、Application assembly、Tauri dependency/background、UI dependency与
  corrected WebSocket guards；
- `cargo xtask tools verify`、`electron-shim-check`、`grok-inventory --check`；
- `cargo xtask parity-check`：0 violation；
- fixed upstream strict recount：`160 passed / 0 mismatch / 0 skipped`。

受限 sandbox 的 Infra 全集曾有 15 项 loopback bind 因 `Operation not permitted` 失败；允许本机 socket
后重跑 `326/0/0`，没有把环境拒绝记成通过。全特性构建使历史 `target/` 膨胀到 107.1 GiB，已只用
`cargo clean` 删除可再生 build artifact，源码/Git/用户未跟踪报告均未动；随后按仓库 pin 重新执行
`cargo xtask tools fetch` 恢复被清掉的四个工具，并由 `tools verify` 逐版本/摘要复验，最终重新完成
所需构建。

## 9. 台账变化与明确剩余

- API：`88/86/174 → 93/81/174`；关闭 T-API-0089–0093；
- fixtures：仍为 `23/22/45`，native latest 仍为 0029，schema 仍为
  `47表/478列/342 NOT NULL/269约束/97索引`；
- parity：`842/868/1710 → 847/863/1710`；
- overlay：`1299/403/2/6`；0 violation。

T-API-0094 legacy call 仍 todo。Plugins/Skills 四个正式 route 与 AppSidebar/AdminSidebar、正式
browser journey/golden/AX、Desktop Local OAuth、custom private user-OAuth、admin delete vendor revoke
runbook、三家 recorded/live provider trace、RMCP/computer/file/shell protocol cancel、完整 G4/G6/G8
均未完成。

本批没有 schema migration/fixture、UI/CSS/locale/bundle、依赖或 Cargo.lock 变化；没有运行被 R63
禁止的 `cargo xtask ci`，没有派发 GitHub Actions，没有运行 npm，没有改 `grok-bot`。外部工具预留的
Golden RGBA comparator、Google Drive brand asset/provenance、BrowserInput→CDP pure mapper 不在本批
改动范围，待独立分支提交后另做审计。
