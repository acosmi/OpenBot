# OpenBot G4 Approval Production Session-Cookie Browser Batch58

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-G4-approval-session-browser`

基线：Batch57 PR #40 已以 merge commit `908e56449c28c84cf6b585da27b784601c3cea43` 合入 `main`。

implementation：`1fdc65c4261c9dfa4aaf4f5df8ce58b94a24630a`

## 1. 结论

本批关闭“approval production PostgreSQL session-cookie resolver 浏览器竖切”，但不把testkit cookie bootstrap冒充OIDC/SAML登录协议，也不关闭完整thread/cancel/computer旅程：

```text
raw fixed test token
  → keyed SessionTokenHash (DB only sh1_…)
  → native sessions row + generation/role/current user
  → browser without cookie: approval API 401 / card 0
  → testkit-only 303 Set-Cookie (host-only, HttpOnly, Lax, no-store)
  → production PostgresSessionAuthResolver
  → same-origin approval GET + WebSocket + fresh POST
  → durable grant/audit/waiter + hard reload
```

变化仍只在`required-features=testkit`的既有UI fixture；production server/main没有bootstrap route、固定token或test env。默认memory模式打印`AUTH_MODE=fixed`，session bootstrap与PG proof API都机械404。

## 2. 真实 production resolver 边界

PG模式现在seed一条native session：

- raw token只存在于testkit二进制常量和浏览器Set-Cookie；
- DB token由`SessionTokenHash::compute(SessionToken, SessionHashKey)`产生；
- proof实得`sessions=1 / hashedSessions=1`，因此库内唯一session必为`sh1_` keyed hash；
- session/user AuthGeneration均为1，created/updated在fresh窗口内，expiry为1小时；
- DB roles含user/admin，resolver仍按production `resolve_effective_role`铸唯一权威角色；
- resolver继续重验revoke、role、session generation、current user generation、expiry/freshness。

Application与approval request仍只认同一deployment/tenant/actor/generation。浏览器cookie不能自报scope或角色。

## 3. testkit bootstrap为什么不是登录旁路

`GET /api/__fixture/session/start`只在PG fixture mode动态挂载；默认memory模式和所有production binary均无该route。它只做：

```http
303 Location: /approvals
Set-Cookie: openbot_session=<fixed-test-token>; Path=/; HttpOnly; SameSite=Lax
Cache-Control: no-store
```

没有Domain，因此host-only；loopback HTTP测试不写Secure，不外推正式HTTPS cookie配置。它不接收body/query/actor/role/token输入，也不模拟OIDC/SAML认证；唯一作用是让不可直接操作HttpOnly cookie的真实浏览器进入已seed session。

## 4. 最终PG17.11/SCRAM浏览器证据

最终独立实例使用专用库`openbot_ui_approval_fixture_batch58`，host=`127.0.0.1`，PG `17.11 (Homebrew)`、`password_encryption=scram-sha-256`、role hash前缀检查真。

浏览器前置：

- 无cookie `GET /api/tool-approvals` = 401；bootstrap=303；
- 浏览器先直开`/approvals`：article=0，显示“无法加载审批列表。”，console warn/error=0；
- 导航bootstrap并跟随303后：article=1，权威target与`[redacted]`可见，证明真实resolver放行同一API/socket。

点击批准后：

```json
{"approvals":1,"granted":1,"grantedAudits":1,"hashedSessions":1,"mode":"postgres","pending":0,"requestedAudits":1,"sessions":1,"summariesCleared":1,"waiter":"granted"}
```

DOM article=0、status=`已提交批准。`、console0；server打印waiter granted。硬重载后article仍0、approval load error=0，证明cookie与PG终态都持续。结束后PG fast shutdown，端口无响应，data/pwfile/script零残留。

## 5. 机械证据

| 面 | 结果 |
| --- | --- |
| fixture bin unit | `2/0/0`；seed/request/DB guard/hash shape与303 cookie属性 |
| fixture bin Clippy `-D warnings` | exit 0 |
| default memory mode | `APPROVAL_MODE=memory / AUTH_MODE=fixed`；PG proof与session start API均404 |
| PG session browser | 无cookie401→bootstrap303→card1→grant proof→hard reload，逐项见§4 |
| `cargo xtask parity-check` | `695/999/1694`；overlay `1600/92/2/0`；branch required revalidate=0；0违反 |
| fmt/diff | `cargo fmt --check`与`git diff --check`通过 |

## 6. 明确未做

- 未测试真实OIDC/SAML登录跳转；testkit 303不是登录协议证据。
- 未把bootstrap、fixed token/hash key或PG proof加入production产品面；无新API/T-ID。
- 未完成approval完整thread transcript/cancel/computer generation旅程。
- 未运行`cargo xtask ci`或Actions（R63 manual-only）。
- P1 Windows/runsc runtime仍红，未进入P2。
- `grok-bot/`零改动；无Grok产品能力、无npm。
