# Batch71：Desktop channel/thread unary typed transport

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G4-G6-desktop-channel-run`
>
> base：`03dd18c7164ebd2e8ae036a63753a493d7e3dd6d`（PR #53 merge commit）
>
> implementation：`502302066c506f8c7373daf14476009197cf1922`
>
> 第一真源：v4 §3.1条4–5、§4.1–§4.3、§5.1–§5.3、§13.1–§13.4、§15.1–§15.3、§21.1、§24 G3/G4/G6、§28.1 R95/R97/R98/R144/R145。

## 1. 结论

本批把同一 Leptos bundle 当前使用的 channel/thread **一元** HTTP 面接到 Tauri custom protocol：

- `GET/POST /api/channels`；
- `GET /api/channels/{channel_id}`；
- `POST /api/route`；
- `POST /api/threads/mint`；
- `GET /api/threads/{thread_id}`；
- `POST /api/threads/{thread_id}/runs`；
- `POST /api/threads/{thread_id}/runs/{run_id}/cancel`；
- `GET /api/threads/{thread_id}/conversation`。

Desktop 没有新增 channel/thread store、权限判断、业务 DTO 或重试规则。九条路都只把 host-bound
`AuthContext` 与 closed path/query/body 投给现有 `ApplicationService`，再按 Server 的 direct/envelope/status
形状响应。

本批明确**没有**把 `/api/channels/events`、thread `/events` 或 `/ws` 接成普通body；三者继续404，等待
`DesktopSession`/Tauri Channel 的structured-event桥。也没有真实Tauri window，因此不关闭任何route/UI/T-ID。

## 2. Authority 与 framing

- 所有请求仍先按host创建的window label取得`AuthContext`；renderer不能自报actor/tenant/role；
- Server这组写面使用`OriginAuthenticated`而非fresh extractor。custom scheme天然同源，所以Desktop只要求
  已绑定window，不擅自加入fresh条件；测试刻意用`fresh_for=None`完成全部写面；
- channel list query只允许至多一个`limit`和一个`cursor`，顺序自由；key/value按form-urlencoded规则处理
  percent与`+`，重复/未知/坏percent/负limit/超4KiB query统一400；
- channel/thread/run path只接受closed segment/segment count；静态`events`与`mint`不落进动态ID；
- create/route/begin的总JSON body上限取contracts的1MiB常量，与Server全局request body cap同量级；
- 用户首消息/route文本所在原始`Vec<u8>`在成功、malformed、oversize三路都逐字节覆零；wrong method、
  generic unknown API与非GET fallback也覆零；
- mint/cancel要求空body；非空不被静默忽略。

## 3. Response shape

| Surface | 成功形状 |
| --- | --- |
| channel list | 200，顶层`ChannelPage` |
| channel detail | 200，`ChannelDetailResponse { channel }` |
| channel create | 201，同一detail envelope |
| route | 200，顶层`ChannelRoutingDecision` |
| thread mint/status | 200，顶层`ThreadMinted` / `ThreadStatus` |
| begin | 首次201；exact replay 200；顶层`ThreadRunStarted` |
| cancel | requested/already-requested 202；already-terminal 200；顶层typed receipt |
| conversation | 200，顶层原子`ThreadConversationSnapshot` |

所有typed JSON响应继续`Cache-Control: no-store`；AppError只发stable code，不发用户文本、message或远端正文。

## 4. 测试矩阵

测试使用一个共享内部状态的`FakeChannelRuntime`，同时实现`ChannelReader`、
`ChannelAdministration`与`ThreadDirectory`，再注入真实`OpenBotApplication`；routing另经真实Application
用例与一个typed backend。测试不是Desktop内复制业务规则。

同一ordinary bound window实得：

- list 200/no-store，unknown/negative/duplicate query 400；
- create 201，detail 200且逐字段相等；
- explicit route 200且首消息canary不在响应；
- mint/status 200/known=true；
- channel-anchor begin首次201、same body replay200，message canary不在响应；
- conversation投影Running/cancellable；cancel首次与重放均202、状态Requested/AlreadyRequested；
- cancel后conversation投影Cancelling/non-cancellable；
- malformed begin 400、1MiB+1 route body 413、non-empty mint body 405，响应canary0；
- thread events与channel events均404，不冒充流。

独立parser测试还覆盖success/malformed/oversize三路body覆零，以及path/query closed grammar。

## 5. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| `cargo test -p openbot-desktop --all-features --locked` | `85/0/0`；doc-test 1 ignored显式可见 |
| Desktop Clippy | all-target/all-feature，`-D warnings`绿 |
| macOS host | all-feature test/check绿 |
| Windows target | `x86_64-pc-windows-msvc` all-feature check绿；compile-only，不冒充runtime |
| Tauri dependency guard | Linux host graph absent；13 build scripts；9 WebView2 payload；既有policy blockers不变 |
| parity | `813/881/1694`，0 violation/0 warning；overlay `1445/241/2/6` |
| strict recount | fixed upstream `891df72f…`，`159/0/0` |
| invariants | Cargo.lock/workflow/Grok diff0；Grok tree`86f5a85f…`；非Grok恰一个package.json；零npm |

本批没有UI/CSS/locale/Cargo依赖变化，所以没有重跑Trunk bundle、Browser、Engine或tools fetch/verify；
Batch70的bundle只能作为历史最新产物，不能冒充本批新证据。未运行`cargo xtask ci`，未派发Actions。

## 6. 台账与边界

- 无新增/关闭T-ID；parity与overlay计数不变；
- 本批关闭的是Desktop framing缺口，不关闭T-ROUTE-0009、T-UI-0126/0127/0129或G4/G6整关；
- structured channel activity/thread event、reconnect cursor、window subscription与Tauri IPC Channel仍缺；
- `/channel/:id`在Desktop可完成detail+snapshot unary读取，但没有event bridge时只能显式显示stream unavailable，
  不能声称完整conversation journey；
- real native window、window close/drop、multi-window subscription、Windows WebView2 runtime与正式golden仍todo。

下一批若继续Desktop，应实现`DesktopSession`到Tauri IPC Channel的structured-event bridge，并让WASM在
Desktop宿主选择该桥而非EventSource；不得把SSE文本塞进Tauri event，也不得让renderer自报subscription scope。
