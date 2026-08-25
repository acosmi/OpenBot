# G6 Batch 19：Dialog 与 Sheet 共享模态内核

> 日期：2026-08-25。分支 `codex/2026-08-25-G6-dialog-sheet-primitives`，base =
> Batch18 `c05906a19be9bea2f58965df7c36bfb045e0c15d`，实现 checkpoint
> `79f696ecdcb1a0da5e30bb14b7ef5e1430c6e968`。本批关闭 Dialog/Sheet 两条 UI
> ledger；不代表任何业务表单 route 完成。

## 1. 已完成并打勾

- [x] T-UI-0003 Dialog；
- [x] T-UI-0016 Sheet。

UI `20/132/152 → 22/130/152`；全 parity
`456/1216/1672 → 458/1214/1672`。其余 ledger/fixtures 不变。

## 2. 共享模态内核

Dialog 与 Sheet 只有 presentation/side 不同，以下代码只有一份：

- trigger：button + unit activation，`aria-haspopup=dialog`、explicit expanded、controls；
- panel：`role=dialog`、`aria-modal=true`、title/description ID 由 root ID 单点派生；
- lifecycle：open 首焦点、body overflow lock、background inert；close/cleanup 恢复并 return focus；
- keyboard：Escape；Tab/Shift+Tab 首末环；无 focusable 时 panel 自身兜底；
- close：内置 close、compound close、backdrop、Escape 均走同一 idempotent close；
- body/footer：body 独立滚动，header/footer 固定。

SheetSide 保留上游 top/right/bottom/left 四值；默认 right。没有自由 side string。

## 3. 背景隔离为什么不用“只写 aria-modal”

单写 `aria-modal=true` 不保证本项目 AX 检查与所有系统 WebView 都把背景从读屏/焦点面排除。
modal layer 又位于页面组件树内部，直接 inert `.ob-app-shell` 会把 modal 自己一起禁用。

实现从当前 modal layer 沿 DOM 祖先路径上行：每一级只把“路径之外的 sibling”标成
`inert + aria-hidden + data-openbot-modal-inert=<modal id>`；modal 自身及祖先路径保留。关闭时
只按该 marker 恢复，HEAD 不标记。这样不需要第二套 portal DOM，也不会把 modal 放进 inert
祖先。当前产品不允许同时打开两个 modal；若未来允许 nesting，必须把 marker 改为栈/引用计数。

## 4. 本机证据

| 面 | 结果 |
| --- | --- |
| UI all-features tests | 42/0/0 |
| UI all-features WASM | 绿 |
| UI/testkit all-targets Clippy `-D warnings` | 绿 |
| i18n/design/css | 306 keys；39 Rust/74 icons；93 classes |
| production bundle | WASM gzip 369319；CSS 44218；fonts 740216；external/inline 1/0 |
| parity/recount | 458/1214/1672；0 violation/warning；157/157/0 |

真实 Chromium：

- Dialog initial expanded=false/hidden；open 后 active=`design-dialog-close`，body overflow=hidden，
  role/modal/label/description exact；focusables=`close,cancel,save`；
- Shift+Tab(close)→save，Tab(save)→close；Escape、Cancel、真实坐标 CUA backdrop 点击均关闭；
  焦点回 `design-dialog-trigger`，close count 递增；
- open path-sibling marker=16、sidebar inert=true、HEAD marker=false；close 后 marker=0、body
  overflow 恢复空；
- Sheet right presentation/side exact，width=360、height=720=viewport；close/done focus 环与
  Dialog 相同；Escape/Done 均 return focus，marker/scroll 恢复；
- closed DOM duplicate ID/unnamed/nested interactive/overflow=`0/0/0/0`；final gallery WASM
  `931d30…` console error=0。

Cargo.lock package delta=0，只扩已锁 web-sys DOM feature。Batch16 supply-chain 红灯不变。

## 5. 仍未完成

- [ ] MessageScroller；
- [ ] Combobox/Menu/Select；
- [ ] Sidebar（含 md rail 与 `<md` Sheet 集成）；
- [ ] 45 业务组件、31 routes、正式 golden/AX 全矩阵；
- [ ] Tauri release identity/binary/window lifecycle 与 MPL/UNIC/Vet。

未运行 `cargo xtask ci`，未派发 Actions，未 push/建 PR，未处理 `grok-bot`。
