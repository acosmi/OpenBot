# OpenBot GUI 设计系统与视觉规格（v2）

> **定位**：第一方 GUI（Server Web 与 Desktop 共用的同一份 Leptos bundle）的视觉、布局、主题、国际化、无障碍与视觉闸门的**唯一真源**。`docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md`（v3）只定义旅程、能力与架构；两者冲突时，视觉/交互以本文件为准、架构/能力以 v3 为准，并同 PR 修订另一方。
> **日期**：2026-08-22。**上游基线**：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`（与 v3 §1.2 相同，本文件所有"上游"数字都在该 commit 的干净克隆上复算，命令见 §17）。
> **v2 就地修订**：2026-08-28，与后端方案 v4（§28.1 R115–R125）同 PR，清单见 §14 条 9：D2 措辞精确化（零 npm 与 engine shim manifest 唯一例外）、Desktop sandboxed component 的具名 a11y fallback（§9.1）、CSS 预算 96 → 128 KiB 与 120 KiB 警戒（§10.5，R123）、品牌概念稿登记（§2）。范围与非目标不变：v4 不新增任何 Grok Bot 产品旅程（v3 R115），本文件的 27 页 golden 矩阵与 152 条 UI 台账口径不变。

---

## 0. 裁决摘要

2026-08-22 用户三条裁决，本文件据此展开，不再复议：

| # | 裁决 | 后果 |
| --- | --- | --- |
| D1 | **自有设计系统**，不复刻上游视觉 | 旅程 / route / 组件行为保持 parity（v3 §3.1、v3 §21.1），**视觉不是 parity 对象**；视觉 oracle = 本项目自己的 golden 截图（§10），不是上游截图 |
| D2 | **Tailwind v4 standalone CLI，零 Node** | 工作区构建链零 Node / npm，没有 `node_modules`；CSS 由钉 sha256 的单文件二进制编译，登记进 v3 §16.3 供应链（§12）。2026-08-28 R117 精确化：仓内唯一允许的 `package.json` 是 Electron engine shim 的 app manifest（v3 §11.3：零 dependencies、零 scripts、无 lockfile）；`grok-bot/` 参考树里的 `package.json` 不参与任何构建（v3 §11.5）。Electron 本身以官方 release zip + sha256 获取（`tools/engine-pins.toml`），不经 npm |
| D3 | **中英双语，首版带 i18n 框架** | `en`（源语言）+ `zh-CN`；字符串进目录，缺译是闸门红（§8） |

三个标签贯穿全文，每一项都必须带其一（与仓库 `CLAUDE.md` §4 "parity vs 新增必须标注"同义）：

- **parity**：上游固定 commit 可观察到的行为，照样保留；
- **新增**：上游没有、本项目新加的能力或契约；
- **替代**：上游用某个 JavaScript 运行时库实现、本项目用 Rust/Leptos/CSS 重做的能力，行为对齐但实现无关。

---

## 1. 证据基线：上游 UI 栈实测

v3 全文 `样式` / `Tailwind` / `设计系统` / `字体` / `深色` / `响应式` / `i18n` 出现 **0** 次；`visual` 4 次全是闸门标签（§18 两行、§19.2、G6）；v3 G6 的 "web/desktop visual/a11y parity" 指的是**同一 bundle 在两个宿主里一致**，不是对上游的视觉一致——所以上游的视觉从未被裁决为目标，D1 与 v3 不冲突。

上游 `app/`（固定 commit）：

| 维度 | 实测 | 对本项目的含义 |
| --- | --- | --- |
| 框架 | Vite 7 + React 19 + TanStack Router/Query/Form | 全部不保留（v3 §0.1） |
| CSS | Tailwind `^4.3.3`（`@tailwindcss/vite`）；`app/src/styles.css` 249 行，`:root` **33** 个变量、`.dark` **32** 个；`--radius: 0.55rem`；`--font-sans: "Inter Variable"` | token 命名沿用其语义层（§4.1 映射表），值全部自有 |
| 组件体系 | shadcn `components.json`：`style: base-nova`、`baseColor: neutral`、`iconLibrary: tabler`；`components/ui` **21** 个原语；`components/` 其余 **45** 个业务组件；`className=` **852** 处 | 21 + 45 全部映射到 Leptos（§6） |
| 无头原语 | `@base-ui/react` **13** 个文件（Button / Combobox / Dialog / Input / Menu / Select / Separator / Switch / Tooltip / merge-props / use-render） | **替代**：自研 Leptos 原语（§6.1） |
| 图标 | `@tabler/icons-react` **32** 个文件，**47** 个不同图标 | **替代**：Lucide 1.33.0，按名映射（§4.6） |
| 动效 | `motion` **8** 个文件；`lib/motion.ts`：`EASE_OUT = [0.23, 1, 0.32, 1]`、`ENTRANCE_SECONDS = 0.2`；`styles.css` 有 1 个 `prefers-reduced-motion` 块 | **替代**：CSS transition/animation（§4.5） |
| 其它运行时库 | `prompt-area` 4 文件（composer）、`boring-avatars` 3 文件（头像）、`streamdown` 1 文件（Bot 正文 markdown，经 `@shikijs/core@3.23.0` 高亮） | **替代**（§6.3） |
| 主题 | 手动 light/dark：`localStorage["openbot-theme"]` + `<html class="dark">`；`prefers-color-scheme` 在 `lib/theme.ts` / `theme-provider.tsx` / `styles.css` 均 0 命中 ⇒ **不跟随系统** | parity = 手动两态；**新增** = `system` 第三态（§7） |
| 响应式 | `sm/md/lg/xl:` 前缀合计 **24** 处（md 15、sm 7、lg 1、xl 1） | 桌面优先是 parity；断点契约是新增（§5.3） |
| a11y | `aria-*` 119、`role=` 55、`sr-only` 9 | 无判据可继承；§9 自定 |
| i18n | 0 个 i18n 框架命中；字符串硬编码英文 | D3 全部新增 |
| route | **31** 个 route 文件 = **26** 个页面 + **5** 个 layout（`__root` / `_authed` / `_authed/_app` / `admin/route` / `settings/route`） | golden 对象 = 26 页 + v3 §3.1 条 7 的 memory 页 = **27** 页（§10.1） |

---

## 2. 范围与非目标

**范围**：`openbot-ui`（v3 §5.1）渲染的全部第一方界面：sign-in、app（channel / bot / agents / skills）、settings、admin、memory（新增页）、compiled component gallery 的**外框**；Server Web 与 Desktop 两宿主。

**非目标**（不在本文件、也不在首版）：

1. 用户发布的 sandboxed component 内部样式——它是不可信数据（v3 §3.3），只约束其**容器**（尺寸、边框、加载/错误态）；
2. 最终品牌名、logo、应用图标——待 v3 §23.4 商标清查后另立文档；本文件只规定品牌标的落位与尺寸。`docs/assets/brand-concept/`（2026-08-28 随 v4 计划提交的 4 个文件）是**候选概念稿**，不是品牌真源：不进 bundle、不被任何 golden 或 icon allowlist 引用，商标清查前不得升格；
3. 移动端、触屏手势、RTL（上游 `components.json` 亦 `rtl: false`）；
4. compiled component 各自的内部视觉（v3 §21.1 条 5 的 golden 按组件各自立案），本文件只给它们可用的 token 与容器；
5. 打印样式。

---

## 3. 设计原则（7 条，违反即评审驳回）

与同一团队既有桌面产品已生效的简约中性范式一致（2026-07-16 裁决），在本项目写成可判定规则：

1. **chrome 恒中性**：按钮 / 胶囊 / 标签页 / 侧栏项的背景只允许 `bg` / `bg-subtle` / `bg-chip` 三档中性色，**零彩色背景、零彩色边框**；唯一合法实心按钮 = `primary`（`bg-inverse` 底 + `fg-inverse` 字）。
2. **语义色只落文字、图标、状态点**：`danger` / `caution` / `success` / `info` 永不作为背景或边框出现（机械闸门：§12.6 的 CSS 反向 grep）。
3. **选中态 = 文字色 + 图标色 + 对勾**，不靠底色；hover = 底色沉一档（`bg` → `bg-subtle` → `bg-chip`）。
4. **卡片无边框、不上浮、不投大阴影**；阴影只存在于 popover 与 dialog 两级（§4.4）。结构性边框只给输入框、选择器、分隔线、表格。
5. **图标一律矢量**（Lucide 描边），`currentColor` 继承文字色；品牌标是唯一例外（§4.6.3）。
6. **信息密度偏紧**：正文 14px、行高 20px、行高度 32/36px——这是桌面工具不是营销页。
7. **动效只用于解释状态变化**（出现 / 消失 / 运行中），不做装饰；`prefers-reduced-motion` 下全部静止。

---

## 4. Design token

### 4.1 单一来源与落点

- **单一来源** = `crates/openbot-ui/design/tokens.toml`（两个主题的全部值）。`build.rs` 从它生成：① `design/tokens.css`（`:root` 亮色块、`.dark` 块、`@media (prefers-color-scheme: dark) :root:not(.light)` 块——三块同源，不手写）；② Rust 常量模块（compiled component 与对比度测试共用）。
- Tailwind 只经 `@theme inline` 把 CSS 变量映射成 utility（`bg-chip` / `text-secondary` / `rounded-md` …）；**组件代码只用 token utility，禁止写字面颜色 / 字号 / 圆角**（闸门 §12.6）。
- **禁止 `dark:` 变体**：主题切换完全由 token 值切换完成，组件对主题无感；需要随主题变化的非颜色量（阴影强度、图片压暗）也做成 token。闸门：`grep -rn 'dark:' crates/openbot-ui/src` 必须为 0。

上游 33 个 `:root` 变量的语义层与本项目 token 的映射（值全部不同）：

| 上游 | 本项目 token | 说明 |
| --- | --- | --- |
| `--background` / `--foreground` | `bg` / `fg` | 页面底与正文 |
| `--card` / `--card-foreground` | `bg` / `fg` | 卡片无独立底色（原则 4） |
| `--popover` / `--popover-foreground` | `bg-popover` / `fg` | |
| `--primary` / `--primary-foreground` | `bg-inverse` / `fg-inverse` | 唯一实心按钮 |
| `--secondary` / `--secondary-foreground` | `bg-chip` / `fg` | 中性按钮 |
| `--muted` / `--muted-foreground` | `bg-subtle` / `fg-muted` | |
| `--accent` / `--accent-foreground` | `bg-chip` / `fg` | hover 态 |
| `--destructive` | `danger` | 只落文字/图标 |
| `--success` | `success` | 同上 |
| `--border` / `--input` | `border` | 结构边框 |
| `--ring` | `ring` | 焦点环 |
| `--chart-1..5` | `chart-1..5` | 仅 gallery 图表数据系列可用 |
| `--radius` | `radius` | |
| `--sidebar*`（8 个） | `bg-sidebar` + 复用 `fg` / `fg-secondary` / `bg-chip` | 侧栏不再独立一套 |

新增 token（上游无）：`fg-secondary`、`caution`、`info`、`shadow-popover`、`shadow-dialog`、
`image-dim`、`avatar-0..7`、`modal-overlay`。

### 4.2 颜色（亮 / 暗两套，WCAG 对比度已实测）

| token | 亮 | 暗 | 用途 |
| --- | --- | --- | --- |
| `bg` | `#FFFFFF` | `#141415` | 页面 / 卡片 / 输入框底 |
| `bg-subtle` | `#F4F4F5` | `#1F1F22` | hover 底、表头、代码块底 |
| `bg-chip` | `#EFEFF1` | `#27272B` | 中性按钮 / 胶囊 / 芯片底、选中行 |
| `bg-sidebar` | `#F7F7F8` | `#0F0F10` | 侧栏底 |
| `bg-popover` | `#FFFFFF` | `#1C1C1F` | popover / menu / dialog 底 |
| `bg-inverse` | `#111111` | `#F4F4F5` | primary 按钮底 |
| `fg` | `#1A1A1A` | `#ECECEE` | 正文、标题、选中态文字 |
| `fg-secondary` | `#5B5B63` | `#A6A6AF` | 次级文字、按钮默认文字、侧栏项 |
| `fg-muted` | `#68686F` | `#8E8E98` | 占位符、时间戳、说明 |
| `fg-inverse` | `#FFFFFF` | `#111111` | primary 按钮文字 |
| `border` | `#E4E4E7` | `#2E2E33` | 输入框 / 分隔线 / 表格线 |
| `ring` | `#1A1A1A` | `#F4F4F5` | `:focus-visible` 环（中性，不用品牌色） |
| `danger` | `#B42318` | `#F97066` | 错误文字 / 图标 / 状态点 |
| `caution` | `#9A5100` | `#FDB022` | 警告 |
| `success` | `#067647` | `#47CD89` | 成功 / 在线 |
| `info` | `#1552C5` | `#84ADFF` | 提示 |
| `chart-1..5` | `#3B5BDB` `#0CA678` `#E8590C` `#AE3EC9` `#868E96` | `#748FFC` `#38D9A9` `#FFA94D` `#DA77F2` `#ADB5BD` | 仅图表数据系列 |
| `avatar-0..7` | `#E8E8EA` `#E8ECEB` `#E9EBE4` `#EEE9E3` `#ECE7EA` `#E7E9EE` `#ECEBE5` `#E5ECEC` | `#2A2A2E` `#26302E` `#303027` `#312B27` `#30282E` `#282C34` `#302F28` `#263132` | deterministic initials avatar；只作 neutral-low-saturation 底 |
| `shadow-popover` | `0 4px 16px rgb(0 0 0 / .10)` | `0 4px 16px rgb(0 0 0 / .45)` | |
| `shadow-dialog` | `0 12px 40px rgb(0 0 0 / .18)` | `0 12px 40px rgb(0 0 0 / .60)` | |
| `image-dim` | `1` | `.88` | 暗色下内容图片 `filter: brightness()` |
| `modal-overlay` | `rgb(0 0 0 / .10)` | `rgb(0 0 0 / .35)` | Dialog/Sheet 共用 backdrop |

