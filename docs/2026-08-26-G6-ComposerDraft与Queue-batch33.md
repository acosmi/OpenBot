# Batch 33：Composer Draft 与 Queue 纯状态

> 日期：2026-08-26。分支：`codex/2026-08-26-G6-composer-state`。
> 基线：Batch32正式head `afd44e5377d942d3954e9d7510a952cda4b5b75f`。
> 实施提交：`a34f68337ecfdc0cbed42637285a56263617520d`。
> 固定上游：`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 未运行`cargo xtask ci`，未派发Actions，未处理`grok-bot`，未修改/暂存/提交`docs/assets/`。

## 1. 已闭合

- [x] `draft.test.ts`固定上游10条逐条移植：chip display→plain text、last Agent ID、command
  order、whitespace empty、single-Agent collapse、prompt/action/chip rewrite；
- [x] `queue.test.ts`固定上游16条逐条移植：idle direct、defensive idle drain、busy park、typed
  order、settle单turn、command首次顺序去重、double-settle零resend、remove/no-op identity；
- [x] `Cow`在类型上区分borrowed no-op与owned mutation，机械保存上游`toBe(same array/object)`；
- [x] queued messages以caller-minted ID区分，相同文字仍是两条；合并用换行，不伪造一句连续文本；
- [x] queue drain固定`agent_id=None`：channel/thread已绑定Agent，mention只保留在人类文字中；
- [x] action command改为closed deferred effect identity，caller在segments提交后显式解释；纯transform
  内没有隐藏闭包、副作用或自由执行入口；
- [x] production release bundle hashed asset名与Batch32逐字相同，证明未被UI消费的状态核心被优化器
  消除，没有静默画出Composer或改变现有页面。

## 2. 边界

- queue明确只活在当前channel view mount；reload/unmount丢失，**不是**PostgreSQL outbox，和固定
  上游注释一致；
- `settle`只消费外部已确认的turn terminal事实；本批没有stop/cancel/steer API，不凭纯函数冒充；
- Segment trigger闭集仅`@`和`/`；command action只是catalog effect ID；
- 全文trim复用contracts唯一ECMAScript TrimString（U+FEFF去、U+0085留）；
- 本批不勾`T-UI-0043` Composer、`T-UI-0123` prompt-area替代、`T-ROUTE-0009` channel route，
  也不渲染queue/stop按钮。

## 3. 本机证据

| 面 | 结果 |
| --- | --- |
| Composer draft | **10 / 0 / 0** |
| Composer queue | **16 / 0 / 0** |
| openbot-ui 全包 | **94 / 0 / 0** |
| UI all-targets/all-features Clippy `-D warnings` | 通过 |
| UI WASM all-targets/all-features | 通过 |
| i18n / design / CSS | **396** leaf；**70 Rust / 74 icons**；**200** class literals |
| release bundle | WASM gzip **669,241 B**；CSS **70,248 B**；fonts **740,216 B**；external/inline **1/0**；hashed asset名与Batch32相同 |
| parity | tests **360/687/1047**；总计 **625/1049/1674**；其余ledger不变 |
| parity violations / warnings | **0 / 0** |
| strict upstream recount | **157 / 157 / 0** |

Cargo.toml/Cargo.lock/CSS/locale/icon/package delta均为0；没有启动fixture、浏览器或PostgreSQL，
因为本批只有纯状态且release产物未变，不用无关运行面填充证据。

## 4. 台账

- `T-TEST-0131–0140`：draft 10条；
- `T-TEST-0141–0156`：queue 16条。

## 5. 明确仍未完成

- [ ] Composer production owner与DOM：Textarea、Enter/Shift+Enter/IME、attachment、send/stop；
- [ ] `@coworker` / `/skill` sources/triggers及真实skills API；
- [ ] turn status/realtime→busy/settle、production stop/cancel、queue visual/remove/steer；
- [ ] transcript markdown、完整channel route/home route、Screen与golden；
- [ ] G3/G4/G6整关继续不勾。
