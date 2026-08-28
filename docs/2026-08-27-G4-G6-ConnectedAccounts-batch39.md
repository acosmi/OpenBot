# Batch 39：Connected Accounts

> 日期：2026-08-27。分支：`codex/2026-08-27-G4-G6-connected-accounts`。
> base：Batch38文档head `f083edc`；WIP恢复点：`b16e013`；
> implementation：`52b2c4f59906da58ae5d2d7db62adfcce90f9af5`。
> 固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批只关闭个人Connected Accounts的index/detail可观察journey。没有运行`cargo xtask ci`，没有派发
Actions，没有生成formal golden；既有未跟踪`docs/assets/`未修改、未暂存、未提交。

## 1. 第一真源缺口与裁决

固定上游index只列同时满足两项的条目：编译期catalogue声明`auth=user-oauth`，且管理员已把该server
加入deployment；然后join本人connection，显示Connected/Not connected。detail发起full-page vendor
consent，并显示vendor实际记录的scope与connectedAt。上游disconnect明确写着尚未实现，本项目按v3
§2.4/§9.4保留既有local-first安全修复。

Batch12/13已有真实actor OAuth、Google Drive GA REST、list与disconnect，但原`McpConnections`只有
已连接行。只靠它无法显示“管理员已启用、本人未连接”；直接列所有`mcp_servers`又会把未知/自定义
MCP冒充reviewed个人连接器。因此新增closed `availableServerIds`：

- wire只传stable server id，不传title、vendor URL、credential id或secret；显示文案由UI i18n所有；
- 当前只认编译期reviewed `google-drive`；DB行必须在url/vendor/provenance/transport四字段精确匹配；
- 同id行被篡改不是“暂时隐藏”，而是`Corrupt(reviewed_server_identity)`失败关闭；
- 未知/custom connection可以继续由既有后端管理，但个人页面构造性不join、不显示、不提供action；
- Google官方品牌资产仍是独立todo，本批只用中性`Plug`+`RowMark`，不伪造品牌。

## 2. 后端与传输边界

- `McpConnections`新增`availableServerIds`，`deny_unknown_fields`保持；契约单测证明顶层恰三个key，
  不接受credential-like额外字段；
- `PostgresMcpConnections::list_connections`先按AuthContext actor读取本人active token，再读取唯一reviewed
  row并逐字段核对；未add时available为空；
- `GET /api/plugins/connections`、`POST /api/plugins/servers/{id}/connect`、
  `DELETE /api/plugins/connections/{id}`成功响应统一`Cache-Control: no-store`；写请求仍由
  `OriginAuthenticated`在业务前守卫；
- UI API镜像后端64字节server-id域，拒绝重复available/connection、空或控制字符scope、越界集合与坏
  callback URI；
- authorization receipt只允许bounded同源根路径，或`https`、有host、无userinfo/password/fragment的
  absolute URL；反斜线、控制字符、scheme-relative与HTTP vendor URL全部拒绝；
- renderer只在Server 200 typed receipt后调用`Location.assign`，不根据query或catalogue字段自行拼vendor URL。

## 3. GUI journey

- 新增`/settings/connected-accounts`与`/settings/connected-accounts/:server_id`，均复用唯一
  `SettingsShell`；secondary nav只在真实route存在后加入Connected accounts，Gallery仍不画断链；
- index有loading/error/retry、无reviewed connector空态、Connected/Not connected状态，以及统一callback
  success/failure提示；`connected=failed`不读取也不渲染vendor错误参数；
- detail只认reviewed id；未知id显示本地化Not found且零连接/断开控件；
- 未连接态只在deployment具有公开callback时启用Connect；已连接态逐字显示vendor scope与RFC3339时间；
- connected action使用既有APG Menu：ArrowDown进首项，Escape返trigger；断开期间trigger/item不可再激活；
- disconnect receipt为`Pending`时只写“本地已断开、供应商撤销待协调”，不冒充vendor已撤销；随后GET
  权威refetch，scope消失；
- connect/disconnect worker都绑定detail稳定Owner，避免语言重渲染或connected→disconnected子树卸载取消
  收尾；断开后先清pending，再下一tick聚焦新Connect按钮。