**对比度判据**（机械，§9.2 单测读同一份 `tokens.toml`）：文字 token（`fg` / `fg-secondary` / `fg-muted` / `danger` / `caution` / `success` / `info`）对每个可作其底的背景 token（`bg` / `bg-subtle` / `bg-chip` / `bg-sidebar` / `bg-popover`）≥ **4.5:1**；`fg-inverse` 对 `bg-inverse` ≥ 4.5:1；`ring` 与 `chart-*` 对 `bg` ≥ **3:1**；`fg` 对 `avatar-0..7` 亮暗 16 组均 ≥4.5:1。当前 core 超集 84 对 + avatar 16 对 = **100 对**全部通过，整体最低仍为暗色 `fg-muted` on `bg-chip` = 4.59:1。改任何值都必须让 `token_contrast_wcag_aa_covers_all_84_required_pairs` 与 `all_avatar_palettes_keep_initials_wcag_aa_in_both_themes` 继续为绿。

### 4.3 字体与排版

- **UI 字体**：Inter Variable **4.1**（SIL OFL 1.1）随包，来源 `rsms/inter` release `v4.1` 的 `Inter-4.1.zip`（sha256 `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e`），只取两文件：`web/InterVariable.woff2`（352,240 B，sha256 `693b77d4f32ee9b8bfc995589b5fad5e99adf2832738661f5402f9978429a8e3`）与 `web/InterVariable-Italic.woff2`（387,976 B，sha256 `e564f652916db6c139570fefb9524a77c4d48f30c92928de9db19b6b5c7a262a`）。`font-display: swap`；零远程字体（v3 §13.1 CSP、v3 §16.2 首次运行零下载）。
- **字体栈**：`"Inter Variable", ui-sans-serif, system-ui, "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei UI", "Noto Sans CJK SC", sans-serif`。CJK 不随包（Noto Sans CJK 体量与授权复杂度都不值得），golden 只在钉了 CJK 字体包的 Linux 容器里跑（§10.2）。
- **等宽栈**：`ui-monospace, "SF Mono", Menlo, Consolas, "Noto Sans Mono CJK SC", monospace`，不随包。
- **字号阶**（Tailwind `@theme` 覆盖默认值）：`xs 12/16`、`sm 13/18`、`base 14/20`（body 默认）、`lg 16/24`、`xl 20/28`、`2xl 24/32`（px/行高 px）。字重只用 400 / 500 / 600。
- 数字一律 `font-variant-numeric: tabular-nums`（时间、计数、表格）；中英混排 `text-wrap: pretty` 仅标题，正文 `overflow-wrap: anywhere` 防长 token 撑破列。

### 4.4 尺寸、圆角、边框、焦点、层级、阴影

- **间距**：Tailwind 默认 4px 阶，允许用 `1 2 3 4 5 6 8 10 12 16`（×4px）；禁止任意值 `[Npx]`。
- **圆角**：`radius = 8px`；`sm 6` / `md 8` / `lg 10` / `xl 14` / `full`。胶囊按钮与芯片 = `full`；输入框、菜单项、卡片 = `md`；popover / dialog = `lg`。
- **边框**：1px `border`；只用于 Input / Select / Textarea / Separator / Table / Combobox 列表容器。
- **焦点**：仅 `:focus-visible`：`outline: 2px solid var(--ring); outline-offset: 2px`；禁止 `outline: none` 而无替代。
- **控件高度/固定尺寸**：`sm 28` / `md 32`（默认）/ `lg 36`；表格行 36；侧栏项 32；顶栏 44；
  Avatar 24/32/40；Kbd 22；Textarea 十行 cap 218；modal gutter 32、Dialog max 512。
- **z-index 阶**：base 0 / sticky 10 / sidebar 与 detail panel 20 / popover·menu·combobox 30 / sheet 40 / dialog 50 / tooltip 60 / toast 70。禁止其它值。
- **阴影**：只有 `shadow-popover` 与 `shadow-dialog` 两个 token（§4.2）；卡片、按钮、输入框零阴影。

### 4.5 动效

- 时长：hover / 底色 **120ms**；popover / menu / sheet 进入 **180ms**、退出 120ms；dialog 进入 **240ms**、退出 160ms；列表入场级联每项 +30ms、上限 8 项；Agent thinking/speaking 状态环 **1200ms**、error 单次位移 **160ms**。
- 缓动：进入 `cubic-bezier(0.2, 0, 0, 1)`，退出 `cubic-bezier(0.4, 0, 1, 1)`。（上游 `EASE_OUT` / 0.2s 是其自有值，本项目**新增**自定，不做 parity。）
- 允许动画的清单（闭合，增项走 PR）：① 底色 / 文字色过渡；② popover / menu / sheet / dialog 出入；③ 列表入场级联（替代上游 `layout/stagger.tsx`）；④ detail panel 滑入（替代 `layout/detail-panel.tsx` 的 motion）；⑤ skeleton 呼吸；⑥ 运行中工具行的文字扫光（上游 `styles.css` 已有，parity）；⑦ agent 状态环（§6.7）。
- `@media (prefers-reduced-motion: reduce)`：以上全部 `transition-duration: 0ms; animation: none`，扫光 / 呼吸 / 状态环降为静态；这是一条 token 级总开关，不允许组件自行豁免。
- 实现只用 CSS transition / `@keyframes`；需要 JS 驱动的（detail panel 的尺寸联动）用 `web-sys` 的 Web Animations API；不引入动画 crate。

### 4.6 图标

#### 4.6.1 图标集

- **Lucide 1.33.0**（ISC AND MIT：其 LICENSE 点名的 Feather 衍生子集另受 MIT；`lucide-icons-1.33.0.zip` sha256 `53831c8def65621f88cae315cdb38ac70db1d937062df35c93546efb00260a98`，1,776 个 SVG）。
- **只随包用到的图标**：`crates/openbot-ui/design/icons/<name>.svg` + allowlist `design/icons.toml`；`build.rs` 生成 `Icon` 枚举与 `view!` 片段。闸门：源码里每个 `Icon::X` 必在 allowlist；目录里每个 SVG 必被引用；两向皆零漂移。许可证文本进 `NOTICE`。
- 尺寸 16（行内 / 文字旁）与 20（导航 / 按钮）；`stroke-width` 1.75（生成时改写 Lucide 默认的 2）；颜色 `currentColor`。

#### 4.6.2 上游 47 个 Tabler 图标的映射（46 Lucide + 1 品牌标，名字已在 1.33.0 的 SVG 集里逐个核实）

| 上游 | Lucide | 上游 | Lucide |
| --- | --- | --- | --- |
| IconAlertTriangle | `triangle-alert` | IconLayoutSidebar | `panel-left` |
| IconArrowDown | `arrow-down` | IconListDetails | `list-checks` |
| IconArrowLeft | `arrow-left` | IconLock | `lock` |
| IconArrowUp | `arrow-up` | IconLogout | `log-out` |
| IconArrowUpRight | `arrow-up-right` | IconPlayerStopFilled | `circle-stop` |
| IconBolt | `zap` | IconPlug | `plug` |
| IconBox | `box` | IconPlus | `plus` |
| IconBrandGoogleDrive | 品牌标 `brand/google-drive.svg`（§4.6.3） | IconPresentation | `presentation` |
| IconBuildingBank | `landmark` | IconPuzzle | `puzzle` |
| IconCheck | `check` | IconRefresh | `refresh-cw` |
| IconChevronDown | `chevron-down` | IconSearch | `search` |
| IconChevronLeft | `chevron-left` | IconSelector | `chevrons-up-down` |
| IconChevronRight | `chevron-right` | IconSettings | `settings` |
| IconChevronUp | `chevron-up` | IconShieldCheck | `shield-check` |
| IconClock | `clock` | IconShieldLock | `shield-lock` |
| IconCode | `code` | IconTable | `table` |
| IconDatabase | `database` | IconTag | `tag` |
| IconDeviceDesktop | `monitor` | IconTerminal2 | `square-terminal` |
| IconDots | `ellipsis` | IconTrash | `trash-2` |
| IconExternalLink | `external-link` | IconUser | `user` |
| IconFile | `file` | IconUsers | `users` |
| IconFileText | `file-text` | IconWorld | `globe` |
| IconFolder | `folder` | | |
| IconKey | `key` | | |
| IconLayoutGrid | `layout-grid` | | |

