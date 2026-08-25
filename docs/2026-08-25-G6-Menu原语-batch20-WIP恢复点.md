# Batch 20 WIP 恢复点：Menu 原语

> 分支 `codex/2026-08-25-G6-menu-primitive`，base = Batch19 正式 head
> `b912c286c42c23e2c8d43a2c8d4a1ab0740bc807`。只跑本地定向测试，不运行
> `cargo xtask ci`，不派发 Actions，不处理 `grok-bot`。

## 本批范围

- [x] Menu root/trigger/content/item/separator/submenu compound；
- [x] trigger click/Enter/Space/ArrowDown/ArrowUp；explicit haspopup/expanded/controls；
- [x] menu ↑↓ wrap、Home/End、disabled skip、500ms multi-character typeahead；
- [x] item Enter/Space/click exactly once，选择关闭 root 并归还 trigger focus；
- [x] Escape 关闭当前层；root Escape return root trigger；submenu `→` open、`←` close并回 parent；
- [x] Tab 关闭且不劫持已有正常焦点移动；outside dismiss layer；
- [x] role=menu/menuitem、tabindex=-1、aria-disabled/haspopup/expanded exact；
- [x] design-gallery + Chromium/AX 全键位证据。

## 不在本批

Combobox/Select、MessageScroller/Sidebar；Menu 完成不冒充三者或任何业务导航/route 完成。

## 当前机器证据

- UI all-features=`44/0/0`；WASM all-features、UI/testkit all-targets Clippy `-D warnings`、
  fmt 全绿；
- i18n=`315` keys、design=`40 Rust/74 icons`、css=`101 classes`；production bundle
  wasm gzip=`369935`、CSS=`46119`、fonts=`740216`、external/inline=`1/0`；production/gallery
  WASM 的 `_design` 字节=`0/1`；
- trigger Enter/Space/ArrowDown→首项，ArrowUp→末项；根菜单 ↑↓ 双向 wrap、Home/End、disabled
  skip 全部精确；英文 `m`+`o` 命中 More，500ms 后 `s` 命中 Settings；
- submenu Right→Copy，子层 Down/Up 不冒泡到根；Left/Escape 只关当前层并回 More；父项
  Escape 的冒泡回归实得 root 仍开、sub 关闭、close count=0；下一次 Escape 才关闭 root；
- item Enter/Space/click 与 child Enter 各自从新页面实得 select/close=`1/1`，焦点回 root
  trigger；disabled 同时为 native disabled + `aria-disabled=true`；真实 CUA 外部坐标点击关闭；
- Tab/Shift+Tab 先保留浏览器原生移动；仅当引擎仍把焦点留在即将隐藏的 menu/body 时做方向性
  恢复。本机真实 CUA 实得 Tab→`design-menu-after`，Shift+Tab→root trigger，根/子层均只关闭
  一次且焦点不落 body；
- 最终 Chromium 断言集=`25/25`；AX 树为命名 root/sub `menu`、命名 `menuitem`、disabled 与
  separator exact；duplicate ID/unnamed/nested interactive/remote resource/overflow=
  `0/0/0/0/0`，console error=0；
- 最终 gallery WASM SHA-256
  `42183490ba04c802f6006fac00779bd65e4fded0bc9b479bb9039ab6df0daf8f`；Cargo.lock
  package delta=0。
