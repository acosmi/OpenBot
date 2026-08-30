# OpenBot G6 Admin Components + Playground Batch67

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-G6-admin-components`

基线：Batch66 PR #49 已以 merge commit
`f948a4ec90c03d90f8343373f1cc0a737ed975a2` 合入 `main`。

implementation：`ecfaf09746bbaf131411b9a8d66fb2d1ecb6c1bc`

## 1. 结论

本批关闭：

- `T-API-0043–0048`：compiled component function/Agent/publication/draft 六条治理写面；
- `T-ROUTE-0015/0016`：Admin Components index/detail；
- `T-ROUTE-0021`：Admin Playground production route；
- `T-TEST-0437–0439/0443–0448/0454–0455`：固定上游 Components/Playground 十一条行为判据。

formal Components/Playground page golden、Desktop 独立 sandbox renderer、sandbox args/作者 JS/无网络/
callback/MessageChannel 正向执行与 G6 整关继续 todo。P1 的 Windows/runsc runtime 仍红，本批没有进入 P2。

## 2. 第一真源、固定上游与 R141

本批只读取固定上游 `891df72f1827454d8b353d108fe5dd2313b7e30d` 的窄面：

- `app/src/routes/_authed/admin/components/index.tsx` 与 `$name.tsx`；
- `app/src/routes/_authed/admin/playground.tsx`；
- `app/src/lib/components/{queries,mutations}.ts`；
- `server/src/components/{routes,store}.ts`。

固定上游把 compiled 与 sandboxed row 共用 generic draft/publication mutation；v4 已先裁决 sandbox source、
description、revision 必须原子发布，且 sandbox 没有 data function。直接照搬会拆开同一治理事实并新增能力。
R141 因此固定：

- compiled row 可在 Components detail 管 Agent withholding、data function、draft 与 publication；
- sandboxed row 在 Components detail 只可管 Agent withholding；
- sandboxed draft/publication 继续只归 Playground 的原子写面；
- sandboxed data function 始终拒绝，不因 UI 隐藏而只靠前端守约。

没有复制或翻译 `grok-bot/` 文本，也没有新增 Grok 产品能力。

## 3. Typed 治理写面

contracts 新增 closed `ComponentGovernanceMutation` 与权威 `ComponentGovernanceReceipt`：

- `SetAgentGrant`；
- `SetFunctionGrant`；
- `SetPublication`；
- `SaveDraft`。

Application 在 port 前重复验证 admin role、component/Agent/function 名称、description 的 NUL 与 64 KiB
边界；成功后逐字段核对 adapter receipt，不能用请求体猜终态。Axum 六条 exact endpoint 都在读取 JSON 前经过
fresh authenticated Origin，transport 再经同一 `ApplicationService`；成功返回 `no-store` 权威 row。Tauri custom
protocol 复用同一 command/reply，body 固定 68 KiB，stale auth 仍为 401。

## 4. PostgreSQL 原子性

`PostgresComponentCatalogue` 用 SERIALIZABLE transaction 与 component row lock：

- Agent withholding 每次重验 Agent 可见性，grant/delete 互斥；
- function grant 每次重验当前 build manifest，grant/revoke 互斥；
- publish 把 draft description 提升为 published description；
- unpublish 不清独立 Agent/function grant；
- save draft 不泄漏到 publication；
- sandboxed draft/publication/function mutation 返回 Conflict；
- allowlisted hash-chain audit 与业务 row 同事务，audit commit unknown 不报假成功。

本批最终 PostgreSQL 17.11 临时实例只监听 `127.0.0.1:55467`，host auth 为 SCRAM-SHA-256；
`component_catalogue` 两条 ignored suite 亲跑为 `2/0/0`。它覆盖重复 grant 的 `created_at` 稳定、revoke→
regrant、publish/unpublish、sandbox fail-closed、强制 audit rollback 后 row 0、description canary 0、missing
component/Agent 均 NotVisible。测试后实例停止，临时 data/socket/log/password 已删除。

## 5. Admin Components 与 Playground

Admin secondary nav 新增真实 Components destination。index 从权威 catalogue 渲染 compiled、stale/future 与
sandboxed rows；preview 位于 `inert aria-hidden` sibling，不把互动 preview 嵌进链接。detail：

- 成功 mutation 只以 Server receipt 替换本地 row；
- switch 请求失败立即回滚；
- unknown name 与 catalogue load failure 分开呈现；
- sandboxed publication/edit/function control 构造性 disabled；
- sandboxed Agent withholding 与 Playground link 保持可用。

Playground 原先只用 component name 作为 keyed identity，同名 revision 发布后会保留 stale row。本批改为完整
closed `SandboxedComponentRecord` 的 SHA-256 identity。production CSP 原先 `default-src 'none'` 且没有
`frame-src`，同源 `/sandbox/runner` 也会被拒；最终只增加 `frame-src 'self'`，没有放宽 script/network。

## 6. Release 浏览器证据

最终 release/offline/locked bundle 的真实浏览器结果：

1. Components index 为 16 卡（13 build + 2 stale/future + 1 sandboxed），detail link 16、inert preview 16、
   nested interactive/focusable-outside-inert/duplicate ID/x-overflow/visible alert 均 0；
2. compiled Activity 保存 draft 后 published 仍旧、`hasChanges=true`；unpublish→publish 后 published 精确提升
   draft 且 `hasChanges=false`；
3. Research Partner withholding `4→3→4`，`botActivity` function grant/revoke 双向，API row 与 DOM 终态一致；
4. sandboxed `custom_delivery_eta` 的 publication/edit/function controls disabled，Agent withholding 双向可用，
   Playground link 可导航；unknown component 显示 not-found；
5. Playground runner HTTP 200、iframe `sandbox="allow-scripts"` exact、preview 显示样例；新建
   `custom_batch67_card` 先 draft-only 再 publish，页面与 hard reload 均显示 `published revision 1`；
6. 1024×640 Playground 与 600×900 detail 均 body width=viewport、overflow0、main1/nav2/h1 1、duplicate/
   nested/alert 0；英文切换后真实显示 Configuration。

页面 mount 后记录的 runtime error、unhandled rejection、console error/warn 均 0。Chromium preload SRI 的既有
warning 与首次 favicon 404 仍如实保留，不冒充本批解决；formal golden 未跑。

## 7. 机械证据

| 面 | 本轮结果 |
| --- | --- |
| Contracts/Application/Agent | `93/151/34` |
| Infra library | `307/0/0`；PG component catalogue另 `2/0/0` |
| Server/fixture | `214/0/0`、`8/0/0` |
| Desktop/UI | `81/0/0`、`166/0/0` |
| Testkit | lib `1/0/0`、xtask `93/0/0`、transport parity `8/0/0`，其余本批相关transport均绿 |
| Clippy | 8 个受影响 crate，all-targets/all-features，`-D warnings` |
| WASM/fmt/diff | UI wasm32、workspace fmt、`git diff --check` 通过；Cargo.lock零变化 |
| i18n/design/CSS | `731` leaf keys；`101` Rust files/`74` icons；`345` class literals |
| bundle | wasm gzip `1,741,699/3,670,016`；CSS `112,331/131,072`；fonts `740,216/819,200`；scripts=`1/0` |
| Engine/tools | Tailwind `4.3.3`、Trunk `0.21.14`、Binaryen `132`、wasm-bindgen `0.2.127`；Electron `43.3.0` zip/bundle/version/integrity全绿 |
| API/routes/tests | `73/97/170`、`23/9/32`、`395/652/1047` |
| parity | `739/955/1694`；0违反 |
| overlay | carry/revalidate/split/superseded=`1540/152/2/0` |
| strict recount | fixed upstream `891df72f…`，`159/0/0`，skip0 |
| Grok/shim | git tree `86f5a85f…`；inventory 2,110；shim `405/600`；单package/零npm lock |

Server 与 Infra 首次在权限沙箱内分别有 9/14 条 loopback bind 用例得到 `Operation not permitted`；相同已编译
二进制在本机环境复跑为 `214/0/0` 与 `307/0/0`，前次权限失败没有计成产品失败或通过。磁盘清理后首次
`tools verify`/`engine verify` 因 `target/` 下载件与 bundle 不存在而按设计判红；随后按 pin 重新 fetch，重建
Rust-only bundle，并以独立 verify 得到 hash/version/integrity 全绿。失败尝试均保留在本批记录中。

## 8. 台账与明确未做

- API `67/103→73/97`，routes `20/12→23/9`，tests `384/663→395/652`；
- 总 parity `719/975→739/955`；overlay `1560/132/2/0→1540/152/2/0`；
- components ledger 仍为 `13/11/24`：本批关闭的是既有 Admin route/API/test 判据，没有把 formal golden 或
  Desktop renderer 伪算成 component runtime done；
- 没有运行全 workspace test、`cargo xtask ci` 或 GitHub Actions（R63 manual-only）；
- 没有使用 live vendor credential，没有运行 Windows/runsc/P2/P3/P4 证据；
- `grok-bot/` 零改动，工作区非 Grok 恰一个五键 `package.json`，dependencies/scripts/lockfile 均为零。
