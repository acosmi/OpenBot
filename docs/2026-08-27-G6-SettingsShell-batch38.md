# Batch 38：Settings Secondary Shell

> 日期：2026-08-27。分支：`codex/2026-08-27-G6-settings-shell`。
> base：Batch37证据head `229ca1f`；
> implementation：`c1cf2f073e445803f536e3f9c0b75d0404fa48a1`。
> 固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批只关闭settings pathless layout的可观察journey。未运行`cargo xtask ci`、未派发Actions、
未生成正式golden；connected accounts/components gallery/computer等页面与`settings-sidebar`业务/golden
子账继续todo。既有未跟踪`docs/assets/`未修改、未暂存、未提交。

## 1. 固定上游与裁决

固定上游`settings-sidebar.tsx`的结构是：Back to app、General(exact)、Connected accounts、
Components gallery；GUI第一真源§5.1把settings/admin二级侧栏宽度固定为200px。

当前production settings destinations只有：

- `/settings`：Batch37 Preferences；
- `/settings/memory`：Batch36新增Memory Controls。

Connected accounts/gallery尚无页面。复制上游全部菜单会制造两个断链；因此本批只渲染已存在的General
和Memory，后者明确标为新增。layout的核心是稳定二级导航与共同容器，不是菜单文字数量；未实现项继续
留在各自route，不用假链接冒充进度。

## 2. 实施

- 新增`SettingsShell`，只包裹`SettingsPreferencesRoute`与`SettingsMemoryRoute`；
- 全局App shell仍是唯一`main`，secondary shell只用`aside`+named`nav`+content div；
- Back to app为`/`；General exact current，Memory只在`/settings/memory` current；
- nav只含三条bounded same-origin绝对路径，不接自由href；
- ≥768px使用`--size-subnav`的200px+content双列，subnav sticky并有边界；
- <768px堆叠为单列，nav横向换行，仍保持X overflow=0；
- CSS只消费已有token，无字面颜色/阴影/新z-index；
- 单测固定真实destination集合恰General/Memory，并证明General不会在Memory页双current。

## 3. 本机机械证据

| 面 | 结果 |
| --- | --- |
| UI all-features | **111 / 0 / 0** |
| Clippy / WASM | UI all-targets/all-features `-D warnings`、UI WASM通过 |
| i18n / design / CSS | **456** leaf；**74 Rust / 74 icons**；**227** source class literals |
| release bundle | WASM gzip **892,016 B**；CSS **76,263 B**；fonts **740,216 B**；external/inline **1/0** |
| parity | routes **4/28/32**；总计 **644/1034/1678**；fixtures **16/22/38** |
| strict fixed-upstream recount | **157 / 157 / 0** |
| parity violations | **0** |

最终release CSS为`app-b80cf9b5fcd08b84.css`，浏览器加载463条规则，Inter Variable生效。

## 4. release WASM浏览器

- `/settings`：secondary width实得200px；链接恰返回应用/通用/记忆，断链目标数0；
- General current恰1，Memory current为0；main=1、nav=2、h1=1、duplicate IDs=0；
- secondary Memory点击到`/settings/memory`，h1=记忆、50条owner memory仍加载，current只剩Memory；
- Memory hard reload后shell/current/h1保持；再点General回Preferences且只General current；
- Back to app进入`/`的真实Approval route，secondary shell数量从1变0；
- 1440/1024/900双列实得200px subnav；600单列堆叠，General与Memory页面X overflow均0；
- 四视口current=1、main=1、nav=2、h1=1、duplicate IDs=0、visible alerts=0；
- 1440深色Settings与600深色Memory截图目视层级、边界、current、折行正常；console error/warn=0。

## 5. 台账与未完成边界

- 关闭`T-ROUTE-0005`；routes从3/29/32变为4/28/32；
- `T-ROUTE-0029/0030` connected accounts、`T-ROUTE-0027/0028` gallery及其它settings页面仍todo；
- `settings-sidebar`业务组件、`T-UI-0150/0152`正式golden仍todo；
- AppSidebar skills/admin、完整Composer/Markdown/Screen、Approval PG浏览器与其余G4–G8继续todo；
- 浏览器tab/fixture已关闭。

