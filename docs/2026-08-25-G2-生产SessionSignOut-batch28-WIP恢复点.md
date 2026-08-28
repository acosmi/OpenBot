# Batch 28 WIP 恢复点：生产 Session Sign-out

> 日期：2026-08-25。分支 `codex/2026-08-25-G2-session-signout`，base =
> Batch27 正式 head `4fcefca48a0a2928ba872770ac08f28329232078`，implementation =
> `36be5747da2e5501fe2d5103635dd63df03668d0`。正式文档为
> `docs/2026-08-26-G2-生产SessionSignOut-batch28.md`。只跑本地定向测试；
> 不运行 `cargo xtask ci`，不派发 Actions，不处理 `grok-bot`，不修改/暂存/
> 提交并行出现的 `docs/assets/`。

## 本批唯一范围

- `GET /api/me/session`：返回闭集 `{revocable}`，让同一 GUI bundle 在multi-user显示登出、
  在single-user loopback隐藏；不改上游parity的 `/api/me {user}` 形状；
- `POST /api/auth/sign-out`：先验已解析 session + trusted Origin，再按
  `(session_id, actor_id)` 删除 PostgreSQL session，最后清 host-only HttpOnly/Lax cookie；
- T-TEST-0728 `forwards logout requests to Better Auth` 改为 Rust 生产 sign-out 证据；
- `api-auth-wildcard` T-API-0107 继续 todo：本批只闭 sign-out，不写成完整 wildcard 对等；
- 新增 T-API-0162 跟踪 session status 新 API。

## 构造性边界

- handler 不接 token/session/actor body字段；只使用 `ResolvedAuth` 内部已验 session ID 与
  `AuthContext.actor`；明文 cookie/token/hash 不进日志/响应/GUI state；
- Origin 在任何 DELETE 前判定；不要求 admin/fresh，登出只撤当前 session；
- single-user 没有可撤数据库 session；status=false，POST 明确冲突而不写假成功；
- 删除发生竞态时幂等收口；不撤其他设备/session，不推进 auth generation；
- AppSidebar/channel realtime 本批不勾；下一批另闭 `/api/channels/events` 后才替换旧壳。

## 计划证据

- server unit：无Origin零revoke、single-user明确拒绝、multi-user revoke恰一次、cookie clear exact/no-store；
- PostgreSQL 17 SCRAM：真 cookie 登出后行消失，同cookie再请求401，其他session仍200；
- transport/ledger/recount、server/auth impacted Clippy；不用 fixed/test-only resolver 代替生产 PG 证据。