新增页面 / 新增控件需要的 28 个（同样已核实存在于 1.33.0）：

新增图标：`brain` `sun` `moon` `sun-moon` `languages` `copy` `x` `eye` `eye-off` `pencil` `loader-circle` `info` `circle-check` `circle-x` `circle-alert` `paperclip` `send` `at-sign` `bot` `archive` `undo-2` `minus` `log-in` `panel-right` `maximize-2` `minimize-2` `keyboard` `command`

用途：`brain` = memory 页；`sun` / `moon` / `sun-moon` = 主题三态；`languages` = 语言切换；其余为通用控件。首版 allowlist = 46 + 28 = **74** 个；`icons.toml` 是真源，本表是其 2026-08-22 快照。

#### 4.6.3 品牌标

sign-in 按钮（Google / Microsoft / Okta，parity 于上游 `auth/provider-logo.tsx`）与连接器行（Google Drive）使用各家官方 SVG 标志，放 `design/brand/`，**不改色、不描边、不与 Lucide 混排进 allowlist**；每个文件的来源 URL、版本与使用条款登记进 v3 §19.3 的 `provenance/sources.spdx.json`。这是原则 5 "语义色不落图标" 的唯一例外，因为品牌标不是语义色。

---

## 5. 布局壳与断点

### 5.1 App shell（parity 于上游 `app-sidebar` + `layout/page-shell`，尺寸新增）

```text
┌──────────┬──────────────────────────────────────────┬────────────┐
│ sidebar  │ topbar 44px                               │ detail     │
│ 240px    ├──────────────────────────────────────────┤ panel      │
│ (rail    │ content                                   │ 360px      │
│  48px)   │ max-width 960px，水平 padding 24px         │ (可选)     │
└──────────┴──────────────────────────────────────────┴────────────┘
```

- sidebar：240px，可折叠为 48px 图标栏（快捷键 `Ctrl/⌘+B`，新增）；底部固定用户菜单；`nav` landmark。
- topbar：44px，放面包屑 / 页面标题 / 页面级动作（中性胶囊按钮）。
- content：配置类页面 `max-width: 960px` 居中；表格类页面 `max-width: 1200px`。
- detail panel：360px，从右滑入（替代 `layout/detail-panel.tsx`），开合状态属于 URL search 参数（parity：可链接、可后退）。
- admin 与 settings 各有二级侧栏（parity：`admin/admin-sidebar.tsx`、`settings/settings-sidebar.tsx`），宽 200px，在 content 区左侧。

### 5.2 四类页面模板

| 模板 | 页面 | 结构 |
| --- | --- | --- |
| Chat | channel、bot、channel/new | 无 content max-width；transcript 占满、内容列 `max-width: 768px` 居中；composer 固定底部、`padding-bottom: env(safe-area-inset-bottom)` |
| List | 首页 channel 列表、agents、skills、admin 八个列表页、settings 列表页 | 标题行 + 动作 → 搜索 / 过滤行 → `Item` 列表或 `Table`；空态用 `EmptyState` |
| Form | coworker 创建/编辑、skill 编辑、credential、identity provider、plugin 配置、OAuth client | `Field` 纵向堆叠，标签在上；主动作在底部右侧（primary + 中性取消） |
| Focus | sign-in、playground、computer live view | 无 sidebar；sign-in 为 400px 居中无边框卡片，按钮纵排 |

### 5.3 断点与最小视口（新增契约）

| 名称 | 宽 | 行为 |
| --- | --- | --- |
| `lg` | ≥ 1024px | 完整三栏 |
| `md` | 768–1023px | sidebar 自动折叠为 rail；detail panel 变为 `Sheet` 覆盖 |
| `< md` | < 768px | sidebar 变 `Sheet`；不承诺可用性（非目标 3） |

- **最小支持视口 1024×640**；Desktop 主窗口 `minWidth/minHeight` 同值、默认 1440×900、按 window label 记忆尺寸与位置（v3 §13.3 多窗口）。
- 上游 24 处断点用法不构成契约，本表是唯一真源。

---

## 6. 组件清单

### 6.1 原语（上游 `components/ui` 21 个 → `openbot_ui::primitives`）

状态集固定为 `default / hover / focus-visible / active / disabled / loading / invalid / selected / open`，每个原语只声明适用子集；状态由 `data-state` 属性驱动，CSS 以 `&:is(:hover, [data-state~="hover"])` 形式同时响应真实交互与强制态，使 §10.3 的设计画廊能静态渲染每个状态。

| 上游文件 | Leptos 组件 | 替代的库 | 键盘 / ARIA 模式（WAI-ARIA APG） | 状态子集 |
| --- | --- | --- | --- | --- |
| `button.tsx` | `Button`（variant `chip`默认 / `primary` / `ghost` / `danger-text`；size `sm/md/lg`） | base-ui Button | button；`Space/Enter` | 全部 |
| `combobox.tsx` | `Combobox` | base-ui Combobox | combobox + listbox；`↑↓ Home End Enter Esc`、typeahead | default/focus/open/disabled/invalid |
| `dialog.tsx` | `Dialog` | base-ui Dialog | modal dialog；焦点陷阱、`Esc` 关闭、关闭后焦点归还触发元素 | open |
| `dropdown-menu.tsx` | `Menu` | base-ui Menu | menu button；`↑↓ Home End Enter Esc`、typeahead、子菜单 `→ ←` | open/disabled |
| `empty.tsx` | `EmptyState` | — | 纯展示 | — |
| `field.tsx` | `Field`（label + 控件 + 说明 + 错误，`aria-describedby` / `aria-invalid` 自动接线） | — | — | invalid/disabled |
| `input-group.tsx` | `InputGroup`（前后缀槽） | — | — | focus-within |
| `input.tsx` | `Input` | base-ui Input | textbox | focus/disabled/invalid |
| `item.tsx` | `Item`（列表行：media / 标题 / 描述 / 动作槽） | — | 可为 link 或 button | hover/selected/disabled |
| `label.tsx` | `Label` | — | `for` 绑定 | — |
| `message-scroller.tsx` | `MessageScroller`（自动贴底、"回到底部"胶囊、新消息时保持阅读位置） | — | `role="log"`、`aria-live="polite"` | — |
| `message.tsx` | `Message` | — | `article` | — |
| `bubble.tsx` | `Bubble`（用户 / Bot 气泡外框） | — | — | — |
| `select.tsx` | `Select` | base-ui Select | select-only combobox；`↑↓ Home End Enter Esc`、typeahead | default/focus/open/disabled/invalid |
| `separator.tsx` | `Separator` | base-ui Separator | `separator` | — |
| `sheet.tsx` | `Sheet`（侧向 dialog） | — | 同 Dialog | open |
| `sidebar.tsx` | `Sidebar`（shell 导航，折叠 / rail / Sheet 三形态） | shadcn sidebar | `nav` landmark；`aria-current="page"` | collapsed |
| `skeleton.tsx` | `Skeleton` | — | `aria-hidden` | — |
| `switch.tsx` | `Switch` | base-ui Switch | switch；`Space` | checked/disabled |
| `textarea.tsx` | `Textarea`（自动增高，上限 10 行） | — | textbox multiline | focus/disabled/invalid |
| `tooltip.tsx` | `Tooltip` | base-ui Tooltip | tooltip；hover/focus 显示、`Esc` 隐藏、延迟 400ms | open |

**新增原语**（上游无，本项目需要）：`Toast`（非阻塞反馈，`role="status"`，5s 自动消失，用于一切 `accepted:false` 类用户可见反馈）、`Badge`（纯文字状态，语义色只落文字 + 状态点）、`Kbd`、`Avatar`（§6.6）、`ThemeToggle`（§7）、`LocaleSwitch`（§8）。

### 6.2 业务组件（上游 45 个 → `openbot_ui::features::<组>`，snake_case 同名，例外单列）

| 组 | 上游文件数 | 说明 / 例外 |
| --- | --- | --- |
| admin | 1 | `admin-sidebar` |
| agents | 8 | `agent-card` `agent-fields` `agent-profile` `callback-token-panel` `new-agent` 同名；`abstract-avatar` → `Avatar`（§6.6）；`orb/agent-orb` + `orb/ai-core` → `AgentPresence`（§6.7，**替代**） |
| app-sidebar | 2 | `app-sidebar` → `Sidebar` 内容；`channel` 同名 |
| auth | 1 | `provider-logo` → 品牌标（§4.6.3） |
| channels | 8 | `channel-chat` `chat-transcript` `conversation-view` `recipient-field` `tool-boundary` `tool-line` 同名；`composer/composer` → `Composer`（§6.5）；`avatar` → `Avatar` |
| computer | 5 | `activity-log` `command-output` `computer-view` `live-screen` `placeholder` 同名 |
| gallery | 8 | `activity` `cards` `charts` `decisions` `frame` `preview` `quote` `refused` 同名（compiled gallery 的 Leptos 重写，v3 §3.3） |
| layout | 4 | `page-shell` `detail-panel` `row-mark` 同名；`stagger` → CSS 级联工具类（§4.5 ③，**替代**） |
| settings | 2 | `settings-sidebar` 同名；`background`（computer frame 的等待态装饰 SVG）→ `ComputerPlaceholderArt`（重绘为中性线稿，**替代**） |
| skills | 4 | `edit-skill` `new-skill` `skill-agents` `skill-fields` 同名 |
| 根 | 2 | `component-preview` 同名；`theme-provider` → `theme` 模块（§7，非组件） |

合计 1+8+2+1+8+5+8+4+2+4+2 = 45，与上游 `find components -name '*.tsx' -not -path 'components/ui/*' | wc -l` 相等。

### 6.3 六个运行时 JavaScript 库的替代

