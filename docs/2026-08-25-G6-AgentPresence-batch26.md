# G6 Batch 26：AgentPresence 状态环

> 日期：2026-08-25。分支 `codex/2026-08-25-G6-agent-presence`，base =
> Batch25 正式 head `ba57391080b48dafb5c254877af084e632c7c9f7`，implementation =
> `5bb9d1ffe7ca8e0a023a5b07739584deee49cc26`。

## 1. 已完成并打勾

- [x] T-UI-0035 `agents/orb/agent-orb.tsx → AgentPresence`；
- [x] T-UI-0036 `agents/orb/ai-core.tsx → AgentPresence`。

两条 ledger 的单位是上游文件，目标同为一个 Rust 组件。UI
`77/75/152 → 79/73/152`；全 parity `513/1159/1672 → 515/1157/1672`。

## 2. 为什么不重写 orb

固定上游 `agent-orb.tsx` 437行、`ai-core.tsx` 395行，是 canvas/shader/音频反应系统。
GUI 第一真源 §6.7 已按原则7收为20px状态环；重建shader不是 parity，而是偏离裁决。

## 3. 实现

- `AgentPresenceState` 是 reactive `Signal` 闭集：`idle/thinking/speaking/error`；
- DOM 固定为20px root + track + primary/secondary arc，状态只经 `data-state` 切换；
- idle = 静止完整中性环；thinking = 单弧，1.2s/圈线性旋转；speaking = 双弧，
  1.2s `alternate/alternate-reverse`；error = danger 环 + 160ms×1 横向位移；
- 1200ms/160ms 进 `tokens.toml`，CSS 只消费生成的
  `--motion-agent-presence-cycle/error`；
- root 是 `role=img`，`aria-label` 只从 en/zh-CN 的 `agents.presence_*` 产生；
- 全局 `prefers-reduced-motion` 对 `*/*::before/*::after` 强制 `animation:none`；静态下
  thinking 仍是单弧、speaking 仍是双弧、error 仍有 danger+名称，不靠动画唯一传信。

## 4. 本机证据

| 面 | 结果 |
| --- | --- |
| UI tests | **57/0/0**；四态与1200/160ms生成token exact；reduced-motion CSS 反向断言 |
| host/WASM/Clippy/fmt | all-features host + wasm32、UI all-targets Clippy `-D warnings`、fmt 绿 |
| i18n/design/css | 361 leaf keys；52 Rust/74 icons；163 source classes |
| production bundle | wasm gzip **375421/3670016**；CSS **62691/98304**；fonts **740216/819200**；external/inline=1/0 |
| Chromium 1024×640 | 四态均20×20；thinking `spin/1.2s/infinite`；speaking双弧 `1.2s/infinite/alternate±`；error `160ms/1`；四个本地化 AX name 各1 |
| Browser DOM | h1/main/nav=1/1/2；duplicate/nested/overflow/remote/console=0/0/0/0/0 |
| parity/recount | **515/1157/1672**；UI **79/73/152**；0 violation/warning；**157/157/0** |

本机浏览器处于 `no-preference`；本批没有可用的 CDP media override。所以 reduced-motion 只声明
全局 CSS + Rust 单测的构造性证据，不写成“本机 reduce 浏览器已实跑”。

## 5. 仍未完成

- [ ] T-UI-0121 `motion` 总替代；sign/transcript/app-sidebar/channel detail 等消费面仍未迁移；
- [ ] `app-sidebar`：生产 sign-out/roster realtime 未闭，不用 `/sign` 纯界面跳转冒充 session revoke；
- [ ] 其余39业务、31routes、brand、6runtime、27golden、Tauri release/supply。

未运行 `cargo xtask ci`，未派发 Actions，未 push/建 PR，未处理 `grok-bot`。
并行出现的未跟踪 `docs/assets/` 未修改、未暂存、未提交。
