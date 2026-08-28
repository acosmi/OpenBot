# Batch 37 WIP：Settings Preferences 与 `/settings`

> 日期：2026-08-27。分支`codex/2026-08-27-G6-settings-preferences`；base为Batch36证据head
> `f2fda5f`。固定上游`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 只跑本机定向测试；不运行`cargo xtask ci`，不派发Actions，不修改/暂存/提交既有
> `docs/assets/`。

## 第一真源裁决

- 固定上游`settings/index.tsx`只有Preferences/General下的Dark theme；设计第一真源§7把
  `system`第三态与Server/Desktop持久化裁决为新增，§8又要求`en/zh-CN`即时切换；
- Batch16已经闭合closed preference contract、PostgreSQL native0021、Server GET/PUT/cookie、
  Desktop Local与共享reactive context；本批不造第二套偏好存储或API；
- `/settings`复用唯一`ThemeToggle`/`LocaleSwitch`。两组件同时存在于Sidebar与页面，因此
  LocaleSwitch必须由调用点提供唯一bounded DOM前缀，不能保留全局固定ID；
- 主题/语言立即作用于当前DOM，再经既有serialized/coalescing PUT持久化；失败必须在当前可见面
  只出现一个localized`role=alert`。在`/settings`时由页面呈现，Sidebar不重复播报；
- 页面description按真实scope写“本deployment内跨设备”，不照抄上游“every deployment”而伪造
  native0021的deployment-scoped PK语义；
- AppSidebar只在真实route存在后添加`/settings` destination；settings二级Sidebar/layout仍未实现，
  不借本页冒充`T-ROUTE-0005`已完成；正式golden仍不冒充。

## 本批实施范围

1. 参数化LocaleSwitch DOM identity并加重复实例负向/正向测试；
2. `SettingsPage`：PageShell/Header/General，Theme与Language两条静态control row；
3. `/settings` production/design-gallery route与AppSidebar Settings destination；
4. en/zh-CN、token-only CSS、deterministic fixture浏览器journey；
5. UI/WASM/Clippy/i18n/design/CSS/bundle、四视口、hard reload persistence、parity/recount；
6. 证据成立后只关闭`T-ROUTE-0026`；`T-UI-0150`仍因正式golden矩阵todo。

## 明确不在本批冒充

- 不实现settings二级Sidebar、connected accounts/components gallery/computer其余route；
- 不改变native0021 schema或preferences API；
- 不处理AppSidebar skills/admin destination、完整Composer/Markdown/Screen；
- 不把浏览器目视截图写成正式Web/Desktop golden。

