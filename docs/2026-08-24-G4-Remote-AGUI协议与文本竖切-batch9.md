# G4 Batch 9：Remote AG-UI 固定协议与文本生产竖切

> 日期：2026-08-24（America/Los_Angeles）
> 第一真源：`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §4.3、§7.1、§7.5、§13.1、§24、§28.1 R72
> 堆叠基线：PR #25 head `11101326e4b4bfc94ee389906bf882c484273882`
> 当前分支：`feat/2026-08-24-G4-remote-agui-protocol`

## 1. 本批闭合边界

本批首次把 package-backed `remote_ag_ui` Agent 接入 production run：

```text
active run/lease + membership
→ PostgreSQL Agent/Profile/configuration
→ authoritative ProviderRoute::RemoteAgUi
→ pinned RunAgentInput 0.0.57
→ unique SafeDialer POST
→ real HTTP body-gap SSE decoder
→ stateful AG-UI protocol decoder
→ normalized assistant text / visible reasoning
→ expected-sequence run journal
→ assistant materialization + unique terminal
```

这条竖切证明 remote Agent 的 lifecycle/text 已进入 Rust/PostgreSQL 真源。它不代表 remote callback、
工具授权、interrupt/resume、所有事件的 durable/UI projection 或 remote Agent 全生命周期已经完成。

## 2. 固定协议裁决

### 2.1 只认固定 0.0.57

实现逐项读取固定 [`@ag-ui/core@0.0.57` 类型产物](https://unpkg.com/@ag-ui/core@0.0.57/dist/index.d.ts)，
并以 [AG-UI Events](https://docs.ag-ui.com/concepts/events) 核对语义；没有按当前 SDK 记忆猜字段。
`AGUI_EVENT_TYPES` 固定 33 个唯一 literal，覆盖：

- lifecycle / text / deprecated thinking；
- tool call/result；
- state snapshot/delta、messages snapshot、activity snapshot/delta；
- step、reasoning、encrypted reasoning；
- raw/custom、interrupt outcome、error。

community Rust SDK 类型不进入 domain/application。开放 payload 只保留为名字明确带 `untrusted` 的
bounded `serde_json::Value`，不能携带 actor、target、capability、policy 或授权事实。

### 2.2 Stateful、fail-closed

- 第一条必须是匹配权威 thread/run 的 `RUN_STARTED`；
- success/interrupt/error 只允许一个 terminal，terminal 后任何字节都拒绝；
- 显式 text/tool/reasoning/step 必须 start/content/end 配对；convenience chunk 在边界展开成同一规范事件；
- tool partial JSON 在 end 时必须得到一个 object；
- RFC 6902 `add/remove/replace/move/copy/test` 在副本上完整成功后才原子替换 state/activity；
- malformed order、identity mismatch、坏 patch、半截 JSON、未知 event type 都只产生稳定本地错误码，不记录远端 body 文案。

### 2.3 RunAgentInput 没有身份自报槽

encoder 写固定 `threadId/runId/state/messages/tools/context/forwardedProps` 形状，保留 durable
assistant/tool 闭合 pair。actor、tenant、target、policy、capability 不存在于远端输入面。
当前只附权威 `openbotBotId` 与实际授予的工具名列表。

### 2.4 未签名工具保持零暴露

第一真源要求 callback token + 10 分钟 signed run assertion 同时绑定 bot/run/actor/tool-set。
本批尚未实现该凭据链，因此 production context 对 remote route 固定：

- `tools=[]`；
- `run_assertion=None`；
- adapter 收到“无 assertion 但非空 tools”时在网络前拒绝；
- remote 即使伪造本地未授予的 tool call，也不能进入 Rust executor。

这不是删减功能，而是下一批 assertion/callback 完成前唯一不越权的中间状态。

## 3. 网络与生产路由

- remote endpoint 只经现有唯一 `SafeDialer`；scheme、精确 CIDR allowlist、DNS 解析绑定、redirect 与
  peer 校验复用同一 egress policy；
- production 默认 HTTPS；只有管理员同时开启 HTTP 与目标 CIDR 时才允许本机/private endpoint；
- response 必须为成功状态和 `text/event-stream`；429/5xx/auth/status 规范化，不把响应 body 当错误；
- stall 测量真实 response body read gap，不把 downstream 消费背压算作远端沉默；
- `ProviderRouter` 按数据库权威 route 精确选择 remote adapter，不回落 package/managed provider；
- standing role 每 run 从 Agent/Profile 重读，并在首条 system message 注入既有 provenance guidance；
- 当前 SQL 只覆盖有 `deployment_packages` 外键的 package-backed remote Agent。`package_id IS NULL`
  的用户创建 remote Agent 仍未接生产 lifecycle，不在本批完成声明内。

## 4. 生产落点

| 子面 | Rust 落点 |
| --- | --- |
| 0.0.57 schema/decoder/JSON Patch/encoder | `crates/openbot-agent/src/agui.rs` |
| remote semantic provider adapter | `crates/openbot-agent/src/remote_provider.rs` |
| authoritative route + transport port | `crates/openbot-application/src/provider.rs` |
| package-backed Agent/Profile/context | `crates/openbot-infra/src/provider/context.rs` |
| SafeDialer HTTP/SSE transport | `crates/openbot-infra/src/remote_agui.rs` |
| exact route selection | `crates/openbot-agent/src/provider_router.rs` |
| production assembly | `crates/openbot-server/src/main.rs` |
| real PostgreSQL/run/SSE E2E | `crates/openbot-testkit/tests/agent_runtime_postgres.rs` |

## 5. 本机证据

本批遵守 R63：没有运行 `cargo xtask ci`，没有派发 GitHub Actions。

| 验收 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` / `git diff --check` | exit 0 / exit 0 |
| `cargo test -p openbot-agent --all-features --locked` | **28/0/0**；其中 AG-UI 8、remote adapter 2 |
| infra 真实 loopback POST + 任意 3-byte SSE 分片 | **1/0/0** |
| `cargo test -p openbot-server --bin openbot-server --all-features --locked` | **7/0/0** |
| PostgreSQL 17.11 TCP SCRAM `agent_runtime_postgres` | **6/0/0**；remote 单条另实得 **1/0/0** |
| application/agent/infra/server/testkit all-targets/all-features Clippy `-D warnings` | exit 0 |
| `bash tools/check-safe-dialer-dependencies.sh` | exit 0 |
| Cargo.lock package 数 | **428**，本批新增 package=0 |
| `cargo xtask parity-check --json` | **325 done / 1335 todo / 1660 total**，0 violations/warnings |
| strict upstream recount | **154/154/0**；上游 commit=`891df72f…` |

