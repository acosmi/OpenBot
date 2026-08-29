# OpenBot G2/G6 Sign-in 与 Authenticated Guard Batch64

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-G6-sign-in-guard`

基线：Batch63 PR #46 已以 merge commit `cceb0d4ef7360d9e7396b4dce4792122b78f52e5` 合入
`main`。

implementation：`5c51626d2f588e2b52b5adcfbed99f05f1743e5a`

## 1. 结论

本批关闭两条固定上游 route/layout 旅程：

- `T-ROUTE-0002` authenticated guard；
- `T-ROUTE-0031` `/sign`。

只有 `/api/me` 的 401 被解释为“未登录”。403、404、网络失败、坏 JSON 与其它 Server 失败都停在
可重试认证失败页，不重定向成 sign-in。受保护 App shell、偏好读取、Sidebar 与 runtime child 只在
`/api/me` 成功后构造；已登录访问 exact `/sign` 时回 `/`，同时覆盖 full navigation 与
AuthenticatedBoundary 内的 client pathname 观察。

`/sign` 等完整 `/api/capabilities` 后一次性画出环境 provider；provider 是
`google | microsoft | okta` 闭集、严格排序去重。动态 SAML/OIDC 仍只投影 `ssoConfigured: bool`，
匿名响应类型装不下 enterprise provider、domain、issuer 或 secret。企业邮箱只 POST route ticket，
收到 exact 202 后才 full-page 导航 `/api/auth/sso/continue`。

## 2. 第一真源与固定上游

固定上游 `891df72f1827454d8b353d108fe5dd2313b7e30d` 本轮实读：

- `_authed.tsx::beforeLoad` 在无 current user 时 redirect `/sign`，runtime provider 位于边界内；
- `sign.tsx::beforeLoad` 在已有 user 时 redirect `/`，并在首屏前取齐 provider options；
- provider 闭集恰为 Google / Microsoft / Okta；三者走同一个 client call；
- enterprise SSO 只收 email，不收 password；provider/domain 不在匿名配置中枚举。

本仓既有 production Axum 面已经具备：`GET /api/me`、`GET /api/capabilities`、环境 OIDC start、
企业 SSO start/continue、OIDC/SAML callback 与 keyed session resolver。本批没有造第二认证器，工作是把
这些权威面接到共享 Leptos route boundary，并把匿名 wire shape 收成 wasm-safe contracts。

## 3. 类型与边界

`openbot-contracts::auth` 新增：

- `AuthProviderId`；
- `AuthenticationCapabilities`；
- `AuthenticationStartResponse`；
- `EnterpriseSsoStartRequest` / `EnterpriseSsoRoutingAccepted`；
- 匿名 email 512-byte transport budget。

Server 从 `PreAuthSurface` 的环境 ID 逐项 parse 到闭集；未知、重复或乱序配置统一 503，不把漂移数据交给
浏览器。UI 再验证 canonical provider list，OIDC target 只接受无 userinfo/fragment 的 HTTPS 绝对 URL。
企业邮箱由 Server 保留原始规范化与 domain routing 权威，UI 只做空值/transport budget 前置。

Desktop custom protocol 新增 `GET /api/me` typed lane：window label 先解析成 host-bound
`AuthContext`，再执行同一个 `AppCommand::GetCurrentUser`。未绑定 window 仍在任何 asset/API 前 401；
没有 cookie、OIDC 或 renderer 自报身份的新入口。

## 4. Guard 与 sign 状态机

`AuthenticatedBoundary` 的状态只有 checking/authenticated/signed-out/redirecting/failed：

- mount 与 retry 使用 checked generation，旧响应不能覆盖新 probe；
- 401 + protected path → replace `/sign`；
- 401 + `/sign` → mount `SignInPage`；
- authenticated + `/sign` → replace `/`；
- authenticated + 其它 path → 构造 protected child；
- 其它失败 → 不构造 child，显示可重试失败页。

`provide_ui_preferences` 从无条件 AppShell 移到通过 guard 后的 `AuthenticatedShell`。Input 的 email type
自动带 `autocomplete=email`；可选 Enter submit 显式保留 IME composition，不会把合成确认当提交。

provider start 的远端错误正文不进 GUI；稳定类别在本地化后点名对应 provider。企业邮箱 Enter 与按钮走
同一函数，提交期间全控件锁定，成功导航期间不先复位造成重复 start。

## 5. Production PG / release 浏览器证据

一次性 PostgreSQL 17.11 只监听 loopback，SCRAM-SHA-256；数据库名受既有
`openbot_ui_approval_fixture_` 前缀 guard 约束。testkit host 仍使用 production
`PostgresSessionAuthResolver` 与 keyed hash session；新增 auth journey middleware 只在
`OPENBOT_UI_AUTH_FIXTURE=1` 的 required-feature test binary 存在，production binary 不挂。

本轮亲跑：

- 无 cookie `/api/me` = 401/no-store；capabilities 精确为三家 + `ssoConfigured:true`；
- 隔离 hostname 的 `/channel/new` 与 `/approvals` 都回 `/sign`，guard 前 protected 请求增量 0；
- Google POST 得 fixture 503 后仍在 `/sign`，可见 alert 精确点名 Google；
- email Enter 得 start=1、continue=1，带 HttpOnly route ticket 回
  `/sign?fixture-sso=continued`；
- test-only 303 bootstrap 后 production resolver 真实 mount `/approvals`，PG proof 为
  `sessions/hashedSessions=1/1`；已登录硬访问 `/sign` 回 `/`，capabilities 计数不增加；
- 最终干净 fixture 的匿名闭集计数：capabilities=4、OIDC start=2、enterprise start=1、
  continue=1、me=5、protected=0；运行日志 0。

最新逐字 bundle 另用不落盘的 loopback 静态 auth host 复验：`/channel/new→/sign`、
`autocomplete=email`、Enter→continue、authenticated mount `/`、运行日志 0。该轻量 host 只证明最终
WASM，不冒充 production Server/PG；client-side exact `/sign` 判定由 UI 单测覆盖，浏览器控制层不提供
可用 page history 改写，故未写成浏览器通过。

1280 机械审计得到 overflow0、main1/h1 1/nav0、provider3、duplicate/nested0；600 宽最终错误态截图
无横向裁切，main1/h1 1/provider3、alert exact、日志0。截图仍只作本轮视觉 QA，不冒充 formal golden。

## 6. 品牌资产审计与明确未完成

GUI 第一真源 §4.6.3 要求 Google/Microsoft/Okta sign-in 标与 Google Drive 标全部使用官方 SVG，
并在 provenance 登记来源、版本与条款。本轮联网只读/临时下载官方来源：

- Google guidelines：`https://developers.google.com/identity/branding-guidelines`；官方
  `signin-assets.zip` 含 Android+Web SVG；
