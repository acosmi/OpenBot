# Batch 38 WIP：Settings Secondary Shell

> 日期：2026-08-27。分支`codex/2026-08-27-G6-settings-shell`；base为Batch37证据head
> `229ca1f`。固定上游`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 本机定向测试；不运行`cargo xtask ci`，不派发Actions，不触碰`docs/assets/`。

## 第一真源裁决

- 固定上游`settings-sidebar.tsx`结构为Back to app + General(exact) + Connected accounts +
  Components gallery；GUI第一真源§5.1要求admin/settings二级侧栏宽200px；
- 当前真实settings destinations只有`/settings`与新增`/settings/memory`。本批只画这两条；
  connected accounts/gallery尚无production route，不能复制上游文字制造断链；
- Memory是31 route外新增settings页面，secondary nav纳入它是新增，不冒充上游parity；
- SettingsShell只包裹`/settings`和`/settings/memory`，不改全局AppSidebar、不嵌套第二个main；
- secondary容器用`--size-subnav`单源200px、named nav、same-origin link、exact current；
  `<768px`虽非最低承诺目标，仍堆叠为横向/换行nav，必须保持X overflow=0；
- 本批只关闭layout`T-ROUTE-0005`；connected accounts/gallery route、formal golden与
  `settings-sidebar`业务组件/golden子账仍独立todo。

## 实施与验收

1. `SettingsShell`：Back to app、General、Memory，唯一named secondary nav；
2. SettingsPreferencesRoute/SettingsMemoryRoute共用同一shell；
3. token-only CSS与current/focus/compact布局；
4. UI/WASM/Clippy/i18n/design/CSS/bundle；
5. release浏览器：两route间导航、back、hard reload、current唯一、四视口、landmark/heading/ID/
   console；
6. 证据成立后更新R101、两份第一真源、CLAUDE、route台账、移交指南与正式Batch38文档。

