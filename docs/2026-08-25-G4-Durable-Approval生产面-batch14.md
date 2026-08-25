# G4 Durable Human Approval 生产面（Batch 14）

日期：2026-08-25  
第一真源：后端方案 §5.2、§6.2 条 10、§7.2、§8.1–§8.6、§13.2–§13.3、§14.3、§15.3、§24 与 R77  
前端真源：GUI 方案 §3、§4.4–§4.5、§6.2、§9  
实施提交：`3ee87b04e3ca7beae80aaaa3a47743fb1b01e3b5`  
堆叠 PR：[#31](https://github.com/acosmi/OpenBot/pull/31)，base=`codex/2026-08-25-G4-drive-rest-runtime`

## 1. 本批结论

本批把 acting MCP 从 production 固定 deny 推进到真实 durable human approval：

```text
acting tool + CEL allow
→ PostgreSQL pending approval + requested audit
→ actor-only no-store projection
→ fresh same-origin grant / deny
→ cross-replica waiter wake
→ current authority/generation/policy/catalog recheck
→ tool_calls.approval_id + decision + attempt 同事务
→ single-use capability
→ vendor execute
→ outcome audit 携同一 approval_id
```

grant 路径真实调用 vendor 一次；deny 路径不创建 decision/attempt、vendor 调用数不增加。approval backend/API 子项据此打勾。

本批没有把纯 Rust view model 冒充 GUI：仓库仍没有 Leptos component tree，用户尚不能在产品 GUI 中点击批准。因此“approval GUI”、G4 整关与 G6 整关保持未勾。

## 2. 本批修正的结构性缺口

### 2.1 Binding 原先漏了 AuthGeneration

第一真源 §6.2 明定 role/access generation 改变后旧 approval 立即失效。原纯领域 binding 只比较 actor、Bot、run、tool、args、target、computer/catalog/document generation、policy 与 expiry，仍可能在 actor 保留另一角色时继续使用旧批准。

R77 给 `ApprovalBinding` / `ApprovalObservation` 增加 `AuthGeneration` 与独立 `auth_generation_changed` 原因；逐字段测试从 12 个失效原因增加到 13 个。

### 2.2 Renderer 不能自报 approval binding

decision HTTP body 闭集只有：

```json
{"decision":"grant"}
```

或 `deny`。actor、Bot、run、tool、args hash、target、effect、approval class、四类 generation、policy、expiry 都只来自 PostgreSQL/Rust authority。unknown 字段 400；fresh-session + trusted-Origin 在 body parse 前执行。

### 2.3 Approval 必须进入 durable decision/audit identity

只在 waiter 内存里拿到 `Granted` 不能回答“这次副作用用了哪份批准”。native 0020 因此同时：

- 新建 `tool_approvals`；
- 给 `tool_calls` 追加 nullable `approval_id` 与 partial index；
- requiring-human decision 缺 approval id 直接 fail-closed；
- journal transaction 重新要求 approval 仍 granted/未过期，且 actor/Bot/run/tool/args/target/effect/class/catalog/policy 全匹配；
- outcome/refusal audit 的 allowlist 新增 `approval_id`。

历史 call 不回填伪造 approval。

## 3. Native 0020 与秘密边界

`tool_approvals` 固定：

- state=`pending | granted | denied | expired | cancelled`；
- approval class=`once_per_run | every_call`；
- effect 只允许 acting 四类，不接受 read；
- generation 非负、args/policy version 为 lowercase SHA-256；
- pending 必须有 arguments summary；resolved 必须清空 arguments/change summary；
- grant/deny 的 `decided_by` 必须等于 acting actor；
- expiry、decision 与 created/updated 时间顺序由 CHECK 固定。

原始参数仍只在 private execution envelope。approval presentation 由 first-party resolver 产生：

- secret-shaped key/value 替换为 `[redacted]`；
- URL userinfo/password/query/fragment 清除或替换；
- string/depth/item/总 JSON 分别有界，总上限 16 KiB；
- presentation 只在 pending 期间存在；resolved transaction 同时清 NULL；
- table Row Debug、DTO Debug、view-model Debug 均不打印 summary。

Audit payload 不保存 summary、模型理由或 diff 原文，只保存 allowlisted identity。

## 4. 多副本与失效语义

- TTL=5 分钟，是本项目新增 runtime budget；DB clock `now >= expires_at` 即 expired；
- same-process 用 `Notify`，跨 replica 每 1 秒从 durable row poll；
- run terminal、membership/lease 失效、role/access/AuthGeneration 变化会把 pending CAS 为 cancelled；
- `once_per_run` 只有整份 binding 逐字段相等且仍未过期时复用；
- `every_call` 每个 Rust-minted call id 新建请求；
- concurrent decision 只有一个 pending CAS 获胜；commit unknown 要求客户端刷新 pending 列表，不自动重放 acting effect；
- grant 醒来后还要重新观察 current actor、catalog 与 policy；之后 journal transaction 再做最后一次 granted/expiry/binding 检查。

当前 production target 是 MCP/Drive，computer/document generation 为非 computer 的 0/None。browser 接入时必须从 engine authority 重新读取这两代际；本批没有用 request snapshot 假装 browser observation 已完成。

## 5. HTTP 与 UI 状态

新增 typed API：

- `GET /api/tool-approvals`：当前 actor、仍 pending、未过期、run/member/lease/AuthGeneration 当前，最多 100 条，`Cache-Control: no-store`；
- `POST /api/tool-approvals/{approval_id}`：fresh same-origin grant/deny，返回 committed receipt，同样 no-store。

DTO 展示真实 effect、target、redacted arguments、可选 first-party change summary、approval class 与 expiry；没有 model reason 字段。

`openbot-ui::features::approvals::ApprovalCardView` 只完成 authority-only pure projection，并复用现有 tool-name projection。它不是 Leptos component，不具备按钮、fetch/in-process action、focus trap、键盘、读屏、倒计时、reduced-motion 或 golden screenshot；这些仍属于 G6 后续批次。

## 6. 本机证据

没有运行 `cargo xtask ci`，没有派发 GitHub Actions。

| 定向证据 | 结果 |
| --- | ---: |
| `tool_approval_runtime` PG17/SCRAM | 1 / 0 / 0 |
| `native_0020` PG17/SCRAM | 2 / 0 / 0 |
| `mcp_protocol`（含真实 acting grant + deny） | 5 / 0 / 0 |
| Server approval framing | 1 / 0 / 0 |
| domain approval | 7 / 0 / 0 |
| domain audit payload | 11 / 0 / 0 |
| domain audit event | 3 / 0 / 0 |
| application tool pipeline | 9 / 0 / 0 |
| infra DB table/redaction | 22 / 0 / 0 |
| UI approval view-model | 1 / 0 / 0 |
| native 0018/0019 历史边界回归 | 4 / 0 / 0 |
| 七 crate all-targets/all-features Clippy `-D warnings` | 通过 |
| contracts/UI `wasm32-unknown-unknown` | 通过 |
| `cargo xtask parity-check --json` | 0 violations / 0 warnings |
| `cargo xtask recount --require-upstream` | 155 passed / 0 mismatch / 0 skipped |

PG approval runtime 逐项实得：actor A pending、actor B list=0/decision refused、grant wake、exact once-per-run reuse、every-call deny、inclusive expiry、AuthGeneration cancel、4 resolved rows summary 全 NULL、requested/granted/denied/expired/cancelled audit 各精确计数、audit payload argument canary=0。

Acting MCP 逐项实得：pending DTO effect/target/args 来自 catalog/resolver；grant 后 `tool_calls.approval_id == mcp.call_succeeded.payload.approval_id == pending.approval_id`，vendor call 增 1；下一 call deny 后 approval_denied refusal，tool call count不增，vendor call 不增。

schema 0020 fixture：42 tables / 398 columns / 291 NOT NULL / 217 constraints / 85 indexes / 4 triggers；SHA-256=`2ccf7193c936d140837dcc9d271e1520fd6924e902920ed97549b81f1a6f3ffe`。

## 7. 台账变化

- API：`35/122/157` → `37/122/159`（新增 2 条并完成）；
- events：`21/56/77` → `26/56/82`（新增五类 approval audit 并完成）；
- tables：`54/0/54` → `55/0/55`（新增 `tool_approvals`）；
- tests：仍为 `265/782/1047`；
- parity：`424/1237/1661` → `432/1237/1669`；
- fixtures：`13/22/35` → `14/22/36`；
- G2 专项队列仍为 `155/79/234`。

## 8. 明确未完成

- 可点击 Leptos approval component 与真实 GUI 用户旅程；
- approval critical realtime event/outbox、窗口级 filtering/lag replay；当前 GUI 只能未来通过 typed GET 拉取；
- browser 当前 computer/document generation 重新观察；
- run/user cancel 向 provider/RMCP notification 的完整传播；
- approval 的 keyboard/AX/reduced-motion/golden 与 Web/Desktop 宿主验收；
- browser/file/shell executor、G5–G8 其余面。

## 9. Git / PR 证据

- 实施提交：`3ee87b04e3ca7beae80aaaa3a47743fb1b01e3b5`；
- PR #31：OPEN / CLEAN / MERGEABLE；
- base 是 PR #30 的 head，不是 `main`；
- `statusCheckRollup=[]`，该 head 的 Actions run 列表为空；
- 合并必须继续按 `baseRefName` 依赖顺序使用 merge commit。
