# Batch 21 WIP 恢复点：MessageScroller 原语

> 分支 `codex/2026-08-25-G6-message-scroller-primitive`，base = Batch20 正式 head
> `9b0d6c0a3907f139ea2237cd179464ad9a2c85bb`。只跑本地定向测试，不运行
> `cargo xtask ci`，不派发 Actions，不处理 `grok-bot`。

## 本批范围

- [x] root/viewport/content/item/end-button compound 与 production `scroll_to_end` controller；
- [x] initial end；following-bottom 模式跟随 append 与 streaming ResizeObserver growth；
- [x] wheel/touch/scroll-key/scrollbar 用户意图立即退出 following；手动回 end 自动恢复；
- [x] free-scrolling 时 append 不改变阅读位置，prepend 保持首个可见 item 的 viewport offset；
- [x] 新 `scrollAnchor` 对齐并保留上一项 48px；mount-time anchors 一次登记，不因同 count swap
  回跳旧 anchor；
- [x] 回到底部控制只在 end 可滚时可见/可聚焦，激活后恢复 following；
- [x] viewport 命名 region + tabindex=0；content `role=log aria-live=polite`；item ID 有界；
- [x] ResizeObserver/MutationObserver 以 rAF 合并，DOM item 遍历不用 parent-realm
  `instanceof HTMLElement`；
- [x] design-gallery + Chromium/AX/真实 wheel/append/prepend/stream/anchor 证据。

## 不在本批

ChatTranscript 业务组件、thread route/history API、queued/Thinking/markdown/tool rendering 与正式
golden；MessageScroller 完成不冒充这些生产接线或 G6 整关完成。

## 当前机器证据

- 固定上游依赖为 `@shadcn/react@0.3.0`；官方 tgz 的 SHA-512 base64 与 `bun.lock` integrity
  `iKN0…WkWqQ==` 逐字相等，只读核验后未入仓；
- UI all-features=`47/0/0`；WASM all-features、UI/testkit all-targets Clippy `-D warnings`、
  fmt 全绿；
- i18n=`327` keys、design=`41 Rust/74 icons`、css=`109 classes`；production bundle
  wasm gzip=`370410`、CSS=`48046`、fonts=`740216`、external/inline=`1/0`；production/gallery
  WASM 的 `_design` 字节=`0/1`；
- Chromium 最终两组断言=`9/9 + 8/8`：initial `scrollTop/height/client=360/680/320` 且
  end=0；真实 wheel 后 top=100、button active；free append top/first/offset 不变；prepend
  top `100→171` 但同一 visible item/offset `message-2/-30` 不变；free resize top/offset不变；
- jump 后 end=0/spacer=0/button hidden；旧流式行收缩+append 后仍 following，随后增长仍 end=0；
  PageUp 意图即使宿主不移动 viewport，也让下一 append 保持 top 并显示 end button；
- 新 user anchor 始终位于 viewport top+48px；四次 reply growth 中 offset 恒48，spacer
  `226→155→85→14→0`，自然输出越过 viewport 后 button 才 active；same-count tail swap
  top/first/offset=`100/message-2/-30` 全不变；
- AX 为命名 region→命名 log，`aria-live=polite`、relevant=`additions text`、viewport
  tabindex=0；button 名称/controls exact；duplicate ID/unnamed/nested interactive/remote/
  overflow=`0/0/0/0/0`，console error=0；
- 最终 gallery WASM SHA-256
  `bf75ce978f9b14253bdcd13489a6420aa1fe2b5c196071852b675e4dcce458c2`；Cargo.lock
  package delta=0，只扩已锁 web-sys 的 DomRect/ResizeObserver/MutationObserver feature。
