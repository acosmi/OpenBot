# Batch 31 已完成恢复点：Agent Roster + `/agents`

> 日期：2026-08-26。分支 `codex/2026-08-26-G4-agent-roster`；
> base = Batch30正式head `4a805a6ee8f43a8a44d1dc1af155750cb8fe06e1`；
> implementation = `55bc2f18d1f6108272864dee8fba6d22c637305a`。
> 本文所在docs提交是Batch31正式head。只跑了本地定向测试；未运行`cargo xtask ci`，
> 未派发Actions，未处理`grok-bot`，未修改/暂存/提交`docs/assets/`。

## 已完成并打勾

- [x] T-API-0019/0020：`GET /api/agents`与`GET /api/agents/{agent_id}`；
- [x] T-TEST-0298–0303：public/private、owner/admin、system/deleted、canRun alias纯领域判据；
- [x] T-TEST-0305、0328–0329、0332：private owner/admin list+get、closed DTO、permission/mine
  分离与missing 404；
- [x] T-UI-0029/0030：abstract-avatar→统一Avatar与AgentCard；
- [x] typed `AgentDirectory`经唯一ApplicationService，scope只从AuthContext取得tenant/actor/admin；
- [x] PostgreSQL package tenant、visibility/owner/admin、soft-delete、per-user hidden前置收窄，
  结果再过domain终判；endpoint/auth/callback只回允许的地址/布尔事实，不回配置、owner或凭据；
- [x] list/detail成功响应`Cache-Control: no-store`；unknown query/malformed=400，
  missing/invisible/deleted/cross-tenant=404，dependency=503；
- [x] Leptos `/agents`真实mine/explore roster、144×180 AgentCard、URL-owned只读profile、
  hard reload/404/close返焦与AppSidebar Agents destination；无API动作一律不画；
- [x] 浏览器实测推翻旧字体相对路径：Trunk最终CSS改为根同源`/fonts/*`，Inter确实加载。

## 最终机器证据

- contracts/domain/application/Server/UI Agent：`2+6+2+4+3 / 0 / 0`；
- PostgreSQL 17.11 host SCRAM：`agent_directory 1/0/0`，临时实例已停止并删除；
- 7 crate all-targets/all-features Clippy `-D warnings`、UI WASM、fmt/diff：绿；
- i18n391、design65 Rust/74 icons、CSS193；bundle
  `wasm gzip=600783 / css=68843 / fonts=740216 / external-inline=1/0`；
- Chromium：4卡=mine2/explore2，list/detail 200+no-store，1440/1024/900/600 overflow0，
  hard reload/404/return-focus/Inter/AX去重通过，external/duplicate/fake-action/forbidden-wire=0；
- parity `560/1113/1673`，API `45/117/162`，tests `300/747/1047`，UI `84/68/152`，
  fixtures `15/22/37`；parity violation/warning `0/0`；strict recount `157/157/0`。

## 仍明确 todo

- [ ] T-TEST-0306以及Agent create/edit/duplicate/hide/unhide/delete对应store/API/audit/concurrency；
- [ ] T-UI-0032完整AgentProfile、T-ROUTE-0007、T-UI-0126及正式golden；
- [ ] `/channel/new`、首页routeMessage/fallback/create transaction与完整channel journey；
- [ ] AppSidebar总项仍等待new-channel/skills/settings/admin；
- [ ] customer auth、三家recorded trace、完整Agent lifecycle、browser/file/shell等G4余面；
- [ ] brand favicon仍todo；Chromium preload SRI warning不冒充应用错误，也不写成warning0。

## 下一批恢复

从本文所在正式head继续，优先以已完成的真实Agent roster为recipient真源，实施
`POST /api/channels` + `/channel/new` + create-time routing/fallback原子闭环；不得先画fake composer。