| 上游库 | 用途 | 替代 | 版本钉 |
| --- | --- | --- | --- |
| `@base-ui/react` | 无头原语 | §6.1 自研 Leptos 原语 | — |
| `motion` | 动效 | CSS transition/animation + Web Animations API（§4.5） | — |
| `streamdown`（+ `@shikijs/core` 3.23.0） | Bot 正文流式 markdown 与代码高亮 | `pulldown-cmark` **0.13.4** 增量渲染 + `syntect` **5.3.0**（`default-fancy` 特性，纯 Rust，WASM 可用）（§6.4） | 进 v3 §1.2 |
| `prompt-area` | composer | `Composer`（§6.5） | — |
| `boring-avatars` | 生成式头像 | `Avatar`（§6.6） | — |
| `tw-animate-css` | 动画工具类 | 自写 `@keyframes`（§4.5 清单） | — |

### 6.4 Transcript 渲染规则（markdown / 代码 / 链接 / 图片）

- 解析：`pulldown-cmark` 开 `TABLES | STRIKETHROUGH | TASKLISTS`，**不开 raw HTML**——`Event::Html` 一律按文本渲染（模型输出是不可信输入）。
- 增量：消息按块切分，已完成块按 `(message_id, block_index)` memo，只有尾块随 delta 重解析；渲染结果与一次性完整渲染逐节点相等（单测 `streaming_render_equals_batch_render`）。
- 代码块：`syntect` 自定义 `SyntaxSet`（固定语言清单 24 种：bash / sh / powershell / rust / toml / json / yaml / sql / ts / tsx / js / jsx / html / css / md / py / go / java / kotlin / swift / c / cpp / diff / dockerfile），scope → 6 个 token 色（`fg` / `fg-secondary` / `fg-muted` / `info` / `success` / `caution`），不用 syntect 自带主题；右上角复制按钮（中性胶囊）。
- 链接：一律 `rel="noopener noreferrer"`；Desktop 经 Tauri `opener` 交宿主（v3 §13.1 external URL 当不可信输入），Web `target="_blank"`；显示域名前缀以防钓鱼文本。
- 图片：只内联应用自身附件（同源 / custom protocol）；远程 `![]()` 渲染为带域名的链接芯片，不发起请求（v3 §13.1 CSP 零宽泛出站；上游 streamdown 对远程图片的行为未核，本条是本项目裁决，标**新增**）。
- 表格超宽在自身容器内横向滚动，页面不横滚。

### 6.5 Composer（parity 于 `channels/composer/*`：`draft.ts` / `queue.ts` / `sources.ts` / `triggers.ts` 四份逻辑与两份测试）

- 草稿按 channel 持久（parity `draft.ts`）；发送队列与 stop/steer（parity `queue.ts`，v3 §3.1 条 4）；`@coworker` 与 `/skill` 触发（parity `triggers.ts` / `sources.ts`）。
- 外观：无边框底 `bg-subtle`、圆角 `lg`、内侧 `Textarea` 自动增高上限 10 行；左下附件（`paperclip`）、右下发送（primary 圆形 32px，`send`）/ 运行中变 stop（`circle-stop`，中性）。
- `@` / `/` 弹出 `Combobox` 列表，键盘模式同 §6.1；`Enter` 发送、`Shift+Enter` 换行、`Esc` 关弹层；输入法组合期间（`compositionstart`–`compositionend`）`Enter` 不发送（中文输入必需，**新增**）。

### 6.6 Avatar（替代 `boring-avatars` + `agents/abstract-avatar.tsx` + `channels/avatar.tsx`）

- 用户 / coworker / Bot 三类统一：圆形 24 / 32 / 40px；有头像图用图，否则首字母（中文取首字、英文取两词首字母）+ 底色由 `SHA-256(principal_id)` 前 8 位映射到 8 个预设中性-低饱和底色（亮暗各一组，与 `fg` 对比 ≥ 4.5:1，同 §4.2 单测）。
- 同一 id 在任何平台、任何时间得到同一头像（golden 依赖这条确定性）。

### 6.7 AgentPresence（替代 `agents/orb/*`）

- 上游是 437 行 canvas/shader、音频反应的 orb；本项目按原则 7 收为 **20px 状态环**：`idle` 静止灰环、`thinking` 慢速旋转弧（`loader-circle`，1.2s/圈）、`speaking` 两段弧交替、`error` 变 `danger` 色静止环 + 单次 160ms 横向位移。
- reduced-motion 下全部静止，只靠颜色与 `aria-label` 表达状态；状态文本经 i18n。

---

## 7. 主题

- 三态：`system`（默认，**新增**）/ `light` / `dark`（后两者 parity 于上游手动切换）。
- 落点：`<html>` 的 class —— `light` 强制亮、`dark` 强制暗、无 class = 跟随 `prefers-color-scheme`（由 §4.1 生成的第三个 CSS 块实现）。
- 持久化：Server 形态存用户偏好（经 `ApplicationService`，跨设备一致）并镜像到 cookie `openbot-ui`（`SameSite=Lax`，只含 `theme` 与 `locale` 两个非敏感字段）；Desktop 形态存本地设置。
- **首帧零 JavaScript**：Axum 发 `index.html` 时从 cookie 改写 `<html class lang>`；Tauri custom protocol 发 `index.html` 时从本地设置改写同一处。不存在内联 `<script>`（v3 §0.1 不保留第一方 JavaScript，v3 §13.1 strict CSP）。
- `ThemeToggle`（settings 页与侧栏用户菜单）：三段中性分段控件，图标 `sun-moon` / `sun` / `moon`，选中态靠文字色 + 对勾（原则 3）。
- 切换即时生效、不重载；token 切换是唯一机制（§4.1 禁 `dark:`）。

---

## 8. 国际化

### 8.1 框架与版本

- `leptos_i18n` **0.6.2**（MIT；依赖 `leptos ^0.8`、`icu_locale ^2.2`，与 v3 钉的 Leptos 0.8.19 兼容）+ `leptos_i18n_build` **0.6.2**（`build.rs` 代码生成）。特性：`csr` `plurals` `format_datetime` `format_nums` `icu_compiled_data`；不开 `cookie` / `dynamic_load`（locale 由本项目自己解析，翻译随 bundle）。
- 二进制体积：按其文档的 ICU4X datagen 路线只打 `en` 与 `zh-CN` 两套数据，进 §10.5 的 wasm 预算。

### 8.2 目录与键

- 文件：`crates/openbot-ui/locales/en.json`（源语言）、`locales/zh-CN.json`；JSON（库默认格式）。
- 命名空间按 §6.2 的组：`shell` `auth` `channels` `agents` `skills` `settings` `admin` `computer` `gallery` `memory` `errors` `common`；键 `snake_case`，句子级而非单词级（`channels.composer.send` 不是 `common.send`，除非真是通用词）。
- 规则：禁止字符串拼接组句（占位符 `{name}`）；复数用 `plurals`（ICU 规则，中文只有 `other`）；日期 / 数字 / 列表用 `t_format!`；禁止把 UI 文案写进 `openbot-domain` / `openbot-application`（错误与状态以 **code** 穿越边界，GUI 端查表本地化，对应 v3 §15.3 错误语义）。

### 8.3 缺译闸门（库的真实行为决定了必须自建）

`leptos_i18n_build` 对缺键 / 多键只发 cargo **warning**（`emit_diagnostics()`，`suppress_key_warnings` 可静音），不是错误，且 cargo 的 `-D warnings` 管不到 build script 的 warning。因此闸门 = `xtask i18n-check`：解析两份 JSON，**键集合必须逐字相等**（多一键少一键都红），占位符集合逐键相等，`en.json` 为真源。CI 必跑；本地 commit 前必跑（§15）。

### 8.4 locale 解析与切换

- 解析顺序（本项目自定，不用库的 cookie / navigator 探测）：用户设置 → Server 形态的 `Accept-Language` / Desktop 形态的 OS locale（`sys-locale` **0.3.2**，在 `openbot-desktop` 读取后写进 `<html lang>`）→ `en`。
- GUI 启动以 `<html lang>` 为准调用 `set_locale`；切换不重载；`<html lang>` 同步更新（屏幕阅读器读音依赖它）。
- 首版 locale 只有 `en` 与 `zh-CN`；繁体与其它语言走 PR 加文件，不改代码。

### 8.5 术语表（`zh-CN` 首版，改动走 PR，`locales/GLOSSARY.md` 是真源）

| en | zh-CN | en | zh-CN |
| --- | --- | --- | --- |
| Bot | Bot（不译） | Boundary | 边界 |
| Coworker | 同事 | Credential | 凭据 |
| Channel | 频道 | Identity provider | 身份提供方 |
| Thread | 会话 | People | 成员 |
| Run | 运行 | Grant | 授权 |
| Tool | 工具 | Approval | 审批 |
| Skill | 技能 | Audit | 审计 |
| Plugin | 插件 | Component | 组件 |
| Connector | 连接器 | Gallery | 组件库 |
| Computer | 计算机 | Memory | 记忆 |
| Deployment | 部署 | Tenant package | 租户包 |

### 8.6 CJK 排版

中文正文 `line-height: 1.6`（英文 1.43 的 14/20 在中文偏挤，按 `:lang(zh)` 覆盖）；标点悬挂 `hanging-punctuation: first`（支持的引擎生效，不支持无害）；中英文之间不自动加空格（由文案维护）。

### 8.7 不翻译的东西

模型可见文本（tool description、系统提示）、审计记录正文、日志、API 错误 `code`、租户包 YAML 键、组件参数 schema。它们是协议不是界面。

---

## 9. 无障碍

### 9.1 目标

第一方 GUI（Web 与 Desktop 主 WebView）达到 **WCAG 2.2 AA** 的可机械判定子集；唯一豁免 = Desktop sandboxed component（v3 §3.3 已写死，帧流画面不可达）。

豁免的**具名 fallback**（2026-08-28 v3 R118 裁决，**新增**）：Desktop 上每个 sandboxed component 的帧画布旁，由 Rust/Leptos 以只读 `<dl>` 渲染该次 tool call 的 `arguments` JSON（键 → 值；嵌套对象展开为 `a.b.c` 路径键；数组展开为 `a[0]`），标题为组件名，容器带 i18n 的 `aria-label`（语义："预览画面不可达，以下为结构化参数"）；画布本身 `role="img"` + `aria-label=<组件名>`；`Escape` 把焦点从画布交回 transcript。fallback 只含 Rust 已按 published JSON Schema 校验过的结构化参数——**不含**作者 HTML/CSS/JS 的任何文本，也**不含**模型自由文本；它是可判定的替代物，不是 a11y parity 的宣称。Web 宿主（iframe）不适用本段，继续按 §9.2 全量判据。

### 9.2 机械判据（全部进 CI）

