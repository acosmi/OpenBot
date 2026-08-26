# Batch 24 WIP 恢复点：47 图标映射三向 join

> 分支 `codex/2026-08-25-G6-icon-ledger-join`，base = Batch23 正式 head
> `af7e255b3b9477e81e48ab2f23641862314447ac`。只跑本地定向测试，不运行
> `cargo xtask ci`，不派发 Actions，不处理 `grok-bot`。

## 本批范围

- [x] 解析 GUI 第一真源 §4.6.2 两对/行的47个 Tabler→target 映射；
- [x] exact join `first source → icons.toml upstream_tabler/name/usage → parity/ui target`；
- [x] 46 Lucide target 同时校 Rust enum variant、SVG/manifest、declared source zip SHA；
- [x] 46 条 ledger 改 done + exact done_evidence，旧“未验证”说明同步消除；
- [x] IconBrandGoogleDrive exact join 到 brand manifest，但 status 必须保持 todo；
- [x] parser 双列+brand exception 单测，xtask bin 78/0/0，Clippy `-D warnings`；
- [x] design-lint 输出 `46/46 Lucide done; brand 1/1 todo`；
- [x] parity/recount 复算与文档同批收口。

## 不在本批

Google Drive/Google/Microsoft/Okta 官方品牌 SVG、使用条款与 provenance 尚未取得，不用 Lucide
占位或手绘标志冒充；45业务组件、6 runtime替代与27页golden也不因映射join自动完成。

## 当前机器证据

- `cargo xtask design-lint`：mapping join=`46/46 done + 1/1 brand todo`；45 Rust、74
  manifest/SVG、安全形状与reverse rules全绿；
- xtask bin tests=`78/0/0`，新增 parser fixture 覆盖一行两映射+brand exception；
- openbot-testkit xtask all-targets Clippy `-D warnings` 绿；fmt/diff绿；
- UI ledger `27/125 → 73/79`；全 parity `463/1209/1672 → 509/1163/1672`；
  fixtures不变15/22/37；strict recount=`157/157/0`；
- Cargo.lock/package/UI production bundle delta=0；未运行 Trunk/浏览器（本批无发行代码/资产变化）；
- 磁盘 ENOSPC 时只清理可重建 `target-xtask` 1.3GiB 与 `target/debug/incremental`
  3.2GiB，未删源码、依赖制品、release/WASM或固定上游克隆。
