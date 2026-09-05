# 外部任务 K：Composer 候选纯逻辑（无需联网调研）

先读同目录 `2026-09-04-v4第二轮外派任务-总则.md` 和所列第一真源。
固定基线 `87d84bb85d0056dfa4dcc2b35be4c2a610a55ae3`；分支 `feat/2026-09-04-G6-composer-trigger-core`。
这是 T-UI-0043/T-UI-0123 的一个明确前置子项，不是完整 Composer、真实 slash skill 执行或 channel route。

## 离线输入

- 固定上游已在本机 `/private/tmp/openbot-v4-upstream-891df72`，HEAD 必须是 `891df72f1827454d8b353d108fe5dd2313b7e30d`。
- 只读 `app/src/components/channels/composer/{draft.ts,triggers.ts,sources.ts,composer.tsx}` 与相邻测试；不运行上游 TS/Node/npm。
- 现有 `crates/openbot-ui/src/features/channels/composer/{draft.rs,queue.rs}`、`recipient_field.rs`，GUI 第一真源 §6.5。
- 用本机现有 Cargo 缓存，测试加 `--offline`；没有外网、凭据、数据库或 Chromium 运行需求。

上游 `sources.ts` 明确只有一个 placeholder summarize prompt，不是已经存在的真实技能目录。
此事实必须保留：可读取已有合法候选的纯接口，不能宣称凭固定上游获得了生产 Skills 授权或填假 catalog。
`prompt-area/helpers` 不在本地源文件中的行为不能猜成 upstream parity；新增的解析/失效规则须明确标“新增”。

## 允许修改

- 新增 `crates/openbot-ui/src/features/channels/composer/sources.rs`、`triggers.rs` 及其模块内测试。
- `composer/mod.rs` 仅新增模块注册；`draft.rs` 仅为复用现有 typed Segment/CommandOption 做必要可见性调整，不改原语义。
- 新增 `fixtures/ui/composer-triggers.json` 与同名 provenance。
- 新增 `docs/2026-09-04-Composer候选纯逻辑-外部交付.md`。

不改 conversation、Markdown、queue、API/Server、Skills 页面、locale、Cargo、中央台账或既有完整 Composer。
无新增依赖，不通过复制上游 package.json、运行 npm 或在线搜 helper 实现扩大本任务。

## 验收

1. roster→AgentOption 保持固定上游过滤顺序、name/title 映射；permitted IDs 的未提供、空集合、非空三态正确，未知候选不会出现；空 roster 可用。
2. name/description 的不区分大小写查询与空查询逐项对照本地源码；Unicode/ECMAScript 差异必须有明确契约与测试，不能把 Rust ASCII 规则冒充 JS。
3. `@` 与 `/` 的候选状态和 selection 是 pure typed value；slash 只在已裁决行首触发，URL/日期中间的 `/` 不触发。输入边界、换行、UTF-8/UTF-16 cursor 转换有明确单位，越界稳定拒绝。
4. 选择只接受当前 snapshot 内的结构化 ID，label 不能变 authority；roster/permitted/channel 或输入代次变化使旧 selection ticket 失效，不能复用旧 index 命中另一候选。
5. 输出复用 `draft::Segment`、`CommandOption`，agent 单选与 prompt/action/chip 的既有 semantics 不被重新解释；未知 action 不执行。纯模块不做网络、文件、secret、数据库或真实 Agent 调用。
6. 输入法 composing 状态不触发发送/确认，Escape 关闭、取消不修改草稿；现有 Combobox APG 的 DOM 接入由后续完整 Composer 实现，本任务不要声称已经完成浏览器键盘旅程。
7. fixture 区分固定上游可观察规则与本项目新规则，单测从同一 fixture 消费；测试含正常、边界、陈旧和跨候选集反例，保留 draft/queue 原测试绿。
8. `cargo test -p openbot-ui --locked --offline composer`、UI Clippy `--all-targets --all-features --locked --offline -- -D warnings`、WASM check（target 已安装时）、fmt 与 diff-check；缺缓存/target 如实报告，不联网下载。

交付一个候选 commit、规则映射与完整机械证据。这个离线任务产出可接入的候选状态机；不以 pure tests 关闭整个 T-UI-0043/0123 或任何产品闸门。
