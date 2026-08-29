# OpenBot G3/G6 Home Composer Batch63

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-G6-home-composer`

基线：Batch62 PR #45 已以 merge commit `947a116118f66fbe47e492f0969a6677eebe61af` 合入
`main`。

implementation：`07bff3a2b0ab010f2d4f4d78702a61d03aa3d1d5`

## 1. 结论

本批把根 `/` 从临时 `ApprovalPage` 改成固定上游的真实 Home Composer：

- roster 只取当前 actor 可见的 non-hidden coworker；
- Explore 恰为 `!mine && visibility=public`；
- 无显式 mention 时 fallback 恰为 `explore[0] ?? agents[0]`；
- `@` 只能通过 Server roster 候选产生 structured Agent ID，自由文本不反解析成 authority；
- 有 mention 时 recipient 由人决定，route 请求只负责记录；记录失败不推翻用户选择；
- 无 mention 时先走 production `/api/route`，transport/audit/坏response均回固定 UI fallback；
- recipient 固定后严格 create channel → native BeginRun → navigate；retry 不重新 routing；
- Explore card 只跳 `/channel/new?agent=...`，不会提前创建空 channel。

只关闭 `T-ROUTE-0006`。formal Home golden `T-UI-0130` 与跨页面完整 Composer
`T-UI-0043`（sources/附件/`/skill`/queue/stop/steer 等）继续 todo。

## 2. 第一真源与实现边界

固定上游 `891df72f` 的 Home route 明确四件事：

1. `explore = agents.filter(!mine && public)`；
2. `fallback = explore[0] ?? agents[0]`；
3. structured `draft.agentId` 是显式选择；显式 route 的返回值丢弃，失败也吞掉；
4. 无 `agentId` 才推断，推断失败回同一个 fallback，随后 `start(agentId,text)`。

本仓 R95 已有 production channel create、routing、native BeginRun 与 `/channel/new`，所以本批没有新增
产品 API。反过来，也没有把 `/channel/new` 的“必须先选人”表单复制到 Home；那会删除自动 routing 的
可观察行为。

## 3. Structured mention 与 ARIA

Home draft 的 `@` 触发规则收紧为：

- `@` 位于字符串开头或前一字符为空白；email 内的 `@` 不触发；
- suffix 按 Agent name / stable ID 做大小写无关过滤；
- Enter 或 option click 从 roster 选择；选择后文本写 `@Display Name `，另存 Agent ID；
- 第二次选择先删除旧 marker，保持最多一个 structured recipient；
- 用户删除 marker 后 selection 自动清空；仅手写同名字符串不会新建 selection。

共享 `Textarea` 只在显式 opt-in 时增加：

```text
role=combobox
aria-autocomplete=list
aria-controls=home-mention-results
aria-expanded=true|false
```

其它 Textarea 语义不变；combobox controls ID 仍受单DOM-token guard。候选 option 支持原生
Tab + Enter/Space，空候选有本地化事实。

## 4. Routing response 与 fallback

`route_channel_message` 新增 browser-side closed response 校验：

- text 复用 `MAX_THREAD_MESSAGE_BYTES`、ECMAScript trim、NUL拒绝；
- Agent ID / name 有界且无控制字符；
- reason 最多500个Unicode scalar；该常量上收到contracts并由domain re-export；
- explicit receipt必须同ID、`viaMention=true`、`fallback=false`；
- inferred receipt必须`viaMention=false`；Home再要求 chosen ID仍在当前 roster。

显式 mention 的 route 调用失败只表示“记录选择”失败；固定上游要求仍按人选的recipient继续。无 mention
的任意 route error/invalid receipt则选UI fallback。recipient与run identity一旦固定就写入`StartAttempt`，
retry不再次调用route，因此同一消息不会因瞬时模型/roster变化换人。

## 5. create → BeginRun shared executor

原 `/channel/new` 已有正确顺序但实现内联。本批把 executor留在既有
`features::channels::new` 所有权下并让Home复用，避免新增顶层module触发全features overlay，也避免两套
状态机漂移：

- definite create failure可按同attempt重试；
- create response丢失标`CreateUncertain`，禁止重发以免重复channel；
- create成功后attempt绑定channel；
- thread缺失或BeginRun失败标`Begin`，retry复用同channel/run ID；
- BeginRun成功但navigation失败仍可replay同run再导航。

`/channel/new` 本轮重新走release浏览器，Risk Analyst URL recipient发送后创建第二条真实channel/thread，
首消息可见、alert/console 0。

## 6. Release 浏览器证据

memory fixture 初始52条channel、5个non-hidden Agent、2个Explore Agent；deterministic routing backend只在
testkit binary存在。proof只投影计数、chosen ID与created channel/Agent/thread ID，不含用户消息或模型
reason。

首屏：

- 根URL `/`、唯一h1“新建频道/Start a new channel”；
- 空Composer发送disabled；
- routing hint可见；
- Explore恰Knowledge Desk/Risk Analyst两卡，href分别是精确`/channel/new?agent=`；
- console error/warn 0。

四条分支：

1. **structured mention success**：输入`@know`后listbox唯一Knowledge Desk；Enter后文本为
   `@Knowledge Desk `、expanded=false。发送后heading=`fixture-system-public`，首消息可见；proof：
   `complete=0 / explicit=1 / recorded=1`。
2. **inferred success**：无mention文本得到Risk Analyst；proof：
   `complete=1 / inferred=1 / lastChosen=fixture-explore-public`。
3. **inferred route/audit failure**：fixture completion置unavailable并让该次record失败；HTTP route失败后
   UI回Knowledge Desk，仍create/begin/navigate；proof：`failedRecords=1`。
4. **explicit audit failure**：先arm next record failure，再选Knowledge Desk；proof：
   `complete=0 / recordAttempts=1 / failedRecords=1 / recorded=0`，页面仍按用户选择创建thread并显示消息。

前三条主矩阵使channel count `52→55`，每条created row均有非空thread ID。root hard reload与Explore card
点击后仍为55，证明零空channel。中文/英文均实跑；1280×720 与600×900 horizontal overflow0；1280 DOM
为main1/nav1/h1 1/Explore2/duplicate ID0/nested interactive0，alert/runtime error0。

最终模块归属/共享executor调整后又从全新release bundle实跑：Home structured Risk mention创建
`channel-created-1`，`/channel/new?agent=fixture-explore-public`再创建`channel-created-2`；两条都有thread与
首消息，回归alert/console 0。一次组合读取在RecipientField周期性重绘时命中detached locator；随后用DOM
确认Risk Analyst并用稳定message ID发送成功，不把控制层读取超时写成产品失败或通过。

## 7. 真实 PostgreSQL 17.11

一次性PG只监听`127.0.0.1:55463`，SCRAM-SHA-256；本轮亲跑既有ignored suites：

```text
channel_create: 2 passed / 0 failed / 0 ignored
channel_routing: 1 passed / 0 failed / 0 ignored
```

覆盖：max_pool=1单连接事务、独立channel/thread identity、六surface原子、Unicode120、四类denial零残留、
admin private正向、profile delete lock/TOCTOU、created thread→真实BeginRun membership/message/run/activity；
以及active grant reach、provider request cap/no tools、genesis hash-chain、消息/模型理由canary0、candidate
变化409且audit仍唯一。PG/socket/password/log/data目录测试后全清。

## 8. 机械证据

| 面 | 本轮结果 |
| --- | --- |
| contracts | `90/0/0` |
| domain routing | `12/0/0` |
| application routing | `6/0/0` |
| UI | `152/0/0` |
| Server | lib `213/0/0`；fixture `6/0/0` |
| PostgreSQL | channel create/begin `2/0/0`；routing/audit `1/0/0` |
| Clippy | contracts/domain/application/Server/UI all-targets/all-features `-D warnings`通过 |
| WASM/fmt | UI wasm32、workspace fmt通过 |
| GUI build | pins verify + release/offline/locked Trunk；零npm |
| i18n/design/CSS | `677` leaf keys；`96` Rust files/`74` icons；`326` class literals |
| bundle | wasm gzip `1,605,538/3,670,016`；CSS `108,118/131,072`；fonts `740,216/819,200`；scripts=`1/0` |
| parity | routes=`15/17/32`；UI=`87/65/152`；总=`702/992/1694`；overlay=`1579/113/2/0`；0违反 |
| strict recount | fixed upstream `891df72f…`，`159/0/0`，skip0 |
| Grok/shim | tree `86f5a85f…`；inventory2,110；shim405/600；单package/零npm锁 |

`parity-check` 初次列81条revalidate，是Home错误挂在顶层`features/mod.rs`使diff前缀命中所有features。
修正所有权到`shell::home`后仍有41条；再把shared start放回既有`channels::new`所有权而非新建
`channels/mod`子模块，最终只剩15条真实受影响目标：Home、ChannelNew、routing11、Textarea、AgentCard。
这些目标均有上述重跑证据；没有批量登记空转revalidate。

## 9. 台账与明确未做

- `T-ROUTE-0006`：todo→done；
- revalidate：`T-ROUTE-0010`、`T-TEST-0873–0883`、`T-UI-0020/0030`；
- routes `14/18→15/17`；总parity `701/993→702/992`；overlay `1594/98→1579/113`；
- `T-UI-0043`完整Composer、`T-UI-0130`Home formal golden继续todo；
- 未实现Home attachments/sources/`/skill`，也未把channel queue/stop/steer完成度扩大到Home route；
- 未运行全workspace test；只运行上述变更面与五crateClippy；
- 未运行`cargo xtask ci`，未派发GitHub Actions（R63 manual-only）；
- P1 Windows/runsc runtime仍红，未进入P2；
- `grok-bot/`零改动，没有Grok产品能力或文本进入本批。

本轮结束前删除可重建`target/`、`target-xtask/`、Trunk `dist/`与固定上游临时克隆，保留提交和源码。