1. `token_contrast_wcag_aa`：Rust 单测读 `tokens.toml`，按 §4.2 的配对与阈值断言；改 token 就必跑。
2. AX 树检查（Web，经本项目自己的 Chromium 引擎 CDP `Accessibility.getFullAXTree`，v3 §11.2）：对 §10.1 的 27 页 × 2 主题，断言：每个可聚焦节点有非空可访问名称与角色；无重复 `id`；每页恰一个 `main`、至少一个 `nav`；`h1` 恰一个且标题层级不跳级；`img` 有 `alt`（装饰图 `alt=""`）；表单控件有关联 label。Desktop 的 WKWebView / WebKitGTK 无 CDP，不跑这项，靠同一 DOM 保证。
3. 键盘旅程（E2E，Web）：sidebar 全部项、每个列表页的首个 `Item`、每个表单的提交、composer 的发送 / stop、Dialog / Menu / Select / Combobox 的 APG 键位（§6.1 表）逐项脚本化；焦点永不丢失到 `body`。
4. `prefers-reduced-motion` 模式下 golden 与动画开启时的终态逐像素相等（证明动画只是过渡不是信息）。

### 9.3 编码规则

- 图标按钮必带 `aria-label`（经 i18n）；装饰图标 `aria-hidden`。
- 颜色不是唯一信息载体：状态点旁必有文字或 `aria-label`。
- `:focus-visible` 环不得被组件覆盖（§4.4）。
- 实时区域：transcript `role="log" aria-live="polite"`；Toast `role="status"`；错误 `role="alert"`。

---

## 10. 视觉 oracle 与闸门（把 v3 G6 的 "visual" 变成可判定）

### 10.1 golden 矩阵

| 维度 | 取值 |
| --- | --- |
| 页面 | 26 个上游页面 route + memory 页 = **27**；外加 §10.3 设计画廊 1 页 |
| 主题 | `light` / `dark`（`system` 以 CDP `Emulation.setEmulatedMedia` 模拟 `prefers-color-scheme` 两次，覆盖 §7 第三态的 CSS 块） |
| 视口 | 1440×900 与 1024×640（最小支持视口），DPR 1 |
| 平台 | Web：Linux x64 容器，用本项目的 Chromium 引擎截图（CDP `Page.captureScreenshot`）；Desktop：macOS arm64 与 Windows x64 各自一套，1440×900 单视口 |

数量：Web = 27 × 2 × 2 + 画廊 1 × 2 = **110** 张；Desktop = 27 × 2 = **54** 张 / 平台。

### 10.2 确定性条件（缺一则 golden 不可信）

- 数据：`openbot-server` 以 `openbot-testkit` 的 fake `ApplicationService` 从 `fixtures/ui/seed.json` 加载（无 PostgreSQL、无模型调用）；seed 含固定用户 / coworker / channel / 消息 / 审计行 / 时间戳。
- 时间：GUI 取时间只经注入的 `Clock`；golden 构建（cargo feature `golden`，**生产 bundle 不编译该特性**）把 `Clock` 钉在 `2026-01-01T00:00:00Z`。
- 动画：CDP 模拟 `prefers-reduced-motion: reduce`；截图前等待 `document.fonts.ready` 与网络空闲。
- 字体：Inter 随包；CJK 由容器镜像钉版的 `fonts-noto-cjk` 提供，镜像 digest 记录在 `fixtures/ui/golden/MANIFEST.toml`。
- 头像：§6.6 的确定性生成。
- 动态区域：`data-golden-mask` 属性标记（仅限无法钉死的区域，如实时 screencast 帧），harness 经 CDP `DOM.getBoxModel` 取矩形并遮罩；首版允许的 mask 清单进 `MANIFEST.toml`，新增 mask 需评审。

### 10.3 设计画廊

路由 `/_design`（cargo feature `design-gallery`，测试与开发构建开启，**生产关闭**，闸门：生产 bundle 的路由表测试断言其不存在）：渲染 §6.1 全部原语 × 各自状态子集（经 `data-state` 强制）+ token 色板 + 字号阶 + 图标 allowlist。它是组件级 golden 的承载面，也是设计评审的唯一对照页。

### 10.4 比对规则与更新流程

- 比对在 `openbot-testkit` 用 `image` **0.25.10** 实现：逐像素任一通道差 > 16/255 记为差异像素；**失败判据 = 差异像素 > 0.1% 或存在任一 8×8 全差异块**（后者防小区域真回归被比例稀释）。
- golden 更新 = 同 PR 提交新 PNG + harness 生成的 diff 图（`fixtures/ui/golden/_diff/` 不入库、附在 PR）+ 评审批准；禁止 CI 自动覆盖。
- Desktop：同一 bundle 摘要（`dist/` 的 sha256 清单）在三平台相等 + 各平台对自己的 golden 基线比对；**不做跨引擎逐像素比对**（WKWebView / WebView2 / Chromium 的字体栅格化不同，逐像素相等构造上不可达）。这就是 v3 G6 "web/desktop visual parity" 的可判定定义。
- Desktop 截图用 `xcap` **0.9.8**（Apache-2.0，仅 testkit 依赖）捕获窗口。

### 10.5 bundle 预算（新增契约，`xtask bundle-budget` 闸门）

| 产物 | 上限 |
| --- | --- |
| `app.wasm`（gzip） | 3.5 MiB |
| `app.css` | **128 KiB**（2026-08-28 R123 由 96 KiB 放宽；**120 KiB 为警戒线**，`xtask bundle-budget` 越线只打印 warning、不判红） |
| 字体（两份 woff2） | 800 KiB（实测 740,216 B） |
| 图标 | 内联进 wasm，不单列 |

超限即红；放宽只能经 delta audit 附实测。

R123 的实测依据（全部是本仓已登记的数字，可复算）：R103（Batch 43）时 `app.css` 余 4,658 B = 93,646 B；R111（Batch 48）余 2,254 B = 96,050 B；Batch 50 实测 97,848 B，余 456 B —— 7 个批次 +4,202 B ≈ 600 B/批，而固定成本（token 三块 CSS + 27 条 primitive）已经全部在里面。剩余 24 个 route journey、完整 Composer/AppSidebar 与 admin 面按 ≤ 30 批估算 +18 KiB → ≈ 116 KiB；96 KiB 下任何下一批 UI 都会判红。128 KiB 留 12 KiB 余量；再放宽同样只能经 delta audit，且必须先证明 CSS 复用已做尽。

---

## 11. Phase 0 产物新增（并入 v3 §19.3）

```text
parity/ui.yaml                      # 21 原语 + 45 业务组件 + 47 图标 + 6 运行时库 + 27 页，每项 upstream / target / label(parity|新增|替代) / owner / test_id / status
fixtures/ui/seed.json               # §10.2 固定数据
fixtures/ui/golden/MANIFEST.toml    # 容器镜像 digest、字体包版本、mask 清单、视口、阈值
fixtures/ui/golden/web/<page>.<theme>.<w>x<h>.png
fixtures/ui/golden/macos-arm64/…    fixtures/ui/golden/windows-x64/…
tools/pins.toml                     # §12.1 工具二进制 sha256 表
crates/openbot-ui/design/tokens.toml  icons.toml  locales/GLOSSARY.md
```

CI 对 `parity/ui.yaml` 的规则与其它 parity 文件相同：未归类项与无证据 `done` 都拒绝。

---

## 12. 构建链与供应链（D2 的落地）

### 12.1 工具二进制钉版（`tools/pins.toml`，`xtask tools fetch` 下载到 `target/tools/bin`，`xtask tools verify` 校验 sha256 与 `--version`，任一不符即红）

| 工具 | 版本 | 来源与校验 |
| --- | --- | --- |
| Tailwind CSS standalone CLI | **v4.3.3**（与上游 `^4.3.3` 同主次版本；2026-07-16 发布，本文件写作时为最新） | GitHub release `tailwindlabs/tailwindcss v4.3.3`，`sha256sums.txt`：`linux-x64` `dc61b3ac6b8c9ca874c0cc4c57b2409791a64c5540404ca5f5367360babc313a` · `linux-arm64` `55fd0b241214eff3de1e8ee4f22796662f2d2e7a49bcfca7477cfd0bac398195` · `macos-arm64` `cdf646702987a743464dff4d9c60fd4480d1c1e73dd819a9a67f1078815dce9d` · `macos-x64` `7922e0953f2110c05976e3bf58f14e643d90427575e766b7d433f5f80cbee7e1` · `windows-x64.exe` `e0e260ce048014e9268f6237ff18f8ccf02cef521cbd0ae04e82c2cdf7aa3955`（musl 两件不用） |
| trunk | **0.21.14**（MIT/Apache-2.0） | `cargo install --locked trunk --version 0.21.14`，由 `xtask` 执行并校验 `trunk --version` |
| wasm-bindgen CLI | = `Cargo.lock` 中 `wasm-bindgen` crate 版本（当前最新 0.2.127，以 lock 为准） | `xtask` 从 lock 读出版本后 `cargo install --locked wasm-bindgen-cli --version <lock>`；trunk 自身也按 lock 校验 |
| wasm-opt（binaryen） | **version_132**（2026-08-12） | GitHub release `WebAssembly/binaryen version_132`，各平台 sha256 进 `pins.toml` |

- trunk 的工具查找已读实：`find_system` 用 `which` 在 PATH 找同名二进制并比对 `[tools]` 钉的版本，匹配即用；`[build] offline = true`（或 `--offline`）下找不到 / 版本不符**直接报错不下载**。因此构建 = `PATH=target/tools/bin:$PATH trunk build --release --offline --locked`，零网络、零 Node。
- `Trunk.toml`：`[build] offline = true, locked = true`；`[tools] tailwindcss = "4.3.3", wasm_opt = "version_132"`；trunk 调 Tailwind 的参数是 `--input/--output/--minify`（读自其 `tailwind_css` pipeline），与 v4 CLI 兼容，不传 `--config`。
- 这些工具**不进发行物**；v3 §0.1 的"第一方非 Rust 源码只有 Electron shim"指发行物，构建期二进制工具按 v3 §16.3 的供应链条目管理（同 PR 写进 v3，见 §14）。

### 12.2 CSS 入口（`crates/openbot-ui/design/app.css`）

