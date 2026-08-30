# Batch69：Desktop Agent custom-protocol typed transport

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-29-G4-G6-desktop-agent-lifecycle`（分支创建于日期切换前）
>
> base：`ecdd9831a86348e6d6919adfda6064e6374e5586`（PR #51 merge commit）
>
> implementation：`5351ec57ec60edd2fdf4ec98cc49fe479b82a9ae`
>
> 第一真源：v4 §5.1–§5.3、§6.4、§13.1–§13.3、§15.1–§15.3、§21.1、§24 G4/G6、§28.1 R143；Batch68/R142 的 Server/Application/PG/Vault 规则原样复用。

## 1. 结论

本批把同一 Leptos `/agents` 所需的 Agent API 接到 Tauri custom protocol：

- `GET /api/agents[?hidden=true|false]`；
- `GET /api/agents/{id}`；
- `POST /api/agents/test-connection`；
- create、update、duplicate、hide、unhide、delete 七个 lifecycle 写面。

Desktop 没有新增 store、权限判断或 Agent DTO；所有请求仍交给同一个
`InProcessTransport -> Arc<dyn ApplicationService>`。响应与 Server 保持同一 closed envelope：
list/detail/test/update 为 200，create/duplicate 为 201，hide/unhide/delete 为 204。

本批没有关闭 T-ROUTE-0007 或 T-UI-0031/0032，也没有新增 T-ID。正式 native-window journey、
callback-token panel/API、`/channel/new` create/run、Windows runtime 与 golden 仍 todo。

## 2. Authority 与 framing

- 每个 request 先以 host-created window label 读取 `WindowAuthority`；renderer 无法自报 actor/role/generation。
- list/detail GET 只要求已绑定 authority；与 Server 普通 authenticated read 一致。
- connection probe 与六个持久 mutation 在解析 JSON 前要求 `fresh_until` 尚未到期；stale body 即使 malformed
  或含 secret 也只得到 401，不进入 application。
- hidden query 只接受 absent、空、`hidden=false`、`hidden=true`；其他一律 400。
- Agent ID 只走既有 closed percent-decoder；raw slash、坏 `%` 不能成为 path authority。
- Agent JSON 固定 64 KiB；未知字段由 contracts `deny_unknown_fields` 返回 400，超限返回 413。
- 解析成功、malformed、oversize 三条路径都在 await/application 前把原始 `Vec<u8>` 全字节覆零。
- AppError 继续只投影 stable code；response 均 `Cache-Control: no-store`，customer Authorization 永不回传。

## 3. 测试结构

同一个 `FakeAgents` 同时实现 `AgentDirectory` 与 `AgentAdministration`，再注入真实
`OpenBotApplication`；因此测试不是 Desktop 内自写业务捷径。矩阵覆盖：

- unbound 既有 401 与 non-fresh malformed secret POST 的 fresh-before-parse；
- visible/hidden list、detail、create、probe、preserving update、private credential-free duplicate；
- hide -> default roster移除 -> hidden roster恢复 -> unhide -> delete -> detail 404；
- exact 200/201/204/400/401/404/405/413 与 no-store；
- forged `ownerUserId` 400、unknown query 400、oversize 413；
- create/probe response 对 `DESKTOP_AGENT_SECRET_CANARY` 命中 0；
- body parser 成功/坏 JSON/超限三路缓冲逐字节全 0。

## 4. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| `cargo check -p openbot-desktop --all-targets --all-features --locked` | 绿 |
| contracts/application/Desktop tests | `95/153/83`，0 fail |
| Desktop Clippy | all-target/all-feature，`-D warnings` 绿 |
| Windows target | `x86_64-pc-windows-msvc` all-feature check 绿；**compile-only，不冒充 runtime** |
| Tauri dependency guard | Linux host graph absent；13 build scripts；9 WebView2 payloads；既有 MPL/UNIC/Vet blockers 原样保留 |
| parity | `808/886/1694`，0 violation/0 warning；overlay `1450/236/2/6` |
| strict recount | fixed upstream `891df72f…`，`159/0/0` |
| invariants | Cargo.lock 0；`grok-bot` tree 仍 `86f5a85f…`；非 Grok 恰一个 package.json；零 npm |

首次全目标编译因 `AgentRoute<'_>` 返回借用缺显式 lifetime 红，改为 `impl<'a> AgentRoute<'a>` 后绿；
首次 Clippy 因 `Result<T, Response<Vec<u8>>>` large Err 红，改为两值 `AgentBodyError` 后绿。两次失败均未计通过。

## 5. 明确未做

- 未运行 `cargo xtask ci` 或 GitHub Actions（R63 manual-only）；
- 未修改 Web UI、CSS、locale、bundle，故不伪造新的 WASM/browser/bundle 证据；
- 未实现 callback-token issue/revoke 的 Desktop framing；T-UI-0033仍todo；
- 未实现 channel/thread API 的 Desktop framing，Agent profile 的 Start channel 仍不是完整 native journey；
- 未运行真实 Tauri window、Windows WebView2 runtime、签名/安装包或 golden；
- P1 Windows/runsc 仍红，P2/P3/P4 未进入。

## 6. 下一步

P1 外部平台证据未到前仍不越门进入 P2。可继续并行补不依赖 P1 的 Desktop channel/thread transport，或选择
G4/G6 其它独立 todo；任何 native route 完成仍需正式窗口/Browser/AX/golden 才能关闭 route/UI ledger。