真库 remote 用例经 production `RunRelay`、context、router、SafeDialer、decoder 与 host，远端以任意
5-byte HTTP body 分片发送 lifecycle、step、state、messages、activity、raw/custom、reasoning 与 text。
最终实得：

1. package provider call=0；
2. production `agent.invoked` audit row 恰 1；
3. visible reasoning semantic chunk 恰 1；
4. assistant materialize=`remote answer`；
5. run 只有一个 completed terminal。

测试里的 loopback endpoint 只替代客户自有网络服务；RunRuntime、ApplicationService、PostgreSQL、
SafeDialer、SSE、AG-UI decoder、journal 和 terminal writer 全是 production 实现，不是 fake runtime。

## 6. 台账变化

| 台账 | Batch 8 | Batch 9 |
| --- | ---: | ---: |
| events | 10 done / 67 todo / 77 | **12 / 65 / 77** |
| env | 49 / 25 / 74 | 不变 |
| tests | 184 / 863 / 1047 | 不变 |
| fixtures | 10 / 22 / 32 | 不变 |
| parity 总计 | 323 / 1337 / 1660 | **325 / 1335 / 1660** |

只将 `agui-lifecycle` 与 `agui-text` 改为 done。没有把下列“能解析但未形成最终产品契约”的族提前关闭：
reasoning（encrypted continuity 未闭合）、tool/result、state/activity/messages、raw/custom、step、
interrupt/resume 与 error UI/durable projection。

## 7. 明确未完成

- callback token、10 分钟 signed run assertion、tool-set grant 与 callback 验证；
- customer endpoint credential/auth header 及同 origin redirect 保留/跨 origin 剥离；
- remote deployment tools 和真实 callback→policy→audit→executor；
- `package_id IS NULL` 的用户创建 remote Agent 注册/更新/删除/连接测试；
- interrupt 的 durable awaiting-human/resume、reconnect/cancel/recovery；
- state/activity/messages/raw/custom/step/error 与 encrypted reasoning 的 durable/UI projection 和完整 official golden；
- 三家 recorded/live vendor trace、human approval GUI、完整 run-wide budget；
- RMCP 3.1.4、Drive、browser/file/shell 与 G5–G8。

因此只勾 §24.1 的固定 decoder 与 package-backed lifecycle/text 生产子项，**G4 整关保持未通过**。

## 8. 恢复点

- implementation commit：`d7e99c42fe7631963d283c52a9e81a213db7040b`；
- exact-schema tightening：`7bb240811cac06c0388e2b07fc70ad83c3895e0a`；
- 分支：`feat/2026-08-24-G4-remote-agui-protocol`；
- PR：[#26](https://github.com/acosmi/OpenBot/pull/26)；
- base：`feat/2026-08-24-G4-tool-loop-remember`（PR #25 head）；
- 创建后机器实得：`OPEN / CLEAN / MERGEABLE`，`statusCheckRollup=[]`；
- implementation head `7bb2408…` Actions run 数：**0**；
- 父 PR #25 同轮复核仍为 `OPEN / CLEAN / MERGEABLE`。

堆叠链尚未进入 `main`；合并必须继续按 `baseRefName` 依赖顺序使用 merge commit。
