# Batch 37：Settings Preferences 与稳定偏好保存队列

> 日期：2026-08-27。分支：`codex/2026-08-27-G6-settings-preferences`。
> base：Batch36证据head `f2fda5f`；
> implementation：`5babb78483d0083085047b21760dbc963a418383`。
> 固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批只关闭`/settings` Preferences核心journey与AppSidebar Settings真实destination。未运行
`cargo xtask ci`、未派发Actions、未生成正式golden；settings二级Sidebar/layout、connected accounts、
components gallery、computer与AppSidebar skills/admin仍todo。既有未跟踪`docs/assets/`未修改、
未暂存、未提交。

## 1. 排序裁决与外部边界

Batch36后，第一真源唯一阻止G3的actual legacy exporter/三次production-scale drill需要经合同/法务
许可的customer export/API。对固定上游源码复核得到的合法读取面只有按已知`(threadId,userId)`的
`getThread`与单thread messages；没有thread枚举、semantic event或observable memory export。
因此不能猜managed private endpoint，也不能用合成数据冒充真实production drill。

在无需外部输入的缺口中：Approval PG浏览器需要重建pending run/lease/tool binding；Markdown完整闭环
还依赖Desktop外链宿主；Settings已有Batch16的closed contract、native0021、Server/Desktop persistence与
共享控件，可以一次闭合真实route，故按依赖最小且证据可完备优先实施。

## 2. 第一真源不变量

1. 固定上游`settings/index.tsx`只有Preferences/General/Dark theme；本项目保留该journey；
2. `system`第三态与`en/zh-CN`是设计第一真源§7–§8明确新增，不冒充上游parity；
3. 页面复用唯一`UiPreferenceContext`、GET/PUT `/api/me/preferences`与native0021，不造第二store；
4. native0021 PK含deployment/tenant/actor，页面不能照抄上游“every deployment”，只能声明
   “当前deployment内跨设备”；
5. ThemeToggle按第一真源同时存在于settings页与Sidebar；LocaleSwitch双实例必须有不相交DOM ID；
6. theme/locale即时作用DOM，再经serialized/coalescing PUT；pending与failure都不得静默；
7. `/settings`显示pending/error时，Sidebar不能重复播报同一live region；
8. Settings route存在后才允许Sidebar画Settings link；二级layout与golden继续独立todo。

## 3. 实施

### 3.1 SettingsPage

- `SettingsPage`使用`PageShell(Content)`、唯一h1、General h2与两条静态control row；
- Theme row复用三态`ThemeToggle`；Language row复用`LocaleSwitch`；
- 中英文description按真实authority scope表达；
- production与design-gallery route set均注册`/settings`；
- AppSidebar新增`Icon::Settings` destination，current只在exact `/settings`成立，不与
  `/settings/memory`双current。

### 3.2 双实例DOM identity

LocaleSwitch原先固定使用`locale-switch-label/current`，在Sidebar+page同时挂载时会重复。
组件改为要求调用点传入不超过96字节、只含ASCII alnum/`-`/`_`的ID前缀，并派生label/current/menu
关系；Sidebar=`sidebar-locale-switch`，页面=`settings-locale-switch`。单测同时覆盖两family不相交与
selector/control字符拒绝。

### 3.3 浏览器实测发现并修复稳定owner缺陷

初版快速连续theme+locale后，PostgreSQL/fixture权威GET已是`dark/en`，但页面
`Saving preferences`永久不消失。根因不是服务端：`enqueue`从ThemeToggle事件创建
`spawn_local_scoped_with_cancellation`，任务绑定触发控件的child reactive owner；locale切换重渲染该
owner，receipt之后的`self.saving.set(false)`被取消。

修订后`provide_ui_preferences`在AppShell层捕获stable `Owner`，所有保存worker都通过该owner启动；
child重渲染不再影响worker。`PreferenceSaveStatus`在pending时给唯一localized`role=status`，完成后卸载；
`/settings`由页面呈现，Sidebar在该path抑制自己的副本。fixture的preference update固定延迟1秒，
使serialized第二次PUT、pending可见与队列排空不依赖浏览器控制往返速度。

## 4. 本机机械证据

| 面 | 结果 |
| --- | --- |
| UI all-features | **110 / 0 / 0** |
| Clippy | UI all-targets/all-features + `openbot-ui-fixture` Server bin，`-D warnings`通过 |
| WASM | UI `wasm32-unknown-unknown`通过 |
| i18n / design / CSS | **455** leaf；**73 Rust / 74 icons**；**221** source class literals |
| release bundle | WASM gzip **885,190 B**；CSS **74,670 B**；fonts **740,216 B**；external/inline **1/0** |
| parity | routes **3/29/32**；总计 **643/1035/1678**；fixtures **16/22/38** |
| strict fixed-upstream recount | **157 / 157 / 0** |
| parity violations | **0** |

最终release CSS为`app-639e953024b6222f.css`，浏览器加载455条规则；Inter Variable生效。

## 5. release WASM浏览器

- Sidebar Settings link从`/settings/memory`真实导航到`/settings`，URL、h1与`aria-current=page`一致；
- 页面+Sidebar恰2个radiogroup、2个locale switch；duplicate IDs=0；
- Dark点击后两处Dark均checked、`html.dark`即时成立；队列排空后hard reload仍为Dark；
- English切换后两处trigger与h1/description即时英文，reload后`html lang=en`且Dark不丢；
- Theme ArrowLeft/Home/End与Locale Enter/ArrowDown/Enter通过，选择后焦点返页面trigger；
- 1秒fixture下快速System+English：即时DOM改变，页面唯一`Saving preferences`，全页status总数1；
  两次PUT排空后status=0、alerts=0，reload后两处System/English均保持；
- 1440×900、1024×640、900×640、600×640：X overflow=0，main/nav/h1各1，row=2、
  页面controls visible=2、duplicate IDs=0、visible alerts=0；
- 宽屏与600px深色截图目视结构、层级、间距和折行正常；console error/warn=0。

## 6. 台账与未完成边界

- 关闭`T-ROUTE-0026`；routes从2/30/32变为3/29/32；
- `T-ROUTE-0005`要求settings二级Sidebar/layout，本批没有实现，继续todo；
- `T-UI-0150`是正式Web/Desktop golden矩阵，本批截图只作目视QA，继续todo；
- connected accounts/gallery/computer、AppSidebar skills/admin、完整Composer/Markdown/Screen、
  Approval PG浏览器及G4/G5/G6/G7/G8其余项继续todo；
- actual legacy exporter/三次production drill等待经许可customer API/数据，不以私有协议猜测代替；
- 浏览器tab与fixture进程已关闭；固定上游临时clone仅用于strict recount，证据固化后删除。

