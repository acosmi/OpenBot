# Batch 44 WIP：Gallery Activity + Data Functions

> 日期：2026-08-28。分支`codex/2026-08-28-G6-gallery-activity-data`；base为Batch43正式head
> `ba38563`。固定上游`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 本机定向测试；不运行`cargo xtask ci`，不派发Actions，不触碰`docs/assets/`。

## 已核实的第一真源

- v3 §3.3固定：data function每次调用重新检查component current state、Bot grant、actor ACL、policy
  与audit；任一缺失都不能执行读取；
- 固定上游Activity恰一个`showActivityReport`，report闭集`activity/refusals`；它故意没有
  Gallery preview，避免Settings/Admin mount触发一次真实部署读取；
- build-owned data functions恰`botActivity/recentRefusals`，前者days默认7且1..90、最多12个Bot，
  后者limit默认10且1..50；数据来自audit trail，模型只拿confirmation，不拿结果数据；
- 本项目现有`list_audit_events`唯一actor ACL为admin。Activity读取同一数据源，不能因component
  function grant绕过这条ACL；
- action policy未配置时default-deny；component data read使用权威actor/Bot、空page与
  `Intent::ReadTool`构造policy context，browser/body不得自报context。

## 本批边界

1. contracts：function summary/call/refusal/result与两类typed report；Activity manifest/schema；
2. application/domain：build registry、参数bounds、admin ACL与当前action policy评估；decision和call都复核；
3. infra：serializable transaction复核Agent/component/function grant后执行两条bounded SQL；called/refused/
   failed hash-chain audit，audit失败不返回数据；
4. Axum/Tauri/UI：`GET /api/components/functions`与`POST /api/components/{name}/call` typed/no-store，
   POST trusted Origin-before-body；
5. Leptos：Activity runtime renderer + loading/refused/error/empty/data状态；Settings Gallery对Activity只显示
   “不可预览”，绝不发起data call；
6. 本批可关闭T-API-0041/0042；production conversation registration/tool projection未闭前，
   `T-CMP-0001`继续todo，不用Gallery detail冒充运行态。
