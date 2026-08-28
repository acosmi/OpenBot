# G6 UI 地基与可点击 Approval（Batch 15）

> 日期：2026-08-25。分支 `codex/2026-08-25-G6-ui-foundation-approval`；基线
> `55cccc49603fc2945a857834b6f64ce200c9c737`；断电 checkpoint
> `ebed80164f1db30decf23bcd2afe45c88fe0e06e`；实现提交
> `6c595660758542969cce37f87f3637fe6c9fdf59`。

## 1. 结论

本批完成了可由本机诚实证明的第一段 G6 生产地基：同一 Leptos CSR/WASM bundle 能由
Axum 条件托管，严格 CSP 下零内联脚本启动；Approval 页面展示服务端权威操作事实并可真实
点击 grant/deny；主题和语言控件完成对应 APG 键位；固定版本工具、i18n、设计规则、CSS 与
bundle 预算均有机械闸门。

本批**没有**关闭 G6 或 G4：浏览器数据来自明确的 test-only fixture，不冒充 PostgreSQL
approval backend；真实 durable approval 的生产证据仍由 Batch 14 承担。Desktop/Tauri、
用户偏好持久化、完整 31 routes/21 primitives/45 components、全量 golden/axe/键盘 E2E 与
cargo-vet 覆盖仍未完成。

## 2. 生产实现

### 2.1 固定工具与确定性 bundle

- exact pins：Tailwind 4.3.3、Trunk 0.21.14、Binaryen version_132、wasm-bindgen-cli
  0.2.127；平台 URL、archive/member SHA、version 输出都进 `tools/pins.toml`；
- 大文件下载可分段，但合并后必须用整包 SHA 再验；macOS Binaryen 同时抽取
  `bin/wasm-opt` 与其相邻 `lib/libbinaryen.dylib`；
- `build.rs` 与 Trunk pre-hook include 同一份 `build_support/assets.rs`，不保留第二个
  token/icon 生成器；clean checkout 不依赖预先存在的 ignored `tokens.css`；
- Trunk 默认 inline initializer 被替换成固定 external `openbot-bootstrap.mjs`；post-hook
  只允许从本次 hashed wasm-bindgen JS/WASM pair 生成相对同源 import；
- `bundle-budget` 同时拒绝 remote/inline script、eval/Function、cookie/localStorage bootstrap。

最终产物连续两次构建的 8 个文件名与 SHA-256 逐项相等。

### 2.2 Axum static GUI

`StaticApp::open` 在启动期完成 canonical path、目录/文件/1 MiB/UTF-8、唯一 `<html>` marker、
恰一个固定同源 bootstrap 与空 script body 校验。Server 仅在 `APP_DIST_DIR` 存在时挂载；
未配置时保持原 API 行为。

首帧只解析封闭 cookie `openbot-ui=v1.<system|light|dark>.<en|zh-CN>`；无合法 cookie 才按
`Accept-Language` quality 在 en/zh-CN 中选择。HTML class/lang 由 Rust 改写，无启动脚本。

SPA fallback 只处理无扩展名的产品 GET route；`/api`、health/readiness/metrics、fonts、
`.well-known` 与有扩展名 asset 不回 index。static route 在统一 layer 之前挂载，故 HTML、
asset、404/405 与 API 共用 request-id、tracing、metrics 与 body-limit 边界。

实际响应头包括：`default-src 'none'`、`script-src 'self' 'wasm-unsafe-eval'`、同源
connect/style/font、`frame-ancestors 'none'`、no-store、nosniff、no-referrer、COOP/CORP、
Permissions-Policy 与 X-Frame-Options DENY。

### 2.3 Approval 与 APG 控件

`/approvals` 每秒调用 `GET /api/tool-approvals`，卡片只消费权威 DTO：effect、target kind/id、
approval class、expiry、已脱敏 arguments 与 change summary。模型理由无渲染字段。POST body
只有 grant/deny，路径 id 是单一编码 segment；同 id 在飞时禁双击，成功移除卡片并以
`role=status` 反馈，失败不把响应正文当可信文案。

- ThemeToggle：radiogroup，显式 `aria-checked=true|false`，roving tabindex；Arrow 四向循环、
  Home/End、即时 `<html>` class 与焦点同步；
- LocaleSwitch：menu button + menuitemradio，显式 expanded/checked、current；Arrow/Home/End、
  Enter/Space/Escape/Tab 与焦点归还，切换同步 `<html lang>`；
- 浏览器审计发现并修复：空/省略布尔 ARIA、未来 expiry 的隐藏术语误写成“已过期”、
  1024×640 因 DOM min-width 与经典 scrollbar gutter 产生 15px 横向溢出；
- EmptyState 改为调用方提供 stable heading id；Badge 补齐 info tone 与封闭值测试。

## 3. test-only fixture 的边界

