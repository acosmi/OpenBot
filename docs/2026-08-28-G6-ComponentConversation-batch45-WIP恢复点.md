# Batch 45 WIP：Compiled Components in Conversation

> 日期：2026-08-28。分支`codex/2026-08-28-G6-component-conversation`；base为Batch44正式head
> `e50062c`。固定上游`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 本机定向测试；不运行`cargo xtask ci`，不派发Actions，不触碰`docs/assets/`。

## 已核实边界

- `PostgresAgentContextSource`是每次sampling的权威history/tool definitions入口；当前只注入remember+MCP，
  不得在provider adapter另造component列表；
- `RunToolExchange`已原子物化assistant toolCalls(name/arguments)+tool result(name/errorCode)，无需新表或第二
  transcript真源；
- ordinary compiled component不是acting effect：provider call在`AuthorizedAgentToolGateway`用fresh
  AuthContext走`DecideComponent`，不伪装成通用tool decision/attempt/executor；拒绝仍由component runtime
  hash-chain audit承担；
- provider args必须先按build schema验证，Activity的function只能由report枚举推导，模型/browser不能自报；
- UI只从durable assistant toolCalls+配对tool result渲染；schema错、unknown/stale或error result走共享
  `RefusedCard`，绝不把任意JSON spread进Leptos props；
- 本批只接kind card/chart的11个ordinary renderer。`askApproval/askChoice`必须等durable HITL挂起/回应
  同批，不能当普通tool自动确认。

## 实施计划

1. contracts：WASM-safe component args validator、schema/confirmation/title lookup；history tool result补closed
   toolName/errorCode；
2. infra context：注入ComponentAdministration，按fresh actor role+Bot列grants，与remember/MCP稳定去重；
3. Agent gateway：component分流、args-derived functions、fresh decision、stable confirmation/refusal reply；
4. UI conversation：pair durable calls/results，按11种schema安全解析并mount renderer/RefusedCard；
5. provider/PG/thread conversation/release browser验证grant→tool definition→call→durable pair→renderer，撤权与
   malformed args零renderer；
6. 证据完整前不改T-CMP-0001/0002/0003/0007，不实现Decisions/admin/sandbox/golden。