```css
@import "tailwindcss";
@source "../src";                      /* 扫描 .rs 里的 class 字面量 */
@import "./tokens.css";                /* build.rs 从 tokens.toml 生成，三块同源 */
@theme inline {
  --color-bg: var(--bg);  --color-bg-subtle: var(--bg-subtle);  --color-bg-chip: var(--bg-chip);
  /* … 其余 token 一一映射 … */
  --font-sans: "Inter Variable", ui-sans-serif, system-ui, "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei UI", "Noto Sans CJK SC", sans-serif;
  --text-xs: 12px; --text-xs--line-height: 16px;  --text-sm: 13px; --text-sm--line-height: 18px;
  --text-base: 14px; --text-base--line-height: 20px;  /* lg/xl/2xl 同理 */
  --radius-sm: 6px; --radius-md: 8px; --radius-lg: 10px; --radius-xl: 14px;
}
@font-face { font-family: "Inter Variable"; src: url("/fonts/InterVariable.woff2") format("woff2"); font-weight: 100 900; font-display: swap; }
@font-face { font-family: "Inter Variable"; font-style: italic; src: url("/fonts/InterVariable-Italic.woff2") format("woff2"); font-weight: 100 900; font-display: swap; }
@media (prefers-reduced-motion: reduce) { *, *::before, *::after { transition-duration: 0ms !important; animation: none !important; } }
```

### 12.3 `index.html`（trunk 模板，零脚本）

```html
<!doctype html>
<html lang="en"><!-- Axum / Tauri 发出时改写 lang 与 class（§7、§8.4） -->
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>…</title>
  <link data-trunk rel="tailwind-css" href="design/app.css">
  <link data-trunk rel="copy-dir" href="assets/fonts">
  <link data-trunk rel="rust" data-wasm-opt="z">
</head>
<body></body>
</html>
```

CSP 不写在 HTML 里：Server 由 Axum 响应头下发，Desktop 由 `tauri.conf.json` 下发（v3 §13.1）。

### 12.4 Leptos 生态钉版（并入 v3 §1.2）

| crate | 版本 | 理由 |
| --- | --- | --- |
| `leptos` | `0.8.19`（v3 既定，不动） | |
| `leptos_router` | **`=0.8.13`** | `0.8.14` 与 `0.8.15` 的依赖是 `leptos ^0.8.20`，在 0.8.19 上无法解析；`0.8.13` 要求 `^0.8.17`。Leptos 升 0.8.20 的 delta audit 时必须同 PR 升 router 到 0.8.15 |
| `leptos_meta` | `0.8.6`（要求 `leptos ^0.8.16`） | `<title>` / `<html lang class>` 同步 |
| `leptos_i18n` / `leptos_i18n_build` | `0.6.2` | §8.1 |
| `pulldown-cmark` | `0.13.4` | §6.4 |
| `syntect` | `5.3.0`（`default-fancy`） | §6.4 |
| `sys-locale` | `0.3.2`（仅 `openbot-desktop`） | §8.4 |
| `image` | `0.25.10`（仅 `openbot-testkit`） | §10.4 |
| `xcap` | `0.9.8`（仅 `openbot-testkit`） | §10.4 |

### 12.5 class 字面量规则

Tailwind 只能从源码里看见**完整的** class 字面量。Leptos 代码：`class="…"` 写全名；条件类用 `class=("name", move || cond)`，名字仍是字面量；**禁止**运行时拼接 class 名（`format!("bg-{}", x)`）。闸门：`xtask css-check` 对比 Tailwind 产出的 class 集合与源码里 `class=` 的 token 集合，源码有而 CSS 无的 class 即红（它一定是拼接出来的）。

### 12.6 反向 grep 闸门（`xtask design-lint`，CI 必跑）

| 规则 | 判据 |
| --- | --- |
| 禁 `dark:` | `grep -rn 'dark:' crates/openbot-ui/src` = 0 |
| 禁字面颜色 / 任意值 | `grep -rnE '(bg|text|border)-\[#|\[[0-9]+px\]' crates/openbot-ui/src` = 0 |
| 语义色不落背景 / 边框 | `grep -rnE '(bg|border)-(danger|caution|success|info)\b' crates/openbot-ui/src` = 0 |
| 阴影只两级 | `rg -nP 'shadow-(?!popover|dialog|none)\b' crates/openbot-ui/src` = 0（需要 PCRE 负向前瞻，ERE 写不出） |
| 图标 allowlist | §4.6.1 两向零漂移 |
| 生产无画廊 | 生产 bundle 路由表不含 `/_design` |

---

## 13. `openbot-ui` 内部结构

```text
crates/openbot-ui/
├── build.rs            # tokens.toml → tokens.css + tokens.rs；icons.toml + svg → icons.rs；leptos_i18n_build 代码生成
├── design/             # app.css  tokens.toml  tokens.css(生成，不入库)  icons.toml  icons/*.svg  brand/*.svg
├── assets/fonts/       # InterVariable{,-Italic}.woff2 + OFL 文本
├── locales/            # en.json  zh-CN.json  GLOSSARY.md
├── index.html  Trunk.toml
└── src/
    ├── primitives/     # §6.1（+ Toast Badge Kbd Avatar ThemeToggle LocaleSwitch）
    ├── features/       # §6.2 十组
    ├── shell/          # App shell、路由表、layout 五个（对应上游 5 个 layout 文件）
    ├── theme.rs  i18n.rs  clock.rs  icons.rs(生成)  tokens.rs(生成)
    └── design_gallery.rs   # feature = "design-gallery"
```

`openbot-ui` 只依赖 `openbot-contracts`（v3 §5.2）；它不持业务规则、不拼 SQL、不调模型。

---

## 14. 对 v3 与 `CLAUDE.md` 的同 PR 修订

v3（记为 R20，进 v3 §28.1 / §28.4；以下编号均为 v3 章节）：

1. §0.1：在 shim 段后补一句——构建期工具（Tailwind standalone CLI、trunk、wasm-bindgen CLI、wasm-opt）是钉 sha256 的二进制，不进发行物、不引入 Node/npm，按 §16.3 管理。
2. §1.2：固定基线表新增 Tailwind standalone `4.3.3` / trunk `0.21.14` / binaryen `version_132` / `leptos_router =0.8.13`（耦合理由）/ `leptos_meta 0.8.6` / `leptos_i18n 0.6.2` / `pulldown-cmark 0.13.4` / `syntect 5.3.0` / Inter `4.1` / Lucide `1.33.0`，并指向本文件。
3. §3.1 末尾：视觉 / 布局 / 主题 / i18n / a11y 真源 = 本文件；主题 `system` 态与 UI 双语标**新增**。
4. §13.1：追加四条——CSS 工具链零 Node；`index.html` 零内联脚本；`<html class lang>` 首帧由 Rust 改写；字体 / 图标随包零远程。
5. §18 两行、§19.2 W11–28、G6：把 "visual" 改写为 "golden 截图（设计系统文档 §10）"，G6 的 parity 条改为 §10.4 的可判定定义。
6. §19.3：追加 §11 的产物清单。
7. §26：GUI 组追加 Tailwind standalone / trunk / leptos_i18n / Lucide / Inter 五个一手来源。
8. §28.1 追加 R20 行；§28.4 追加 §17 的 UI 计数命令。
9. （2026-08-28，v2）与 v3 R115–R125 同 PR：§0 D2 措辞、§2 品牌概念稿登记、§9.1 Desktop sandboxed component 具名 fallback、§10.5 CSS 预算 128 KiB / 120 KiB 警戒、§15 G6 文本与 §15.1 勾选；`CLAUDE.md` §2 / §4a 的预算与零 npm 措辞同步；`crates/openbot-testkit/src/xtask/ui_gates.rs` 的 `CSS_LIMIT` / `CSS_WARN` 同批落地。

`CLAUDE.md`：①"真源"段加第二真源（本文件）；② §2 允许的非 Rust 例外加构建期工具；③ 新增 §4a「GUI 视觉约束」（原则 7 条的压缩版 + 三条裁决 + 反向 grep 闸门；编号 4a 是为了不重排 §5–§11 的既有引用）；④ §3 固定基线表加 Leptos 生态 / GUI 构建工具 / GUI 资产三行；⑤ §10 闸门加 `cargo test -p openbot-ui` / `xtask i18n-check` / `design-lint` / `css-check` / `bundle-budget` / golden / AX 检查。

---

## 15. 闸门与 DoD 增补

G6 重写后的文本（替换 v3 原四条）：

- 31 route journey 100% + memory 页 journey；
- compiled gallery 全部 Leptos；sandbox escape = 0；
- multi-window ACL、Tauri XSS、queue saturation/shutdown；
- **视觉**：§10.1 矩阵的 golden 全部通过（Web 110 张、Desktop 每平台 54 张），§10.4 判据；三平台 bundle 摘要相等；
- **a11y**：§9.2 四项机械判据全绿（Desktop sandboxed component 豁免，v3 §3.3）；
- **i18n**：`xtask i18n-check` 绿；`zh-CN` 下 27 页 golden 另录一套（进 §10.1 的主题维度之外、首版只 1440×900 亮色，27 张）；
- `xtask design-lint` / `css-check` / `bundle-budget` 绿（`app.css` 上限 128 KiB、警戒 120 KiB，R123）。

本地 commit 前必跑（与 v3 §16.3 并列）：`cargo test -p openbot-ui`（含 `token_contrast_wcag_aa`、`streaming_render_equals_batch_render`）+ `xtask i18n-check design-lint css-check`；golden 与 AX 检查在 CI。

### 15.1 当前实施勾选（2026-08-28，Batch 15–50）

- [x] exact GUI 工具链、token/icon/font 生成、strict-CSP Trunk bundle 与 Axum static/首帧改写；
- [x] `/approvals` 可点击 authority-only 竖切；ThemeToggle/LocaleSwitch APG 键盘与 ARIA；
- [x] Server 用户偏好 typed PostgreSQL/native 0021 + closed cookie；Desktop Local closed 原子设置；
  UI startup read/serialized partial write/reload persistence；
- [x] Tauri 2.11.5 production custom-protocol adapter：window-label authority、typed in-process、
  本地首帧/CSP/canonical asset；依赖只进入 macOS/Windows Desktop target；
- [x] 截至 Batch23，27条 primitive 子账全 done：Batch18前20条 + Dialog/Sheet +
  Menu + MessageScroller + Combobox/Select + Sidebar，当时 UI=`27/125/152`；
  Batch24又以三向 join 关闭46条 Lucide 映射，当时 UI=`73/79/152`；
  Batch25关闭 layout 组 detail-panel/page-shell/row-mark/stagger 四条，当时
  UI=`77/75/152`；Batch26又关闭orb/ai-core→AgentPresence两条，当前
  UI=`79/73/152`；Batch27又以唯一中性线稿关computer/placeholder与settings/background
  两条，当时 UI=`81/71/152`；Batch30又关闭独立ChannelRow；Batch31关闭
  abstract-avatar→Avatar与AgentCard；Batch32关闭RecipientField，当前UI=`85/67/152`；
  AppSidebar总项与其余32业务/brand/runtime/golden仍按各自证据保持 todo；
- [x] `design-gallery` compile feature 才有 `/_design`，production bundle WASM `_design` byte=0；
  当前画廊用于状态/键盘/AX/目视 QA，不冒充正式 golden；
