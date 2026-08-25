# Batch 15 WIP 恢复点：G6 UI 地基与可点击 Approval

> 日期：2026-08-25。当前分支：`codex/2026-08-25-G6-ui-foundation-approval`；基线
> `55cccc49603fc2945a857834b6f64ce200c9c737`。本文件是断电前 checkpoint，**不是完成证书**。

## 已落源码

- Leptos 0.8.19 / router 0.8.13 / meta 0.8.6 / leptos_i18n 0.6.2 精确钉版；真实 CSR
  `App`、`/approvals` route、单一 `main`/`nav`/`h1`、skip link。
- `tokens.toml` build.rs 确定性生成 `tokens.css` / Rust 常量；字体字节与 SHA-256 校验；
  array-of-table token 不再漏生成。
- 74 个 SVG 与 `icons.toml` 两向校验，生成强类型 `Icon`；拒 script、event handler、外链等
  非法 SVG 形状。
- en / zh-CN 占位符修为 leptos_i18n 的真实 `{{ name }}` 语法；新增 Approval 文案。
- Button / Badge / EmptyState / IconView / ThemeToggle / LocaleSwitch 地基；token-only CSS。
- Approval UI 每秒读取 `GET /api/tool-approvals`，展示服务端权威 effect/target/已脱敏参数/
  change/expiry；POST 只发送 server id + grant/deny，同 ID 在飞时禁双击，响应正文不进入错误。
- 新增 `cargo xtask i18n-check`、`design-lint`、`css-check`、`bundle-budget`、
  `tools fetch|verify`。

## 当前机器证据（随后又有小改动者须恢复后复跑）

- `cargo test -p openbot-ui --offline`：18 passed / 0 failed。
- UI native all-targets check：绿；WASM all-targets check：绿。
- openbot-ui Clippy `-D warnings`：绿；xtask Clippy `-D warnings`：绿。
- `cargo xtask i18n-check`：258 leaf keys，键及占位符集合 exact。
- `cargo xtask design-lint`：18 Rust files、74 icons，反向规则绿。
- Tailwind macOS arm64：已下载并安装，SHA-256
  `cdf646702987a743464dff4d9c60fd4480d1c1e73dd819a9a67f1078815dce9d` 精确匹配 pins。
- Binaryen version_132 macOS arm64 archive 已校摘要并抽取 `wasm-opt`。
- 未运行 `cargo xtask ci`，未触发 GitHub Actions。

## 断电时明确未完成

- `cargo install trunk 0.21.14` 已主动中断；wasm-bindgen-cli 0.2.127 尚未开始。恢复后重跑
  `cargo xtask tools fetch`，它会复用已校验的 Tailwind/Binaryen。
- 尚未跑真实 `trunk build --release --offline --locked`，因此 `css-check`、`bundle-budget`、
  浏览器 AX/键盘/golden 均无证据。
- Server Axum static bundle、CSP、cookie 首帧 theme/locale 改写与 Desktop custom protocol 未接；
  G6/G4 整关不能勾。
- 最后一次 UI Clippy/WASM 之后改了 ThemeToggle radiogroup/icons，恢复后必须复跑针对性检查。
- `cargo vet --locked` 实得 182 unvetted GUI/tooling dependencies。自动生成 182 条
  `safe-to-deploy` exemptions 的请求被安全审查拒绝；未绕过、未改 `supply-chain/config.toml`。
  后续必须获得用户对这一广泛精确版本豁免的明确授权，或完成等价的真实审计覆盖。
- parity/ui 台账、R78、CLAUDE/移交指南与正式 Batch 15 证据尚未更新；本文件不能替代它们。

## 恢复顺序（禁止 CI）

1. `git status --short --branch`，确认本 checkpoint commit 与工作区。
2. `cargo xtask tools fetch`；`cargo xtask tools verify`。
3. 仅跑 UI/xtask fmt、Clippy、18+ 单测与 WASM check。
4. 从干净生成链跑 Trunk offline/locked，再跑 `css-check`、`bundle-budget`。
5. 接 Axum static GUI/CSP/首帧偏好并做真实浏览器审批点击验收。
6. 机器证据成立后才更新 parity/UI、R78 与正式 batch 文档；仍不运行 `cargo xtask ci`。
