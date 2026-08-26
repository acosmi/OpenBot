# Batch 26 WIP 恢复点：AgentPresence

> 日期：2026-08-25。分支 `codex/2026-08-25-G6-agent-presence`，base =
> Batch25 正式 head `ba57391080b48dafb5c254877af084e632c7c9f7`。只跑本地定向测试；
> 不运行 `cargo xtask ci`，不派发 Actions，不处理 `grok-bot`，不修改/暂存/
> 提交并行出现的 `docs/assets/`。

## 本批唯一范围

- T-UI-0035 `agents/orb/agent-orb.tsx → AgentPresence`；
- T-UI-0036 `agents/orb/ai-core.tsx → AgentPresence`；
- GUI 第一真源 §6.7：20px 状态环，idle/thinking/speaking/error 四态，
  thinking 1.2s/圈，speaking 两弧交替，error danger+单次160ms横移，reduce 下全静止且
  只靠可见形状/颜色与本地化 `aria-label` 表达。

## 构造性边界

- 两个上游文件合并到同一个组件，但 ledger 仍按上游文件各关一条；
- T-UI-0121 `motion` 总替代仍 todo；sign/transcript/app-sidebar/channel detail 等消费面未迁移；
- 不重建上游437行 canvas/shader/audio orb；第一真源已按原则7裁为20px状态环；
- `app-sidebar` 本批不勾：生产 sign-out 与 roster realtime 仍未闭合，不用只清界面的
  `/sign` 链接冒充 session revoke。

## 计划证据

- Rust 闭集状态/token drift/reduced-motion 单测；UI host + WASM + Clippy；
- production/design-gallery Trunk、i18n/design/css/bundle gates；
- 真实 Chromium 四态可见形状、本地化 AX name、动画时长与 reduce 终态；
- 只在实现与证据同时成立后勾两条 ledger。