- [x] Message 命名 article、neutral Bubble、跨平台 Kbd、SHA-256 deterministic Avatar、5s
  generation-safe polite Toast、400ms hover/focus/Escape Tooltip 已过真实 Chromium/AX；
- [x] Dialog/Sheet 共享 modal kernel：双向 focus trap、Escape/backdrop/return、scroll lock 与
  path-sibling inert；Sheet top/right/bottom/left closed side；
- [x] Menu compound：open/disabled `data-state`、根/子层 APG 全键位、500ms 多字符
  typeahead、disabled skip、exactly-once activation、outside dismiss 与不落 body 的双向 Tab；
- [x] MessageScroller：initial/following/free/anchored 三态，streaming resize、prepend reading
  offset、48px user anchor、generation-safe content-change settlement 与命名 log/live/button；
- [x] Combobox/Select 共用唯一 listbox 内核：editable filter/empty 与 select-only 500ms
  typeahead，committed/active 分离、Field 自动接线、命名 AX、exactly-once selection；
- [x] Sidebar：lg240/rail48、md auto rail、compact shared Sheet，Ctrl/Command+B、named nav/
  current、external trigger返焦与同一 children 单挂载；
- [x] §4.6.2 的46条Tabler→Lucide由design-lint做第一真源→icons.toml→UI ledger三向join；
  `IconBrandGoogleDrive` 因官方SVG/条款/provenance缺失保持唯一icon todo；
- [x] layout 组四条业务组件：PageShell 的960/1200/768闭集宽度与44px topbar、
  same-origin back/PageSection/Rows/Empty；RowMark 中性 vendor tile；Stagger 纯CSS 30ms/8cap；
  DetailPanel 由URL信号驱动四态，优先WAAPI、同token CSS fallback、reduce=0ms，关闭卸载并返焦；
- [x] `AgentPresence` 同时关orb/ai-core两上游文件：20px、四态Signal、完整环/单弧/双弧/
  danger环分形，thinking/speaking=1200ms、error=160ms×1，本地化AX，reduce全局静止；
- [x] `ComputerPlaceholderArt` 是唯一1200×800中性currentColor线稿，无gradient/filter/
  noise/shadow/defs/ID/remote/字面色；`ComputerPlaceholder` 只复用它，两入口均纯装饰AX隐藏；
- [x] AppSidebar 的sign-out生产依赖：`GET /api/me/session` 只回revocable，
  `POST /api/auth/sign-out` 以已验session+Origin只撤当前PG行并清cookie；UI helper只接受204；
- [x] AppSidebar 的roster realtime生产依赖：`channels.last_message*`为PG真源，
  `/api/channels/events`只发送不含member IDs的bounded提示；每帧回查当前membership，断线/错误/
  queue pressure均要求客户端重连并refetch，不把NOTIFY当真源；
- [x] ChannelRow：同源percent-encoded `/channel/:id`、中性装饰Avatar、name/last-message/
  localized relative time/current；真实50→52分页、可见字段搜索、socket refetch与nested hard reload绿；
- [x] `/agents`真实读面：GET list/detail只消费Server权威secret-free DTO；固定上游mine与
  `!mine && public`分组、144×180 AgentCard、URL-owned只读profile、404/error/close返焦；
  AppSidebar Agents destination已接。已有同名文字旁Avatar AX隐藏；Trunk最终CSS以根同源
  `/fonts/*`加载Inter。T-UI-0029/0030已勾，mutation/start与整条route/golden仍不勾；
- [x] `/channel/new`真实首发：静态route先于dynamic channel id；无recipient发送禁用且刷新零
  create；URL/hard reload可恢复hidden但有权的Agent；RecipientField复用唯一Combobox键盘模型。
  首发只按create channel→native BeginThreadRun→成功navigate；begin失败同channel/run-id重试，
  create响应未知禁止二次提交。release WASM浏览器实得52→52→53、四视口overflow0、AX/id/
  console绿；未实现的Enter提示已删除，完整Composer仍todo；
- [x] Composer draft/queue纯状态：固定上游10+16条逐项Rust移植；`Cow`保留no-op identity，
  single Agent、command prompt/chip/deferred action effect、busy park/settle/remove/一次turn合并与command
  首次顺序去重均闭合。Queue刻意只活在当前mount、不写PG；Batch35已把text-only busy park、逐条
  remove、busy→idle单次settle与Stop后drain接进production conversation，并以hard reload 1→0证明
  mount边界。sources/附件/per-channel draft/steer仍未落，故T-UI-0043/0123与channel route继续todo；
- [x] Channel conversation production slice：原子PG snapshot携history/foreground/active tail/cursor，
  EventSource从cursor接durable replay/live并在terminal后refetch；Message/Bubble/Scroller呈现durable
  user/assistant/tool activity，Enter/ShiftEnter/IME与idle send/无thread mint均接真实API。浏览器硬刷新
  历史不丢、四视口X/Y overflow0；raw error只按closed terminal本地化。Batch35又接actor-owned durable
  Stop→Cancelling→Cancelled、跨副本host cancel与mount-local queue/remove/settle。Markdown、完整tool
  boundary、sources/附件/per-channel draft/steer/Screen未落，故ChannelChat/ChatTranscript/
  ConversationView/Composer条目仍todo；
- [x] `/settings/memory`新增route：native 0022以tenant/actor独立持久化writesEnabled，缺行默认开启；
  disabled只拒绝GUI remember/correct与built-in remember tool，查看/recall/forbid/delete保持可用。
  页面以typed no-store API呈现50→52 owner keyset、status/kind/sensitivity/scope/source/origin/tags，
  correct生成replacement、forbid/delete擦除content；取消返原按钮，成功权威refetch后聚焦变更行。
  release CSS真实445规则，中英、1440/1024/900/600 overflow0、duplicate IDs/visible alerts/console均0；
- [x] `/settings` Preferences真实route：保留上游General/Theme，并按§7–§8增加system与locale；
  页面/Sidebar复用同一native0021 context/API。LocaleSwitch由调用点传唯一bounded ID，双实例零重复；
  快速theme+locale连续更新时页面唯一`role=status`，worker绑定AppShell稳定owner，locale重渲染不再
  取消receipt收尾；队列排空后status消失且reload保留合并值。Sidebar Settings真实导航、APG键盘、
  1440/1024/900/600 overflow0、console0；正式golden仍todo；
- [x] Settings secondary shell：`/settings`、Connected Accounts、Components Gallery与
  `/settings/memory`共用aside+named nav，`--size-subnav`实得200px；Back/General exact/Connected
  prefix/Gallery prefix/Memory exact按上游顺序且current恰1。既有四视口shell证据保持；
- [x] Connected Accounts index/detail：contract只传reviewed stable server id，PG要求管理员已add且
  Google Drive的url/vendor/provenance/transport与编译期identity逐字段相等；未知/custom不进入个人页。
  list/connect/disconnect均no-store，authorization receipt只接受同源根路径或安全HTTPS；full-page
  callback、Connected/Not connected、vendor实际scope/time、APG Menu与local-first disconnect已接。
  pending不冒充vendor revoked，权威refetch后返焦Connect；fixture不冒充真实Google OAuth。正式brand/
  golden、Desktop Local OAuth与restricted-scope发布验证仍todo；
- [x] Components Gallery index/detail只消费typed治理DTO：build manifest由Server所有，browser复述必须
  逐字段相等；PG additive sync首次published且insert+audit同事务，existing管理员治理零覆盖。当前登记
  `showQuote`、Cards四项、Charts五项与Activity共11个ordinary真实renderer；stale published有诚实
  fallback，unpublished按不存在。`GalleryFrame`遵守中性chrome与四值语义tone；Cards schema/preview保留
  独立tool identity，Checklist只读且semantic badge背景0。Axum/Tauri同一typed command。ordinary 11项
  已接fresh provider grant/decision、closed args、durable call/result/Agent三向配对与conversation runtime；
  Activity两种非空report的follow-up ask已逐字接唯一BeginThreadRun，绑定component真实Agent；busy禁用，
  首次503后Retry复用同一run id。Batch47为`askApproval`/`askChoice`建立独立native0023 durable
  request/list/answer/wait、closed authority/answer与Axum/Tauri typed控制面；Batch48再把两个Decision
  同批加入13项manifest/provider/schema/Leptos registry。Agent在answer前进入`AwaitingHuman`，回答只回
  `ExecutingTools`，durable exchange checkpoint后才resample；cancel waiter继续到PG写cancelled audit。
  conversation按actor/current run画pending，Approval/Choice从recorded result重建complete；Choice支持
  Enter/Space，默认文案与Input placeholder即时i18n。release实得approve/decline/Choice Enter与hard
  reload、中英、四视口、Gallery14 tile/2 Decision；CSS96050B，预算余2254B。Refused sandbox共用与
  formal golden仍todo；
- [x] Sandboxed component 已分两批接通可独立验收的子面：Batch49以SERIALIZABLE事务闭合admin
  draft/save/publish/delete、revision、sample与Axum/Tauri同一ApplicationService；Batch50再把当前
  published/未withheld定义接入per-Agent provider与call-time authorize，沙箱port结构上无data function，
  current JSON Schema/args与external `$ref`均fail-closed。Web production conversation与
  `/admin/playground`复用唯一`SandboxedComponentFrame`；iframe `sandbox`恰为`allow-scripts`、无srcdoc，
  source/args/capability只在fragment，Server runner逐响应32-byte nonce与exact CSP。custom/Tauri scheme
  直接复用RefusedCard且零iframe。release IAB实得Playground invalid sample抑制iframe、会话双拒绝卡、
  sandbox/srcdoc/query随机与DOM/overflow/console负面边界；但该环境没有可用MessageChannel/postMessage且
  Chrome不可用，所以args注入、作者JS、无网络/回调、channel正向握手和sample正向执行不提前标绿。
  当前components=`13/9/22`；Desktop独立Chromium renderer、帧流/input broker、
  CPU/内存硬隔离、具名a11y豁免、admin正式route journey/golden/AX仍todo。Batch51只补其前置
  HumanLease/epoch与closed BrowserInput，browser-operations=`7/39/46`、总parity=`693/993/1686`；没有
  Electron/CDP/ScreenHub实证，不改变上述Desktop renderer与a11y todo；
- [ ] AppSidebar总项仍不勾：production roster/current-user/session/sign-out与三断点同一children已落，
  new-channel/Agents/Memory/Settings已接，但skills/admin真实destinations尚未迁移；完整channel route也仍缺
  markdown/sources/attachments/per-channel draft/steer/screen，不得用已接Stop/queue冒充完整journey；
- [ ] 其余24个route journey、AppSidebar总项与其余未完成业务组件、1 brand icon、6 runtime替代、
  sandboxed正向执行/Desktop renderer、multi-window lifecycle/ACL及真实macOS/Windows binary
  尚未闭合；
