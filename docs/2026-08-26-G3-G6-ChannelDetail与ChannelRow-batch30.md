# Batch 30：G3/G6 Channel Detail、共享 Thread 与 ChannelRow

> 日期：2026-08-26。分支 `codex/2026-08-26-G6-app-sidebar-channel-shell`；
> base = Batch29 正式 head `be56a9c0a7bce695f5cf7fc1c668445e7c9fd7b9`；
> implementation = `33273d135ee16e70519f727a1d863c229567a0e7`。
>
> 本批只运行本地定向测试；**没有**运行 `cargo xtask ci`，没有派发 GitHub Actions，
> 没有处理或暂存 `docs/assets/`，没有处理 `grok-bot`。

## 1. 本批完成项

- [x] T-API-0034：`GET /api/channels/{channel_id}`；
- [x] T-UI-0038：`features::app_sidebar::channel::ChannelRow`；
- [x] T-TEST-0385–0390、0396–0397、0411、0416–0420，共14条；
- [x] typed `GetVisibleChannel` command/reply、`ChannelDetail/Response`、
  `ChannelReadScope` 与唯一 ApplicationService dispatch；
- [x] production list/detail 从 native `threads` 投影当前 deployment/tenant 的 shared channel
  thread，不再把 `thread_id` 恒写 `None`，也不读 legacy Intelligence mapping；
- [x] channel-anchor 的 status/begin/history/realtime 按**当前 channel membership**授权；
  direct-bot 仍按 thread membership。新channel成员首次begin时补 downstream run membership，
  撤销channel membership后旧thread membership不能扩大detail/status/history/realtime；
- [x] data-backed `/channel/:channel_id` destination shell、同源percent-encoded link、nested route
  直接硬刷新；完整chat journey继续独立todo；
- [x] production roster content：50条keyset页、load-more、仅name/last-message搜索、两类empty、
  localized relative time、current row、WebSocket reconnect-refetch、current user/session/sign-out；
- [x] strict-CSP bootstrap nested-route修复：JS保持module-relative，WASM改根绝对同源路径。

## 2. 权限与真源

固定0012的`channels/channel_memberships`没有deployment/tenant列，故channel可见性只诚实地按
materialized actor membership表达；不得伪造物理scope。G3 `threads`有deployment/tenant，
因此detail/list只投影同时满足scope、`anchor_kind='channel'`、`anchor_id=channel.id`、非deleted
的最新thread。

共享channel不能沿用direct-bot的私有thread判据。Batch30把四条读/续跑主链改为：

- direct-bot → current `thread_memberships`；
- channel → current `channel_memberships(channel_id=t.anchor_id, user_id=actor)`。

真库用例构造同channel的foreign deployment、foreign tenant、非deleted/deleted、不同创建者与
不同thread membership；当前channel member能读shared history和replay event 0，撤权后下一次
catch-up发`not_visible`并断流，history为空、status=false、detail=None。完整tool/approval/screen
control journey仍未在本批宣称完成；G3整关不勾。

## 3. UI 边界

roster query始终是真源；`/api/channels/events`只触发全量first-page refetch。连接尝试、合法event、
malformed/error/close均不能让陈旧缓存静默维持；重试500ms指数退避、30s封顶。页间若因activity
重排产生重复ID，不渲染重复行，而是回first page重取。

ChannelRow只显示name、last message、localized relative time；搜索也只匹配这两个可见字段，
不暗搜history。Avatar在row里纯装饰，避免链接可访问名称重复。single-user的`revocable=false`
不显示退出；multi-user只在Server 204后跳`/sign`，最终fixture session随即401。

`/channel/:id`已是真实detail destination，不是404/占位路由；但本批明确不画假composer、stop、
steer或screen control，只显示生产能力尚未完成。因此 T-ROUTE-0009 继续todo。AppSidebar总项仍缺
new-channel/skills/agents/settings/admin destination，T-UI-0037继续todo；只关闭独立ChannelRow。

## 4. 本机证据

| 面 | 实得 |
| --- | --- |
| contracts/application | **73 + 125 / 0 / 0** |
| Server channel handler/WS | **6 / 0 / 0** |
| Axum↔in-process transport parity | **8 / 0 / 0**，新增detail逐字段/port scope对拍 |
| UI | **63 / 0 / 0** |
| xtask | **78 / 0 / 0**，含nested bootstrap单测 |
| PostgreSQL 17.11 host SCRAM | channel_repo **7 / 0 / 0**；channel_detail/shared thread **1 / 0 / 0** |
| Clippy | contracts/application/infra/agent/server/UI/testkit all-targets/all-features `-D warnings`绿 |
| WASM | contracts/UI wasm32 all-targets/all-features绿 |
| UI gates | i18n **381**；design **62 Rust/74 icons**；CSS **178**；dependency guard绿 |
| bundle | WASM gzip **544139**；CSS **65607**；fonts **740216**；external/inline scripts **1/0** |
| ledger | parity-check 0 violation/0 warning；strict recount **157/157/0** |

浏览器在真实Axum static + production Leptos bundle上实得：

1. roster首屏50行，load-more后52行且按钮消失；
2. name/last-message搜索各精确1行，hidden-history查询为0并显示有区别的no-match状态；
3. socket `data-roster-generation` **1→4**，证明event/close/reconnect触发权威refetch；
4. `/channel/channel-00`唯一current row、唯一h1、无假composer；直接硬刷新仍成功且console 0；
5. 1440×900 sidebar **240→48→240**；900×700自动48且trigger隐藏；600×700共享Sheet
   240px、open/closed可见性正确、3个inert marker、Escape返焦、body scroll恢复；三档横向overflow=0；
6. 204后到`/sign`：h1=1、main=1、nav/sign-out=0，随后session status=401；
7. 最终 error/warn=0、重复可访问Avatar名称=0。

浏览器数据来自明确的testkit fixture，只证明UI行为，不冒充生产权限/数据库；后者由上述PG证据承担。
测试结束后fixture停止、临时tab关闭。所有`/private/tmp/openbot-pg17-channel30*`实例均停止并精确
删除，没有触碰用户数据库。因磁盘只余7.8GiB，本批运行`cargo clean`只删除可重建Cargo缓存
24.8GiB；随后从钉版官方URL恢复并校验Tailwind4.3.3/Trunk0.21.14/Binaryen132/
wasm-bindgen0.2.127。Cargo.lock新package=0，只增加UI对既有`futures-util`的target direct edge；
`gloo-net 0.6.0` WebSocket feature/no-build.rs由dependency guard锁定。

## 5. 台账变化与未完成项

| 口径 | Batch29 | Batch30 | 变化 |
| --- | ---: | ---: | ---: |
| API | 42 / 120 / 162 | **43 / 119 / 162** | +1 done |
| tests | 276 / 771 / 1047 | **290 / 757 / 1047** | +14 done |
| UI | 81 / 71 / 152 | **82 / 70 / 152** | +1 done |
| 全 parity | 530 / 1143 / 1673 | **546 / 1127 / 1673** | +16 done |
| fixtures | 15 / 22 / 37 | **15 / 22 / 37** | 0 |

继续todo：

- [ ] T-UI-0037 AppSidebar剩余真实nav destinations；
- [ ] T-ROUTE-0009 channel transcript/composer/@/queue/stop/steer/failure/screen完整journey；
- [ ] T-API-0031 create、T-API-0033旧activity compatibility、channel-new route；
- [ ] channel tool/approval/screen/control的完整current-membership撤权矩阵；
- [ ] G3 Memory GUI、legacy production drills，以及其余G4–G8。
