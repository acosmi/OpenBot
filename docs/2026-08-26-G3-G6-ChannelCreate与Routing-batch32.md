# Batch 32：Channel Create、Routing 与 `/channel/new`

> 日期：2026-08-26。分支：`codex/2026-08-26-G3-channel-create-routing`。
> 基线：Batch31正式head `00ad915da6a1844551c943d866f170f0d40e41f8`。
> 实施提交：`f9eb1594a5aad634be992c3f24f6dc1d21e2f806`。
> 本批只运行本地定向测试/真库/浏览器验收；**未运行 `cargo xtask ci`，未派发 Actions**，
> 未处理 `grok-bot`，未修改/暂存/提交 `docs/assets/`。

## 1. 已闭合的生产路径

- [x] 纯 Rust routing 固定上游 11 条：固定 prompt、0.6 confidence、fenced JSON、坏响应/
  roster 外 ID/低置信度/provider unavailable 全部回确定性 default；单候选零模型调用。
- [x] typed `POST /api/route`：Origin/auth 在 JSON 前；roster 只取当前 tenant/actor/admin 的
  `AgentDirectory`；显式 recipient 零模型，inference 才取 active MCP reach 并调用 package
  OpenAI Chat Completions；成功响应是五字段 closed DTO 与 `no-store`。
- [x] routing audit 只保存 chosen/candidate IDs、closed reason、fallback/via-mention；不保存原消息
  或模型理由。audit transaction 以 serializable snapshot 复读完整 visible/non-hidden roster，候选集
  变化/序列化冲突稳定返回 409，零第二条 audit。
- [x] typed `POST /api/channels`：只接收 non-empty `agentIds`；Application 以 ECMAScript trim
  canonicalize/sort/dedup，scope 只取 AuthContext。PostgreSQL 在同一连接/事务内按 canonical ID
  顺序 `FOR UPDATE` profile，复核 tenant + domain access，再写 channel、creator membership、
  channel_agents 与 deployment-owned native channel thread；不写 Intelligence mapping。
- [x] channel 名按 locked Agent 名的 canonical 顺序拼接并按 Unicode code point 截至120、末尾
  一个省略号；重复选择铸造独立 channel/thread identity。
- [x] 新增 `POST /api/threads/{thread_id}/runs` 只是既有 typed `BeginThreadRun` 的 native HTTP
  framing：path 铸 thread id，body 无 scope/thread 字段，201 表示新建、200 表示 exact replay，
  均 `no-store`；没有第二套 run business path。
- [x] production `main` 同时注入 `ChannelRepo` read/write 与 `PostgresChannelRouting`；routing
  provider 与 built-in Agent 共用 package model/credential/Vault/SafeDialer，但协议固定 Chat
  Completions。无 credential/provider failure 由 Application fallback，不让模型错误泄漏给用户。
- [x] Leptos `/channel/new` 静态路由先于 `/:channel_id`：无 recipient 可写草稿但发送禁用；
  `?agent=` 是 recipient 真源，默认 roster 可键盘选择，hidden 但仍有权的 Agent 可直接 URL/hard
  reload；刷新不创建空 channel。首发严格 create → native begin → 成功后 navigate；begin 失败保留
  同一 channel/run-id 重试，create 响应未知时禁止二次提交以免重复。
- [x] AppSidebar 接入真实 New channel destination；`RecipientField` 复用唯一 Combobox/listbox。

## 2. 构造性边界

- renderer 不能自报 channel id/name/thread id/actor/tenant/admin/active；unknown field 当场400。
- missing/deleted/cross-tenant/无权 Agent 统一404；profile lock 后才读 deleted/access，消除 create
  与 soft-delete 的 TOCTOU；canonical lock 顺序消除反序死锁。
- native thread ID 继续使用 deployment fingerprint UUIDv8；channel thread 初建时不造私有
  thread_membership，第一次 `BeginThreadRun` 按当前 channel membership 物化。
- routing model 只给建议；最终 ID 必须在权威 roster，audit 写失败或候选变更绝不返回未记录选择。
- audit candidate list 上限256且 non-empty；model reason 仅用于有界 UI response，不进 durable audit。
- Browser 验收时发现页面仍显示“Enter 发送”但本批没有完整 Composer keyboard handler，已删除该
  虚假提示；没有用文案冒充 Enter/IME/queue/stop/steer 能力。

## 3. 本机证据

