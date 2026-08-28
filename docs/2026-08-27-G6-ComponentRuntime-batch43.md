# Batch 43：Compiled Component Runtime Authorization

> 日期：2026-08-27。分支`codex/2026-08-27-G6-component-runtime`；base `19ff9a8`；
> WIP `f13bcff`；implementation `15fdb401851f0fca666399e25270fc98cdd4a381`；
> 固定上游`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批只闭合compiled component运行时授权的两条API：Agent实际持有列表与每次调用再授权。Cards/
Charts/Quote的生产conversation renderer尚未接线，Activity data-function执行、Decisions HITL、admin mutation、
sandbox与formal golden均不在本批冒充完成。未运行`cargo xtask ci`或Actions，未触碰`docs/assets/`。

## 第一真源裁决

- 初始工具列表只是快照；组件每次调用都必须重新检查当前build renderer、published description、
  per-Agent withholding及本次声明的全部data-function grant；
- actor/tenant/admin只来自`AuthContext`。请求只能提交Agent id、组件path name与function names，不能自报
  actor、role、tenant、description或renderer身份；
- Agent授权复用`openbot_domain::agent::profile_policy::can_run_agent`，package tenant仍是最外层scope；
- domain/application不携用户文案。拒绝通过closed `ComponentDecisionRefusal` code到GUI，再由同一code生成
  人与模型一致的文字；
- 拒绝会写审计，因此decision与hash-chain audit同一事务。审计失败不能返回一个没有审计事实的拒绝；
- 当前build manifest是可执行renderer全集。数据库里残留的published旧row不能被运行时重新广告。

## 实施

- contracts新增`GrantedCompiledComponents`、`ComponentDecisionRequest/Decision/Refusal`与两条封闭
  `AppCommand/AppReply`；wire `deny_unknown_fields`，allowed/refusal交叉一致性由Application与UI双检；
- domain新增纯`decide_component_grant`与`decide_component_function_grant`；unknown → unpublished →
  withheld顺序固定，function grant独立且缺行即拒；
- Application将function集合按既有128-byte component name语法校验、排序去重；Agent id按audit可表达的
  256-byte无控制字符边界校验；manifest names由Server build生成，不接受browser列表；
- PostgreSQL list在repeatable-read snapshot内复核Agent后，仅投影当前manifest中published、description
  non-null且无该Agent exclusion的name/description；
- PostgreSQL decision在serializable事务内复核Agent与组件，再逐项检查function grants；拒绝写
  `component.refused`或`component.function_refused`，payload只含权威`bot`、stable `error_code`及可选
  `function`，不存自由reason；
- Axum GET/POST、Tauri custom protocol与Leptos WASM helper全部typed/no-store；Axum POST使用trusted
  Origin且在body解析前拒绝；
- Agent gateway与Channel HTTP的封闭reply错配表显式加入两种新reply，没有用通配分支掩盖协议变化。

## 证据

| 面 | 本轮亲自运行结果 |
| --- | --- |
| contracts | component定向 **3 / 0 / 0** |
| domain | component **2 / 0 / 0**；audit payload allowlist **12 / 0 / 0** |
| application / Server / UI / Desktop | **4 / 0 / 0**；**2 / 0 / 0**；**1 / 0 / 0**；**1 / 0 / 0** |
| PostgreSQL 17.11 host SCRAM | component runtime **1 / 0 / 0**；仅监听`127.0.0.1:55443`，测后停止并删除 |
| Clippy / WASM | 8 crate all-targets/all-features `-D warnings`；contracts/UI WASM绿 |
| i18n / design / CSS | **498** leaf；**83 Rust / 74 icons**；**265** source class literals |
| dependency/lock | UI/Tauri/release-target guards通过；Cargo/package delta **0**；Tauri guard仍如实报告既有MPL/UNIC/Vet blockers |
| parity / recount | API **56/111/167**；总计 **654/1024/1678**；strict **157/157/0**；0 violation |

真库矩阵覆盖：public/owner/admin、private-other、deleted、package cross-tenant；unpublished、null published
description、per-Agent exclusion、stale build renderer、function granted/missing；撤权前grant snapshot包含
`showQuote`，插入exclusion后旧调用在decision被拒；4条成功拒绝审计逐条无`reason`；trigger强制拒绝
审计失败时零decision返回且审计总数不变。

本批没有视觉/生产renderer变化，因此没有重建bundle、没有跑浏览器或formal golden；继续沿用Batch42
`dist`不构成本批bundle证据。`T-API-0039/0040`转done；`T-CMP-0001/0002/0003/0004/0006/0007/0008`
仍todo。下一条完整纵切应接Activity的build-owned data-function registry + `/call`，或同批接
conversation component tool projection；只做更多Gallery preview不会推进运行时闭环。
