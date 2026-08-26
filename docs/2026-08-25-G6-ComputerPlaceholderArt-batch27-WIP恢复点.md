# Batch 27 WIP 恢复点：ComputerPlaceholderArt

> 日期：2026-08-25。分支 `codex/2026-08-25-G6-computer-placeholder-art`，base =
> Batch26 正式 head `3f97bc2e3c8894f0fe553d59ebafcf2a9e7caef0`。只跑本地定向测试；
> 不运行 `cargo xtask ci`，不派发 Actions，不处理 `grok-bot`，不修改/暂存/
> 提交并行出现的 `docs/assets/`。

## 本批唯一范围

- T-UI-0052 `computer/placeholder.tsx → features::computer::placeholder`；
- T-UI-0065 `settings/background.tsx → ComputerPlaceholderArt`；
- 固定上游两文件是逐字同职责的162行彩色渐变+噪声/filter SVG；本仓按
  GUI 第一真源 §6.2/原则1/3/7收为一份中性线稿，ComputerPlaceholder 只复用该 art。

## 构造性边界

- SVG 只用 `currentColor`/token surface，不用字面色、gradient、filter、noise、shadow、remote asset、
  `<defs>` ID；多实例不会重复 DOM ID；
- 装饰图必须 `aria-hidden=true` + `focusable=false`，不制造假 status/live region；
- 不勾 activity-log/command-output/computer-view/live-screen，不冒充 Screen/Computer runtime、
  screencast 或 G5 isolation 完成；
- 不把本批的内联装饰 SVG 混进 Lucide icon allowlist。

## 计划证据

- Rust 单测锁 viewBox/preserveAspect、零字面色/gradient/filter/ID/remote marker；
- UI host/WASM/Clippy，production/design-gallery Trunk、i18n/design/css/bundle gates；
- Chromium 真实 SVG 装饰 AX 隐藏、currentColor、两 wrapper 复用、重复ID/远程资产/溢出零。
