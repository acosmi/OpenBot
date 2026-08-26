# G6 Batch 22：Combobox 与 Select 共享 listbox 内核

> 日期：2026-08-25。分支 `codex/2026-08-25-G6-combobox-select-primitives`，base =
> Batch21 正式 head `88b4dcf5ca23ae4e4d7d7dbe51f608af04b13033`，实现 checkpoint
> `088625c78a97dc83f8a9190d38fdee09b450b3e1`。本批关闭 Combobox/Select 两条 UI
> ledger；不代表 Composer、channel/new、业务表单 route 或 golden 完成。

## 1. 已完成并打勾

- [x] T-UI-0002 Combobox；
- [x] T-UI-0014 Select。

UI `24/128/152 → 26/126/152`；全 parity
`460/1212/1672 → 462/1210/1672`。其余 ledger/fixtures 不变。

## 2. 唯一共享内核

两组件不各写一份近似键盘逻辑。唯一 `ListboxContext` 持有 open/value/query/committed label/
active-descendant/empty、Field 状态、owner/content refs、selection callback 与 500ms typeahead。
共享代码负责：

- bounded root/option IDs 与 control-free value；popup/listbox、option、dismiss；
- visible/enabled option 发现，disabled skip，↑↓ wrap、Home/End、active scroll containment；
- committed value 与 active suggestion 分离；option click/keyboard 选择只走一次 callback；
- owner 保持 DOM focus，`aria-activedescendant` 指向 `tabindex=-1 role=option`；
- named listbox、active `aria-selected`、committed check、disabled native+ARIA；
- open/disabled/invalid/focus closed `data-state`；outside/Escape/Tab 的 close/value/focus 边界。

Combobox 只增加 editable 差异：native `<input>`、`aria-autocomplete=list`、contains filter 与 empty；
Left/Right 等系统文本编辑键不在 handler match 中。Select 只增加 select-only 差异：button-like
combobox、Space、可打印字符 prefix typeahead、Tab commit active。Escape 两者都不提交 active。

## 3. 第一源裁决与构造性修正

- 固定上游 Combobox 还导出 chips/multi-select，但 GUI 第一真源 §6.1 只定义 single-value
  combobox/listbox，固定产品也只消费单 recipient。本批不反向扩大输入面；
- 上游 Select 的独立 scroll arrows 是长列表 presentation；本实现 popup native overflow 可滚，
  不把箭头复制成第二套滚动状态；
- Field 的 control ID 是唯一真源。owner 直接使用 root ID，不加 `-input/-trigger`；嵌套 Field
  时 ID 必须 exact，disabled/invalid/described-by 自动合并。Chromium 实得 label for exact；
- 初版 popup 用 owner `aria-labelledby`，Chromium AX 仍给 unnamed listbox。共享 context 现在接
  同一 reactive owner label 并直接给 popup `aria-label`，不是复制文案；
- editable 与 select-only 由 closed `data-kind` 分辨：Combobox 占满父容器，Select 保持紧凑。
  最终 gallery input/root 均 430.5px；
- active option 只手动调整 popup scrollTop，不调用 `scrollIntoView`，因此不会顺带滚动外层页面。

## 4. 本机证据

| 面 | 结果 |
| --- | --- |
| UI all-features tests | 49/0/0 |
| UI all-features WASM | 绿 |
| UI/testkit all-targets Clippy `-D warnings` | 绿 |
| i18n/design/css | 342 keys；44 Rust/74 icons；126 classes |
| production bundle | WASM gzip 375269；CSS 51695；fonts 740216；external/inline 1/0 |
| production/design gallery | `_design` bytes 0/1；gallery SHA-256 `5d8719194c250dbc6b1f8091e618de14ff9f47028a209a714d2313517123ca7a` |
| parity/recount | 462/1210/1672；0 violation/warning；157/157/0 |

真实 Chromium（fixture 在最终 build 后重启，index/CSS/WASM 同批 hash）：

- Combobox `13/13`：ArrowDown 首项、disabled skip、双向 wrap、Home/End；Enter/click 各增一次；
  Ada 只剩一项，`qqq` 得 empty+active null；Escape/outside/Tab 不提交并恢复旧 label；中文
  `张` 命中张三；
- Select `14/14`：committed 与 active 分离；Escape retain；英文 `p`+`u`→Public，550ms 后
  `p`→Private；Enter/Space/click 各增一次；Tab commit active；outside cancel；
- final smoke `10/10`：两 popup 均为命名 listbox；active/disabled option AX exact；Field invalid
  described-by 与 disabled propagation exact；editable full width；
- 四个 aria-controls target missing=0，active-descendant target missing=0，所有 option
  tabindex=-1；duplicate ID/unnamed/nested interactive/remote resource/overflow=
  `0/0/0/0/0`，console error=0。

Cargo.lock/package/dependency delta=0；UI dependency guard 与六 target bans/sources 本批复跑绿；
MPL×5、runtime
UNIC unmaintained×5 与 Cargo Vet macOS 270/Windows 269 仍红。

## 5. 仍未完成

- [ ] Sidebar（含 md rail 与 `<md` Sheet 集成）；
- [ ] Composer `@coworker`/`/skill`、channel/new、credential/agent forms 的业务接线；
- [ ] 45 业务组件、31 routes、正式 golden/AX 全矩阵；
- [ ] Tauri release identity/binary/window lifecycle 与 MPL/UNIC/Vet。

未运行 `cargo xtask ci`，未派发 Actions，未 push/建 PR，未处理 `grok-bot`。
