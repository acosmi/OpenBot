# Batch99：MCP Admin backend 与 custom private egress

日期：2026-09-04

implementation：`5070283b5f61c4780405de3f614435363042e138`

第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` v4 §6.2、§8.6、§9.1–§9.4、§15.3、§24 G4/G6、§28.1 R173
固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`

## 1. 本批结论

本批闭合四条固定上游 API 的 Rust-owned 后端纵向：

- `GET /api/plugins`；
- `POST /api/plugins/servers/custom`；
- `DELETE /api/plugins/servers/{id}`；
- `POST /api/plugins/servers/{id}/refresh`。

Server 与 Desktop custom protocol 都只做认证、freshness/framing/大小限制，业务统一穿
`Arc<dyn ApplicationService>` 的 typed command。custom MCP 的私网授权不再是 URL 字符串检查：
native 0029 保存管理员显式给出的 canonical numeric CIDR；同一集合同时进入 catalog transport
fingerprint、真实 `tools/list` refresh 和每次 `tools/call` 的 SafeDialer。默认空集合继续拒绝
loopback/private/metadata/special address；每个 redirect 仍重新解析、校验并绑定实际 SocketAddr。

本批不是“完整 MCP Admin/UI 完成”。skills/grants/for-agent/legacy call API、Plugins/Skills 四个
正式 route、AppSidebar destination、golden/a11y，以及管理员删除后的 vendor-side OAuth revoke/runbook
仍保持 todo；G4/G6 整关不勾。

## 2. 固定上游复核

在独立临时目录浅克隆固定 commit，逐文件读取：

- `server/src/plugins/routes.ts`；
- `server/src/plugins/store.ts`；
- `server/src/plugins/catalogue.ts`；
- `app/src/lib/plugins/queries.ts`；
- `app/src/lib/plugins/mutations.ts`；
- `app/src/routes/_authed/admin/plugins/index.tsx`；
- `app/src/routes/_authed/admin/plugins/$key.tsx`；
- `app/src/routes/_authed/admin/plugins/$key_.tools.$tool.tsx`。

固定上游当前 catalogue 恰有 Google Drive 一项；读面向所有 signed-in actor 开放，server
新增/删除/refresh 为 admin 写面。custom server 的 provenance 必须是 `custom`，未知工具按 write
处理，不能占用 reviewed catalogue slug。Rust 版保留这些语义，同时应用 v4 的 stronger rule：
private destination 只有显式 exact CIDR authority 才能通过唯一 SafeDialer。

## 3. native 0029

`native_0029_mcp_private_egress` 只给 `mcp_servers` 追加 nullable
`egress_allow_cidrs text[]` 与一个具名 CHECK：

- `NULL` 是 legacy/public-only；
- 非空/空数组都只有 `provenance='custom'` 可保存；
- 最多 32 项、无 NULL 元素、拼接后最多 2,048 bytes；
- Rust 再拒绝裸 IP、hostname、空白、host bits、非 canonical network，并排序去重。

PostgreSQL 17.11 实得 schema：

| 项 | 0029 | 相对 0028 |
| --- | ---: | ---: |
| public 表 | 47 | 0 |
| 列 | 478 | +1 |
| NOT NULL | 342 | 0 |
| 约束 | 269 | +1 |
| 索引 | 97 | 0 |
| 触发器 | 4 | 0 |

fixture：`fixtures/db/schema-0029.json`，5,612 行 / 162,098 bytes，SHA-256
`c661463f04f2bd38c308191697b6f84625fb091c8089e1fc7b1e42a453e7e4dc`；regeneration 开/关
各 `1/0/0`，逐旧列证明 0028 是 0029 子集，native ledger 恰 17。数据库负向证明
first-party row 携 private-egress CIDR 会被 CHECK 拒绝。

## 4. typed administration contract

contracts 新增 closed DTO：catalogue、configured server/current tool/current active grants、actor-visible
skills、custom registration 和 removal receipt。公开投影只暴露 credential 是否存在，不暴露 UUID、
密文或 token；`lastError` 只投影 `mcp_catalog_unavailable`，不复制 remote body。

新增 typed commands：

- `ListMcpAdminPage`；
- `AddCustomMcpServer`；
- `RemoveMcpServer`。

Application 在 port 前执行 admin role gate；signed-in list 保持固定上游开放语义。command kind、Agent
gateway 的 exhaustive reply 拒绝、channel handler exhaustive match，以及 testkit command ledger 同批更新，
无 wildcard 漏口。

## 5. custom registration、refresh 与 removal

custom registration 只接受：

- 2–40 bytes lower-kebab id，且不得 shadow `google-drive`；
- 1–256 bytes title；
- ≤8 KiB HTTPS base URL，无 userinfo/query/fragment；
- 可选 active、`kind=mcp`、`provider=server_id` 的 deployment bearer UUID；
- 最多 32 项/2,048 bytes exact numeric CIDR。

