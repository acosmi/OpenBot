# Batch 45：Compiled Components in Conversation

> 日期：2026-08-28。分支 `codex/2026-08-28-G6-component-conversation`；base `e50062c`；
> WIP `9790e62`；implementation `b28801d0a007274376fef069be9f9d72f47f6d59`；
> 固定上游 `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批把 11 个 ordinary compiled component 接入生产 Agent sampling、调用时授权、durable history 与
Leptos conversation renderer。`askApproval/askChoice` 仍是独立 HITL 路径；未运行 `cargo xtask ci`
或 Actions，未触碰 `docs/assets/`，未 push/建 PR。

## 第一真源裁决

- provider definitions 必须来自 Server build registry 与 fresh `for-agent` governance，不得由 browser、
  model 或 provider 自报；published description 与 per-Agent withholding 在每次 sampling 重读；
- model arguments 先通过与 renderer 同源的 closed validator，再由参数推导 Activity data function；模型
  不能自报 function。每次调用仍使用 fresh AuthContext 走 `DecideComponent`；
- ordinary card/chart 不是 acting effect，不能伪装成通用 tool policy/executor；允许后只返回上游逐字
  confirmation，拒绝返回 stable error code 与 bounded model-visible 句子；
- durable `messages` 是唯一 transcript 真源。assistant tool call 与 tool result 只有在 call id、tool name、
  Server-derived Agent identity 三者一致时才合成 renderer；缺失、错配、错误结果或 schema 失配统一画
  `RefusedCard`；
- Agent identity 必须由 message 的 `run_id → runs.bot_id` 关联取得，不能猜频道 roster 第一项；否则
  multi-Agent channel 的 Activity 会对错 Agent 授权或读取；
- UI 不把任意 JSON spread 进 Leptos props，只在共享 validator 成功后逐字段构造 11 种 typed renderer。
  `askApproval/askChoice` 继续等待 durable HITL 挂起/回应同批实现。

## 实施

- contracts 新增 ordinary registry lookup：11 个 exact schema/title/confirmation、closed argument validator，
  Activity 只由 `report=activity|refusals` 推导 `botActivity|recentRefusals`；
- `PostgresAgentContextSource` 复用同一 `PostgresComponentAdministration`，按 fresh actor admin role、Agent、
  current build 列 provider tool definitions；built-in/remote AG-UI 都收到相同 schema/description；
- `AuthorizedAgentToolGateway` 在 generic acting tool 前分流 compiled component，invalid args 不触发
  application，合法调用 fresh decision；成功/拒绝均形成一个 durable tool reply；
- `ThreadHistoryMessage` 增加只读 `agentId/toolName/toolErrorCode`。PostgreSQL history/conversation query 从
  message 的 run 关联 Agent；tool 字段结构坏继续 fail closed；
- conversation projection 按 provider call id 合并 assistant/tool 两行；Agent/name 任一错配、result missing
  或 error code 存在都不渲染模型参数，只显示共享拒绝卡；
- 新增 `ConversationComponent`，逐字段构造 Activity、Quote、四 Cards、五 Charts；Activity 复用 Batch44
  typed data API，普通组件零 data read。

## 证据

| 面 | 本轮亲自运行结果 |
| --- | --- |
| contracts / Agent gateway | component registry **5 / 0 / 0**；gateway **5 / 0 / 0** |
| UI / Server | conversation **11 / 0 / 0**；runtime helper **1 / 0 / 0**；Server thread HTTP **21 / 0 / 0** |
| PostgreSQL 17.11 host SCRAM | provider context **1 / 0 / 0**；thread conversation **1 / 0 / 0**；thread history/Agent identity **1 / 0 / 0** |
| compile / Clippy / WASM | 8 crate all-targets/all-features check 与 Clippy `-D warnings`；contracts/UI WASM 绿 |
| tools / i18n / design / CSS | pins 全绿；**515** leaf；**85 Rust / 74 icons**；**271** class literals |
| production bundle | WASM gzip **1,247,562 B**；CSS **93,646 B**；fonts **740,216 B**；external/inline **1/0** |
| parity / recount | API **58/109/167**；events **29/53/82**；components **5/17/22**；总计 **663/1015/1678**；strict **157/157/0** |

真库 provider-context 先 unpublish Notice、withhold Quote，实得 definitions 恰为 build manifest 减两项，
逐项 description/schema exact；随后新增 Bar withholding，下一次 load 立即少一项。Thread history 真库证明
run-linked user/assistant/tool 都投影同一个 `bot-1`，system/summary 不伪造 Agent；缺 `toolCallId` 仍报
closed corruption。既有 atomic conversation snapshot 回归保持通过。

Release 浏览器的 conversation API 明示六条 durable message 均带权威 `agentId=bot-0`，两组 component
call/result 的 id/name/Agent 一致。页面实得 Quote 文案与 figure 各 **1**、Refused status **1**，被拒
Notice 的 `Refused fixture` 正文 **0**，普通 assistant 文本仍在；hard reload 后相同。1440×900、
1024×640、600×800 均 `scrollWidth=clientWidth`；nested article、duplicate id 为 0，main/nav/h1 各 1。
应用异常为 0；浏览器另报告既存的 Chromium preload-SRI 支持警告 1 条与未声明 `/favicon.ico` 404
1 条，本批未把它们伪写成 console 0，也未为消警告削弱 SRI。

关闭 `T-CMP-0002/0003/0007`。`T-CMP-0001` 仍因上游可选 conversation follow-up `ask` action 缺失
而 todo；`T-CMP-0008` 仍因 sandboxed path 尚未复用 `RefusedCard` 而 todo；Decisions/HITL、admin、
sandbox/Desktop renderer 与 formal golden 继续未完成。CSS 预算仍只余 **4,658 B**。
