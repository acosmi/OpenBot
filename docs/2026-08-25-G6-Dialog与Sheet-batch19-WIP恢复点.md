# Batch 19 WIP 恢复点：Dialog 与 Sheet

> 分支 `codex/2026-08-25-G6-dialog-sheet-primitives`，base = Batch18 正式 head
> `c05906a19be9bea2f58965df7c36bfb045e0c15d`；implementation checkpoint
> `79f696ecdcb1a0da5e30bb14b7ef5e1430c6e968`。正式证据文档为
> `2026-08-25-G6-Dialog与Sheet-batch19.md`。只跑本地定向测试，不运行
> `cargo xtask ci`，不派发 Actions，不处理 `grok-bot`。

## 本批范围

- [x] Dialog/Sheet 共用唯一 modal context，不复制 focus/security 规则；
- [x] button trigger，explicit aria-haspopup/expanded/controls，unit activation；
- [x] role=dialog + aria-modal + title/description exact IDs；
- [x] open 后首个 focusable，Tab/Shift+Tab 环，Escape/close/backdrop 关闭并归还 trigger 焦点；
- [x] body scroll lock；关闭/cleanup 恢复原值；
- [x] Dialog centered/max-height/body scroll/header+footer；
- [x] Sheet top/right/bottom/left 四 side，与 Dialog 同一焦点/关闭语义；
- [x] compile-gallery open state及真实 Chromium/AX 证据。

## 不在本批

Menu/Combobox/Select、MessageScroller/Sidebar；modal 出现不等于这些依赖项或任何业务表单 route
完成。test-only gallery 不冒充正式 golden。

## 当前机器证据

- UI all-features=`42/0/0`；WASM all-features、UI/testkit all-targets Clippy `-D warnings`、
  fmt 全绿；
- i18n=`306` keys、design=`39 Rust/74 icons`、css=`93 classes`；production bundle
  wasm gzip=`369319`、CSS=`44218`、fonts=`740216`、external/inline=`1/0`；
- Dialog：initial expanded=false/hidden；打开后 role=dialog、aria-modal=true、label/description IDs
  exact、active=`design-dialog-close`、body overflow=hidden；focusables=close/cancel/save；
  Shift+Tab(close)→save，Tab(save)→close；Escape/Cancel/真实 CUA backdrop 点击均关闭并把焦点
  还给 trigger，close count 依次递增；
- 打开期间 path-sibling inert+aria-hidden markers=16、sidebar inert=true、HEAD 不标记；关闭后
  marker=0/body overflow恢复；modal path 自身不 inert；
- Sheet right：presentation=sheet/side=right、width=360、panelHeight=viewportHeight=720；close/done
  focus 环与 Dialog 相同；Escape/Done 均 return focus，marker/scroll 全恢复。top/left/bottom
  closed enum 与 CSS selector 由 Rust/CSS gates 覆盖；
- 最终 gallery WASM `931d30…` console error=0；closed DOM duplicate ID/unnamed/nested/
  overflow=`0/0/0/0`；
- Cargo.lock package delta=0，只扩已锁 web-sys DOM feature。
