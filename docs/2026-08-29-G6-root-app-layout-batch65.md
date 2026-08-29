# OpenBot G6 Root / App Layout Batch65

日期：2026-08-29（America/Los_Angeles）

分支：`feat/2026-08-29-G6-root-app-layout`

基线：Batch64 PR #47 已以 merge commit `d28865e385edcc775e0484d3a49c92c1759ae626` 合入
`main`。

implementation：`d614a6213755a47ce4d5399ce0ae4af1b2124447`

## 1. 结论

本批关闭两条 pathless layout：

- `T-ROUTE-0001` root layout；
- `T-ROUTE-0003` authenticated app shell layout。

关闭前，CSS 与可见 shell 大部存在，但台账 target 指向的 `openbot_ui::shell::layout::{root,app}`
并不存在：root/app 结构分散在 `app.rs::App`、`AppShell` 与 `AuthenticatedShell`，root 也没有显式
full-height wrapper。因此不能只凭历史截图把两条 ledger 改成 done。

本批新增具名 `RootLayout` / `AppLayout`，将原结构原样收敛到唯一 owner，并给 root 增加
`width:100%` + `min-height:100dvh`。没有改变路由 URL、Sidebar 业务、偏好存储或 tooltip 状态机。

## 2. 第一真源与固定上游

固定上游 `891df72f1827454d8b353d108fe5dd2313b7e30d` 本轮实读：

- `__root.tsx` = `min-h-dvh w-full` wrapper + ThemeProvider + TooltipProvider + Outlet；
- `_authed/_app.tsx` = one-viewport/overflow-hidden SidebarProvider + AppSidebar + inner main；
- 注释逐字要求外壳不滚，pane 在内部滚。

机制替代以 GUI 第一真源为准：

- 上游 localStorage 两态 ThemeProvider 已被 host 首帧 `<html class/lang>` 与 system/light/dark typed
  preference 替代；偏好读取继续只在 R138 auth guard 内；
- 上游 Base UI TooltipProvider 是库级共享机制，本仓每个 `Tooltip` 已拥有唯一 compound provider，
  delay 固定 400ms，focus/hover/Escape 已在 T-UI-0021 闭合；root 不造第二 tooltip 状态真源；
- App shell 使用 GUI §5.1 的 sidebar 240/48、topbar44、main pane 与可选二级侧栏，不复制上游340px。

## 3. 实现

`RootLayout` 位于 Router 内、AuthenticatedBoundary 外，因此 sign 与全部 authenticated route 都恰经一次
root wrapper。`AppLayout` 只在边界成功后构造，拥有：

- 边界内 `provide_ui_preferences`；
- 唯一 SidebarProvider 与 AppSidebar；
- 唯一 topbar；
- 唯一 `main#main-content`；
- AppRoutes child。

CSS 不变量：

- root width100% / min-height100dvh；
- app shell height100dvh / min-height0 / overflow hidden；
- app stage/main min-height0；
- main overflow auto，pane 自己拥有滚动。

新增 host 单测直接拆取三段 CSS 并断言上述值，避免只看 class 名。

## 4. Release 浏览器

最终 release/offline/locked WASM 使用不落盘 loopback host，仅提供闭集 auth/current-user/empty roster
响应；它只证明 layout，不冒充 production auth/channel backend。

未登录 `/channel/new→/sign`，1280×800：

- root/app/main/nav=`1/0/1/0`；
- root/body client+scroll height=`800/800`；
- x overflow=0；logs=0。

authenticated `/`，1280×800：

- root/app/main/nav=`1/1/1/1`；
- app/main/body height=`800/756/800`；
- body scroll height=800，x overflow=0；logs=0。

GUI 第一真源最小支持视口 1024×640：

- root/app/main/nav=`1/1/1/1`；
- app/main/body height=`640/596/640`；
- x overflow=0；duplicate IDs=0；nested interactive=0；logs=0。

1024×640 截图只作视觉 QA，不是 formal golden，也没有更新 golden baseline。

## 5. 机械证据

| 面 | 本轮结果 |
| --- | --- |
| UI | `156/0/0` |
| Clippy | openbot-ui all-targets/all-features `-D warnings` |
| WASM/fmt | wasm32 check、workspace fmt 通过 |
| i18n/design/CSS | `689` leaf keys；`98` Rust files/`74` icons；`335` class literals |
| bundle | wasm gzip `1,651,268/3,670,016`；CSS `109,435/131,072`；fonts `740,216/819,200`；scripts=`1/0` |
| routes | `19/13/32` |
| parity | `711/983/1694`；0违反 |
| overlay | carry/revalidate/split/superseded=`1568/124/2/0` |
| strict recount | fixed upstream `891df72f…`，`159/0/0`，skip0 |
| invariants | `grok-bot` tree=`86f5a85f…`；workspace单package/零npm lock |

## 6. 台账与未做

- `T-ROUTE-0001/0003`：todo→done；
- routes `17/15→19/13`；总 parity `709/985→711/983`；
- overlay `1570/122/2/0→1568/124/2/0`；
- 没有关闭 AppSidebar 总项、formal shell/sign golden、Tauri binary/window lifecycle；
- 没有运行全 workspace test、`cargo xtask ci` 或 GitHub Actions（R63 manual-only）；
- P1 Windows/runsc runtime仍红，未进入P2；
- `grok-bot/`零改动，没有新增Grok产品能力或复制其文本。
