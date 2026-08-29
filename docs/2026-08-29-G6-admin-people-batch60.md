# OpenBot G6 Admin People Batch60

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-G6-admin-people`

基线：Batch59 PR #42 已以 merge commit `05e792d30601728de47207519a5ce73669a0bfc6` 合入 `main`。

implementation：`aeb38f90b43b1796fd961fc02a3a10cb1b71f359`

## 1. 结论

本批把既有 production People list/role/access 原子能力接成受 AdminShell 保护的 `/admin/people` 真实旅程：

- 固定 50 条 keyset、250 ms server-side search，不拿首屏做本地假搜索；
- 当前用户与 `INITIAL_ADMIN_EMAILS` 配置管理员的角色/访问控件都禁用；
- 普通成员可以 user/admin 双向切换，也可以 remove/restore 访问；
- mutation body 只含 `role` 或 `revoked`，身份、actor、generation 和 admin 权限仍只由 Server 铸造；
- non-admin 在 child 构造前统一 NotFound，People API 请求为 0。

只关闭 `T-ROUTE-0020`。`T-UI-0028` 是完整上游 AdminSidebar，IdP/Credentials/Skills/Plugins/Components/Computers 等 destination 仍未实现；`T-UI-0140` 是 formal People golden，本轮仍 0 张，因此两者都保持 todo。

## 2. Typed transport 与页面不变量

共享 contracts 新增三个 closed wire 类型：

```text
ChangePersonRole { role }
ChangePersonAccess { revoked }
PersonResponse { person }
```

UI 只请求：

```text
GET  /api/admin/people?limit=50[&search=<encoded>][&cursor=<opaque>]
POST /api/admin/people/<encoded-id>/role    { role }
POST /api/admin/people/<encoded-id>/access  { revoked }
```

全部请求都是 same-origin credentials、no-store、redirect-error。响应继续检查每页不超过 50、person id 页内唯一、跨页不重复、provider 列表有界且严格排序、cursor/字段无控制字符；mutation receipt 必须回同一 person id，且结果逐字段等于请求方向。

搜索使用固定上游的 250 ms debounce；每次输入与每次请求分别用 checked generation/epoch，旧 timer 和旧响应都不能覆盖新查询。计数耗尽直接显示失败，不回绕。

## 3. 浏览器发现并修复的 stable-owner 缺陷

第一版 release WASM 首屏 50 行正常，但点击 Show more 后真实出现 `RuntimeError: unreachable`。根因是分页 worker 在自己的 reactive owner 内创建 52 个 row `RwSignal`；worker 完成即 dispose 该 owner，下一次分页/渲染读取已失效信号。

修复后 loader 的启动与 row signal 创建都固定到 `AdminPeoplePage` 的 stable Owner，和 Batch56 Approval receipt 的生命周期裁决一致。重新构建、重启 fixture、打开全新 tab 后：

- 50 → 52，unique row id 52，Load more 消失；
- WASM/runtime console error 0；
- hard reload 与后续 mutation 不再触发 disposed signal。

这条缺陷没有被 host 单测掩成完成；旧 bundle、旧 host 和旧 console 全部丢弃后才重跑证据。

## 4. Release memory fixture 浏览器证据

确定性 fixture 有 52 行，其中 `Search Target` 位于首屏之外，并另含 self、configured-admin、revoked 三种状态。

- 首屏 rows=50、Load more=1；点击后 rows/unique=52/52、button=0；
- 输入 `Search Target` 后只发一次 server query，结果恰 `fixture-search-target` 一行；
- self 与 configured-admin 的 access button / admin switch 都 disabled；revoked 行显示 Restore；
- Search Target 完成 user→admin、remove→restore，hard reload 后仍 admin 且 active；
- zh-CN/English 即时切换后 h1、描述、按钮、switch accessible name、search placeholder 全部对应 locale；
- 1280×900 与 600×900 均 horizontal overflow=0；600px actions 落第二行；
- main=1、nav=2、h1=1、admin current=1、duplicate DOM id=0、visible alert=0。

Chromium 对 modulepreload 上不适用的 integrity 属性固定报告 1 条浏览器 warning；本轮正向 app/WASM console error 为 0。该 warning 没有被写成应用错误，也没有伪报为 0 warning。

## 5. 真实 PostgreSQL/session 浏览器竖切

一次性 PostgreSQL 17.11 仅监听 `127.0.0.1:55460`，host auth 为 SCRAM-SHA-256；数据库名满足既有 testkit guard 的 `openbot_ui_approval_fixture_` 前缀。PG 模式继续使用 production `SessionTokenHash`、`PostgresSessionAuthResolver`，本批把 People port 切到 production `PostgresPeopleAdministration`。

前置实得：

```text
GET /api/admin/people（无 cookie） = 401
GET /api/__fixture/session/start   = 303
```

浏览器只能经该 host-only HttpOnly/Lax/no-store 响应取得 cookie。进入 People 后实得 actor/target/configured-admin 三行；self/configured 控件禁用。对 target 完成 user→admin→user 与 remove→restore 后，数据库最终逐字段为：

```text
auth_generation | role | revoked rows | target sessions | role audits | revoke audits | restore audits | actor sh1_ sessions
3               | user | 0            | 0               | 2           | 1             | 1              | 1
```

中间 remove 状态另实得 `2|admin|1|0|1|1`，hard reload 仍显示 admin + revoked，证明不是前端本地改行。

负向在一个事务中删除当前 actor 的 admin role，并把 user/session generation 同步到 2；全新 tab 访问 `/admin/people`：

- 只发生 global probe + shell gate 两次 `/api/admin/status` 403；
- `/api/admin/people` 请求 0；
- localized NotFound，global Admin link/admin nav/People rows/search 全部 0；
- 两条 console error 是上述预期 403 network resource 事件，WASM/runtime error 0。

fixture 停止时 approval waiter 因当前 actor generation/role 已变而收口为 denied，符合既有 fail-closed 语义。随后 fixture 与 PG 均停止；data/socket/log/password 精确删除，`pg_isready 127.0.0.1:55460` 无响应。

## 6. 机械证据

| 面 | 结果 |
| --- | --- |
| `openbot-contracts` | `88/0/0` |
| `openbot-ui` | `139/0/0` |
| `openbot-server --all-features` | lib `213/0/0`；main `7/0/0`；migrate `3/0/0`；fixture `3/0/0`；PG-only suites按定义 ignored |
| Clippy | contracts/UI/Server all-targets/all-features `-D warnings` 通过 |
| UI wasm32 | all-features/locked check 通过 |
| GUI build | pins verify + release/offline/locked Trunk build 通过；零 npm |
| i18n/design/CSS | 591 leaf keys；93 Rust files/74 icons；303 class literals |
| bundle | wasm gzip `1,498,159/3,670,016`；CSS `100,390/131,072`；fonts `740,216/819,200`；scripts=`1/0` |
| parity | routes=`12/20/32`；UI=`87/65/152`；总=`699/995/1694`；overlay=`1596/96/2/0`；0违反 |
| strict recount | `159/0/0`，固定上游 `891df72f…`，skip 0 |
| Grok/shim | tree `86f5a85f…`；inventory 2,110；shim `405/600`；单 package/零 npm 锁守卫通过 |

首次 `cargo test -p openbot-ui` 在链接前因磁盘只余 323 MiB 报 `No space left on device`，没有执行测试。只删除可再生成的 Cargo target；随后仓库 pins 重新下载并由 `tools verify` 校验，原 UI 测试重跑为 `139/0/0`。该环境失败不冒充测试失败，也不计作通过。

## 7. 明确未做

- 未实现完整 AdminSidebar 其余 destination；`T-UI-0028` 保持 todo。
- 未生成 People formal golden；`T-UI-0140` 保持 todo。
- 未把 testkit 303 冒充 OIDC/SAML 登录；真实登录协议证据范围不变。
- 未新增 invite、批量操作或其它上游没有的 People 产品能力。
- 未运行 `cargo xtask ci`，未派发 GitHub Actions（R63 manual-only）。
- P1 Windows/runsc runtime 仍红，未进入 P2；`grok-bot/` 零改动，无 Grok 文本/能力进入本批。
