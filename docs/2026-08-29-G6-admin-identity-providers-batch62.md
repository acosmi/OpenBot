# OpenBot G2/G6 Admin Identity Providers Batch62

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-G6-admin-identity-providers`

基线：Batch61 PR #44 已以 merge commit `da135aee9ecf8afcc59cac1f417afd4d0776c8db` 合入 `main`。

implementation：`a90c0891b222822c403fbdbb8f3de11407abf139`

## 1. 结论

本批把 W-7b 已有的 production deployment-owned SAML/OIDC store、register/list/delete 与 keyed
session 接成受 `AdminShell` 保护的 `/admin/identity-providers` 真实管理旅程：

- 页面只列动态注册 provider；环境配置的 Google、Microsoft、Okta 明示不在本页；
- SAML 与 OIDC 表单互斥，domain 决定 SSO routing；
- SAML 使用管理员直接粘贴的 metadata XML，不由 Server 抓取 metadata URL；
- OIDC discovery URL 只能由 issuer 推导，界面没有自由 endpoint 输入；
- register/delete 成功后重新读取 production list，不用乐观本地行冒充权威状态；
- non-admin 在 child 构造前统一 NotFound，IdP list/register/remove 请求不增加。

只关闭 `T-ROUTE-0019`。完整 AdminSidebar `T-UI-0028` 与 Identity Providers formal golden
`T-UI-0138` 继续 todo。

## 2. 公开 contract 与 secret 边界

原 `RegisteredIdentityProvider` / `SsoProtocol` 位于 infra，UI 无法按 §5.1 的依赖方向消费。本批把以下
browser-safe wire 上收到 `openbot-contracts::identity_provider`：

- `SsoProtocol`；
- `RegisteredIdentityProvider`；
- `IdentityProvidersResponse`；
- `IdentityProviderRemoved`；
- Server/UI 共用的 provider/domain/client/metadata/URL 上限。

公开 projection 的类型没有 client secret、SAML metadata、certificate 或 entry point 字段。Server list
直接序列化该 closed envelope，UI 反序列化时 `deny_unknown_fields`，再校：

- 最多 256 行；
- provider ID 形态与 ID 唯一；
- 每行最多 16 个 canonical ASCII domain，跨 provider domain 也唯一；
- issuer / registered_by 有界且无控制字符。

注册体刻意与 Server 输入分成两个 Rust 类型：

- browser 侧 `RegisterIdentityProviderRequest` 只实现 `Serialize`，字段私有，不实现
  `Clone` / `Debug` / `Deserialize`；
- Server 侧仍由 infra `SecretInput` 反序列化 client secret；该类型不可 `Clone` / `Serialize`，
  `Debug` 固定 `[REDACTED]`；
- 两者共享同一 JSON wire，由 contracts 两条 exact JSON test 与 infra 四条 config test闭合；
- OIDC constructor只接受 issuer/client，内部生成
  `issuer-without-one-trailing-slash + /.well-known/openid-configuration`；
- 取消、成功或 page owner drop 后表单信号销毁/清空；普通 response、日志与探针均取不到 secret。

这不是声称浏览器内存可被物理擦除；只保证没有持久化、回传、普通日志或长寿命 projection。

## 3. 表单、transport 与 owner

UI 在发请求前执行与 Server 同方向的约束：

- provider ID = `[a-z0-9][a-z0-9_-]{0,63}`；
- domain 按逗号切分、trim、小写、排序，拒空项、重复、非 ASCII、下划线与坏点/连字符形态；
- OIDC issuer 必须 HTTPS、有 host、无 userinfo/query/fragment；
- SAML entity ID 接受 `urn/http/https`，实际 sign-on endpoint 必须 HTTPS；
- client ID/secret、metadata、URL 逐项消费 contracts 上限；
- 空提交逐字段 `aria-invalid` + Field error，并有总表单 alert。

三个 transport 均 same-origin、credentials same-origin、redirect error、no browser cache：

```text
GET    /api/admin/identity-providers
POST   /api/auth/sso/register
DELETE /api/admin/identity-providers/{provider_id}
```

list 先验 closed collection；register receipt 必须与请求的 provider ID / issuer / protocol / canonical
domain 相同；delete 必须得到 `removed:true`。load/register/delete worker 都固定到
`AdminIdentityProvidersPage` stable owner；成功后 refetch，失败分别留在 dialog 或 page 上。

Admin Home 与 secondary nav 只在 route 真实闭合后才加入 Identity Providers。Components、Credentials、
Computers 等未闭合 destination 仍未加入，没有一次画出假 AdminSidebar。

## 4. Release 浏览器 + 真实 PostgreSQL

使用最新 release/offline/locked Trunk bundle。一次性 PostgreSQL 17.11 只监听
`127.0.0.1:55462`，host auth 为 SCRAM-SHA-256；数据库名
`openbot_ui_approval_fixture_batch62` 命中既有 loopback+prefix guard。fixture 复用：

- production `SessionTokenHash` / `PostgresSessionAuthResolver`；
- production `DynamicSsoService` / v2 `SsoConfigVault`；
- production baseline/native schema、users/roles/session/audit；
- `https://fixture.openbot.test` 只作 SAML SP callback identity，不是网络 endpoint。

fixture 的新增 proof 只存在于 `required-features=testkit` bin：一个只给 provider/config/audit 计数，
另一个只给 IdP list/register/remove HTTP 计数；均不进入 production Router。

浏览器经 testkit-only host-only HttpOnly/Lax/no-store session bootstrap 后实得：

1. 初始列表为空，console error 0；
2. 空提交出现 5 个 invalid Field 与总 alert，HTTP mutation 0；
3. 使用仓内既有 SAML 测试证书登记 `acme-saml`，输入 domain
   `Second.Example, acme.example`；页面权威行是：

