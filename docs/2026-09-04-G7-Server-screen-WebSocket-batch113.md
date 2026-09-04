# Batch113：Server ScreenSession 与票据化 binary WebSocket

日期：2026-09-04（America/Los_Angeles）

分支：`feat/2026-09-04-G7-server-screen-ws`

implementation：`5a7444e1db068d504157260da66ba9696f513dc6`

第一真源：`CLAUDE.md`；v4 §5.2、§12.2–§12.4、§12.6、§13.4、§24 G5/G7、§28.1 R188

## 1. 本批结论与边界

Batch113把Batch111的ScreenHub/ticket核心接成真实Server数据路径：同一typed `ApplicationService`发放
actor/generation/origin-bound ticket，Axum在same-origin session authority下从
`Sec-WebSocket-Protocol`消费ticket，只选择非秘密base protocol，并把真实Engine frame作为binary
WebSocket frame发给浏览器。生产Server main构造一个共享ScreenHub，Application port与WebSocket handler
持有该Hub的同一状态。

本批只关闭Server frame transport，不关闭完整`T-BROP-0027`或G7。以下仍未完成：production
`BrowserRuntimeManager → EngineProcess → ScreenHub::attach` source装配、真实PostgreSQL session-cookie浏览器、
外部TLS终止、Desktop loopback WebSocket、viewer input、bandwidth/idle、最后viewer断开2秒停流、fps/p95及
Windows/runsc。Desktop application assembly显式注入fail-closed `NoScreenSessionAdministration`，没有把缺失
loopback服务伪装成可用。

## 2. 唯一ApplicationService发票

- contracts新增`ScreenSessionTarget`：只含ComputerId、ComputerGeneration、TabId；没有actor/tenant/auth
  generation/origin/window/ticket自报字段。
- Server的POST body只反序列化上述target；exact Origin由`OriginBoundAuthenticated`在body parse前从已验证
  header保留，再由handler构造`ScreenViewerBindingRequest::Server`。坏Origin+坏JSON实得403 Origin错误，
  证明未先读body。
- `AppCommand::IssueScreenSession`与`AppReply::ScreenSession`进入既有无通配穷举；Agent、Server channel与
  transport-parity的wrong-reply/route台账同步扩展。未注入port稳定503。
- `ScreenSessionService`在ScreenHub内按current AuthContext查唯一可见的computer/generation/tab，不接受scope
  digest。零个或多个可见候选统一404；viewer cap统一409。
- raw ticket先由`IssuedScreenTicket`的`SecretBytes`持有，跨边界ticket string的Debug始终`[REDACTED]`且
  Drop主动zeroize当前String allocation。HTTP序列化只在already-authenticated no-store响应中发生。

## 3. Server HTTP/WebSocket安全边界

新增：

- `POST /api/screen/sessions`：same-origin authenticated、Origin先于JSON、typed ApplicationService、no-store；
- `GET /api/screen`：无query；requested protocols必须恰为base+一个ticket，顺序不限、重复/缺失/多余均400；
  ScreenHub在101前验证actor/auth generation/origin/stream/TTL并单次消费；upgrade response只选择
  `openbot.screen.v1`，绝不回显ticket。

frame行为：

- 握手后立即发current `OBSCRN01` binary，之后等待同一个size-one watch latest；慢socket不会创建per-frame
  Rust队列；
- output上限从Computer单源`MAX_ENGINE_IMAGE_BYTES + 68 = 16,777,284`，不是Server重复常量；
- inbound message/frame上限1KiB；当前viewer input尚未装配，Text/Binary统一1008
  `screen_input_not_enabled`，Ping/Pong/Close保持协议语义；
- source结束或auth generation撤销统一1008 `screen_revoked`，错误帧不带actor/origin/ticket/stream ID；
- `?ticket=...`等任意query即使同时携合法protocol也在消费前400；测试只用固定canary query，不把真实ticket
  再复制进URL。

Server生产默认每stream最多8个active+pending viewer；`ScreenHub::new`的硬上限仍为256。该8是R188新增的
保守Server默认，不冒充固定上游值。

## 4. 真实Engine纵切

显式ignored的macOS测试实际启动官方Electron 43.3.0 BrowserComputer role：

```text
Page.startScreencast
→ authenticated UDS frame pipe
→ Rust ScreenIngress
→ ScreenHub
→ ApplicationService IssueScreenSession
→ Axum TCP WebSocket
→ client binary OBSCRN01 + JPEG
```

客户端收到viewer sequence与`StartedSession.frame.sequence`相等，offset68开始为JPEG
`ff d8 ff`；upgrade选中base-only。客户端关闭后Engine stop/shutdown有界完成，临时profile/temp清理。该测试
使用ExactAuth以隔离frame transport；真实PG cookie与外部TLS明确不在证据内。

## 5. Fixture、台账与验证

新增`fixtures/computer/server-screen-websocket-v1.json`：1970 bytes，SHA-256
`6e7365a084c8ee9b33ed916ccfbd5e5cca399a23189a8cdd0be5ec784bfb2a2d`。

| 检查 | 本轮结果 |
|---|---|
| Contracts | `105/0/0`；ticket wire/Debug/zeroize contract |
| Application | `166/0/0`；command dispatch与default fail-closed |
| Computer | lib=`65/0/0`；fixture=`4/0/0`；host=`0/0/2 ignored`；screen port定向=`1/0/0` |
| Server | lib=`226/0/0`；screen定向=`4/0/0`；真实Engine→WSS=`1/0/0` |
| Desktop | `131/0/3 ignored`；当前screen port显式不可用 |
| transport parity | `8/0/0`；新command无通配登记 |
| workspace | all-target/all-feature locked check通过 |
| Clippy | Contracts/Application/Agent/Computer/Infra/Server/Desktop/Testkit八crate `-D warnings`通过 |
| cross-target | Contracts WASM通过；Computer Windows Clippy与Linux check通过，runtime未跑 |
| guards | WebSocket四caller精确guard、Tauri/UI/六release-target deny guards通过 |
| engine | protocol/epoch=`3/3`、shim=`596/600 LOC`、ASAR/fuses/integrity/signature通过 |
| parity | API=`96/80/176`；总=`873/839/1712`；overlay=`1275/429/2/6`；0 violation/warning |
| fixtures | `32/21/53`；新增T-FIX-0053 |
| recount | non-strict=`71/0/89 skipped`；未配置上游，strict未跑 |

新增T-API-0175/0176关闭发票与Server frame WebSocket路由；它们不替代仍todo的T-BROP-0027。Cargo.lock只
增加既有workspace内部依赖边，package总数仍829、无新第三方版本。

中间红灯均未计通过：首次`--locked`正确拒绝未同步内部依赖记录；Server Origin负向行为正确但测试期待了
错误泛化码，实得2/3后改成权威码；旧WebSocket guard先拒绝第四caller，随后只加入exact screen文件与
1KiB/query/base-only/1008棘轮，没有放宽依赖或字符串范围。

本批无schema/native migration、UI/CSS/locale/Web bundle、engine shim/protocol、npm/Grok/workflow变化。未运行
R63禁止的`cargo xtask ci`，未派发Actions。

## 6. 下一步

下一步应把Server production `BrowserRuntimeManager`真正接到EngineProcess并在session start时attach同一Hub，
再用真实PostgresSessionAuthResolver cookie+Origin跑浏览器；同时实现auth-generation事件hook、带宽/idle和
2秒停流。Desktop必须另建loopback binary WS并由Tauri window binding发票，不能复用Server listener或把
No-port改成静默fallback。viewer input接线时还须重验Batch112 frame sequence/geometry与HumanLease。
