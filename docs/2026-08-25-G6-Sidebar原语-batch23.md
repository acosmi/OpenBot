# G6 Batch 23：Sidebar 三形态与 mobile Sheet

> 日期：2026-08-25。分支 `codex/2026-08-25-G6-sidebar-primitive`，base =
> Batch22 正式 head `4edbf103a4b143b087d9f3bc47ee1a922b4483ed`，实现 checkpoint
> `d71347b37c6f581bbdf04e3329dbf670d844a45b`。本批关闭 Sidebar 一条 UI ledger，
> 至此 27 条 primitive 子账全 done；不代表业务组件/routes/golden 或 G6 整关完成。

## 1. 已完成并打勾

- [x] T-UI-0017 Sidebar。

UI `26/126/152 → 27/125/152`；全 parity
`462/1210/1672 → 463/1209/1672`。其余 ledger/fixtures 不变。

## 2. 第一真源三形态

唯一 `SidebarProvider` 持有 real viewport、user collapsed、mobile open、trigger ref 与 shortcut；
`Sidebar` 只选择 presentation，同一 `ChildrenFn` 在分支切换时只挂载一次：

- `lg ≥ 1024`：expanded=240px；用户 click/Ctrl/Command+B 后 rail=48px；
- `md 768–1023`：强制 rail=48px，trigger hidden+disabled，Ctrl/Command+B 不 prevent/no-op；回 lg
  恢复原 user preference；
- `<md`：desktop aside 卸载；同一 nav tree 进入既有 left Sheet/modal kernel。panel=240px；
  focus trap、Escape、body lock、path-sibling inert 全复用，不复制第二套安全规则。

`SidebarHeader/Content/Footer/Group/GroupLabel/NavList/NavLink` 是 closed compound；Footer 以 flex
自动固定底部。NavLink 只接 bounded same-origin absolute path；native nav 有名称，current link
显式 `aria-current=page`。Rail 隐藏 visible label/group label，但每个 link 保留 aria-label，current
仍有 check，颜色不是唯一信息。

## 3. shortcut、viewport 与返焦边界

- Document element ResizeObserver 读取 `window.innerWidth`，断点由 Rust closed enum 与第一真源
  1024/768 常量决定；进入非 compact 时强制关闭 mobile Sheet，但不改 collapsed preference；
- window KeyboardEvent listener 只在 key=B + Ctrl/Meta + no Alt 且当前不是 md 时 preventDefault；
  cleanup 精确移除 listener；
- SidebarTrigger 动态 controls desktop aside 或 mobile panel；large expanded/rail、compact open/closed
  同步 aria-expanded；md native disabled；
- external SidebarTrigger 不是 Sheet 内建 DialogTrigger。context 因此保存其 NodeRef：Sheet
  Escape、再次 shortcut close 与 open-resize-to-lg 都显式返焦该 trigger。没有这座桥会落 body；
- design gallery 在 700px 时仍被尚未迁移的旧 `app.rs` 硬编码 sidebar 覆盖 pointer 坐标；compact
  开合使用真实 Ctrl+B 验收，large trigger 已走真实 pointer。同一 toggle boundary 只有一份。

## 4. 本机证据

| 面 | 结果 |
| --- | --- |
| UI all-features tests | 51/0/0 |
| UI all-features WASM | 绿 |
| UI/testkit all-targets Clippy `-D warnings` | 绿 |
| i18n/design/css | 350 keys；45 Rust/74 icons；142 classes |
| production bundle | WASM gzip 371519；CSS 55073；fonts 740216；external/inline 1/0 |
| production/design gallery | `_design` bytes 0/1；gallery SHA-256 `6a7765adb2ab4aa7f3af4241b4bc54582a628c995a8b7748f47bb534dffff10d` |
| parity/recount | 463/1209/1672；0 violation/warning；157/157/0 |

真实 Chromium viewport override：

- lg/md `8/8`：1200px initial expanded/240；pointer→rail/48/count1，labels hidden、link names与
  current check保留；Ctrl+B→expanded/count2，Meta+B→rail/count3；900px auto rail/48、trigger
  hidden+disabled、shortcut count不变；回1200恢复 expanded；footer gap=12；
- compact `8/8`：700px closed 时 aside absent、controls mobile panel exact；Ctrl+B 打开 left
  Sheet，dialog/modal/nav/current exact，panel240、focus close、body overflow hidden、markers16；
  Escape 与 Ctrl+B close 均 markers0/overflow空/focus trigger；open→resize1200 同样全清并挂回
  expanded desktop aside；
- lg named nav/current AX；compact named dialog→nav/link AX；controls target missing/current count/
  duplicate ID/unnamed/nested/scope overflow=`0/1/0/0/0/0`，lg page overflow=0、console error=0。

Cargo.lock package delta=0；只扩已锁 web-sys EventTarget/KeyboardEvent API feature。UI dependency
guard 与六 target bans/sources 绿；MPL×5、runtime UNIC unmaintained×5 与 Cargo Vet macOS
270/Windows 269 仍红。

## 5. 仍未完成

- [ ] `shell/` 真实 AppSidebar 内容与旧 app.rs sidebar 替换；
- [ ] app-sidebar/channel、admin-sidebar、settings-sidebar 与用户 Menu/ThemeToggle；
- [ ] 45 业务组件、31 routes、110 Web + 两平台各54 golden、完整 AX/键盘 journey；
- [ ] Tauri release identity/binary/window lifecycle 与 MPL/UNIC/Vet。

未运行 `cargo xtask ci`，未派发 Actions，未 push/建 PR，未处理 `grok-bot`。