- [ ] Web 110 + zh-CN 27 + Desktop 每平台 54 张 golden、完整 AX/键盘/reduced-motion 与三平台
  bundle 摘要尚未闭合；
- [ ] Tauri 图的 MPL-2.0×5、runtime UNIC unmaintained×5、Cargo Vet macOS 270/Windows 269
  仍红；不得把 bans/sources 已绿写成供应链整关已绿。
- [x] 2026-08-28 R123：`xtask bundle-budget` 的 `CSS_LIMIT` 96 → 128 KiB 并新增 `CSS_WARN` 120 KiB
  警戒（`crates/openbot-testkit/src/xtask/ui_gates.rs`），实跑结果见 v3 R123 证据列；本轮没有重新构建
  bundle，Batch 50 的 97,848 B 实测不变，不冒充新 bundle 证据；
- [ ] 2026-08-28 R118：Desktop sandboxed component 的 Electron component role 渲染路径与 §9.1 结构化参数
  fallback 尚未实现，T-CMP-0021（owner 已改 openbot-computer，R124）/ T-CMP-0022 继续 todo。

完整证据见 Batch16–18 文档、`docs/2026-08-25-G6-Dialog与Sheet-batch19.md`、
`docs/2026-08-25-G6-Menu原语-batch20.md` 与
`docs/2026-08-25-G6-MessageScroller原语-batch21.md`、
`docs/2026-08-25-G6-Combobox与Select原语-batch22.md` 与
`docs/2026-08-25-G6-Sidebar原语-batch23.md`、
`docs/2026-08-25-G6-图标映射三向join-batch24.md` 与
`docs/2026-08-25-G6-布局业务组件-batch25.md`、
`docs/2026-08-25-G6-AgentPresence-batch26.md`、
`docs/2026-08-25-G6-ComputerPlaceholderArt-batch27.md` 与
`docs/2026-08-26-G2-生产SessionSignOut-batch28.md`、
`docs/2026-08-26-G3-ChannelActivity与WebSocket-batch29.md`、
`docs/2026-08-26-G3-G6-ChannelDetail与ChannelRow-batch30.md`、
`docs/2026-08-26-G4-G6-AgentRoster与AgentsRoute-batch31.md`、
`docs/2026-08-26-G3-G6-ChannelCreate与Routing-batch32.md`、
`docs/2026-08-26-G6-ComposerDraft与Queue-batch33.md`、
`docs/2026-08-26-G3-G6-ChannelTranscript与IdleSend-batch34.md`、
`docs/2026-08-27-G3-G6-DurableCancel与Queue-batch35.md`、
`docs/2026-08-27-G3-G6-MemoryControls-batch36.md`、
`docs/2026-08-27-G6-SettingsPreferences-batch37.md`、
`docs/2026-08-27-G6-SettingsShell-batch38.md`、
`docs/2026-08-27-G4-G6-ConnectedAccounts-batch39.md`、
`docs/2026-08-27-G6-ComponentsGalleryQuote-batch40.md`、
`docs/2026-08-27-G6-GalleryCards-batch41.md`、
`docs/2026-08-27-G6-GalleryCharts-batch42.md`、
`docs/2026-08-27-G6-ComponentRuntime-batch43.md`、
`docs/2026-08-28-G6-GalleryActivityData-batch44.md`、
`docs/2026-08-28-G6-ComponentConversation-batch45.md`、
`docs/2026-08-28-G6-ActivityFollowUp-batch46.md` 与
`docs/2026-08-28-G6-ComponentHumanDecisions-batch47.md`、
`docs/2026-08-28-G6-ComponentDecisionsRuntime-batch48.md`、
`docs/2026-08-28-G6-SandboxedComponentGovernance-batch49.md` 与
`docs/2026-08-28-G6-WebSandboxRuntime-batch50.md`、
`docs/2026-08-28-G5-HumanLease与输入协议-batch51.md`；
G6 整关继续不勾。

---

## 16. 一手来源

- Tailwind CSS standalone CLI release v4.3.3：<https://github.com/tailwindlabs/tailwindcss/releases/tag/v4.3.3>
- trunk v0.21.14 源码（`src/tools.rs` 工具查找、`src/pipelines/tailwind_css.rs` 调用参数、`src/config/models/build.rs` offline）：<https://github.com/trunk-rs/trunk/tree/v0.21.14>
- leptos_i18n 0.6.2 与文档（配置 / 文件结构 / locale 解析）：<https://github.com/Baptistemontan/leptos_i18n>
- Lucide 1.33.0（ISC AND MIT，Feather 衍生子集）：<https://github.com/lucide-icons/lucide/releases/tag/1.33.0>
- Inter 4.1（OFL-1.1）：<https://github.com/rsms/inter/releases/tag/v4.1>
- WAI-ARIA Authoring Practices Guide（键盘模式）：<https://www.w3.org/WAI/ARIA/apg/patterns/>
- WCAG 2.2 对比度（1.4.3 / 1.4.11）：<https://www.w3.org/TR/WCAG22/#contrast-minimum>
- CDP `Emulation.setEmulatedMedia` / `Accessibility.getFullAXTree` / `Page.captureScreenshot`：<https://chromedevtools.github.io/devtools-protocol/>

---

## 17. 计数复算命令

上游（在 `891df72f…` 干净克隆的 `app/src` 执行）：

```bash
find routes -name '*.tsx' | wc -l                                                   # 31
find routes -name '*.tsx' | grep -vE '(__root|_authed|_app|route)\.tsx$' | wc -l    # 26（页面）
ls components/ui | wc -l                                                            # 21
find components -name '*.tsx' -not -path 'components/ui/*' | wc -l                  # 45
grep -rhoE 'className=' --include=*.tsx . | wc -l                                   # 852
grep -rhoE '\bIcon[A-Z][A-Za-z0-9]+' --include=*.tsx --include=*.ts . | sort -u | wc -l   # 47
awk '/^:root/{r=1;next} r&&/^}/{exit} r' styles.css | grep -cE '^\s*--'             # 33
awk '/^\.dark/{r=1;next} r&&/^}/{exit} r' styles.css | grep -cE '^\s*--'            # 32
wc -l < styles.css                                                                  # 249
grep -rhoE '\b(sm|md|lg|xl|2xl):[a-z]' --include=*.tsx . | wc -l                    # 24
grep -rhoE 'aria-[a-z]+' --include=*.tsx . | wc -l                                  # 119
grep -rhoE '\brole=' --include=*.tsx . | wc -l                                      # 55
grep -rlE 'useTranslation|i18next|react-intl|next-intl|<Trans\b' --include=*.tsx --include=*.ts . | wc -l   # 0
grep -c 'prefers-color-scheme' lib/theme.ts components/theme-provider.tsx styles.css   # 各 0（不跟随系统）
grep -c 'prefers-reduced-motion' styles.css                                         # 1
for p in '@base-ui/react' motion prompt-area boring-avatars streamdown '@tabler/icons-react'; do grep -rlE "from [\"']$p" --include=*.tsx --include=*.ts . | wc -l; done   # 13 8 4 3 1 32
grep -cE '"@shikijs/core@' ../../bun.lock                                           # 1（3.23.0）
```

本文件自身（在仓库根目录执行）：

```bash
D='docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md'
grep -oE '\| Icon[A-Za-z0-9]+ \|' "$D" | sort -u | wc -l                           # 47（§4.6.2 映射表，含 1 品牌标）
grep -E '^新增图标：' "$D" | grep -oE '`[a-z0-9-]+`' | wc -l                        # 28（新增图标）
sed -n '/^### 4.2/,/^### 4.3/p' "$D" | grep -cE '^\| `'                              # 20（§4.2 token 行）
```

§4.2 对比度复算（与 `openbot-ui` 将来的 `token_contrast_wcag_aa` 同一公式；2026-08-22 实跑输出 `failures: 0`）：

```bash
python3 - <<'EOF'
def lum(h):
    r,g,b=(int(h[i:i+2],16)/255 for i in (1,3,5))
    f=lambda c: c/12.92 if c<=0.03928 else ((c+0.055)/1.055)**2.4
    return 0.2126*f(r)+0.7152*f(g)+0.0722*f(b)
def cr(a,b):
    x,y=sorted((lum(a),lum(b)),reverse=True); return (x+0.05)/(y+0.05)
L={'bg':'#FFFFFF','bg-subtle':'#F4F4F5','bg-chip':'#EFEFF1','bg-sidebar':'#F7F7F8','bg-popover':'#FFFFFF','bg-inverse':'#111111',
   'fg':'#1A1A1A','fg-secondary':'#5B5B63','fg-muted':'#68686F','fg-inverse':'#FFFFFF','ring':'#1A1A1A',
   'danger':'#B42318','caution':'#9A5100','success':'#067647','info':'#1552C5',
   'chart-1':'#3B5BDB','chart-2':'#0CA678','chart-3':'#E8590C','chart-4':'#AE3EC9','chart-5':'#868E96'}
D={'bg':'#141415','bg-subtle':'#1F1F22','bg-chip':'#27272B','bg-sidebar':'#0F0F10','bg-popover':'#1C1C1F','bg-inverse':'#F4F4F5',
   'fg':'#ECECEE','fg-secondary':'#A6A6AF','fg-muted':'#8E8E98','fg-inverse':'#111111','ring':'#F4F4F5',
   'danger':'#F97066','caution':'#FDB022','success':'#47CD89','info':'#84ADFF',
   'chart-1':'#748FFC','chart-2':'#38D9A9','chart-3':'#FFA94D','chart-4':'#DA77F2','chart-5':'#ADB5BD'}
text=['fg','fg-secondary','fg-muted']; bgs=['bg','bg-subtle','bg-chip','bg-sidebar','bg-popover']
pairs=[(t,b,4.5) for t in text for b in bgs if not (t=='fg-muted' and b=='bg-popover')]
pairs+=[('fg-inverse','bg-inverse',4.5)]
pairs+=[(s,b,4.5) for s in ('danger','caution','success','info') for b in (('bg','bg-chip','bg-sidebar') if s=='danger' else ('bg','bg-chip'))]
pairs+=[('ring',b,3.0) for b in ('bg','bg-chip','bg-sidebar')]+[('border','bg',1.0)]+[(f'chart-{i}','bg',3.0) for i in range(1,6)]
bad=0
for name,T in (('light',L),('dark',D)):
    T=dict(T,border={'light':'#E4E4E7','dark':'#2E2E33'}[name])
    for f,b,m in pairs:
        c=cr(T[f],T[b]); bad+= c<m
print('pairs:',2*len(pairs),'failures:',bad)
EOF
```

对不上 = 上游 commit 变了或本文件漂了 → 先核 `git rev-parse HEAD`，再按 v3 §1.2 走 delta audit。
