# Batch 29：G3 Channel Activity 与只读 WebSocket

> 日期：2026-08-26。分支 `codex/2026-08-26-G3-channel-activity-app-sidebar`；
> base = Batch 28 正式 head `b8df5d661956aff1031bef8409944db8b83bda9a`；
> implementation = `572bb3c81e76dd18867b7925f3e947b45dcf7e38`。
>
> 本批只运行本地定向测试；**没有**运行 `cargo xtask ci`，没有派发 GitHub Actions，
> 没有处理或暂存 `docs/assets/`，没有处理 `grok-bot`。

## 1. 完成项

- [x] T-API-0030：`GET /api/channels/events` production WebSocket；
- [x] T-TEST-0391–0395：roster activity、membership、stale、200 code-point、未挂接 Bot；
- [x] T-TEST-0398–0402：member-only、多连接、detach、单连接失败隔离、跨 PG 连接投递；
- [x] typed `SubscriptionRequest::ChannelActivity` → `ApplicationService::subscribe` →
  `ThreadDirectory::subscribe_channel_activity`；
- [x] channel-anchored user begin 与 assistant terminal 在各自既有 PostgreSQL transaction 内
  更新 `channels.last_message*` 并发送 bounded NOTIFY；
- [x] Desktop broker 把 channel activity/error 归为 Critical：队列压力时断开并要求
  reconnect+roster refetch，不静默丢帧。

## 2. 构造性边界

`channels` 行是唯一 roster 真源；`openbot_channel_activity` 只负责低延迟唤醒。写事务回滚时
PostgreSQL 不投递通知，stale 时间戳不更新也不通知。预览先移除 C0/C1、折叠空白，再限制为
200 Unicode code points（超限为 199+省略号）；NOTIFY 另有 7000-byte 上限，超限只跳通知，
绝不回滚真源行。

通知与 WebSocket frame 都不携 `memberIds`。每条通知解析为 closed
`ChannelActivityEvent` 后，以 upgrade 时已经验证的 actor 回查**当前**
`channel_memberships`；撤权后的下一帧即被过滤。deployment/tenant 没有被伪装成 channel
列：固定 0012 `channels/channel_memberships` 物理表没有这两列，当前 production 可见性与既有
`ChannelRepo` 一样只以 actor membership 为权威。

WebSocket 固定子协议 `openbot.channel-activity.v1`，要求 session 与 trusted Origin；客户端
Text/Binary 一律 1008，输入 frame/message cap 为 1KiB。LISTEN/依赖失败先发稳定
`{"error":{"code":…}}`，再 1011 关闭；该流没有 durable cursor，客户端每次重连必须先
重新 `GET /api/channels`。错误不能靠继续断流伪装成功。

## 3. 本机证据

| 面 | 实得 |
| --- | --- |
| contracts + application | **71 + 123 passed / 0 failed / 0 ignored** |
| Server Axum/TCP WebSocket | **4 / 0 / 0**：401 且 port call=0、Origin/protocol、typed frame、Text→1008、dependency frame→1011 |
| infra pure preview | **1 / 0 / 0** |
| Desktop delivery budget | **7 / 0 / 0** |
| PostgreSQL 17.11 host SCRAM | **1 / 0 / 0** |
| Clippy | contracts/application/infra/server/desktop all-targets/all-features `-D warnings` 绿 |
| WASM | `openbot-contracts` wasm32 all-targets 绿 |
| 格式/台账 | fmt、diff check、parity-check、strict recount **157/157/0** 全绿 |

真库用例在临时 PostgreSQL 17.11、仅 `127.0.0.1`、host SCRAM 下实跑，并证明：

1. 两个 member 与一个 outsider 同时订阅，只有 member 收到；
2. 同一 actor 两条连接均收到，drop 后 `pg_stat_activity` 的 LISTEN 数按 **4→3→2** 收敛；
3. 撤销 actor membership 后，assistant terminal 只到仍有权限的连接；
4. assistant event 与 `ChannelRepo` roster 的 message/time/Bot 逐字段相等；
5. 非 member 与未挂入 channel 的 Bot 在 production begin 路径返回 NotVisible；
6. 2100 年的既有 activity 不被当前 DB clock 覆盖，且零 channel notification；
7. 长文本/ESC/newline 经 production transaction 后恰 200 code points 且零控制字符。

所有本批临时 PostgreSQL 实例均已停止，所建 `/private/tmp/openbot-pg17-channel.*` 精确目录均已
删除；没有触碰用户数据库。

## 4. 台账结果

| 口径 | Batch 28 | Batch 29 | 变化 |
| --- | ---: | ---: | ---: |
| API | 41 / 121 / 162 | **42 / 120 / 162** | +1 done |
| tests | 266 / 781 / 1047 | **276 / 771 / 1047** | +10 done |
| UI | 81 / 71 / 152 | **81 / 71 / 152** | 0 |
| 全 parity | 519 / 1154 / 1673 | **530 / 1143 / 1673** | +11 done |
| fixtures | 15 / 22 / 37 | **15 / 22 / 37** | 0 |

`cargo xtask parity-check --json` 实得 violations/warnings = **0/0**；严格 recount 为
**157 passed / 0 mismatch / 0 skipped**。

## 5. 明确未完成

- [ ] T-TEST-0396/0397：channel 创建时间与 activity 交错的两个精确 roster ordering journey；
- [ ] T-UI-0037/0038：AppSidebar 与 channel row；
- [ ] 真实 channel destination route、channel create/detail API 与完整 ChatTranscript/Composer journey；
- [ ] 31 route journey、其余业务组件/golden/Tauri release；
- [ ] G3 的 Memory GUI 与实际 legacy exporter/production migration drills。

WIP 原计划把 AppSidebar 与 realtime 同批关闭，但第一性原理复核后没有这样做：当前没有可到达的
真实 channel destination route，提前渲染 channel link 会生成断链导航。实时生产依赖已经完成；
UI 两条继续保持 todo，须与真实 route/journey 同批验收。G3/G6 整关都不勾。
