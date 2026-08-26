# Batch 22 WIP 恢复点：Combobox 与 Select 共享 listbox 内核

> 分支 `codex/2026-08-25-G6-combobox-select-primitives`，base = Batch21 正式 head
> `88b4dcf5ca23ae4e4d7d7dbe51f608af04b13033`。只跑本地定向测试，不运行
> `cargo xtask ci`，不派发 Actions，不处理 `grok-bot`。

## 本批范围

- [x] 唯一 listbox context/navigation/selection/typeahead/dismiss 内核，Combobox/Select 不复制；
- [x] editable Combobox：native text editing 不被劫持，输入过滤、empty、Arrow/Home/End、Enter、Esc；
- [x] select-only Select：Arrow/Home/End、Enter/Space、Esc cancel、Tab commit、500ms typeahead；
- [x] owner 保持 DOM focus；listbox/option 通过 aria-activedescendant、aria-selected、tabindex=-1；
- [x] committed value 与 active suggestion 分离；Escape 不改旧值，选择 callback exactly once；
- [x] disabled/invalid/open/focus closed data-state 与 native/ARIA 同步；disabled option 全路径跳过；
- [x] click、keyboard、outside、Tab 后 popup/active/focus/value 语义精确；
- [x] design-gallery + Chromium/AX/过滤/无结果/中英文匹配证据。

## 第一源裁决

固定上游 `combobox.tsx` 还导出 chips/multi-select，`select.tsx` 还导出滚动箭头；但 GUI 第一真源
§6.1 只把两条定义为 single-value combobox/listbox 与 select-only combobox，固定产品消费面也只有
channel/new 单 recipient 与三处单选表单。本批不擅自扩大为 chips/multi-select；长列表 native
overflow 可滚，但独立 scroll-arrow presentation 不是 ledger 完成条件。

## 不在本批

Composer `@coworker`/`/skill` 业务接线、channel/new route、credential/agent form、远程数据加载与
正式 golden；两原语完成不冒充这些业务组件/routes 或 G6 整关完成。

## 当前机器证据

- UI all-features=`49/0/0`；WASM all-features、UI/testkit all-targets Clippy `-D warnings`、
  fmt 全绿；
- i18n=`342` keys、design=`44 Rust/74 icons`、css=`126 classes`；production bundle
  wasm gzip=`375269`、CSS=`51695`、fonts=`740216`、external/inline=`1/0`；production/gallery
  WASM 的 `_design` 字节=`0/1`；
- Chromium 当前 bundle：Combobox=`13/13`、Select=`14/14`、final smoke=`10/10`。Combobox
  ArrowDown 首项、disabled skip、双向 wrap、Home/End、Enter/click exactly once；Ada 过滤只剩
  一项，`qqq` 得 empty+active null，Escape/outside/Tab 不提交且恢复旧 label；中文 `张` 命中张三；
- Select committed value 与 active 分离；Escape 保留，英文 `p`+`u` 命中 Public，550ms 后
  `p` 重置命中 Private；Enter/Space/click 各只增一次，Tab commit active；outside cancel；
  中文 prefix/contains 由同一纯 helper 的 `所有`/`张` 单测覆盖；
- DOM focus 始终在 owner，active-descendant target 存在；命名 combobox→命名 listbox，option
  `tabindex=-1`、active aria-selected、disabled native+ARIA exact；
- Field 嵌套实得 label for=owner exact、invalid described-by=`design-combobox-invalid-error`、
  disabled Select native=true；editable root/input 宽同为430.5，Select 保持紧凑；
- aria-controls target missing/active-descendant target missing/duplicate ID/unnamed/nested/
  remote/overflow=`0/0/0/0/0/0/0`，console error=0；fixture 重启后确认 index/CSS/WASM
  同批 hash，不混用缓存旧 CSS；
- 最终 gallery WASM SHA-256
  `5d8719194c250dbc6b1f8091e618de14ff9f47028a209a714d2313517123ca7a`；Cargo.lock
  package delta=0。