确定性fixture公开Google Drive为available，起始无connection；connect延迟350ms后返回同源callback路由并
写入合成connection，disconnect延迟350ms后返回`Pending`。它只用于UI旅程，不是Google OAuth、PKCE、
token exchange或revocation网络证据；这些继续由真实PG协议测试承担。

## 4. 本机机械证据

| 面 | 结果 |
| --- | --- |
| contracts / application / Agent / Server / UI | **78 / 138 / 28 / 203 / 114**，均0失败 |
| infra host | **306 / 0 / 0** |
| Server plugins HTTP | **4 / 0 / 0**；list/connect/disconnect均实得200+no-store |
| Axum/in-process transport | **8 / 0 / 0** |
| PostgreSQL 17.11 SCRAM | Google Drive全边界 **1 / 0 / 0** |
| Clippy / WASM | affected crates all-targets/all-features `-D warnings`；contracts/UI WASM通过 |
| i18n / design / CSS | **473** leaf；**75 Rust / 74 icons**；**233** source class literals |
| release bundle | WASM gzip **1,053,824 B**；CSS **78,190 B**；fonts **740,216 B**；external/inline **1/0** |
| parity | routes **6/26/32**；总计 **646/1032/1678**；fixtures **16/22/38** |
| strict fixed-upstream recount | **157 / 157 / 0** |
| parity violations / warnings | **0 / 0** |

PG实例由`initdb`以host `scram-sha-256`创建；实测`17.11 (Homebrew)`、
`password_encryption=scram-sha-256`、当前role hash形状为SCRAM。真用例证明：管理员add前available为空；
add后唯一`google-drive`；原add/register/code/refresh/grant/Agent首次401 retry/正文不落库/local-first
disconnect/pending reconcile均保持；把DB URL改为`https://attacker.invalid/drive`后list精确返回
`Corrupt { field: "reviewed_server_identity" }`。临时PG已停止删除。

`Cargo.lock`没有新增package，只给已锁定`url 2.5.8`增加UI direct edge，以真正URL parser替代安全边界上的
字符串前缀判断。最终Trunk命令为`--release --offline --locked`；用户清理缓存后缺少的locked crate先按
lock补齐，最终构建全程离线成功。

## 5. release浏览器

- index实得Google Drive Not connected；点击detail→Connect后full-page进入同源callback结果，index显示
  Connected；hard reload保持；
- detail实得精确`https://www.googleapis.com/auth/drive.readonly`与RFC3339 connectedAt；
- Menu ArrowDown后active element是唯一disconnect menuitem，Escape返trigger；断开请求中trigger disabled；
- receipt后scope区域消失，Pending文案准确，active element为`connected-account-connect`；
- `?connected=failed&error_description=SECRET-VENDOR-CANARY`只出现统一错误，页面canary=0；
- unknown custom detail为Not found且connect/scope均不存在；中英切换后route/current与内容同步；
- 最终release在浏览器固定1280×720表面实得secondary 200px、X overflow0、main/nav/h1=
  `1/2/1`、current1、duplicate IDs/visible alerts/console均0。

本轮浏览器表面不提供viewport resize能力，因此不把Batch38既有四视口SettingsShell证据冒充为新页面的
formal responsive golden；源CSS仍沿用同一个`<768px` shell断点，T-UI-0148/0149保持todo。

## 6. 台账与未完成边界

- 关闭`T-ROUTE-0029/0030`；routes从`4/28`变为`6/26`，总parity从`644/1034`变为
  `646/1032`；
- `T-UI-0148/0149` formal page golden、`T-UI-0066`完整SettingsSidebar、Google brand仍todo；
- 通用MCP admin add/refresh/grant/effect UI、private egress、Desktop Local OAuth仍未完成；
- 未使用真实Google credential；restricted-scope verification/security assessment仍是外部发布前置；
- Approval真实PG浏览器、browser/file/shell、完整AG-UI、Composer/Screen、Tauri binary/window lifecycle、
  经许可legacy production drills与其余G4–G8保持未完成，不能据本批勾整关。
