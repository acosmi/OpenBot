# Batch 33 WIP：Composer Draft 与 Queue 纯状态

> 日期：2026-08-26。分支 `codex/2026-08-26-G6-composer-state`；base = Batch32正式head
> `afd44e5377d942d3954e9d7510a952cda4b5b75f`。固定上游
> `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 只跑本地定向测试；不运行`cargo xtask ci`，不派发Actions，不处理`grok-bot`，不修改/
> 暂存/提交`docs/assets/`。

## 本批范围

1. 逐条移植`app/src/components/channels/composer/draft.test.ts`固定上游10条：
   segment→plain draft、single Agent、command order、whitespace empty、prompt/action/chip重写；
2. 逐条移植`queue.test.ts`固定上游16条：idle direct、busy park、settle合并、command去重、remove与
   no-op identity；
3. Rust以`Cow`表达borrowed/owned transition，机械保留上游`toBe(same array/object)`无变化语义；
4. command action不把闭包藏进纯状态，改成closed/bounded effect identity，由未来Composer owner解释；
5. 文本trim复用contracts唯一ECMAScript TrimString；queued text按换行合并、command ID保持首次顺序；
6. 仅在26条定向测试、UI WASM/Clippy、ledger/parity/recount成立后关闭精确tests条目。

## 第一性原理边界

- queue严格是当前channel mount内存，不进PostgreSQL、不冒充durable outbox；reload/unmount丢失与固定上游一致；
- 本批没有production stop/cancel/steer API，`settle`只表示“外部已确认turn terminal”的事实；
- 不渲染Composer按钮/queue，不勾`T-UI-0043`、`T-UI-0123`或`T-ROUTE-0009`；
- Segment trigger先只允许第一真源`@`和`/`；Agent/command ID是内部catalog identity，不从HTML自由执行；
- action effect只是数据，副作用只能在owner提交新segments后按allowlist执行。

## 预期关闭

- `T-TEST-0130–0139`（draft 10）；
- `T-TEST-0140–0155`（queue 16）。
