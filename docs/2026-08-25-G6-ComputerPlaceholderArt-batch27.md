# G6 Batch 27：ComputerPlaceholderArt 共享中性线稿

> 日期：2026-08-25。分支 `codex/2026-08-25-G6-computer-placeholder-art`，base =
> Batch26 正式 head `3f97bc2e3c8894f0fe553d59ebafcf2a9e7caef0`，implementation =
> `aa2c0a480009c54fd08b9db7210f7e3e483e9a94`。

## 1. 已完成并打勾

- [x] T-UI-0052 `computer/placeholder.tsx → ComputerPlaceholder`；
- [x] T-UI-0065 `settings/background.tsx → ComputerPlaceholderArt`。

UI `79/73/152 → 81/71/152`；全 parity `515/1157/1672 → 517/1155/1672`。

## 2. 一份源码，两个业务入口

固定上游两文件都是162行同职责 SVG，包含彩色gradient、噪声filter、blend mode、
`<defs>` ID 与字面色。本仓不复制两份：

- `settings::ComputerPlaceholderArt` 持有唯一中性线稿 SVG；
- `computer::ComputerPlaceholder` 只负责3:2 frame，内部调用同一 Art；
- Art 保留 `viewBox="0 0 1200 800"`，改为 `xMidYMid meet`、`fill=none`、
  `stroke=currentColor`；表面只来自 `bg-subtle/border`，线条只来自 `fg-muted/secondary`；
- 零 gradient/radial/filter/noise/shadow/defs/DOM ID/style attr/remote href/src/字面 fill·stroke 色；
- SVG `aria-hidden=true` + `focusable=false`；wrapper 也是装饰，不伪造 status/live region。

## 3. 本机证据

| 面 | 结果 |
| --- | --- |
| UI tests | **59/0/0**；唯一SVG、wrapper零SVG、中性/无ID/无远程反向判据 |
| host/WASM/Clippy/fmt | all-features host + wasm32、UI all-targets Clippy `-D warnings`、fmt绿 |
| i18n/design/css | 362 leaf keys；56 Rust/74 icons；168 source classes |
| production bundle | wasm gzip **375910/3670016**；CSS **63205/98304**；fonts **740216/819200**；external/inline=1/0 |
| Chromium 1024×640 | 两SVG同为1200×800、`xMidYMid meet`、currentColor；实测容器均约324.5×215.7，3:2 |
| SVG/AX/DOM | defs/gradient/filter/style/id/remote/literal-color=0；AX img/focusable=0；duplicate/nested/overflow/console=0 |
| parity/recount | **517/1155/1672**；UI **81/71/152**；0 violation/warning；**157/157/0** |

## 4. 仍未完成

- [ ] `activity-log/command-output/computer-view/live-screen`、Screen/Computer runtime、screencast 与 G5 isolation；
- [ ] 其余37业务、31routes、brand、6runtime、27golden、Tauri release/supply。

未运行 `cargo xtask ci`，未派发 Actions，未 push/建 PR，未处理 `grok-bot`。
并行出现的未跟踪 `docs/assets/` 未修改、未暂存、未提交。
