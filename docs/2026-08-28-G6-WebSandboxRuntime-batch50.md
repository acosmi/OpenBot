# Batch 50：Web Sandboxed Component Runtime

> 状态：WIP。日期：2026-08-28。分支 `codex/2026-08-28-G6-web-sandbox-runtime`；
> base `40dac52`；固定上游
> `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批实现 Server Web 的 opaque-origin iframe sandbox、production conversation 与 Admin Playground
共用 wrapper、published source 读取和 shared RefusedCard。Desktop 主 Tauri WebView 必须明确拒绝执行
用户脚本；独立零 capability Chromium renderer 不在真实进程/帧流/input broker 落地前提前标绿。

## 第一真源裁决

- iframe `sandbox` 恰为 `allow-scripts`，没有 `allow-same-origin`、navigation、popup、download、
  form 或 storage token；CSP 逐字由单一 builder 生成，网络只允许 `img-src data: blob:`；
- 每次 mount 由 Web Crypto CSPRNG 生成新 nonce 与一次性 channel capability。宿主只接受 transferred
  `MessagePort` 上的 closed ready/failed lifecycle；不监听 renderer 的全局 `postMessage`；
- authored HTML/CSS/JS 与 args 都是数据。HTML/CSS 只进入 opaque sandbox；JS 先作为安全 JSON 字符串
  进入固定 bootstrap，再由 wrapper-owned nonce script 执行，不能靠字符串拼接突破 broker closure；
- wrapper 在执行作者 JS 前写 `window.__args`。production 与 Playground 调用同一个 document builder；
- published runtime 只读 Batch49 的 published DTO。draft/sample 只有 fresh admin Playground 能读写；
- `http:`/`https:` 才允许 Web iframe 路径。任何 custom scheme（含 Tauri）显示明确不可用状态且不创建
  iframe，从构造上避免用户 JS 进入主 Tauri WebView；
- CPU/memory hard containment、Desktop frame/input broker 与 Desktop a11y 豁免必须保留未完成边界。

## 待完成与证据

- [ ] pure wrapper/CSP/nonce/channel contract 与 WASM iframe host；
- [ ] published runtime/RefusedCard 与 Admin Playground 共用 wrapper；
- [ ] fixture + release browser 的 args/order/network/top-nav/storage/channel replay/host-mode 负例；
- [ ] ledger、第一真源、项目状态与定向质量闸门；
- [ ] 清理本批生成产物。

本批不运行 `cargo xtask ci` 或 GitHub Actions，不触碰 `docs/assets/`，未经本轮明确授权不 push/建 PR。
