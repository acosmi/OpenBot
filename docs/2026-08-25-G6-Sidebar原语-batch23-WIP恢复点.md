# Batch 23 WIP 恢复点：Sidebar 三形态原语

> 分支 `codex/2026-08-25-G6-sidebar-primitive`，base = Batch22 正式 head
> `4edbf103a4b143b087d9f3bc47ee1a922b4483ed`；implementation checkpoint
> `d71347b37c6f581bbdf04e3329dbf670d844a45b`。正式证据文档为
> `2026-08-25-G6-Sidebar原语-batch23.md`。只跑本地定向测试，不运行
> `cargo xtask ci`，不派发 Actions，不处理 `grok-bot`。

## 本批范围

- [x] SidebarProvider 唯一 viewport/collapse/mobile/shortcut context；
- [x] lg≥1024：240px expanded / 48px user-collapsed rail；
- [x] md 768–1023：自动 48px rail，不把用户偏好永久改写；
- [x] `<md`：desktop aside 卸载，复用既有 Sheet modal kernel 显示同一 nav children；
- [x] Ctrl/⌘+B 在 lg toggle collapsed、compact toggle Sheet；md 不偷系统快捷键；
- [x] trigger expanded/controls/disabled exact；nav landmark 命名，link aria-current=page；
- [x] header/content/footer/group/list/link compound；Footer 固定底部；link 只接 bounded same-origin path；
- [x] rail 隐藏可见 label 但保留 aria-label/current，selected 仍有 check；
- [x] design-gallery + Chromium/AX/三 viewport/shortcut/focus/inert/scroll 证据。

## 不在本批

真实 AppSidebar 内容、用户 Menu/ThemeToggle 接线、31 routes/current URL resolver、admin/settings
二级侧栏、topbar/detail panel 与正式 golden；Sidebar 原语完成不冒充这些业务组件或 G6 整关完成。

## 当前机器证据

- UI all-features=`51/0/0`；WASM all-features、UI/testkit all-targets Clippy `-D warnings`、
  fmt 全绿；
- i18n=`350` keys、design=`45 Rust/74 icons`、css=`142 classes`；production bundle
  wasm gzip=`371519`、CSS=`55073`、fonts=`740216`、external/inline=`1/0`；production/gallery
  WASM 的 `_design` 字节=`0/1`；
- Chromium 当前 bundle：lg/md=`8/8`、compact Sheet=`8/8`，另有 Meta+B 实测。1200px initial
  expanded/width=`expanded/240`；pointer toggle→rail/48/count1，label/group display none但三 link
  aria-label保留、current check可见；Ctrl+B→expanded/count2，Meta+B→rail/count3；
- 900px 自动 rail/48、trigger hidden+disabled、Ctrl+B count不变；回1200恢复 expanded，证明 md
  没改 user collapsed preference；
- 700px desktop aside 卸载，trigger controls mobile panel exact；Ctrl+B 打开 left Sheet，panel
  width=240、dialog/modal/nav/current exact、focus close、body overflow hidden、inert markers=16；
  Escape 与再次 Ctrl+B 都 close→markers0/overflow空/focus trigger；open 时 resize→1200 同样全清；
- lg named nav/aria-current、compact named dialog→nav AX exact；controls target missing/current count/
  duplicate ID/unnamed/nested/scope overflow=`0/1/0/0/0/0`，lg page overflow=0、console error=0；
- 当前 app.rs 的旧硬编码 sidebar 尚未替换，在700px会覆盖 gallery trigger坐标；因此 compact
  打开用全局真实 shortcut验收，trigger click已在lg走真实pointer，同一 toggle boundary不另写；
- 最终 gallery WASM SHA-256
  `6a7765adb2ab4aa7f3af4241b4bc54582a628c995a8b7748f47bb534dffff10d`；Cargo.lock
  package delta=0，只扩已锁 web-sys EventTarget/KeyboardEvent feature。