配置、credential pointer/generation、旧 credential 本地 retirement 与空 payload
`configuration.changed` hash-chain audit 在同一事务。提交后立即以 fresh credential broker 执行真实
catalog refresh。credential 或 CIDR 改变都会改变 current binding；v2 transport fingerprint 包含
endpoint/vendor/provenance/transport/protocol/CIDR 集合，旧 grant 在 refresh 中转
`suspended_missing`，重新出现不自动复活。即使 refresh 失败，旧 fingerprint 与新配置不再相等，
runtime decode 仍 fail-closed。

删除在同一事务中：本地 revoke deployment 与 actor token、删除 exact MCP grant refs、删除 server，
再由既有 FK cascade tools/user connection joins，最后写 hash-chain configuration audit。这里证明的是
local capability 与本地 credential retirement；删除后已无 endpoint/client material可做 vendor revoke，
因此 operator-driven vendor-side revoke/runbook 仍归完整 MCP lifecycle/G8，不在本批冒充完成。

## 6. 私网出口绑定到真实 effect

`SafeDialer::with_egress_policy` 只替换 destination policy，保留同一个 resolver 与 TLS roots。
`SafeRmcpClient::with_egress_allowlist` 被两处消费：

1. catalog `tools/list` refresh；
2. Agent runtime 每次 bound `tools/call`。

数据库里的 CIDR 先按 canonical、唯一、排序规则解码；任何篡改都返回 corrupt/503，而不是静默
降为 public policy。fingerprint 在锁前/锁后各算一次，配置并发变化使 refresh 失败关闭。Google Drive
REST 不可携 CIDR，继续只走默认 public SafeDialer。

## 7. 真实 PostgreSQL/TLS RMCP 证据

临时 PostgreSQL 17.11 先以隔离 cluster 生成 fixture，随后切换 TCP `scram-sha-256` 重跑。
真实 TLS fixture 使用 `idp.test` 证书/SNI、resolver 绑定到 `127.0.0.1`，TLS 后透明进入 pinned
RMCP 3.1.4 Streamable HTTP server：

1. 同一 private endpoint、空 CIDR：`mcp_connection_unavailable`，server row 保留并仅显示 stable error；
2. 加 `127.0.0.1/32`：真实 `initialize → tools/list → close`，得到 6 tools；
3. 给 `search_issues` 建 current active grant；管理页精确显示一个 Agent；
4. CIDR 增加 `10.0.0.0/8` 并轮换 deployment credential：catalog generation 1→2，旧 grant
   suspended=1，旧 credential revoked；
5. 删除后 server/tool/grant=`0/0/0`，两代 credential revoked=`2`；四次 config mutation 的
   audit `row_hash` 全非 NULL。

同时回归：

- MCP catalog missing/schema/vendor change + governed tool runtime：`3/0/0`；
- MCP OAuth connect/rotation/401 retry：`2/0/0`；
- production Agent callback→governed RMCP：`2/0/0`；
- schema/Drizzle ledger/secret decode（SCRAM）：`13/0/0`；
- native 0029 fixture/ledger：`1/0/0`。

## 8. 本机守门结果

- Contracts `103/0/0`；Application `164/0/0`；Infra `326/0/0`；Server `220/0/0`；
  Desktop `131/0/3 ignored`；transport parity `8/0/0`；
- 六个受影响 crate all-target/all-feature Clippy `-D warnings`；testkit transport Clippy；
- Contracts WASM 与 UI all-feature WASM check；
- SafeDialer、RMCP、MCP OAuth、Application assembly、Tauri dependency/background assembly guards；
- `cargo xtask tools verify`、`electron-shim-check`（3 files / 405 LOC / 单一 package）、
  `grok-inventory --check`；
- `cargo xtask parity-check`：0 violation；
- fixed upstream strict recount：`160 passed / 0 mismatch / 0 skipped`。

曾出现三次有效红灯并均修后重跑：trust PG 令 wrong-password 负向前提失效；新增 fixture 后四个 recount
期望仍是旧计数；Batch98 将 Desktop Agent constructor 改成 `start_with_remote_interrupts` 后 Tauri guard
仍匹配旧名字。三者都没有被记成成功。

## 9. 台账变化与明确剩余

- API：`84/90/174 → 88/86/174`；关闭 T-API-0080、0082、0084、0085；
- fixtures：`22/22/44 → 23/22/45`；新增 T-FIX-0045；
- parity：`838/872/1710 → 842/868/1710`；
- overlay：`1299/403/2/6`；0 violation。

仍为 todo：T-API-0089–0094（skills/grants/for-agent/legacy call）、T-ROUTE-0022–0025、
T-UI-0142–0144及正式 Plugins index golden、完整 AdminSidebar/AppSidebar、Desktop Local installed-app
OAuth、custom private user-OAuth metadata/token/revoke、管理员删除后的 vendor-side revoke/runbook、三家
recorded/live provider trace、RMCP协议级cancel及完整G4/G6/G8。

本批没有 UI/CSS/locale/bundle 变化，没有运行被 R63 禁止的 `cargo xtask ci`，没有派发 GitHub
Actions，没有运行 npm，没有改 Cargo.lock，`grok-bot` Git tree 仍为
`86f5a85f560f721677fa7e587a67ac0ffc036cb5`。