- Microsoft guidelines：`https://learn.microsoft.com/en-us/entra/identity-platform/howto-add-branding-in-apps`；
  页面直接提供 343-byte Microsoft symbol SVG；
- Okta newsroom：`https://www.okta.com/en-gb/newsroom/`；官方 2025-04 press kit
  `logos-04-2025.zip` 共 17 文件，Auth0 有 SVG，但 Okta 自身只有 black/white、L/M/S 六个 PNG；
  `https://developer.okta.com/copyright/` 明示 Okta 标为其商标且 all rights reserved。

因此本轮没有把 Okta PNG 描摹、第三方 icon 或固定上游 currentColor 圆环伪装成官方 SVG，也没有只落
Google/Microsoft 两家造成按钮不等权。`T-UI-0039` 与 Google Drive brand icon 继续 todo，
`design-lint` 仍机械打印该状态。关闭 route 不等于关闭 brand component。

## 7. 机械证据

| 面 | 本轮结果 |
| --- | --- |
| contracts | `92/0/0` |
| UI | `155/0/0` |
| Server | lib `213/0/0`；fixture `7/0/0` |
| Desktop | `80/0/0`（doc ignored 1） |
| Clippy | contracts/Server/Desktop/UI all-targets/all-features `-D warnings`；最终 UI 单独复跑 |
| WASM/fmt | release/offline/locked Trunk 与最终 workspace fmt 通过 |
| i18n/design/CSS | `689` leaf keys；`97` Rust files/`74` icons；`334` class literals |
| bundle | wasm gzip `1,659,274/3,670,016`；CSS `109,378/131,072`；fonts `740,216/819,200`；scripts=`1/0` |
| parity | routes=`17/15/32`；tests=`377/670/1047`；总=`709/985/1694`；0违反 |
| overlay | carry/revalidate/split/superseded=`1570/122/2/0` |
| strict recount | fixed upstream `891df72f…`，`159/0/0`，skip0 |
| invariants | `grok-bot` tree=`86f5a85f…`；workspace单package/零npm lock |

## 8. 台账变化

- routes：`T-ROUTE-0002`、`T-ROUTE-0031` todo→done；
- tests：`T-TEST-0166/0168/0169/0171/0172` todo→done；
- revalidate：`T-UI-0007/0008` 与上述新 done 目标；
- routes `15/17→17/15`；tests `372/675→377/670`；总 parity `702/992→709/985`；
- overlay `1579/113/2/0→1570/122/2/0`。

`T-TEST-0167`（callback URL）与 `T-TEST-0170`（环境 provider 成功外跳）继续 todo：本批没有 live
environment IdP credential/HTTPS redirect endpoint，不把 503 失败态或 enterprise continue 替代为成功 OIDC。

## 9. 未做与清理

- 未运行全 workspace test；只运行变更面定向矩阵；
- 未运行 `cargo xtask ci`，未派发 GitHub Actions（R63 manual-only）；
- 未使用 live OIDC/SAML credential，不把 test-only auth host 冒充真实 IdP 登录；
- P1 Windows/runsc runtime仍红，未进入P2；
- `grok-bot/` 零改动，没有新增Grok产品能力或复制其文本；
- 一次性浏览器标签、viewport override、fixture、PG/socket/data/log、官方品牌临时下载均已清理；
  可再生 target/tools/debug/dist 在提交前后按磁盘约束再清理。