```text
SAML · acme.example,second.example · https://idp.example/entity
```

4. DB proof：

```text
providers=1
samlConfigs=1
v2Envelopes=1
plaintextMetadata=0
registeredAudits=1
removedAudits=0
```

5. hard reload 保持同一 canonical 行；中文与英文标题/说明/操作切换；
6. OIDC branch 显示 Client ID + `type=password` Client secret，metadata 字段 0、selected=true；
   临时填写 secret 后 Cancel，再打开 OIDC，secret value 精确为空；
7. Remove 后页面回权威空态、alert 0、console error 0；DB proof 变为：

```text
providers=0
samlConfigs=0
v2Envelopes=0
plaintextMetadata=0
registeredAudits=1
removedAudits=1
```

8. 1280 浏览器实得 horizontal overflow 0、main1、h1 1、nav2、duplicate ID0、nested
   interactive0、runtime error0。

浏览器控制层的 documented viewport override 连续设置 600×900 后 `innerWidth` 仍是 1280；新 tab
同样如此，备用 Chrome 又未连接。因此 **600px 浏览器证据未跑**，只保留已过 `css-check` 的 media rule，
不把设置调用或静态 CSS 冒充窄屏实测。formal golden 仍 todo。

本批也未用 live OIDC provider 跑 discovery 正向。OIDC browser body/exclusive fields/secret clear 有 unit+
release browser 证据，production OIDC discovery/store 属既有 W-7b/API done；本批没有把其历史证据改写成
“本轮 live OIDC 浏览器通过”。

## 5. 非管理员 fail-closed

为独立计算 child 请求，重启 fixture 后 HTTP probe 初值为 `0/0/0`；管理员加载一次页面后为：

```text
list=1 register=0 remove=0
```

随后在临时 DB 中原子删除 `fixture-actor` 的 admin role，并同步 user/session generation：

```text
roles=user
user_generation=2
session_generation=2
```

hard reload 后页面只显示本地化 NotFound；Identity Providers section/add button/dialog 均为 0，runtime
error 0。HTTP probe 仍精确为 `list/register/remove=1/0/0`，证明不是“先发业务请求再把页面藏掉”。

fixture 停止时 approval waiter denied，符合降权后的 fail-closed；Server、PG、socket、password、log、data
目录与浏览器 tabs 全清。

## 6. 机械证据

| 面 | 本轮结果 |
| --- | --- |
| contracts | `90/0/0` |
| UI | `147/0/0` |
| Server | lib `213/0/0`；fixture `5/0/0` |
| infra SSO config | filtered `4/0/0`（303 filtered） |
| Server dynamic SSO integration target | 编译通过；按定义 `0 passed / 1 ignored`，没有冒充 include-ignored 实跑 |
| Clippy | contracts/infra/Server/UI，all-targets/all-features，`-D warnings` 通过 |
| WASM | `cargo check -p openbot-ui --target wasm32-unknown-unknown --locked` 通过 |
| GUI build | pins verify + release/offline/locked Trunk build通过；零 npm |
| i18n/design/CSS | `670` leaf keys；`95` Rust files / `74` icons；`318` source class literals |
| bundle | wasm gzip `1,585,631/3,670,016`；CSS `105,118/131,072`；fonts `740,216/819,200`；scripts=`1/0` |
| parity | routes=`14/18/32`；UI=`87/65/152`；总=`701/993/1694`；overlay=`1594/98/2/0`；0违反 |
| strict recount | fixed upstream `891df72f…`，`159/0/0`，skip 0 |
| Grok/shim | tree `86f5a85f…`；inventory 2,110；shim `405/600`；单package/零npm锁守卫通过 |

初次 `cargo xtask tools fetch` 在受限网络内 DNS 失败，停止后经授权访问官方发布源成功，最终
`tools verify` 四项 exact。第一次 Trunk 启动受宿主 `NO_COLOR=1` 与裁剪 PATH 影响分别在编译前拒绝；
修正为 Trunk 接受的布尔值并把钉版工具/Rust cargo 放入显式 PATH 后，最终两次 release build均成功。
一次误写的 `parity-check --strict` 被命令自身拒绝；正确门是 `parity-check` 加
`OPENBOT_UPSTREAM_DIR=… recount --require-upstream`，最终分别 0违反与159/0/0。失败尝试均未记为通过。

## 7. 台账与边界

- `T-ROUTE-0019`：todo → done，target 改为真实 `AdminIdentityProvidersPage`；
- overlay 新增 `T-ROUTE-0019 revalidate`；
- routes `13/19 → 14/18`；总 parity `700/994 → 701/993`；
- API list/delete 已在 W-7b done，本批不重复新增产品 API 或 T-ID；
- `T-UI-0028` 完整 AdminSidebar与 `T-UI-0138` formal page golden保持todo。

## 8. 明确未做

- 未运行 live OIDC discovery 浏览器正向；
- 未取得 600px 浏览器证据，原因与控制层返回值已如实记录；
- 未生成 formal golden；
- 未新增 update-provider GUI，因为固定上游页面没有该旅程；
- 未实现 Credentials/Computers/Components admin route；
- 未运行全 workspace test，只按变更面跑上述 targeted tests；
- 未运行 `cargo xtask ci`，未派发 GitHub Actions（R63 manual-only）；
- P1 Windows/runsc runtime仍红，未进入P2；
- `grok-bot/` 零改动，没有 Grok 产品能力或文本进入本批。

为响应磁盘清理要求，本批开始前曾删除约19.3 GiB `target/`；证据阶段按 pins 重建必要工具/目标。
交付前再次删除可重建的 `target/`、`target-xtask/` 与 Trunk `dist/`，源码与提交不受影响。
