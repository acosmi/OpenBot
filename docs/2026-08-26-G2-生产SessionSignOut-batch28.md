# G2/G6 Batch 28：生产 Session Sign-out

> 日期：2026-08-26。分支 `codex/2026-08-25-G2-session-signout`，base =
> Batch27 正式 head `4fcefca48a0a2928ba872770ac08f28329232078`，implementation =
> `36be5747da2e5501fe2d5103635dd63df03668d0`。

## 1. 已完成并打勾

- [x] T-TEST-0728：上游“forwards logout requests to Better Auth”改为 Rust 生产 sign-out；
- [x] 新增 T-API-0162：`GET /api/me/session` closed revocable status；
- [ ] T-API-0107 `api-auth-wildcard` 继续 todo：本批只关 sign-out子面。

API `40/121/161 → 41/121/162`，tests `265/782/1047 → 266/781/1047`；
全 parity `517/1155/1672 → 519/1154/1673`。G2 专项21文件仍是155/79/234。

## 2. 生产边界

- `ResolvedAuth` 只新暴露 `has_revocable_session()` bool；session ID 仍私有；
- `AuthResolver::revoke_session` 由 production `PostgresSessionAuthResolver` 实现，SQL 只是
  `DELETE sessions WHERE id=? AND user_id=?`；两值均来自已验 `ResolvedAuth`，wire 零输入；
- `POST /api/auth/sign-out` 先验 session，再验 trusted Origin，后删行，最后回204并清
  host-only `HttpOnly; SameSite=Lax; Max-Age=0` cookie；Secure 跟既有 public URL 策略；
- 坏/缺 Origin 在 DELETE 前拒绝；登出不要求admin/fresh，不撤其他设备session，
  不推进auth generation；
- single-user loopback 无数据库session；status=false，POST回409 `request_conflict`，不写假成功；
- `GET /api/me/session` 只回 `{revocable}` 且 no-store，不改 `/api/me {user}` parity形状；
- `openbot-ui` 已有 WASM-safe `load_session_status` / `sign_out_current_session`；后者只接受204。

## 3. 本机证据

| 面 | 结果 |
| --- | --- |
| Server framing | **2/0/0**；status true/false、缺Origin零revoke、trusted revoke恰一次、cookie clear/no-store exact、single-user409 |
| PostgreSQL | **17.11 Homebrew / host SCRAM / 1/0/0**；坏Origin两session仍在；正确204后只剩session-2；旧cookie访问channels=401，其他cookie=200 |
| contracts/UI | contracts closed wire **1/0/0**；UI API host test **1/0/0**；contracts+UI wasm32 绿 |
| Clippy/fmt | contracts/server/UI all-targets all-features `-D warnings` 与 fmt 绿 |
| production bundle | wasm gzip **374903/3670016**；CSS **63205/98304**；fonts **740216/819200**；external/inline=1/0 |
| parity/recount | **519/1154/1673**；API **41/121/162**；tests **266/781/1047**；0 violation/warning；**157/157/0** |

为运行真库证据，本机在 `/private/tmp` 建了临时 PostgreSQL 17.11 集群，仅监听
`127.0.0.1:55428`，host 为 SCRAM。测试后已停止进程并删除整个临时目录；没有连接/修改用户数据库。

## 4. 仍未完成

- [ ] Better Auth wildcard 下其余兼容路由与逐条错误矩阵；
- [ ] `/api/channels/events` PostgreSQL-backed WebSocket、AppSidebar/channel 生产接线与旧shell替换；
- [ ] G2 外审/KMS/Windows与其余G3–G8未闭面。

未运行 `cargo xtask ci`，未派发 Actions，未 push/建 PR，未处理 `grok-bot`。
并行出现的未跟踪 `docs/assets/` 未修改、未暂存、未提交。
