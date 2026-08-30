# OpenBot G6 Direct Bot Chat Batch66

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-G6-direct-bot-chat`

基线：Batch65 PR #48 已以 merge commit
`9107ce02c96cb862b3ca5f4f4124b032086f1a6d` 合入 `main`。

implementation：`794c6ec5f2b42d5deccadae77cd285f54d9947e2`

## 1. 结论

本批关闭：

- `T-ROUTE-0008`：`/bot?agent=<id>` Direct Bot Chat；
- `T-TEST-0173–0179`：per-Agent thread key 与 remembered-thread 选择七条固定上游判据。

formal page golden `T-UI-0127`、AppSidebar 总项、完整跨页面 Composer、Tauri window 与 G6 整关继续
保持 todo。P1 的 Windows/runsc runtime 仍红，本批没有进入 P2。

## 2. 第一真源与固定上游

本批只读取固定上游 `891df72f1827454d8b353d108fe5dd2313b7e30d` 的四个窄面：

- `app/src/routes/_authed/_app/bot.tsx`；
- `app/src/lib/copilot/bot-thread.ts`；
- `app/tests/bot-thread.test.ts`；
- `app/src/lib/copilot/stopped-turn.ts` 与对应测试。

保持的产品判据：URL 选 Agent、每 Agent 独立 remembered thread、已知 thread 恢复、权威 unknown 才
fresh、检查失败不丢历史、New chat 只在 mint 成功后换 thread。机制替代遵守 v4：

- 不保留硬编码 `risk-analyst`；无 query 时只取当前 Server roster 的稳定首行，选择仍来自部署数据；
- thread ID 只能由 Rust Server mint，不能在浏览器用随机 ID 伪造 deployment identity；
- 不复制上游 raw provider/Error 文本；失败/取消/reconciliation 继续使用已闭合的 typed terminal code 与
  本地化文案；
- 不接 CopilotKit Intelligence；status、snapshot、SSE、begin、cancel 都走 PostgreSQL native thread 面。

## 3. 产品实现

`BotChatPage` 位于 authenticated AppLayout 内，新增 AppSidebar 的真实 `/bot` destination。页面：

- `?agent=` 先 join 当前 roster；URL 指向有权但 hidden 的 Agent 时再走 typed detail GET；
- 无 query 时使用 Server roster 首行，不出现产品内硬编码 Agent ID；
- `RecipientField` 改选后提交 same-origin URL，Agent 切换会重建 keyed thread pane；
- 本地 key 固定为 `openbot.bot-thread.<agent-id>`，读取值必须是长度不超过 64 的 UUID；坏值删除；
- localStorage 不可用或写满只损失跨 reload 记忆，不阻断当前会话。

`thread_to_use` 的闭集：

- 无 remembered：fresh；
- remembered + `known=true`：keep；
- remembered + `known=false`：fresh；
- remembered + inconclusive：keep，并显示 history unavailable，不以网络波动删除会话。

New chat 在初始解析期间禁用，点击后只在 Server mint 成功时替换当前 thread；失败保留当前会话。

## 4. 唯一 ConversationSurface 与 fresh 边界

Channel 与 Direct Bot 共用同一个 `ConversationSurface`。差异只有 typed anchor：

- Channel：`ThreadRunAnchor::Channel { channel_id }`；
- Direct Bot：`ThreadRunAnchor::DirectBot`。

首版浏览器实测发现：Server mint 只铸造 identity，首个 run 前尚无 PostgreSQL snapshot；若立即 GET
conversation，会得到预期 404 并污染网络错误日志。最终规则改为：

1. 本次刚由 Server mint 的 fresh thread 直接安装空状态，不请求 snapshot、不打开 SSE；
2. 首个 begin 成功后立即关闭 fresh 例外；
3. 此后 snapshot 404 一律 fail-closed，不再解释为空会话；
4. begin 失败保留 exact thread/run/Agent/message，Retry 继续用同一 run ID，避免 unknown commit 重发。

## 5. Testkit 与真实 PostgreSQL

既有 `required-features=testkit` fixture 只增加非产品 proof：

- unknown minted thread 的 conversation 与 production 一样返回 NotVisible；
- proof 只投影 mint/status/conversation/direct-run 计数与 typed IDs，不含消息正文；
- unit 断言 mint 未持久化、begin 后持久化、DirectBot/Agent 绑定以及正文 canary 不泄漏。

真实 PostgreSQL 17.11 一次性实例仅监听 `127.0.0.1:55466`，host auth 为 SCRAM-SHA-256。既有
ignored suites 本轮亲跑：

```text
thread_begin:        3 passed / 0 failed / 0 ignored
thread_conversation: 1 passed / 0 failed / 0 ignored
thread_directory:    1 passed / 0 failed / 0 ignored
```

覆盖：DirectBot 首建 thread/membership/lease/message/run/event/outbox 七面同事务、exact replay、late
outbox 冲突全回滚、legacy/foreign UUID 边界、scope-aware known 判定，以及 active text/cursor/live terminal/
materialized history。测试后实例 fast-stop，`pg_isready` 无响应，data/socket/log/password 全删。

## 6. Release 浏览器

最终 bundle 由 pins 校验后的 Trunk/Tailwind/Binaryen/wasm-bindgen 以
`--release --offline --locked` 构建。干净 localStorage：

1. 首屏 `mint/status/conversation=1/0/0`，空 transcript 可发送；
2. 首发完成后 `directRuns/directThreads/persistedDirectThreads/lastMessages=1/1/1/2`，Agent、thread、
   localStorage 三向一致；
3. hard reload 后 mint 仍 1、status=1，同一 user/assistant 历史各 1；
4. New chat 后 mint=2、storage 改为新 thread、旧消息 DOM=0，但旧 persisted thread 仍为 1；
5. 第二次发送后 runs/threads/persisted=`2/2/2`；
6. Combobox 改选 Risk Analyst 后 URL 为 `?agent=fixture-explore-public`，生成第二个 namespaced key 且
   mint=3；切回 Research Partner 后 mint 不增、status=2，恢复第二会话而不是第一会话。

中文、英文均真实切换。1024×640 与 600×900 下 body width 等于 viewport，x overflow=0，main/nav/h1
各 1，duplicate ID=0、nested interactive=0、alert=0。页面挂载后记录的 runtime error、unhandled
rejection、console error/warn 均 0。Chromium 对 preload `integrity` 的已知 warning 仍存在；它与
formal golden/brand favicon 一样不在本批冒充已解决。

## 7. 机械证据

| 面 | 本轮结果 |
| --- | --- |
| UI | `164/0/0` |
| Server fixture | `8/0/0` |
| PostgreSQL | thread begin/conversation/directory=`3+1+1/0/0` |
| Clippy | openbot-ui/openbot-server all-targets/all-features `-D warnings` |
| WASM/fmt | UI wasm32、workspace fmt 通过 |
| tools | Tailwind `4.3.3`、Trunk `0.21.14`、Binaryen `132`、wasm-bindgen `0.2.127` exact |
| i18n/design/CSS | `700` leaf keys；`100` Rust files/`74` icons；`337` class literals |
| bundle | wasm gzip `1,683,002/3,670,016`；CSS `109,859/131,072`；fonts `740,216/819,200`；scripts=`1/0` |
| routes | `20/12/32` |
| tests | `384/663/1047` |
| parity | `719/975/1694`；0违反/0警告 |
| overlay | carry/revalidate/split/superseded=`1560/132/2/0` |
| strict recount | fixed upstream `891df72f…`，`159/0/0`，skip0 |
| Grok/shim | git tree `86f5a85f…`；inventory2,110；shim405/600；单package/零npm lock |

## 8. 台账与明确未做

- `T-ROUTE-0008`、`T-TEST-0173–0179`：todo→done；
- routes `19/13→20/12`；tests `377/670→384/663`；总 parity `711/983→719/975`；
- overlay `1568/124/2/0→1560/132/2/0`；
- `T-UI-0127` formal Bot page golden 保持 todo；
- Agent create/edit/duplicate/hide/delete lifecycle、完整 channel markdown/sources/附件/draft/steer/Screen、
  AppSidebar skills/完整 admin destinations 仍未闭合；
- 没有运行全 workspace test、`cargo xtask ci` 或 GitHub Actions（R63 manual-only）；
- P1 Windows/runsc runtime仍红，未进入P2；
- `grok-bot/`零改动，没有新增Grok产品能力或复制其文本。

环境记录：首次 `tools fetch` 在受限网络内因 GitHub DNS 失败，授权后从官方源成功并通过 hash/version；
首次 `initdb` 在沙箱内因 SysV shared memory 被拒且自行删除未完成 data，沙箱外同参数使用 POSIX shared
memory成功。两次失败均未计作产品测试通过或失败。
