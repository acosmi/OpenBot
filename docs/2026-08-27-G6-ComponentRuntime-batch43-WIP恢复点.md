# Batch 43 WIP：Compiled Component Runtime Authorization

> 日期：2026-08-27。分支`codex/2026-08-27-G6-component-runtime`；base为Batch42证据head
> `19ff9a8`。固定上游`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 本机定向测试；不运行`cargo xtask ci`，不派发Actions，不触碰`docs/assets/`。

> 已完成：implementation `15fdb401851f0fca666399e25270fc98cdd4a381`；正式证据见
> `docs/2026-08-27-G6-ComponentRuntime-batch43.md`与R106。本文件保留为恢复点历史。

## 已亲自复核的第一真源

- v3 §3.3：compiled gallery必须同时保留参数schema、published、per-Bot withholding、
  data-function grant与tool-call-time再授权；Gallery预览存在不等于生产链完成；
- `parity/components.yaml`：`GET /api/components/for-agent/:agentId`只返回该Bot实际持有的
  `{name,description}`；`POST /:name/decision`必须先验证当前actor可使用该Bot，再检查发布状态、
  withholding和本次声明的data functions，失败关闭并记录拒绝；
- 固定上游`server/src/components/store.ts`：组件默认开放，`component_exclusions`只表达例外；
  unpublished、无published description、被该Bot withheld三者都不可注册；
- 固定上游`app/src/lib/copilot/gallery-tools.tsx`：初始工具列表只是快照，每次工具调用仍须重新
  decision；拒绝必须成为模型与人的同一事实，不能继续渲染旧授权；
- `CLAUDE.md`：Rust是actor/target/policy/audit唯一铸造者；ApplicationService是唯一业务入口；
  未获授权不运行`cargo xtask ci`或Actions。

## 本批边界

1. contracts/domain：closed request/reply、stable refusal code与纯component decision；wire不接受actor、
   tenant、role或自由description；
2. application/infra：从已验`AuthContext`校验Bot可用性；PostgreSQL用同一权威查询实现
   for-agent与call-time decision；拒绝审计与判定同一事务，审计失败则整个决定失败关闭；
3. server/desktop transport：typed GET/POST、trusted Origin、bounded body与`Cache-Control: no-store`；
4. UI transport：fail-closed helper与可轮询的stable grant snapshot；生产conversation注册存在前不把
   Gallery preview冒充runtime；
5. 真实PostgreSQL/Axum/Application/Contract定向证据覆盖owner/other/admin、cross-tenant、unpublished、
   null description、withheld、function missing、audit rollback与撤权后旧snapshot再调用；
6. Decisions、Activity data functions、admin mutation、sandbox、formal golden若无法与上述边界同批完整闭合，
   分到后续独立批次，不提前修改对应parity为done。
