# Batch 39 WIP：Connected Accounts

> 日期：2026-08-27。分支`codex/2026-08-27-G4-G6-connected-accounts`；base为Batch38证据head
> `f083edc`。固定上游`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 本机定向测试；不运行`cargo xtask ci`，不派发Actions，不触碰`docs/assets/`。

> 已完成：implementation `52b2c4f59906da58ae5d2d7db62adfcce90f9af5`；正式边界与证据见
> `docs/2026-08-27-G4-G6-ConnectedAccounts-batch39.md`与R102。本文件保留为恢复点历史。

## 第一真源与缺口

- 上游index只列`auth=user-oauth`且管理员已add的catalogue，join本人connection后显示Connected/
  Not connected；detail负责Connect full-page vendor consent、connected menu、scope与connectedAt；
- Batch12/13已有actor OAuth、list、local-first disconnect、Google Drive GA REST与真PG协议证据；
- 当前`McpConnections`只有已连接行+redirect URI，没有“已启用reviewed connector”集合。只靠连接行
  无法显示Not connected，不能冒充上游index；
- 新增closed `availableServerIds`：只包含编译期reviewed catalogue且DB server identity逐字段匹配的条目；
  当前集合最多`google-drive`。未知/自定义server不借此进入个人页；
- title/summary等用户文案留UI i18n，只让stable server id穿contract；Google Drive官方品牌SVG仍是
  独立brand todo，本批用中性Plug/RowMark，不伪造品牌；
- Connection GET/connect/delete均`no-store`；connect/delete trusted Origin保持先于业务；
- authorization URL只接受bounded root-relative同源或HTTPS无userinfo/fragment；UI只在Server receipt后
  full-page navigation。fixture相对URL只模拟callback，真实OAuth/PKCE/issuer/token/revoke由既有PG测试证明；
- disconnect receipt区分vendor revoked/pending，UI不把pending写成vendor已撤销；本地connection立即消失。

## 实施范围

1. contract/application/PG：`availableServerIds` closed projection与identity负例；
2. HTTP no-store与Server/transport parity更新；
3. UI API：list/begin/disconnect严格response/URL/id验证；
4. `/settings/connected-accounts`与`/:server_id` index/detail；SettingsShell加入真实destination；
5. connected menu/disconnect、scope/time、callback failed/connected outcome、empty/load/error；
6. deterministic fixture模拟available→connect callback→connected→disconnect pending；
7. PG17 existing OAuth/Drive回归、Server/UI/WASM/Clippy/bundle/browser/parity/recount。

## 不冒充

- 不把fixture callback当真实vendor OAuth；真实协议只引用既有PG网络测试；
- 不实现admin custom connector/catalogue UI、Google brand asset、connected accounts formal golden；
- 不实现Desktop Local OAuth、restricted-scope外部verification/security assessment；
- 证据不足时不关闭T-ROUTE-0029/0030。
