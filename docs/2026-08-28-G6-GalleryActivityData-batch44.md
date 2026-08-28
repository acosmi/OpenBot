# Batch 44：Gallery Activity + Data Functions

> 日期：2026-08-28。分支`codex/2026-08-28-G6-gallery-activity-data`；base `ba38563`；
> WIP `e79e540`；implementation `c742adbfb23d1bdf03b36ffb09ce9dac2d696e2b`；
> 固定上游`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批实现`showActivityReport`、build-owned `botActivity/recentRefusals`、`GET /api/components/functions`
与`POST /api/components/{name}/call`。未运行`cargo xtask ci`或Actions，未触碰`docs/assets/`。

## 第一真源裁决

- v3 §3.3要求data function每次调用重检component current state、Bot grant、actor ACL、policy与audit；
  `component_functions`有行不是单独充分条件；
- 两个函数都读deployment audit trail；本仓同一数据源的管理读面唯一ACL是admin，故普通用户必须拒绝，
  不能因component grant绕过；
- action policy沿用全局default-deny。权威context为`tool.name=component_data__<function>`、已验Bot/
  actor、空page、`Intent::ReadTool`；browser不能自报context；
- build manifest固定renderer→function映射：Activity恰声明`botActivity`或`recentRefusals`一项，当前其余
  renderer必须零function。任意把数据函数挂到Quote/Card/Chart的body在port前400；
- Activity故意没有Settings Gallery preview；index/detail只能显示“读取实时部署数据，无法预览”，绝不
  mount runtime renderer；
- read success与`component.function_called`同事务；bounded read失败先回滚savepoint，再写
  `component.function_failed`。audit失败不返回data/error假结果。

## 实施

- contracts新增Activity manifest/schema、两function registry、closed call/refusal/error与typed
  `BotActivityReport/RecentRefusalsReport`；untagged `data`保持上游body形状；
- Application将days按上游规则默认7、截整并clamp 1..90；limit默认10、clamp 1..50；只接受`days/limit`
  两个known key，unknown key在port前400；
- PostgreSQL serializable事务顺序为Agent→component/build→function build identity→admin ACL→hot compiled
  policy→component-function grant→bounded SQL→audit→commit；
- `botActivity`在数据库聚合并按actions desc/Bot id稳定排序，最多12行；`recentRefusals`最多50行，只投影
  fixed refusal event、stable error/rule code，不回传原policy/body；
- 读取失败使用savepoint区分“允许但失败”与“授权拒绝”；`function.failed`只含stable error code，零DB原文；
- Axum POST trusted Origin-before-body；Axum/Tauri/UI均typed/no-store，失败body以502保留
  `{allowed:true,error:"read_failed"}`；
- Leptos Activity runtime具有reading/refused/failed/empty/activity/refusals状态，bar只消费chart token；
  Settings preview专门走unpreviewable句子；production conversation尚未mount该runtime组件。

## 证据

| 面 | 本轮亲自运行结果 |
| --- | --- |
| contracts / domain | **4 / 0 / 0**；component **2 / 0 / 0** + audit allowlist **12 / 0 / 0** |
| application / Server / UI / Desktop | **5 / 0 / 0**；**3 / 0 / 0**；**2 / 0 / 0**；**1 / 0 / 0** |
| PostgreSQL 17.11 host SCRAM | runtime **1 / 0 / 0**；11-entry catalogue **1 / 0 / 0** |
| Clippy / WASM | 8 crate all-targets/all-features `-D warnings`；contracts/UI WASM绿 |
| tools / i18n / design / CSS | pins全绿；**514** leaf；**84 Rust / 74 icons**；**271** class literals |
| production bundle | WASM gzip **1,174,338 B**；CSS **93,646 B**；fonts **740,216 B**；external/inline **1/0** |
| parity / recount | API **58/109/167**；events **29/53/82**；components **2/20/22**；总计 **660/1018/1678**；strict **157/157/0** |

真库覆盖：public/private/admin/deleted/cross-tenant；component unknown/unpublished/withheld；build function
unknown、ordinary actor ACL、unconfigured policy、allow policy、function grant/missing；days 120.9→90、limit2；
两份真实report与两条called audit；append-only trigger继续拒UPDATE；插入hash外形合法但payload超界的tamper行后，
读取在savepoint失败、返回`read_failed`并写failed audit，300字节污染值零进入新audit；forced audit trigger时
零decision/data且零残留。Catalogue forced-audit 11 row0、成功added11/audit11、重复0。

Release浏览器：12个published tile=11 current+1 stale，unpublished future0；Activity tile1、figure0、专用
不可预览句子；stale仍用另一renderer-unavailable文案。Activity detail点击与硬刷新保持h1/不可预览，runtime
DOM0；main/h1/current各1、duplicate/nested interactive/overflow/console warning-error均0。

关闭`T-API-0041/0042`、`T-EVT-0025/0026/0028`与`T-CMP-0006`。`T-CMP-0001`继续todo：缺
production conversation tool registration/args projection、runtime mount与上游可选conversation follow-up ask action；
同理Cards/Charts/Quote虽已有decision API，也还未因本批勾整条。CSS预算只余 **4,658 B**，后续新增样式必须
优先复用或删冗余，不能靠调高96 KiB上限掩盖。
