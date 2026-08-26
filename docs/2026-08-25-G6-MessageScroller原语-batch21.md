# G6 Batch 21：MessageScroller 阅读位置与流式跟随

> 日期：2026-08-25。分支 `codex/2026-08-25-G6-message-scroller-primitive`，base =
> Batch20 正式 head `9b0d6c0a3907f139ea2237cd179464ad9a2c85bb`，实现 checkpoint
> `521873c3449a8bd68d541383eb385cc063348897`。本批关闭 MessageScroller 一条 UI
> ledger；不代表 ChatTranscript、thread route 或 golden 完成。

## 1. 已完成并打勾

- [x] T-UI-0011 MessageScroller。

UI `23/129/152 → 24/128/152`；全 parity
`459/1213/1672 → 460/1212/1672`。其余 ledger/fixtures 不变。

## 2. 锁定契约与第一性裁决

固定上游 `bun.lock` 钉 `@shadcn/react@0.3.0`。本机只读下载官方 tgz，SHA-512 base64
逐字等于 lock integrity `iKN0…WkWqQ==`，然后实读其发布 JS/d.ts 与上游 ChatTranscript 调用。
第一真源的“自动贴底 + 回到底部 + 保持阅读位置”具体落为：

- `FollowingBottom`：initial end；append 和最后一行 streaming resize 后仍为自然 end；
- `FreeScrolling`：wheel/touch/Arrow/Page/Home/End/Space 或真实 scrollbar 意图立即让出；append、
  nested content growth 都不改 scrollTop；手动回 end 自动恢复 following；
- `AnchoredToMessage`：新 user `scroll_anchor` 对齐到 viewport top+48px；后续 reply growth
  保持该 offset，直到自然输出越过 viewport 才显示 end button；
- prepend 不按“新增高度”盲补。每次稳定态记录首个可见 item 与 viewport offset；变更后只补
  实际 offset 差，所以 Chromium 自带 scroll anchoring 开/关都不会双补；
- 初次非空 render 把所有 mount-time anchor 一次登记；same-count replacement 不会把阅读者
  拉回旧 anchor。整批 IDs 全换则视为 transcript replacement，重新 initial end；
- `MessageScrollerController::scroll_to_end` 是业务层唯一 imperative escape hatch，内建 button
  走同一函数；button 在不可向 end 滚动时 hidden，因而不占 Tab/AX；
- viewport 是命名、可聚焦 `region tabindex=0`；content 是同名
  `role=log aria-live=polite aria-relevant="additions text" aria-atomic=false`；layout spacer
  永久 `aria-hidden`。

## 3. 两个实测后修正的缺陷

1. 初版 prepend 按 `scrollHeight` delta 补偿。Chromium 已先做 native anchoring，再补一次让
   message-2 跳成 message-3。改为 reading-item viewport offset 后，prepend 实得 top
   `100→171`，但 item/offset 恒 `message-2/-30`。
2. 旧流式行收缩与 append 同 commit 时，宿主 clamp 产生 scroll event，被初版误判为用户。
   现在 Mutation/Resize callback 在 rAF 前置 `content_change_pending`，本组件 scroll 另有锁定包
   同值 180ms generation-safe settling；真实 wheel/touch/key 仍能立即取消。最终 append 后
   再增长实得自然 end=0。

Observer 只在真实尺寸/child-list 变化时调度；rAF 合并。anchor spacer 从总高减已知 spacer
复算 natural height，同值不写 DOM，避免 ResizeObserver 自激环。item 遍历直接消费
`HtmlCollection` 给出的 `Element`，不做 parent-realm `instanceof HTMLElement`，避开固定库已知的
same-origin iframe realm 误判形状。

## 4. 本机证据

| 面 | 结果 |
| --- | --- |
| UI all-features tests | 47/0/0 |
| UI all-features WASM | 绿 |
| UI/testkit all-targets Clippy `-D warnings` | 绿 |
| i18n/design/css | 327 keys；41 Rust/74 icons；109 classes |
| production bundle | WASM gzip 370410；CSS 48046；fonts 740216；external/inline 1/0 |
| production/design gallery | `_design` bytes 0/1；gallery SHA-256 `bf75ce978f9b14253bdcd13489a6420aa1fe2b5c196071852b675e4dcce458c2` |
| parity/recount | 460/1212/1672；0 violation/warning；157/157/0 |

真实 Chromium 最终两组断言 `9/9 + 8/8`：

- initial `scrollTop/scrollHeight/clientHeight=360/680/320`、end=0；真实 wheel 后 top=100，
  end button active；free append 与 resize 的 top/first/offset 全不变；
- prepend 后 scrollTop 增 71，但 visible item/offset 不变；same-count tail swap 的
  top/first/offset 仍为 `100/message-2/-30`；
- jump 清 spacer、end=0；following 状态先收缩旧流式行再 append、再增长新回复，两步均 end=0；
- PageUp 意图即使宿主测试驱动没有原生移动，也阻止下一 append 抢滚动；
- user anchor offset 恒48；四次 reply 后 spacer `226→155→85→14→0`，button 只在自然内容
  真正越过 viewport 后出现；
- AX named region→log/button exact；duplicate ID/unnamed/nested interactive/remote resource/
  overflow=`0/0/0/0/0`，console error=0。

Cargo.lock package delta=0；只扩已锁 web-sys 的 DomRect/ResizeObserver/MutationObserver API
feature。六 target bans/sources 本批复跑全绿；MPL×5、runtime UNIC unmaintained×5 与 Cargo Vet
macOS 270/Windows 269 仍红。

## 5. 仍未完成

- [ ] Combobox / Select；
- [ ] Sidebar（含 md rail 与 `<md` Sheet 集成）；
- [ ] ChatTranscript/Composer/Thread route 与完整 streaming event projection；
- [ ] 45 业务组件、31 routes、正式 golden/AX 全矩阵；
- [ ] Tauri release identity/binary/window lifecycle 与 MPL/UNIC/Vet。

未运行 `cargo xtask ci`，未派发 Actions，未 push/建 PR，未处理 `grok-bot`。
