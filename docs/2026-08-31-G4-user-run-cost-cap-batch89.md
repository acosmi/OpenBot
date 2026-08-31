# Batch89：G4 actor-scoped per-run cost cap

> 日期：2026-08-31（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G4-user-run-cost-cap`
>
> base：`1401b3ae6af841b500e1cdb01352d5487b517d9c`（Batch88 PR #71 merge commit）
>
> implementation：`29c4201b7908bf2d2e9268e0ea4abf223d255813`
>
> 第一真源：v4 §7.4、§8.4、§13.1–§13.3、§14.3、§15.1–§15.3、§17.3、§24 G3/G4/G6、§28.1 R161–R163。

## 1. 结论

Batch88 已按 run 持久化 operator-attested provider cost upper bound，但它只计数，不会因用户金额
上限停止运行。本批关闭该缺口：用户可在 Settings 配置 actor-scoped、currency-bound 的正数 per-run
cap；新 run 在 `BeginThreadRun` 同一 PostgreSQL 事务冻结当时的 cap，设置后改不影响既有 run。

cap 只消费 Batch88 的保守 cost upper bound。cap 存在但 provider/model 没有 rate snapshot，或 rate 与
cap 币种不相等时，Agent 在 `ProviderAdapter::start` 前分别以
`run_cost_budget_unpriced` / `run_cost_budget_currency_mismatch` 收口；已发生 sampling 后成本上界超过
cap，则先如实记账，再以 `run_cost_budget_exceeded` 收口。三者都写同一 closed
`agent.run_cost_budget_refused` hash-chain audit，payload 只含 stable error code。

本批不做价格抓取、汇率换算或 vendor bill；也不把并发 tool / computer runtime budget 冒充完成。
因此 G4 整关仍未通过。

## 2. 闭合边界

### 2.1 Closed contract 与 ApplicationService

- wire 只有 `{"cap":null}` 或 `{"cap":{"currency":"USD","maxCostMicroUnits":"250000"}}`；
- 金额用 canonical 十进制字符串传输，避免 WASM/JavaScript 对 PostgreSQL `bigint` 的 2^53/浮点舍入；
- currency 恰三位 uppercase；amount 为 `1..=i64::MAX` 的 whole micro currency units；
- actor/deployment/tenant/run/provider/model/rate 均没有输入字段；
- `RunCostBudgetAdministration` 以 verified `AuthContext` 三轴读写，`cap:null` 物理删除偏好；
- shared PostgreSQL ApplicationService assembly 同时供 Server 与 Desktop 使用；Desktop 不建第二份本地
  cost preference 真源。

### 2.2 native0026 与 run freeze

native0026 新建 `user_run_cost_budgets`，主键为
`(deployment_id, tenant_id, actor_user_id)`，actor FK cascade；`runs` 只追加
`budget_cost_currency` / `budget_max_cost_micro_units` 两列与一个 all-null/all-valid shape CHECK。

`BeginThreadRun` 在写 run 的同一事务读取当前 actor preference 并复制两列。已有 run 的幂等 replay 不
重读当前设置；并发设置更新与 run 创建线性化为旧值或新值之一，不会出现半形或运行中漂移。

schema0026 来自本轮独立 PostgreSQL 17.11 实例机械生成：46 表、455 列、326 NOT NULL、253 约束、
92 索引、4 触发器、4 enum、1 public function、0 extension；5,341 行，SHA-256=
`ad5375da9abc5d03f1fa9587f5efda3e76e2cb89edf470e3bc4650a58670ba2c`，native ledger 恰 14。

### 2.3 Agent budget 与 audit

`ProviderRequest` 从 production PostgreSQL context 取得 frozen `RunCostCap`。Agent 在每次 tool-loop
context reload 都核 cap/rate 不漂移；unpriced/mismatch 不触发 provider effect。`RunRuntime` 再从
locked run 读取 cap，调用方 cap 不等即 conflict；成本比较只在 aggregate 边界对
`ProviderCostUpperBound` 向上取整，`cost > cap` 才超限。token ceiling 与 cost cap 同轮同时超限时保留
既有 token budget 优先级，避免改变 Batch87 已有稳定错误语义。

quota audit 使用一个 event type 配三个 closed error code，不记录金额、币种、rate、provider/model 或
用户输入。audit commit 失败时转 `journal_commit_unknown`，不伪装成“已审计拒绝”。

### 2.4 Server / Desktop / Settings

- Server：authenticated/no-store `GET /api/me/run-cost-budget`；PUT 的 trusted Origin 在 JSON parse 前；
- Desktop：同一路径只映射 closed typed command/reply，authority 来自 host-bound window；
- UI：Settings 新增启用开关、currency 与十进制主金额（最多六位小数），纯整数转换，不使用 float；
- UI 文案明确这是保守上界而非实际账单；保存/加载/失败/关闭状态可见，中英键逐字对齐；
- release CSS 115,524 B，低于 128 KiB hard limit 与 120 KiB warning line。

## 3. 本轮亲跑证据

| 证据 | 最终结果 |
| --- | --- |
| Contracts / Domain / Application / Agent / Infra | `100/0/0`、`371/0/0`、`158/0/0`、`38/0/0`、`323/0/0` |
| Server / UI / Desktop all-feature | `218/0/0`、`180/0/0`、`130/0/3`；Desktop 三条 sandbox 假红在沙箱外逐条及整批复跑均绿 |
| PostgreSQL 17.11 | native0026 regeneration 开/关各 `1/0/0`；native0026整批 `2/0/0`；production context `1/0/0`；shared assembly `1/0/0`；既有 run runtime `5/0/0` |
| schema0026 | `46/455/326/253/92`；SHA `ad5375…ba2c`；ledger14；逐旧列证明0025子集 |
| Clippy | 最终 Domain/Application/Agent/Infra all-target/all-feature `-D warnings`；Server/Desktop/UI 同批全目标全feature绿，audit delta 后改动四crate再次全量绿 |
| release GUI | Trunk release/offline/locked 绿；WASM gzip `1,868,812/3,670,016`，CSS `115,524/131,072`，fonts `740,216/819,200`，external/inline script=`1/0`；CSS class `365/365` |
| i18n/design/tools | i18n `799` keys；design `104 Rust / 74 icons`；tools verify 为 Tailwind 4.3.3、Trunk 0.21.14、wasm-opt 132、wasm-bindgen 0.2.127 |
| parity | `823/881/1704`；API=`82/90/172`，events=`35/53/88`，tables=`59/0/59`，fixtures=`20/22/42`；overlay=`1293/403/2/6`；0 violation/warning |
| recount | 本仓 `71 passed / 0 mismatch`；未设 `OPENBOT_UPSTREAM_DIR`，89 skipped；strict 未跑 |
| Grok/npm/workflow | Git tree `86f5a85f…`、inventory check绿；非Grok `package.json`恰1；Cargo all-feature package=825；workflow manual-only，未派发 Actions |

没有运行 `cargo xtask ci`。

## 4. 首跑失败与修正

- 本轮多次因历史编译图把磁盘打满；每次均不计成功，清理对应 target 后定向重跑，最终退出前再次清理；
- PostgreSQL `initdb` 在沙箱内因 SysV shared memory EPERM 失败；沙箱外独立实例真跑后才生成 fixture；
- Trunk 首次被宿主 `NO_COLOR=1` 的非法布尔值挡在编译前；只对该命令覆盖 `NO_COLOR=true` 后重跑；
- Desktop all-feature 首跑三条 Keychain/child-process 用例受沙箱权限假红；沙箱外逐条与整批最终
  `130/0/3`；
- Clippy 指出 provider usage helper 8 参数，改为 typed `ProviderUsageRecord` 后重跑；
- 最终复核 §17.3 时发现只有 stable terminal 还不足以宣称 quota 子项完成，因此补入 closed
  hash-chain audit 与 T-EVT-0088，并重新跑 Agent/Domain/Infra、Clippy、parity/recount。

## 5. 未闭合边界

- 所有 production tool 当前仍 `parallel_safe=false`，没有可诚实验证的并发 tool 正向路径；
- browser/computer executor 尚未落，computer runtime budget 不能假完成；
- remote AG-UI 没有本地 contract rate，actor 开启 cap 时会在 remote effect 前明确 unpriced 拒绝；
- 三家 live/recorded vendor trace、完整 thread/cancel/computer 集成、正式 Desktop/Wry/golden 等 G4/G6
  余项保持原状态；
- 本批不关闭完整 budget 或 G4 整关，不修改 `grok-bot/`，不新增 Grok 产品能力、npm 或价格服务。