`openbot-ui-fixture` 只在 `testkit` feature 下构建，固定一个已脱敏 approval DTO 与 fixed auth，
但复用 production `ServerBuilder`、static host、敏感写 Origin guard、真实 Leptos/WASM 与真实
HTTP GET/POST framing。decision 会从内存移除固定记录，便于 golden/AX/键盘验收。

它不连接 PostgreSQL、不代表跨 replica/durable approval。生产后端事实仍取 Batch 14 的
native 0020 + PG17/SCRAM + 真实 acting MCP grant/deny 证据；两组证据不能合并偷换口径。

## 4. 本机证据（未运行 CI）

| 面 | 机器实得 |
|---|---|
| UI tests | 19 passed / 0 failed / 0 ignored |
| Server static tests | 4 / 0 / 0；另 173 filtered |
| 编译/lint | UI wasm32 all-targets 绿；UI、Server lib、fixture、xtask Clippy `-D warnings` 绿 |
| tools | Tailwind 4.3.3 / Trunk 0.21.14 / Binaryen 132 / wasm-bindgen 0.2.127 全绿 |
| i18n | 261 leaf keys；en/zh-CN key 与 placeholder set exact |
| design | 18 Rust files；74 manifest/SVG icons；reverse rules clean；84 对 WCAG AA 测试绿 |
| CSS | 43 source class literals 全进入产物 |
| bundle | WASM gzip 359,136 / 3,670,016；CSS 27,002 / 98,304；fonts 740,216 / 819,200；external script 1 / inline 0 |
| reproducibility | 连续两次 offline/locked Trunk build，8 个产物 SHA 逐项相等 |
| Browser 1440×900 | 页面完整，无横向溢出；权威 target/参数/diff 与批准/拒绝可见 |
| Browser 1024×640 | `scrollWidth == clientWidth == 1009`，纵向滚动但横向越界元素 0 |
| Browser AX/DOM | main/nav/h1=1/1/1；duplicate id=0；heading jump=0；unnamed focusable=0；remote resource=0；image without alt=0；runtime 只有 external bootstrap + Leptos 插入的空 script |
| Browser interaction | theme End/Arrow/Home 状态/焦点正确；locale menu/checked/lang/焦点正确；真实 click 后 approval card 1→0，status=`Approval committed.` |
| Axum headers | loopback curl 200，CSP/no-store/nosniff/no-referrer/COOP/CORP/Permissions/XFO/x-request-id 全存在 |
| parity | 436 done / 1233 todo / 1669；API 38/121/159；UI 3/149/152；0 violations / 0 warnings |
| strict recount | 固定上游 `891df72f…`：157 passed / 0 mismatch / 0 skipped |

未运行 `cargo xtask ci`；未 dispatch GitHub Actions。

## 5. 供应链实得

详见 [UI 供应链 delta 审计](2026-08-25-G6-UI供应链delta审计-batch15.md)。最终：

- UI dependency guard：30 build scripts、2 licenses、2 compile-time unmaintained advisories、
  2 Windows import archives、2 unreachable maintainer scripts，全为 exact hash/consumer；
- cargo deny offline：exit 0，四段 errors=0；保留 30 条 multiple-version warning 与 1 条
  OFL asset 不在 Cargo graph 的 warning；
- cargo audit：加载 1225 advisories，扫描 640 dependencies，三条带 guard 的精确 ignore
  （既有 RSA + 两条 compile-time unmaintained）后 exit 0；
- Phase 0 已记录的 Lucide 1.33.0 原 LICENSE 证明不是单一 ISC：Feather 衍生子集另受 MIT。
  本 PR 把两份第一真源与 `CLAUDE.md` 同步修正为 `ISC AND MIT`，不改既有 NOTICE/SPDX；
- cargo vet：**185 unvetted**，全部缺 `safe-to-deploy`；未改 `supply-chain/config.toml`，
  未自动生成 exemption。这是 G6/供应链仍未闭合的硬事实。

Trunk 自己的安装 lock 还含 yanked `crossbeam-channel 0.5.14`、`zip 2.6.1`；
wasm-bindgen-cli 构建报告两个 future-incompat 包。它们不进产品 Cargo.lock/发行物，但仍是
构建工具风险，不能省略。

## 6. 台账勾选与剩余工作

本批只新勾：

- `T-API-0105 middleware-serve-static`；
- `T-UI-0005 EmptyState`；
- `T-UI-0023 Badge`；
- `T-UI-0026 ThemeToggle`。

LocaleSwitch 虽完成浏览器交互，仍因 Server ApplicationService 偏好持久化/cookie 镜像与
Desktop 本地设置未落而保持 todo；Button 仍缺全 9 状态与 design gallery，保持 todo；
46 个 Lucide mapping 尚缺 ledger→icons.toml 逐条机械 join，Google Drive 品牌标原件也缺，
因此不批量勾图标。

下一批按第一真源优先完成用户 UI preference typed port + Server cookie + Desktop local/custom
protocol；随后扩展原语/route/golden。G6、G4、G0、G2、G3、G5、G7、G8 整关均保持未勾。