| 面 | 结果 |
| --- | --- |
| contracts closed HTTP DTO | **1 / 0 / 0** |
| domain routing / channel name / audit payload | **12 + 2 + 12 / 0 / 0** |
| application channel create / routing | **2 + 6 / 0 / 0** |
| infra provider completion | **2 / 0 / 0** |
| Server channel create / routing / native begin | **3 + 6 + 2 / 0 / 0** |
| Server production protocol assertion | **1 / 0 / 0** |
| UI native | **68 / 0 / 0** |
| PostgreSQL **17.11 host SCRAM** | channel create/native begin **2 / 0 / 0**；routing/audit **1 / 0 / 0** |
| seven-crate all-targets/all-features Clippy `-D warnings` | 通过 |
| UI `wasm32-unknown-unknown` all-targets/all-features | 通过 |
| tools / i18n / design / CSS | tools exact；i18n **396** leaf；design **67 Rust / 74 icons**；CSS **200** class literals |
| release bundle | WASM gzip **669,241 B**；CSS **70,248 B**；fonts **740,216 B**；external/inline scripts **1 / 0** |
| parity | API **48/115/163**；tests **334/713/1047**；routes **1/31/32**；UI **85/67/152**；总计 **599/1075/1674**；fixtures **15/22/37** |
| strict upstream recount | **157 / 157 / 0** |
| parity violations / warnings | **0 / 0** |

真库 create 证据同时覆盖：`max_pool_size=1`、独立 identity、120 Unicode、六 surface 原子计数、
四类 denial 零残留、admin private positive control、并发 delete 持锁后 create 阻塞并在 commit 后
拒绝，以及**刚创建的 native thread**确实经生产 `PostgresThreadDirectory::begin_thread_run`写出
thread membership/message/run/channel activity。routing 真库覆盖 active grant→`google-drive` reach、
provider request cap/no tools、genesis hash-chain、消息/模型理由 canary 为0、候选隐藏后409且 audit count
仍为1。两次一次性PG实例均只监听127.0.0.1，测试后停止并精确删除。

真实 release WASM 浏览器证据：

- 初始无 recipient：1 main / 1 nav / 1 h1、发送 disabled、重复ID0、横向overflow0；
- hard reload 前后 channel 数 **52 → 52**；
- hidden direct recipient `fixture-hidden-private` URL与hard reload均恢复为 `Hidden Counsel`；
- Combobox `Knowledge` + ArrowDown + Enter 选中 `fixture-system-public` 并写回 URL；
- 首发后 channel 数 **52 → 53**，详情返回native thread并导航`/channel/channel-created-1`；
- 1440×900 / 1024×640 / 900×640 / 600×640 均overflow0、composer可见、landmark/h1 exact；
- 最终 console error/warn **0 / 0**。

fixture只证明GUI行为；生产事务/权限由上面的PG17.11 SCRAM承担，不混作一条证据。浏览器tab、
fixture进程、PG实例已全部关闭；为避免磁盘耗尽只删除了可重建的`target/debug/incremental`缓存。

Cargo.lock package 数 **822 → 822**；只给 `openbot-ui` 增加既有 `uuid` 直接边并开启其官方
WASM `js` CSPRNG feature，新增 package=0。

## 4. 精确台账变化

- API：`T-API-0031`、`T-API-0095`、新增 `T-API-0163`。
- tests：`T-TEST-0403–0415`、`T-TEST-0421–0426`、`T-TEST-0873–0888`，共 **34** 条。
- route：`T-ROUTE-0010`。
- UI：`T-UI-0045`。

## 5. 明确仍未完成

- [ ] `T-ROUTE-0009`完整channel transcript/history/realtime Composer、draft/queue、`@`/`/skill`、
  Enter/Shift+Enter/IME、stop/steer/failure recovery与Screen detail；本批没有画这些控制。
- [ ] 首页`T-ROUTE-0006`完整Composer和自动routing UI；`POST /api/route`生产面完成不等于首页完成。
- [ ] `T-UI-0043`完整Composer、`T-UI-0041/42/44`channel chat/transcript/conversation-view，
  以及`T-UI-0129`正式golden仍todo。
- [ ] 多Agent同一channel UI、Agent create/edit/duplicate/hide/delete lifecycle、Memory GUI、
  browser/file/shell、G5/G7、三家recorded/live vendor trace与正式发布面仍未完成。
- [ ] G3/G4/G6整关继续不勾；AppSidebar仍缺skills/settings/admin destinations。
