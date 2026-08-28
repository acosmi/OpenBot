# Batch 51：HumanLease 与 Browser Input Protocol

> 状态：WIP。日期：2026-08-28。分支
> `codex/2026-08-28-G5-human-lease-input-protocol`；base `bc9fdbb`；固定上游
> `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

Batch50 后 Desktop sandbox renderer 仍被一个真实前置阻塞：`openbot-computer` 只有 Phase 0 边界说明，
仓内没有 §11.1 的单一 Electron/Chromium engine。直接另做 component-only renderer 会引入第三种生产
渲染器并偏离第一真源。本批先实现所有 browser/component renderer 共用的 HumanLease epoch 栅栏与
封闭输入协议；不在没有 engine 进程、CDP 与帧流实证前关闭 Desktop renderer 或输入执行条目。

## 第一真源裁决

- 固定上游 `control.ts` 的可观察状态机保持：Bot request 只提出请求；human take 后 Bot acting 立即拒绝，
  不排队；release 删除旧 reason；pending secret 只保存 label/ref/document generation，不保存值；
  unanswered help request 恰 10 分钟后在读取时过期，human holder 不被该 help TTL 自动释放；
- v3 §12.5 的新增 HumanLease 绑定 authority-owned actor/computer/tab/computer generation/auth generation、
  epoch 与显式 expires-at。take、transfer、release、expiry、navigation、computer restart 均推进 epoch；
  旧输入即使已在队列里也 fail-closed；
- HumanLease 的有效期数值在第一真源中没有默认值。本批不猜常量：调用方必须传权威 expires-at；
- BrowserInput 是 closed enum：mouse move/down/up、wheel、key down/up、insertText、secret insert；
  没有 IME composition、drag、upload、file chooser 或自由 CDP 变体；
- secret value 使用可清零、不可 Clone、Debug 脱敏的容器，只进入独立 typed command；不进普通 key event、
  control state、日志、frame 或 transcript；
- 本批可以关闭纯状态机与结构性负面条目；CDP 映射、真实 Electron execution、frame/input broker、
  Desktop renderer 与 a11y 豁免仍须真实 engine 证据，不能靠 enum 冒充。

## 待完成与证据

- [ ] `openbot-computer` control/HumanLease 状态机、authority envelope 与竞态测试；
- [ ] closed BrowserOperation/BrowserInput 与 secret redaction/zeroize 构造；
- [ ] 更新 browser-operation ledger，只关闭被本批真实行使的条目；
- [ ] 定向 tests/Clippy/fmt/parity/recount，正式证据与真源回写；
- [ ] 清理本批生成产物。

本批不运行 `cargo xtask ci` 或 GitHub Actions，不触碰 `docs/assets/`，未经本轮明确授权不 push/建 PR。
