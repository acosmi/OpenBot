# OpenBot 全量 Rust 重写：终版研究、前置审计与实施方案

> 日期：2026-08-21（America/Los_Angeles）；第二轮前置审计就地修订：2026-08-22；第三轮就地修订（v4：范围冻结、`grok-bot` 参考源定位、Electron 双 role engine、阶段闸门）：2026-08-28
>
> 文档状态：终版实施基线 v4（v3 + 2026-08-28 第三轮就地修订 R115–R125，修订清单见 §28.1，修订方法与真源优先级见 §28.5；`docs/2026-08-28-OpenBot-TauriGUI-ElectronChromium-GrokBot大面积Rust迁移-v4修订计划-用户裁决版.md` 已被吸收，只作历史记录）
>
> 目标：将 `CopilotKit/openbot` 的当前可观察产品能力完整重写为 Rust 实现
>
> 审计方式：两份输入文档只读；本文件为新结论，不反向改写输入文档。自 2026-08-22 起本文件是 `acosmi/OpenBot` 仓内唯一实施真源，与任何输入文档冲突时以本文件为准
>
> 结论口径：源码、固定提交、一手规范或实测不能支持的说法，不进入实施事实；本文件中每个计数都必须能被 §28.4 的命令复现

## 0. 最终裁决

### 0.1 可行性

**Go。** 按本文件给出的边界，OpenBot 可以完成全量 Rust 重写。

本项目对“全量 Rust”的最终定义固定为：

> GUI、业务、内置 Agent、策略、数据库访问、线程与记忆、实时事件、认证、凭据、审计、Supervisor、Desktop authority / window ACL / coordinator、Browser/Computer scope、engine 生命周期、ScreenHub、HumanLease、file/shell 执行域、平台 sandbox helper、egress、进程树和全部高权限控制面由 Rust 实现；Chromium/Electron 仅作为受监管、可替换、被 Rust 拥有的 OS 约束包住的浏览器执行引擎，承担两种 role（Browser Computer 与 Desktop sandboxed component 渲染，§11.1）。零 JavaScript 业务控制面、零 JavaScript authority、零 Node coordinator / host / Agent / local-exec。（R115 / R117 / R118 精确化）

下列内容不违反该定义：

1. Leptos 编译成 WASM，并由 Tauri 的系统 WebView 渲染 HTML/CSS。
2. PostgreSQL、Chromium、Electron、操作系统 Keychain/KMS 作为外部引擎或系统设施存在。
3. 用户自己发布的 HTML/CSS/JavaScript 组件作为不可信数据，在零 Tauri 权限的独立沙箱中执行：Server Web 是 opaque-origin iframe，Desktop 是同一 Electron engine 的 component role（§3.3）。
4. 用户接入的远程 AG-UI Agent 可以由任何语言实现；它属于外部不可信扩展，不属于第一方内置 Agent 或控制面。
5. 模型、MCP、Google Drive、OIDC/SAML IdP 是外部服务；所有调用、身份、凭据、授权和审计仍由 Rust 侧控制。
6. Electron 主进程自带的 Node runtime 是引擎事实，不是第一方控制面：它被 §10.3 的 OS 约束包住，shim 不得使用 `child_process` / `fs` / `http` 等能力（§11.3，R119）。

最终生产发行物中允许存在的第一方非 Rust 源码只有最小 Electron engine shim（clean-room，§11.3：文件 allowlist、非空 LOC ≤ 600、Electron/Node API allowlist）。该 shim 不拥有产品身份、策略、审批、审计、模型/MCP/OIDC 凭据、任意文件或任意命令能力；其职责限定为 Chromium 生命周期、CDP、画面帧、封闭输入指令，以及 component role 下渲染会话的一次性注入。除此之外，不保留 React、Hono、Bun、TypeScript Agent、TypeScript MCP runtime 或 JavaScript 业务控制面。

构建期工具链（Tailwind CSS standalone CLI、trunk、wasm-bindgen CLI、wasm-opt）是钉 sha256 的二进制，只在构建机运行、不进发行物、不引入 Node/npm；它们按 §16.3 的供应链条目登记，版本与校验和见 `docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md` §12.1（2026-08-22 裁决：零 Node）。Electron 本身以官方 release zip + sha256 获取（`tools/engine-pins.toml`，§1.2），不经 npm；工作区内唯一允许的 `package.json` 是 shim 的 app manifest（零 dependencies / scripts / lockfile），`grok-bot/` 参考树内的 `package.json` 不参与任何构建（R117）。

### 0.2 最终产品形态

固定交付两个发行物，共用同一个 Rust Domain/Application Core 和同一份 Leptos GUI：

| 发行物 | 固定用途 | 数据与通信 | Browser Computer |
| --- | --- | --- | --- |
| `openbot-server` | 多用户、团队、自托管 Web 部署 | Axum HTTP/SSE/WebSocket；PostgreSQL 17 | 每个 `ComputerSecurityScope` 一个受监管容器；生产固定 `runsc` |
| `openbot-desktop` | 单机个人使用或连接团队服务器 | 本地模式使用 Tauri typed in-process；远程模式使用同一 Axum API；本地 PostgreSQL 由 Rust 监管 | 每个 `ComputerSecurityScope` 一个受监管 Electron/Chromium 进程 |

数据库只实现 PostgreSQL 一套语义。桌面版管理仅监听本机的 PostgreSQL 17 sidecar，避免 SQLite/PostgreSQL 双 schema、双 migration 和双事务语义长期漂移。数据库引擎不是第一方业务代码；schema、migration、repository、transaction、加密和访问逻辑全部是 Rust。

### 0.3 一句话架构

```text
Leptos/WASM GUI
  ├─ Tauri typed in-process broker（Desktop Local）
  └─ Axum HTTP/SSE/WS（Server / Desktop Remote）
          │
          ▼
Rust Application Service
  ├─ Auth / People / Channel / Coworker / Component / Plugin
  ├─ Native Thread / Message / Memory / Realtime
  ├─ Rust built-in Agent + remote AG-UI client
  ├─ Tool / CEL Policy / Approval / Audit / Vault
  └─ ComputerManager / Supervisor / ScreenHub
          │ authenticated, versioned, closed protocol
          ▼
Electron/Chromium browser engine（无业务裁决权）
```

### 0.4 交付基线

（R125，2026-08-28）**12 人 / 52 周的日历基线作废**：§19 改为只用入口条件、产物与退出证据控制的阶段门（P0-code → P1 → … 与既有 G3 / G4 / G6 余项并行），不再给没有实证的周数。两次不参与编码的独立安全审计保留：第一次仍按 §24 G2，第二次在 G8 之前、P3 之后。范围约束不变：任何新增数据库、额外 MCP 协议面、第二浏览器 driver、移动端、Firecracker、ACP、新模型专用集成，或**来自参考源的产品能力**（R115），都不得挤入本次重写范围。

## 1. 第一真源与证据冻结

### 1.1 两份只读第一真源

| 文档 | SHA-256 | 本轮处理 |
| --- | --- | --- |
| `2026-08-21-OpenBot全量Rust重写与CrabCode复用审计结论.md` | `de9a0ed40522848d8cad4746beb87ac481036a1be48372e8caefac3c869cb95c` | 全文通读；未修改 |
| `2026-08-21-OpenBot全量Rust未完成能力深度研究与实现方案.md` | `5db37a2ca2471687e8d6e9c829c67cbc13484d1c1ee0d46b8c12182b7aaf49d5` | 全文通读；未修改 |

两份输入文档**未随 `acosmi/OpenBot` 仓归档**（2026-08-22 核对：仓内只有 `README.md` 与本文件），只以 SHA-256 登记供历史追溯。Phase 0 的 evidence bundle 必须把两份原件按上述摘要归档到 `docs/inputs/`；归档前任何人不得以"输入文档里写过"作为实施依据，只认本文件。

### 1.2 固定源码基线

本文件不再引用会漂移的 `main` 作为实施真源。固定版本如下：

| 来源 | 固定版本 |
| --- | --- |
| CopilotKit/OpenBot | `891df72f1827454d8b353d108fe5dd2313b7e30d` |
| 本机 CrabCode | `98f971bcf7411f056e8489e6e5a8e826f6672d38` |
| OpenAI Codex 研究快照 | `4f39251a010a8bd7d692d25fb33832ff06f1635a` |
| xAI Grok Build 研究快照 | `19d42e35c07a9c9244f03f6df0c4c353f970d4f9` |
| AG-UI | `e42bdbedc27cdf982ed9b5de904215acd73a17fb` |
| RMCP Rust SDK | `4a738b9dd99eaca418b614afa433a0cbdaf8d056`；发布版 `3.1.4` |
| Tauri | 稳定版 `2.11.5` |
| Leptos | `0.8.19`（2026-08-22 crates.io 最高稳定版已是 `0.8.20`，`0.9.0-beta` 在飞；本文件仍固定 0.8.19，升级走 delta audit） |
| Leptos 生态 | `leptos_router` **`=0.8.13`**（`0.8.14` / `0.8.15` 的依赖是 `leptos ^0.8.20`，在 0.8.19 上无法解析；`0.8.13` 要求 `^0.8.17`。升 Leptos 的 delta audit 必须同 PR 升 router）；`leptos_meta` `0.8.6`；`leptos_i18n` / `leptos_i18n_build` `0.6.2`（依赖 `leptos ^0.8`、`icu_locale ^2.2`） |
| GUI 构建工具（不进发行物） | Tailwind CSS standalone CLI `4.3.3`（七平台 sha256 见设计系统文档 §12.1）；trunk `0.21.14`（`--offline` + `[tools]` 钉版，PATH 上的同版本二进制即用、缺失即红不下载）；wasm-bindgen CLI = `Cargo.lock` 的 `wasm-bindgen` 版本；binaryen `version_132` |
| GUI 运行时 / 测试 crate | `pulldown-cmark` `0.13.4`；`syntect` `5.3.0`（`default-fancy`）；`sys-locale` `0.3.2`（仅 desktop）；仅 testkit：`image` `0.25.10`、`xcap` `0.9.8` |
| GUI 资产与视觉真源 | Inter Variable `4.1`（OFL-1.1，两份 woff2 sha256 见设计系统文档 §4.3）；Lucide `1.33.0`（ISC AND MIT，Feather 衍生子集另受 MIT，zip sha256 见其 §4.6.1）；视觉 / token / 布局 / 主题 / i18n / a11y 的唯一真源 = `docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md` |
| CEL 引擎 | crate **`cel`** `0.14.3`，**`default-features = false`**（关掉 `regex` + `chrono`；理由与实测见 §28.1 R28）（仓库 `cel-rust/cel-rust`；旧名 `cel-interpreter` 0.10.0 已停更，不用）；golden oracle = 上游锁定的 `cel-js@0.8.2` |
| OIDC / SAML | `openidconnect` `4.0.1`，**`default-features = false`**（关掉自带的 `reqwest` + `rustls-tls`，出网由注入的 safe dialer 承担，见 §28.1 R29）；`samael` `0.0.22`（`njaremko/samael`，0.0.x 版号即"未稳定"信号，§6.2 的独立外审因此不可免） |
| Browser engine | Electron `43.3.0`（2026-08-04 发布；Chromium `150.0.7871.212`，Node `24.18.1`）。官方 release zip 五平台（darwin-arm64 / darwin-x64 / linux-x64 / linux-arm64 / win32-x64）的 sha256 钉在 `tools/engine-pins.toml`，来源是上游 `SHASUMS256.txt` 的仓内副本 `tools/electron-v43.3.0.SHASUMS256.txt`；不经 npm（R117）。CrabCode `kernel-pin.json` 只是本 pin 的历史来源，不再是获取渠道 |
| `grok-bot/` 参考树 | tree `86f5a85f560f721677fa7e587a67ac0ffc036cb5`（R116 移除两个原始安装包 LFS 指针后；之前为 `b68f2497…`）。Anysphere（Cursor）Grok Bot 0.18.0 的反编译重建，**不是** xAI Grok Build；只作架构 / 执行 / 状态机参考，方法见 §11.5；改动它 = 新 hash + R 行 |
| runsc | 版本在 P1 spike（§19.1）实测后钉入本表与 `tools/engine-pins.toml`；判据见 §24 G5 / R121。在此之前 Server 生产 readiness 判据（§10.4）不变 |
| Rust 工具链 | `1.98.0`（2026-08-18 发布的当时最新稳定版），edition 2024。2026-08-22 由 `1.94.1` 升级，delta audit 见 `docs/2026-08-22-Rust工具链1.94.1升1.98.0-delta审计.md` |
| PostgreSQL | 17（上游 compose 用 `pgvector/pgvector:pg17`；Rust 版不需要 pgvector，平装 `postgres:17` 即可，§14.1） |
| PostgreSQL 驱动 | `tokio-postgres` `0.7.18` + `deadpool-postgres` `0.14.1` + `postgres-types` `0.2`（derive）。**刻意不用 `sqlx`**：它的 `query!` 宏要么在构建期连活库、要么把 `.sqlx` 离线元数据入库并保持同步，两条路都让 `cargo build --locked` 的答案取决于跑在哪台机器上；SQL 显式手写，由对真库的集成测试验证，构建期与数据库彻底解耦（G1 裁决 D3） |

上游 oracle 运行时版本（来自固定 commit 的 `bun.lock` / 各 `package.json`，fixture 录制与 golden 对照只认这些版本）：

| 上游组件 | 锁定版本 | 用途 |
| --- | --- | --- |
| `@copilotkit/runtime` / `@copilotkit/react-core` | `1.68.3` | Intelligence 模式 runtime、`BuiltInAgent`、`OpenGenerativeUIActivityRenderer` 沙箱渲染器 |
| `@ag-ui/core` / `@ag-ui/client` / `@ag-ui/encoder` | `0.0.57` | AG-UI 事件族与 `HttpAgent` 行为 oracle |
| `better-auth` / `@better-auth/sso` | `1.7.1` | 旧 session/SSO 语义（只做失效与数据关联，不反向工程） |
| `@modelcontextprotocol/sdk` | `1.30.0` | 上游 MCP Streamable HTTP client 行为 oracle |
| `cel-js` | `0.8.2` | policy corpus golden oracle（§8.3） |
| `playwright` | `1.62.1` | 浏览器操作 / screencast / 输入 oracle |
| `drizzle-orm` / `postgres` | `0.45.2` / `3.4.9` | 28 表 schema 与 13 条 migration 的生成器 |
| `hono` / `zod` | `4.13.3` / `4.4.3` | 95 条 handler 的 framing / 校验 oracle |
| `dockerode` | `5.0.1` | Supervisor 容器参数 oracle |
| `bun` | `1.3.14` | 上游测试运行器（Phase 0 跑基线测试用） |
| 管理 Bot `agent-langgraph` | `@langchain/{openai,anthropic,google-genai}` + `@langchain/langgraph` 1.4.x | `BOT_PROVIDER` 三家 provider 行为 oracle（§7.3） |

后续升级任一来源时，必须新建 delta audit，记录旧/新 commit、协议差异、许可证差异、迁移影响和回归结果；禁止静默更新 lockfile 后继续沿用本结论。

### 1.3 当前 OpenBot 静态基线

本轮在独立审计副本中得到：

| 项目 | 当前实况 |
| --- | ---: |
| 版本控制文件 | 504 |
| TS/TSX 行数（含生成路由） | 72,000 |
| PostgreSQL 表定义 | 28 |
| SQL migration | 13 |
| 前端 route 文件 | 31 |
| `server/src` 静态 Hono route 注册 | 95 = `app`/`routes` 上全部 HTTP method handler（含 4 处多行写法）；**不含** 9 处 `app.route()` 模块挂载、2 处 `use()` 中间件、1 处 `app.on(["GET","POST"], "/api/auth/*")` Better Auth 动态挂载与 CopilotKit `/api/copilotkit` 动态注入；另有 Supervisor 5 条及 agent-computer 29 个 `url.pathname` 手写路径 |
| `.test.ts/.test.tsx` 文件 | 105 |
| 版本控制测试文件中的 `test()` / `it()` 词法命中 | 1,007 |

这些数字用于完整性核算，不等于质量证明；1,007 是词法命中，不冒充 AST 解析后的精确 test 数。2026-08-22 第二轮审计在本机独立克隆上把八个数字全部重新算了一遍，逐个相等（命令见 §28.4）。本轮依赖安装在审计环境中长时间未完成后被终止，因此本文件不宣称上游测试当前全部通过。Phase 0 必须在干净、可联网的受控 CI 中运行、生成 AST 级测试 inventory 并归档原始结果。

## 2. 前置审计结论

### 2.1 保留的正确结论

下列结论证据充分，原样进入正式方案：

1. Tauri 2 + Leptos 可以承担第一方 Rust GUI；Leptos/WASM 不等于原生控件，也不消除系统 WebView。
2. Tauri WebView 不是跨平台统一 Chromium；产品 GUI 与 Agent Browser 必须分层。
3. Rust 必须是身份、策略、凭据、审计、调度、数据库访问和生命周期的唯一控制面。
4. CrabCode 只能按边界清晰的模块、协议和 fixture 选择性复用，不能整体复制 monolith。
5. CrabCode 当前生产 Agent 仍启动 Bun worker；MCP 主 runtime 仍是 TypeScript；`in_process::embed()` 仍是 stub，不能记为 Rust 已实现。
6. snapshot ref 必须绑定 tab/document generation；restart/reset 后旧 ref、ticket、approval、capability 和 lease 全失效。
7. human lease 生效期间，Agent acting 操作一律立即拒绝，绝不排队。
8. 工具的 `attempt outcome` 与外部副作用的 `commit state` 必须分开；`Unknown` 非幂等操作禁止自动重试。
9. AG-UI 的 Rust community SDK 当前存在维护和协议漂移风险，领域模型不能直接依赖其类型。
10. Codex 的 transport、tool router、sandbox/approval 模式和 Grok Build 的 session actor、prompt queue、lifecycle、journal 模式有参考价值，但不能复制其产品主循环。

### 2.2 必须低修订的结论

| 原有表述 | 审计后的正式表述 |
| --- | --- |
| “实时 screencast 是未完成能力” | CrabCode 缺少目标实现，但 OpenBot 当前已有 Playwright CDP `Page.startScreencast`、ACK、WebSocket 输入和接管视图。Rust 工作是行为移植、二进制化和安全加固，不是从零证明可行。 |
| “多 Bot 隔离尚未实现” | OpenBot 已有每 Bot 容器、profile/workspace volume、可选 gVisor/SPIRE；Rust 版要移植并修正“仅按 bot_id 隔离”的跨用户缺口。 |
| “每 Bot 一个 MCP ConnectionSet” | 当前 OpenBot 的 user-OAuth Streamable HTTP 故意 per-call 建连并关闭。首版保持 per-call，禁止跨 Bot/用户池化。 |
| “Rust MCP runtime 要覆盖 stdio/resources/prompts/tasks/elicitation” | 当前产品只暴露远程 Streamable HTTP 的 `tools/list`、`tools/call` 和 OAuth；首版只实现该协议面，并保留 Google Drive REST adapter。 |
| “Tauri in-process 应回植 Codex AppServer JSON-RPC” | OpenBot 没有 Codex AppServer 兼容义务。Tauri 直接调用 typed `ApplicationService`，流使用有界 channel；只吸收 Codex 的反压、终态、取消和关闭不变量。 |
| “Browser Engine 不持有 secret” | Chromium profile 必然持有该安全域的网站 cookie/session。正式限制是：engine 不得取得产品主密钥、模型/MCP/OIDC 凭据或其他安全域的浏览器会话。 |
| “profile scope 等于 bot_id” | public Bot 可被不同用户调用，仅按 bot_id 会共享个人登录。profile 必须绑定 tenant + bot + credential principal。 |
| “PostgreSQL/pgvector 存 knowledge 文档” | 当前上游已删除本地 customer document index；`knowledge.yaml.sources` 只解析、不执行。Rust 版不得复活已否决的文档复制/ACL 索引。 |
| “RMCP 3.0.x 是当前基线” | 本轮固定 RMCP 3.1.4；只启用产品实际需要的 client 能力，并运行官方相关 conformance suite。 |
| “CrabCode A 级代码可直接抽取” | CrabCode 根 NOTICE 明示其整体为闭源专有软件。所有直接抽取先经过逐文件权属授权与来源台账；本机可读或 Cargo workspace 的 `license = MIT` 不能替代授权。 |

### 2.3 明确删除的过度设计

以下内容不属于本次功能对等重写，不进入实现 backlog：

1. Firecracker microVM。Server v1 固定 Docker/containerd + `runsc`；Firecracker 是独立 Linux/KVM 产品层。
2. youki。它是另一 OCI runtime，“Rust 编写”不自动产生更强安全边界。
3. Restate、Temporal 或其他通用 durable execution 平台。
4. Codex AppServer 全协议、JSON-RPC 初始化层和 pending-map 的逐文件复制。
5. MCP stdio、resources、prompts、tasks、elicitation 产品面和常驻 ConnectionSet。
6. SQLite/PostgreSQL 双数据库实现。
7. xAI 专用 provider crate；Grok 可走 OpenAI-compatible adapter，Grok Build 只作实现参考。
8. ACP 第二 Agent 协议、Rig/Goose 整体框架依赖。
9. 全域事件溯源。只有 thread/run/tool/outbox/audit 使用事件或追加日志；CRUD 域使用事务表。
10. Electron 和 chromiumoxide 两套正式 browser driver；首版只维护一套受监管 Electron/Chromium engine。
11. 将任意用户 JavaScript 放入主 Tauri WebView。
12. 长期持久化原始 provider stream、完整 HTTP body、screen frame、文件内容或秘密。
13. （R120，2026-08-28）Docker Desktop 或任何本地容器 / VM 作为 Desktop 的隔离层（`grok-bot` 的 "local Docker box"）；Desktop 只有 `HostLocal` 执行域，隔离执行由 Server 的 `ScopedContainer` 提供（§10.6）。
14. （R118）VNC 或 `<webview>` 作为 Screen UI（`grok-bot` 的 computer shell）；画面只走 §12 的帧流。
15. （R117 / R119）npm / Node 构建链、Electron `autoUpdater`、`--no-sandbox` / `sandbox:false` / `webviewTag:true`。
16. （R115）依赖 Grok Bot 云后端的能力（cloud agents、forever-box、box-store-sync、managed-setup、webauthn-proxy、host-upgrade、cross-user-sharing、teach-recording）与 Statsig / analytics / telemetry 家族；以及**任何来自参考源的新产品能力**——v4 不新增 Grok Bot 产品能力，候选只登记在 §11.5 的表里且无承诺。

本节对来自任何参考源（`grok-bot` / Codex / Grok Build / CrabCode）的条目同样生效：inventory、吸收或"参考源里有"都不构成重新立项。

### 2.4 当前上游缺陷：不得照译

固定 OpenBot commit 的下列公开问题或已知语义必须在 Rust 版中修正：

| 缺陷 | Rust 版确定语义 |
| --- | --- |
| [Agent endpoint 30x 可绕过初始 URL 检查；DNS rebinding 仍未解决](https://github.com/CopilotKit/openbot/issues/36) | 每一跳重新做 scheme/host/IP policy；安全 dialer 固定已校验 IP 与 TLS SNI；Server 再以 egress gateway 强制执行 |
| [malformed AG-UI `message.content` 可使 transcript 崩溃](https://github.com/CopilotKit/openbot/issues/44) | 所有外部 payload 做结构验证；未知/损坏事件隔离成可展示错误，UI 不崩溃 |
| [credential rotate 先写新值、再 revoke 旧值，失败会留下 orphan](https://github.com/CopilotKit/openbot/issues/53) | 单事务切换 active pointer；外部 revoke 独立进入 reconciliation；失败时新凭据不生效 |
| [从未运行的 thread history 返回 500](https://github.com/CopilotKit/openbot/issues/72) | 明确返回空 history；真实上游/数据库错误仍返回 5xx |
| [withdrawn tool 的 stale grant 可能在 transport 切换后复活](https://github.com/CopilotKit/openbot/issues/106) | catalog refresh 将 grant 标为 `suspended_missing`；工具重现后仍需管理员重新启用，永不静默复活 |
| Google Drive disconnect 尚未实现 | 本地立即 deny 并 tombstone；调用 vendor revoke；失败进入 `revocation_pending` 重试，UI 不谎报 vendor 已撤权 |
| [`allowed_groups` 已存储但没有身份组写入/授权使用](https://github.com/CopilotKit/openbot/issues/82)（上游 2026-08-21 以"改文档、声明它不是控制"关闭；固定 commit 里 `users.groups` 无任何写入路径，`synchronizeTenantPackage` 也不为包声明的 channel 写任何 membership 行——**包里声明的 channel 在当前产品中对所有人不可达**，包括 `OPENBOT_SINGLE_USER` 的唯一管理员） | 语义按 §6.5 固定：保留字 `all` = 全体有效用户；具名组在多用户 Server 必须有 IdP group mapping，否则包校验失败并指出原因；单用户模式（Server `OPENBOT_SINGLE_USER` / Desktop Local）把唯一 principal 直接写入全部包 channel 的 membership 并在包报告中注明"单用户：组不参与裁决"；空 `allowed_groups` 的包 channel 是校验错误（"无受众"），不再静默不可达 |
| channel `list` / `get` 的可见性判据把 `intelligence_channel_mappings` 当作 INNER JOIN，而分页查询只 join `channel_memberships` —— 两处判据不一致：一页可以在 `nextCursor` 非空的同时返回更少甚至零条。更要紧的是 Intelligence 按 §4.1 退役、该表按 §14.2 降级为 legacy provenance 之后，这个 join 会把 §6.5 刚补上 membership 的包 channel **原样过滤回不可达** —— §6.5 的修复被一个没人重新审视的 join 静默撤销 | 可见性只查 materialized membership（§6.5 条 5 逐字如此），thread 关联走 §4.3 的 native `threads`；`intelligence_channel_mappings` 仅作只读 legacy provenance，**不得进入任何运行时可见性判据**。并且分页与 hydration 必须用**同一个**可见性判据，否则 cursor 会承诺不存在的页 |
| MCP success/failure 审计发生在 vendor 调用之后 | action 前持久化 decision + attempt，action 后持久化 outcome + commit state |

## 3. 完整产品范围

本次重写不是把 Hono 翻译成 Axum，也不是只交付一个 Agent loop。以下能力全部进入 parity ledger；缺一项均不允许声明“全量完成”。

### 3.1 GUI 与用户旅程

1. 登录页、Google/Microsoft/Okta 按钮、企业 SSO email-domain routing。
2. 首页、channel 列表、分页、last-message、创建 channel。
3. coworker 创建、编辑、复制、隐藏、恢复、软删除、启动。
4. channel transcript、composer draft/queue、`@coworker`、自动 routing、stop/steer、失败恢复。
5. direct Bot chat、agent selector、新会话、历史恢复和损坏消息隔离。
6. personal skills、deployment skills、`/` 命令选择。
7. settings、主题、connected accounts、component gallery，以及 native memory 的查看/删除/禁用控制（**这是 31 个上游 route 之外的新增页面**：固定 commit 的 `app/src` 没有任何 memory UI/API，上游"memory"只存在于 Intelligence 托管侧；它是 §4.1 退出 Intelligence 后的替代面，route ledger 记为 31 + 1，不得冒充 parity 项）。
8. admin people、identity providers、credentials、computers、boundaries、audit。
9. admin plugins、connector、单 tool grant 页面、OAuth client 配置。
10. compiled components、sandboxed component playground、draft/publish/unpublish、HITL decision。

31 个现有 route 文件全部映射到一个 Leptos route 或 layout。路由可以合并代码，不能删除用户可观察能力。

视觉、布局、主题、国际化与无障碍的真源是 `docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md`（2026-08-22 用户裁决：自有设计系统、Tailwind v4 standalone CLI 零 Node、中英双语带 i18n 框架）；本节只定义旅程与能力。其中主题的 `system` 第三态与 UI 双语都是**新增**，不得冒充 parity：上游主题是手动两态且不跟随系统（`prefers-color-scheme` 在 `app/src/lib/theme.ts` / `components/theme-provider.tsx` / `styles.css` 均 0 命中），上游零 i18n 框架。31 个 route 文件 = 26 个页面 + 5 个 layout（`__root` / `_authed` / `_authed/_app` / `admin/route` / `settings/route`）；golden 截图对象 = 26 页 + memory 页 = 27 页。

### 3.2 Coworker、Channel、Routing 与 Tenant Package

必须保持：

- public/private、owner/admin 访问规则；无权访问统一返回 404，避免资源枚举；
- per-user hidden roster；
- coworker soft delete 后旧 channel 可读、不可再次运行；
- channel membership 覆盖所有读写、realtime、screen 和 control 路径；
- 显式 `@` 选择优先，未标注消息进入 routing；routing 只在 channel 创建时做一次并把 coworker 钉在该 thread 上（上游 `routing/classify.ts` 语义），router 调用失败、返回不可解析、命中不在 roster 上的 id 或置信度低于阈值，一律落到部署默认 coworker 并在 audit 里说明"未确定匹配"——router 永不抛错；
- routing audit 记录候选、选择和原因，不记录原始用户消息；
- 包声明的 channel 的 membership 由 §6.5 规则在包同步与登录时 provision（保留字 `all` / 具名组 / 单用户模式三档），用户创建的 channel 仍只在创建事务里写创建者 membership；
- tool/connector holdings 参与 routing 候选描述，但 discovery 不产生权限；
- tenant package 的 brand、agents、channels、model、knowledge 五类 YAML 继续做 schema 与引用检查；
- Tenant Package loader **只读上述五份 YAML**；旧 `skin.stylesheet` 只记
  `compatibility_input_ignored`，不读取/执行 `theme.css`。视觉与主题仍唯一来自 GUI 第一真源的
  `crates/openbot-ui/design/tokens.toml`；包环境展开只看启动层显式 allowlist，绝不把完整进程环境
  （尤其 secret）交给 YAML；
- `knowledge.sources` 保留为兼容输入并产生“不执行本地同步”的明确状态，不建立 customer document index。

### 3.3 Generative UI 与 Components

正式实现同时覆盖两条路径：

1. **Compiled gallery**：现有 React gallery 全部重写为 Leptos component；保留参数 schema、published 状态、per-Bot withholding、data-function grant 和 tool-call-time 再授权。
2. **Sandboxed component**：保留用户 authored HTML/CSS/JavaScript、draft/publish/revision/sample arguments；它属于不可信用户数据，不属于第一方 GUI 控制面。

Sandboxed component 固定运行位置：Server Web 在浏览器 opaque-origin sandbox iframe（Batch 50 已落，R113）；Desktop 不在主 Tauri WebView 创建用户脚本 iframe，而是在 §11.1 单一 Electron engine 的 **component role** 里运行（R118）：每个 Desktop 应用实例恰一个 role=SandboxedComponent 的 engine 进程，按需启动、零会话 30 秒后退出（**新增**）；每次渲染是该 engine 的一个 render session = 一个 `TabId`，独立 in-memory partition、独立 opaque origin（`component://<render-id>`，render-id 由 Rust 铸造、一次性），Chromium site isolation 保证 renderer 进程互不共享；帧经 §12.2 同一条 ScreenIngress → ScreenHub → viewer ticket 路径回主 GUI，输入只有 §12.5 的 closed `BrowserInput`（指针 / 滚轮 / 键盘 / insertText），epoch fencing 与 ControlService 与 browser computer 共用（Batch 51，R114）。Desktop Remote 的取源路径与 Web 相同（同一 ApplicationService 投影给出 published source 与 args），不新增 API 面。

用户脚本可见的**运行时契约固定为上游现状**（`app/src/lib/copilot/sandboxed-tools.tsx` 与 `admin/playground.tsx`）：wrapper 先注入 `window.__args = <本次调用参数 JSON>`，再执行作者的 `jsFunctions`；作者拿到的只有 `window.__args` 与自己那份 DOM。沙箱脚本**没有** data function、没有网络、没有向宿主回调的通道（上游 `component_functions` 只服务 compiled component），R1 不新增任何一种。迁移后已发布的 `sandboxed_components` 行必须在新沙箱里逐行渲染通过（Phase 0 把每行的 published html/css/jsFunctions/sampleArguments 录成 fixture）。

Desktop 独立 renderer 的两条必然后果在此写死，不留给实现期发现：① renderer 是同一个 Electron/Chromium engine 进程类（§11.1 单 engine），不引入第三种渲染器；② 画面以帧流回主 GUI，文本不可选中、屏幕阅读器不可达——Desktop 的 sandboxed component **不承诺** a11y parity，§18 / G6 的 a11y 要求对 Desktop sandboxed component 明确豁免并在产品文档写明；Web 路径（iframe）保持 a11y 要求。豁免的具名 fallback（R118，**新增**）：画布旁由 Rust/Leptos 以只读 `<dl>` 渲染该次 tool call 已经 schema 校验过的 `arguments`（键 → 值），不含作者 HTML/CSS/JS 文本、不含模型自由文本；细则在 GUI 真源 §9.1。

固定运行规则：

- 独立 opaque-origin iframe/Chromium renderer，不在主 Tauri WebView 执行；
- `sandbox="allow-scripts"`，不得使用 `allow-same-origin`、top navigation、popup、download 或 storage；
- CSP 固定为 `default-src 'none'; connect-src 'none'; script-src 'nonce-<per-render-random>'; style-src 'unsafe-inline'; img-src data: blob:`；Rust wrapper 只给本次封装后的用户脚本设置 nonce，默认无网络；两宿主逐字相同；
- 不加载 Node、preload、Electron API 或 Tauri API；
- Web：使用一次性 MessageChannel capability 与 Rust typed broker 通信；Desktop：没有 MessageChannel，作者脚本只看到 DOM 与 `window.__args`，宿主通信只有 engine pipe 上的 render session 协议（§11.2），T-CMP-0018 按宿主拆分（R124）；
- Desktop component role 的零 egress 是三层（R118）：session 级 proxy 指向黑洞地址（`127.0.0.1:1`）+ `webRequest` 在 `component://` 与 `data:` / `blob:` 之外全部 cancel + 上述 CSP；任一层失效都不能靠另两层"兜底"当作合规，三层缺一即 engine conformance 判红；
- Desktop component role 的硬预算（R118，**新增**，默认值；管理员 policy 只能收紧不能放宽）：同时活跃 render session ≤ 8（viewport 驱动：离开视口 2 秒后停会话，回到视口重新渲染）；每 session renderer RSS ≤ 256 MiB；bootstrap 到首帧 ≤ 5 秒；帧 ≤ 1280×800、≤ 5 fps；console error ≤ 100 条。超限只终止该 session 并显示与 compiled refusal 共用的 RefusedCard；
- data function 每次调用重新检查 component revision、Bot grant、actor ACL、policy 和 audit（仅 compiled component；沙箱结构上无 data function）；
- 组件崩溃、超时或 schema 错误只终止该 iframe / render session，不影响 transcript 或主 GUI。

### 3.4 外部扩展兼容面

- 任意 remote AG-UI endpoint；
- write-only endpoint authorization header；
- standing role 去重注入；
- per-agent callback token hash（`obot_agt_` 前缀、32 字节、库内只存 SHA-256）；
- 10 分钟、绑定 actor/bot/run/tool 的签名 run assertion；
- **deployment-wide `AGENT_TOOL_TOKEN` 旧路径不保留**：固定 commit 的 `verifyCallback` 仍接受该共享 token（只认证、不授权，Bot/actor 仍取自 assertion），供"盒内 Bot"回调。Rust 版的盒内 Bot 是进程内 built-in Agent，不需要回调；任何仍以共享 token 回调的外部 Bot 必须在 cutover 前换发 per-agent token——迁移 preflight 列出最近 30 天用共享 token 回调过的 endpoint，未换发的部署不进入 §20.4 第 7 步；
- **managed Bot 插槽**：上游 `MANAGED_AGENT_AG_UI_URL` + `MANAGED_AGENT_TOKEN` 指定"产品内创建的 coworker 默认端点"（开发期指向 `agent-langgraph`；未设则省略随包的 Risk Analyst coworker，且拒绝创建没有自带端点的 coworker）。Rust 版该插槽默认由进程内 built-in Agent 承接，两个变量保留为可选外部覆盖（语义不变：server 以 `x-openbot-agent-token` 向该端点自证），`.env` 里有 token 无 URL 继续被忽略；
- deployment tool 与 surface/HITL tool 分流；
- stall watchdog、cancel、断流、未知事件和 provider 错误；
- Google Drive REST adapter、per-user OAuth 和 connected accounts；
- custom MCP Streamable HTTP tool servers；
- tenant packages 和外部示例 Agent 作为协议 fixture。

### 3.5 示例、脚本与明确退休项

- `agent-bot` 的手写 AG-UI reference 行为重写为 Rust `openbot-reference-agent`；
- `agent-langgraph`、Mastra 等框架样例不进入第一方生产仓代码，兼容性通过固定上游 container/trace fixture 验证；用户自己的远程 Agent 继续支持；
- 当前 `worker` 只返回 `{status:"idle"}` 且没有 job，Rust 版不发布空 worker binary；该测试标记 `not-applicable-with-proof`；
- config generation、architecture diagram、migration drift、release manifest、SBOM 和 fixture 工具统一进入 Rust `xtask`/testkit；
- 删除旧 connector/index、React build、Bun launcher、Hono server 和 TypeScript MCP/Agent runtime 后，不保留“以防万一”的隐藏启动路径。

## 4. 权威数据所有权

### 4.1 CopilotKit Intelligence 退出裁决

当前 OpenBot 把 durable threads、memory/learning、realtime gateway 和部分 run coordination 交给 CopilotKit Intelligence。CopilotKit 官方资料表明，云托管与企业自托管 Intelligence 都是独立平台与许可边界；自托管仍需要企业许可并部署其平台组件。

严格按本项目“业务、数据库和高权限控制面归 Rust”的定义，最终运行链作如下裁决：

1. Rust/PostgreSQL 是 thread、message、run、memory、realtime cursor 和 run lock 的唯一真源。
2. 最终产品在未设置任何 `INTELLIGENCE_*`、`COPILOTKIT_*` 环境变量时必须完整运行。
3. CopilotKit Intelligence 只用于旧数据导出、迁移核对和一次性导入；最终请求路径不读、不写、不连接它。
4. 不做 live 双写，也不把它保留成隐藏 fallback。
5. 托管后端的 learning 算法没有进入 OpenBot MIT 源码，无法逐实现复刻；验收目标是 OpenBot 可观察功能、thread 生命周期、记忆控制、实时恢复和用户旅程等价，不宣称内部算法相同。

### 4.2 数据类别严格分离

| 数据 | 真源 | 保留原则 |
| --- | --- | --- |
| 用户、角色、Bot、channel、grant、component、policy | PostgreSQL 事务表 | 按产品/合规策略 |
| thread/message/semantic run event | PostgreSQL 追加记录 + materialized state | 可恢复、可导出、可删除 |
| execution journal/tool attempt/outbox | PostgreSQL | 直到 terminal + reconciliation 完成，再按策略归档 |
| audit | 独立 append-only 表/分区 | 与 telemetry 分开配置 |
| memory | PostgreSQL，有 scope/provenance/supersession | 用户可查看、删除、失效 |
| screen frame/input delta | 内存 latest-value | 默认永不落盘 |
| activity UI projection | 内存 + 可从 audit/run journal 重建 | 不是调查真源 |
| raw provider/HTTP stream | 默认不持久化 | 诊断采样须显式开启、加密、24 小时 TTL |
| secret | Rust Vault 加密记录 | 永不进入 transcript/audit/普通日志 |

### 4.3 Native thread/realtime/memory 最小模型

新增或接管以下表：

```text
threads
thread_memberships
messages
runs
run_events
thread_leases
tool_calls
tool_attempts
outbox
memories
memory_events
intelligence_import_cursors
```

固定不变量：

1. 每个 thread 同时只允许一个 foreground run；`thread_leases` 使用 expiry + fencing token，旧 owner 即使恢复也不能提交。
2. `(run_id, seq)`、`(thread_id, event_seq)` 唯一；所有 terminal event 恰好一次。
3. 写 domain state、semantic event 和 outbox 在一个数据库事务中完成。
4. PostgreSQL `LISTEN/NOTIFY` 只作为唤醒，不是真源；消费者按 sequence 从表中补取，通知丢失不丢事件。
5. outbox 是 at-least-once；destination 使用 `(aggregate_id, seq, destination)` 去重，任何外部非幂等 effect 不通过普通 outbox 自动重放。
6. WebSocket/SSE reconnect 必须携带 last cursor；服务端先 replay，再进入 live。
7. token delta 以 50 ms 或 8 KiB 为上限合并成 semantic chunk 后持久化，避免逐 token 行爆炸。
8. memory 只保存两类：用户明确要求保留的 preference、带来源的事实；每条记录包含 scope、source message/thread、sensitivity、created_by、supersedes、expires_at。**写入只有两个显式入口**：built-in Agent 的 `remember` tool（经 §8.1 管线，effect=write）与用户在 GUI 的"记住这条"动作；R1 没有后台抽取、没有跨会话"learning" job。Bot 的 operational checkpoint 不是 memory，归 `runs` / `run_events`。Intelligence 托管侧的隐式记忆行为在 OpenBot 源码里不可观察、算法不可复刻（§4.1 条 5），这条替代面作为**已披露的行为差异**写进发布说明与 §22。
9. R1 memory retrieval 使用 PostgreSQL full-text、结构化 tag、scope 和 recency；不重建 customer document index，也不要求 pgvector。
10. 自动 thread summary 是上下文压缩产物，不冒充用户事实；原消息仍是可追溯来源。
11. 用户可以列出、删除、纠正和禁止 memory；撤权后 realtime subscription 和 memory recall 同时失效。

## 5. Rust workspace 与唯一业务入口

### 5.1 精简 workspace

```text
crates/
├── openbot-contracts       # native/wasm-safe ID、DTO、event、error、schema
├── openbot-domain          # 纯领域状态、不变量、policy 类型、Agent reducer
├── openbot-application     # 所有 use case 的唯一入口
├── openbot-infra           # PostgreSQL、vault、HTTP safe dialer、provider adapters
├── openbot-agent           # built-in loop、AG-UI、tool runtime、MCP、connectors
├── openbot-computer        # manager、supervisor、browser protocol、screen、file/shell
├── openbot-server          # Axum HTTP/SSE/WS、static GUI、health/readiness
├── openbot-ui              # Leptos CSR/WASM
├── openbot-desktop         # Tauri、window ACL、in-process、sidecar/update
└── openbot-testkit         # golden trace、fault injection、fake provider、xtask
```

拆分规则固定为：只有独立安全边界、独立发布单元、明显不同的 feature graph 或可单独复用的纯协议才建 crate；其余以 module 组织。原补充方案的 17 个以上细粒度 crate 会放大编译、循环依赖和跨 crate 重构成本，不进入首版。

### 5.2 Hexagonal ownership

`openbot-application` 暴露 typed service：

```rust
#[async_trait]
pub trait ApplicationService: Send + Sync {
    async fn execute(
        &self,
        auth: AuthContext,
        command: AppCommand,
    ) -> Result<AppReply, AppError>;

    async fn subscribe(
        &self,
        auth: AuthContext,
        request: SubscriptionRequest,
    ) -> Result<AppEventStream, AppError>;
}
```

Axum、Tauri、测试和迁移工具只做认证、framing、输入大小限制和错误映射，不各自实现业务规则。任何 transport 都不得接受自由 method string、renderer 自报角色、renderer 自报 `principal=admin` 或任意数据库 query。

### 5.3 核心 ID

本节下列核心 ID 是 string newtype（R23 裁决的两个 generation 除外），不擅自限定为 UUID；
创建端可以使用 UUIDv7/ULID，兼容端必须接受上游既有字符串。

```text
DeploymentId
TenantId
ActorId
BotId
ChannelId
ThreadId
RunId
ToolCallId
CredentialPrincipalId
ComputerId
ComputerGeneration
TabId
DocumentGeneration
PolicyDecisionId
AuditEventId
```

跨 crate 的内部状态轴不伪装成上述第 16+ 个公开 wire ID，但也不允许每层自建
同名 newtype。`AttemptId` / `CapabilityId` / `CatalogGeneration` 由
`openbot-contracts::ids`唯一定义，`AuthGeneration` 由 `openbot-contracts::auth`唯一定义并直接
存入 `AuthContext`；四者都不 serde，外部不能自报这些内部授权状态。`SecretId` 与
`CredentialGeneration` 当前没有任何 domain 外消费者，因此继续留在 vault；第一条
真实 repository/application 用例出现时再与用例同批上收。（§28.1 R47）

`AuthContext` 只能由 Rust 根据 session、连接 peer、数据库 ACL 和资源映射构造。模型、renderer、MCP server、remote Agent 或 browser engine 传来的同名字段一律视为普通不可信输入。

## 6. Auth、People、Session 与 Vault

### 6.1 Desktop 与 Server 身份模型

| 模式 | 身份真源 | 固定行为 |
| --- | --- | --- |
| Desktop Local | 当前 OS 用户 + 本地 app instance | 单用户 admin；不启动 Web SSO；本机 capability 不可远程使用 |
| Desktop Remote / Server | Rust session + OIDC/SAML identity | 多用户、role、membership、revocation、fresh authorization |

无 IdP 时，Server 只有显式 `OPENBOT_SINGLE_USER=true` 才启动；该模式只允许 loopback 或管理员明确配置的受控网络绑定。`NODE_ENV` 不改变此规则。
Server 单用户的兼容身份固定为 `id=dev-local-user`、`email=dev@openbot.local`、唯一有效角色
`admin`；重复启动恢复 canonical email/name 但不回退 `auth_generation`（R53）。固定 id 是既有
thread/memory 的归属键，不能随重写改名。

### 6.2 必须实现的认证面

1. Google、Microsoft Entra、Okta 可同时配置。
2. 动态注册 deployment-owned OIDC/SAML provider，以 email domain routing。
3. OIDC 使用 Authorization Code + PKCE S256、state、nonce、issuer/audience 校验、JWKS rotation 和精确 redirect URI。
4. SAML 验证签名覆盖、schema、Destination、Audience、Recipient、`InResponseTo`、时间窗和 assertion replay；拒绝 SHA-1、外部实体与 unsigned response/assertion。
5. Entra email 解析按 `email → upn → preferred_username`，仍需 verified issuer/tenant policy。
6. `INITIAL_ADMIN_EMAILS` 是 admin floor；被列出的用户不能在 UI 中自我降权或被撤销。
7. 普通管理员不能撤销自己；最后一个有效管理员不能被删除。
8. 删除用户会清理 active session，并以规范化 email 写入 `revoked_access`，阻止下一次 IdP 自动 provision。
9. IdP 归 deployment 所有，不随登记它的管理员删除。
10. role、membership 或 access generation 更新后，现有 WS subscription、screen ticket、approval、run assertion 和 capability 立即失效。

`AuthContext` 已把 auth generation 作为所有长连接 / ticket / capability 的共同失效轴，但上游 0012 schema 没有权威持久化列。Rust-owned 0014 因此只在 `users` 末尾追加 nullable `auth_generation bigint` 与非负 CHECK：旧行 `NULL` 在读侧等价 0，role/access 的真实变化在同一 acting 事务内用 `coalesce(auth_generation, 0) + 1` 推进；兼容窗口不 `SET NOT NULL`、不回填伪造历史代际。该列是服务端授权状态，不进入 `/api/admin/people` 的公开 DTO。（§28.1 R39）

OIDC discovery/JWKS 与任何 IdP metadata fetch 使用和 remote Agent/MCP 相同的 safe dialer、redirect/IP 校验、大小/时间上限；SAML metadata 默认由管理员粘贴/上传并离线验证，不允许一个未验证 URL驱动 server 内网请求。

pre-auth surface 只公开环境配置的 provider ID 和“存在企业 SSO”布尔值，不列出企业 domain/provider；email routing 的成功/失败使用统一响应并按 IP/email hash 限速，避免组织枚举和 callback flood。

Rust 选型：OIDC 使用 `openidconnect` 4.x；SAML 使用固定版本 `samael`/xmlsec 组合并接受独立 XML signature wrapping/replay 外审。SAML 外审未通过时不得发布 GA，不能关闭功能冒充对等完成，也不能用 Node/Java sidecar 绕过 Rust 边界。

### 6.3 Session

- server cookie：`HttpOnly`、`SameSite=Lax`，host-only，短 idle + 绝对期限；`Secure` 当且仅当 `OPENBOT_PUBLIC_URL`（§15.4：它接替 `BETTER_AUTH_URL` 成为唯一公共地址来源）是 `https` 时设置——上游 `docs/deployment.md` 写的是"把 TLS 放在前面"而不是"拒绝 HTTP"，且 CHANGELOG 明确修过"plain HTTP 真实地址上无法开始会话"，所以非 loopback 的 plain HTTP 部署仍可登录，但启动日志告警、`/health` readiness 附带 `insecure_transport: true`；不另设开关。对应地，Leptos GUI 不得依赖只在 secure context 才有的浏览器 API（`crypto.subtle`、`crypto.randomUUID` 等）——ID 与随机数一律由服务端签发或走 `crypto.getRandomValues`；
- session token 数据库只保存 keyed hash，不保存可直接使用的明文 token；
- 敏感 admin 写操作要求 fresh session，并校验 CSRF/origin；
- refresh/reauth 不沿用旧 auth generation；
- WebSocket handshake 绑定 fresh session，之后每次高权限 server request 再检查 generation；
- 从 Better Auth 切换到 Rust Auth 时，旧 Better Auth session 全部失效，用户统一重新登录一次；不反向工程其 cookie。

Rust session 复用上游 `sessions` 表但不复用其明文 token 语义：`token` 固定写
`sh1_` + `HMAC-SHA256(OPENBOT_SESSION_SECRET, raw_token)`，resolver 对 cookie 同样计算后等值查询，
数据库永不保存 bearer 原文。native 0015 只在 `sessions` 末尾追加 nullable
`auth_generation bigint` + 非负 CHECK；新 session 写签发时 generation，旧 Better Auth 行保持
NULL/明文，resolver 两项都不认，统一 401 重新登录。每次请求同一查询读取 session、user 当前
generation、deny list 与 role，随后按 timeline → generation → absolute → idle 判定；普通请求认证
通过即 touch，敏感写只有 Origin/fresh guard 也通过后才 touch，被拒 CSRF 不得续 idle；缺角色
返回 403，不降级成 user。敏感 role/access 写还必须携带 live-session assurance 与可信 Origin，
四种拒绝各有稳定码。（§28.1 R42）

### 6.4 Vault

| 环境 | Master key | Record key |
| --- | --- | --- |
| Desktop | Keychain / Windows Credential Manager / Secret Service | 每记录随机 DEK，由 master key 包装 |
| Server | KMS/HSM 或受控 secret manager 中的 tenant KEK | 每记录随机 DEK，由 tenant KEK 包装 |

record AEAD 的 AAD 固定绑定 `tenant_id + secret_id + kind + owner + consumer + key_version`。secret 数据模型同时记录 resource、scope、expiry、credential generation 和 revocation state。

迁移期必须兼容读取当前 AES-GCM v1 envelope（12 字节 IV、无 AAD）。迁移顺序固定为：读 v1 → 解密 → 事务写 v2 → 校验回读 → 标记旧 envelope retired。不能在同一 release 同时更换 Auth、KEK 和 credential schema。

以下值永不进入 Leptos state、Agent prompt、AG-UI、browser event、普通日志、trace、metric、crash dump 或 screen URL：model key、MCP/OAuth refresh token、OIDC/SAML secret、computer bootstrap secret、run signing key、updater key。

持有这些明文/密钥字节的 `SecretBytes` 内部固定为 `zeroize::Zeroizing<Vec<u8>>`：
drop 时清除 `Vec` 当前长度和整个 capacity，并以稳定 Rust 优化屏障保证写不被删。
它仍不伪称能擦除所有副本：调用方交出所有权之前的读缓冲/拷贝，以及该 `Vec`
交出前扩容留下的旧 allocation，都不在类型可达范围。`SecretBytes` 仍无 Clone /
Serialize / Display / PartialEq，只能显式 `expose`。（§28.1 R46）

### 6.5 Group access 的修正

当前 `users.groups` 与 `channels.allowed_groups` 不是生效的控制（[#82](https://github.com/CopilotKit/openbot/issues/82)），而且包声明的 channel 没有任何 membership 写入路径，对所有人不可达（§2.4）。Rust 版作确定修正：

1. 每个动态 IdP 可配置一个明确的 group claim path 和规范化规则。
2. `allowed_groups` 的取值固定三档：保留字 `all`（精确匹配、区分大小写）= 部署内全体有效用户（含 `INITIAL_ADMIN_EMAILS` 与单用户 principal），不需要任何 IdP mapping——这是随包示例 `examples/fintech/channels.yaml` 已在用的写法；具名组 = 必须由至少一个已配置 IdP 的 group mapping 解析；空列表 = 包校验错误 "channel has no audience"，不再静默不可达。
3. 多用户 Server 上出现具名组但没有任何 IdP 配置 group mapping 时，package 校验失败并指出缺哪一家的 mapping。单用户模式（Server `OPENBOT_SINGLE_USER=true` / Desktop Local）只有一个 principal，组无法区分任何人：该 principal 被 provision 进全部包 channel，包报告注明"单用户：组不参与裁决"，不拒绝启动。
4. 登录时将 verified group claims 写入 membership projection；每次 session refresh 重算；包同步时对 `all` 做一次全量 provision，新用户首次登录时补齐。
5. group 只负责 provision channel membership，所有运行时 channel route 仍检查 materialized membership。
6. IdP 撤组后递增 auth generation 并撤销相应 membership；不等待下次应用重启。

包同步同样走 materialized membership：`all` 对全部有效用户全量 provision；单用户 principal
进入全部包 channel；audience 收紧时 membership 删除、auth generation 与 session 清理同事务。
现有 `users.groups` 没有持久化 provider/normalization provenance，故同步期对具名组只做逐字匹配
（宁可暂时少授予，不借另一家 IdP 的规则扩大等价类），下一次 verified 登录再用权威 mapping
经同一 `project_membership` 补齐。（§28.1 R60）

## 7. Rust Agent、Provider 与 AG-UI

### 7.1 产品中存在两类 Agent

| 类型 | 实现责任 | 语言约束 |
| --- | --- | --- |
| built-in Agent | `openbot-agent` 内的 Rust loop | 第一方生产逻辑必须 Rust |
| remote AG-UI Agent | 外部 endpoint | 任意语言；按不可信服务处理 |

上游 `RegisteredAgent` 实际有三种：`built_in`（包声明、CopilotKit `BuiltInAgent`）、`remote_ag_ui`（外部端点，含 §3.4 的 managed Bot）、`unavailable`（profile 已软删除但旧 channel 仍可读的 tombstone，每次 run 直接拒绝、不联系端点）。Rust 版把前两种分别落到上表两行，并把 `built_in` 与 managed Bot 插槽统一为同一个 Rust loop；`unavailable` 保留为 Agent 注册表的第三种终态（§3.2 "soft delete 后旧 channel 可读、不可再次运行"的实现面），不是新类型。

Rust loop 不能删除 BYO Agent。remote Agent 也不能直接调用 vendor 或 computer 绕过 Rust gateway；它只能使用运行时给出的工具，并以 per-agent token + signed run assertion 回调 Rust。

### 7.2 Agent reducer

核心保持 pure reducer：

```rust
pub fn reduce(
    state: &AgentState,
    event: AgentEvent,
) -> Result<(AgentState, Vec<AgentEffect>), InvariantViolation>;
```

固定状态：

```text
Queued
→ Preparing
→ Sampling
→ AwaitingApproval | AwaitingHuman
→ ExecutingTools
→ CommittingResults
→ Sampling
→ Succeeded | Failed | Cancelled | ReconciliationRequired
```

数据库、provider、MCP、browser、file 和 shell 都是 runtime effect。Reducer 不持有 Tauri handle、SQL pool、HTTP client、Electron socket 或 secret。

每个 thread 一个 foreground actor，串行处理 prompt、steer、cancel、tool result、MCP/computer lifecycle 和 timeout。任何后台任务必须是独立 durable run，不共享 foreground mutable future。

run 预算三条，**哪条是 parity、哪条是新增，分开写**：

| 预算 | 来源 | R1 固定值 |
| --- | --- | --- |
| tool step cap | parity：`server/src/copilot.ts::TOOL_STEPS = 8` | 8，不可配置（与上游一致） |
| 流静默看门狗 | parity：`AGENT_STALL_TIMEOUT_MS`（未设或 `0` = 关；`.env.example` 发 `60000`；触发写 `agent.stream_stalled`） | 变量名、语义、audit 事件名原样保留；判据是"远端 body 真实 read 间隔"（§7.5） |
| run 绝对期限 | **新增**：固定 commit 没有任何 run 级绝对期限（全仓唯一 30 分钟常量是 `COMPUTER_BROWSER_IDLE_MS`，那是浏览器空闲驱逐，另一概念） | `OPENBOT_RUN_DEADLINE_MS`，默认 `1800000`，`0` = 关；到点按 §7.4 走 `Cancelling → Cancelled` 并写 audit，不是静默杀进程 |

### 7.3 Provider adapters

首版固定三类 provider：

1. `openai-compatible`：OpenAI Chat Completions/Responses，以及明确声明兼容的网关/xAI endpoint；
2. `anthropic`；
3. `google-generative-ai`。

三类的 parity 依据必须说清，否则就是 §0.4 禁止的"新模型专用集成"：固定 commit 里，包声明的 built-in Bot 被 `tenant-package.ts` 钉死 `model.provider must be openai`；而管理 Bot `agent-langgraph`（产品内创建的 coworker 的默认端点）按 `BOT_PROVIDER=openai|anthropic|google` 选 provider，并读 `BOT_MODEL` / `BOT_RESPONSES_API` / `{OPENAI,ANTHROPIC,GOOGLE_GENERATIVE_AI}_BASE_URL`。Rust built-in Agent 同时承接这两个角色，所以三家是 parity 面。provider 的选择位置也按上游分两层固定：包 Bot 读 `model.yaml`，`provider` 继续只接受 `openai`（放宽是独立产品变更）；managed 插槽读 `BOT_PROVIDER` / `BOT_MODEL` / `BOT_RESPONSES_API` 三个部署级变量，缺 key 时与上游一致拒绝启动而不是静默降级。`agent-bot` 的 Rust 重写 `openbot-reference-agent` 保持 OpenAI Chat Completions 单协议（上游注释："proof-of-concept Bot is OpenAI only by construction"）。

包 Bot 的 OpenAI 协议与 credential 选择按固定发布物进一步写死：上游锁定的
`@ai-sdk/openai@3.0.99` 中，默认 `createLanguageModel` 直接委托 `createResponsesModel`，所以包 Bot
固定走 Responses；`BOT_RESPONSES_API` 只控制 managed 插槽，不跨层改变包 Bot。每次 sampling 都重新按
`credentials(kind='model', provider='openai', key_id=model.yaml::credential_secret_ref,
revoked_at IS NULL)`、`created_at DESC, id DESC` 选择并经 Vault 解封；无 active matching row 才回落
trim 后的 `OPENAI_API_KEY`，matching row 损坏不得回落。包的 `default_model` 与 `system_prompt` 同样每
run 从权威 package/Agent 投影取得，不得被 `BOT_MODEL` 或旧缓存覆盖。（§28.1 R69）

不建立 `xai` 专用 crate。每个 adapter 用 `reqwest`/SSE 或 WebSocket 隔离实现，并输出统一事件：

```rust
pub enum ProviderEvent {
    ResponseStarted { response_id: String },
    OutputItemAdded { index: u32, kind: OutputKind },
    TextDelta { index: u32, delta: String },
    ReasoningDelta { index: u32, delta: String },
    ToolCallStarted { index: u32, call_id: String, name: Option<String> },
    ToolArgumentsDelta { index: u32, call_id: String, delta: String },
    ToolCallCompleted { index: u32, call_id: String, name: String, arguments: Value },
    Usage(Usage),
    Completed,
    Failed(ProviderFailure),
}
```

解析器必须接受 skeleton item、字段延迟、交错的并行 tool arguments、partial JSON、未知扩展事件和 UTF-8 分片；聚合键使用稳定 index + call ID，不假设 provider item ID 永不变化。原始事件默认不持久化，只把规范化事件写 journal。

managed provider 的首版协议再固定如下：Anthropic 走 Messages streaming API，API key 只放
`x-api-key`，`anthropic-version=2023-06-01`，system 与 messages 分离；未显式配置 output cap 时才按
锁定 `@langchain/anthropic@1.5.6` 的 model table 取默认。Google 走
`v1beta/models/{model}:streamGenerateContent?alt=sse`，API key 只放 `x-goog-api-key`，不得进入 query；
`systemInstruction`、`contents`、`functionDeclarations` 与 `functionResponse` 分域。锁定
`@google/generative-ai@0.24.1` 的 stream DTO 没有稳定 response id，Rust 侧以首个规范 chunk 的
SHA-256 合成仅用于 trace/correlation 的确定 id，不把它当授权或业务 identity。三家 adapter 都必须在
`Completed` 前给出单调、自洽的 normalized `Usage`；缺失、重复或回退按 invalid response fail-closed。
这些是本地协议实现证据，不等于 provider gate 要求的三家 recorded vendor trace。（§28.1 R70）

### 7.4 Retry、Cancel、Budget 与 Commit

- `CancellationToken` 按 run → provider/tool/computer/process tree 传播；
- UI 先显示 `Cancelling`，收到子任务终止事实后才显示 `Cancelled`；
- 429、明确可重试 5xx 和连接前失败可指数退避；认证、schema、policy 错误不重试；首版按锁定 `@langchain/core@1.2.8` 固定为首次请求后最多 6 次重试、1s×2 指数退避、`[1,2)` jitter、指数项最多 64s，`Retry-After` 取其与指数项较大值，外层 absolute deadline 始终是总上限；只允许 pre-send `Unavailable` 或 session 首事件就是明确 429/5xx 时重试，见到任何响应身份/增量后的 transport/commit-unknown 永不自动重放；
- 非幂等请求已发送但未确认时，`commit_state=Unknown`，进入 reconciliation；
- tool 只有显式 `parallel_safe=true` 且资源锁不冲突才并行；结果按原 call 顺序回注；
- 一次 sampling 先收齐 complete tool-call batch，再按稳定 output index 排序；`parallel_safe=false`（首个 `remember` 即此类）严格串行。provider call id 只配对 assistant call/result，Rust gateway 另铸 UUIDv7 + per-run sequence 作为 decision/attempt/capability 的唯一身份。每个确定 outcome 先以 assistant/tool 两条 message + `tool_exchange` checkpoint 同事务持久化，context 重读成功后才开始下一次 sampling；exact expected-sequence replay 返回原 receipt，任何参数/结果篡改 conflict。batch 跨 sampling 累计仍受 8-step cap，超过上限时一个新 effect 都不执行；
- budget 同时限制 absolute deadline、idle deadline、provider token、tool steps、并发 tool、computer runtime 和用户配置的费用上限；首版新增 `OPENBOT_PROVIDER_MAX_OUTPUT_TOKENS`，缺省 16384，只接受 1..=1000000，0 不能静默关闭；它是**每次 sampling 输出上限**，三家 request 和 host 的 normalized usage 双重校验。真实 tool loop 落地时仍须累计整个 run 的多步 input/output token 与费用，本变量不能冒充完整 budget；
- Agent durable activate 后、读取 context/provider 前写 `agent.invoked`；真实 body read gap 到点时先停止 session，再写 `agent.stream_stalled`，最后 failed terminal；absolute deadline 到点时先停止 child，再写新增 `agent.run_deadline_exceeded`，最后走 `Cancelling → Cancelled`。三类 audit 只带权威 run/actor 与 allowlisted stable code，任一 audit 写失败都进入 reconciliation，不继续 sampling/提交普通终态；
- 上下文压缩保留 system/standing role、未完成 tool pair、最近对话和 provenance；压缩摘要带 source range。

### 7.5 Remote AG-UI

`openbot-agui` 是 `openbot-agent` 内的边界模块，不把 community Rust SDK 类型暴露进 domain。协议 ID 使用 string newtype。

必须支持固定 AG-UI schema中的 lifecycle、text、tool call/result、state snapshot/delta、messages snapshot、activity、step、reasoning、raw/custom、interrupt/resume 和错误；每个 run 恰好一个 terminal event。

安全链路：

```text
Rust session 确定 actor
→ 为 bot/run/actor/tool-set 铸造 10 分钟 run assertion
→ safe dialer 连接 remote AG-UI endpoint
→ standing role + granted tools + assertion
→ remote Agent 以自己的 callback token 回调
→ Rust 同时验证 token hash、assertion、bot、actor、run、tool 与 expiry
→ grant → policy → audit → act
```

endpoint 注册和每次运行都使用同一 safe dialer：最多 3 次 redirect，每一跳重新检查；禁止 metadata、loopback、link-local、private/reserved IP，除非管理员配置了精确 CIDR allowlist；连接绑定已验证解析结果，防止检查后 DNS rebinding。auth header 只对同 origin redirect 发送，跨 origin 必须剥离。

stall watchdog 测量 remote body 的真实 read 间隔，不把下游消费者背压误判为 Agent 沉默；超时写一个 terminal error、取消上游并留下脱敏 audit。

### 7.6 Codex 与 Grok Build 的使用边界

OpenAI 官方把开源 Codex 描述为可嵌入产品的 agent harness，并把 app-server 用于 persistent conversation、stream、interrupt 和 approval。正式利用方向：

- Codex：tool routing、approval/sandbox 分层、cancel、critical event、bounded shutdown；
- Grok Build：per-session actor、prompt queue、lifecycle、workflow journal、可重放测试；
- CrabCode：本机 supervisor、permission/sandbox/exec、browser protocol 和历史 Tauri 生命周期。

| 参考实现 | 吸收内容 | 本次明确拒绝 |
| --- | --- | --- |
| Codex app-server in-process | bounded queue、critical vs coalescible event、lag visibility、oneshot cleanup、finite shutdown | Codex JSON-RPC DTO、initialize handshake 和 AppServer 兼容层 |
| Codex tool router/sandbox | request 与 execution 分域、approval、cancel、sandbox plan、结果回注 | Codex 工具名称、coding-terminal 产品假设、消费者账号 |
| Codex MCP manager | identity/generation、required failure、catalog provenance 的测试思想 | 常驻 ConnectionSet、跨 actor 连接复用 |
| Codex provider parser | skeleton、partial delta、unknown event 的负向 fixture | 把 OpenBot domain 绑定 OpenAI event 类型 |
| Grok session actor/prompt queue | foreground turn 串行、steer/cancel、prompt queue、lifecycle message | shell/TUI session monolith |
| Grok workflow journal | request hash、sequence、deterministic replay、failure fixture | SQLite journal、通用脚本引擎越过 PolicyGate |
| Grok tool runtime | Tool/dispatch/error/notification contract 思想 | xAI 产品 DTO、二次移植工具的错误来源归属 |
| Grok MCP | generation/liveness/ingest limits 的测试案例 | 较旧 protocol pin 和直接作为本项目 runtime |

不复制 Codex/Grok 的完整 session loop、产品 DTO、账号、终端 UI 或消费者 OAuth。Grok Build 中来自 Codex/OpenCode 的工具必须追溯原始来源，不能重复记作 xAI 独立来源。

## 8. Tool、Policy、Approval 与 Audit

### 8.1 唯一执行管线

```text
RequestedToolCall
→ schema/size validation
→ resolve authoritative actor/target
→ effect classification
→ CEL + structural/content policy
→ optional human approval
→ DB transaction: decision + attempt
→ mint single-use capability
→ execute
→ outcome + commit_state
→ projection/outbox
→ redacted model-visible result
```

decision 写入失败即不执行。执行发生但 outcome 无法写入时，run 进入 `ReconciliationRequired`，不能继续工具循环或自动重试。

Application 执行面固定为同一条构造性路径：`ToolInvocation` 不含 actor/effect/policy/approval/target 自报字段；`AgentToolGateway` 用 UUIDv7 与 per-run sequence 铸造 call 身份；application 先用权威 catalog 校验 metadata/参数，再要求 scope resolver 回给与 invocation 逐字段相等的 run/Bot/sequence 以及不可反序列化的 `PolicyContext`。`ToolPolicyEvaluation` 只能从 domain 的 deny-first/default-deny/dry-run 结论构造；enforce/approval 拒绝先写 allowlisted audit 且不创建 attempt，dry-run 先记拒绝再继续。放行路径在 PostgreSQL 同事务写 decision + attempt，commit 后才拿 receipt；capability CAS 绑定成功后 application 才能构造字段私有的 `AuthorizedToolCall`，executor 必须消费它才能得到参数与 redeemed proof。执行后 outcome 与 audit 同事务；audit 写失败回滚 outcome 并返回未受理 reconciliation，已持久化的 `commit_state=unknown` 返回已受理 reconciliation，二者都不得成为成功 `ToolResult`。（§28.1 R41）

这条最初闭合的是**通用 application/infra/agent gateway 边界**。R71 后首个真实集合成员 `remember` 已以 production PostgreSQL effect 穿完：fresh ACL → metadata/schema → CEL → decision+attempt → capability CAS → `origin=remember_tool` memory → outcome+audit → durable tool pair → 下一次 sampling。Fact provenance、owner、Bot/thread target 都从当前 run/DB 取得；模型参数没有自报 ID 的位置。`AuthGeneration` 随不可序列化 execution scope 进入 executor，并在 memory INSERT 同一事务再次比较 generation/revoked/role，堵住 capability mint 后撤权竞态。R74–R76 又让 RMCP read、credential-backed MCP 与 Drive REST 穿完。R77 把 acting MCP 从固定 deny 推进到 durable pending→human grant/deny→decision/attempt：grant 的 `approval_id` 同时进入 tool_calls 与 outcome audit，deny 仍零 attempt/vendor call。browser/file/shell 与可点击 Leptos approval UI 尚未闭合，所以本条仍不能写成“G4 整关通过”。（§28.1 R71 / R74–R77）

### 8.2 Tool metadata

每个 tool 固定声明：

```text
name/schema_hash/catalog_generation
effect = read | write | execute | network | credential
idempotency = idempotent | keyed | non_idempotent
parallel_safe
timeout/deadline
approval_class
sandbox_requirement
input/output/redaction limits
resource_lock keys
```

未知 effect 固定按 write/execute；MCP annotations、server description、工具名称和模型声明都不是可信分类来源。

### 8.3 CEL

使用 crate `cel` `0.14.3`（§1.2；"cel-rust"是仓库名不是 crate 名，Cargo 里不存在），但不能凭语法相似宣称替代 `cel-js`。Phase 0 从现有默认、测试和生产脱敏 policy 构建 corpus；Rust 对每条 expression、context、结果和错误语义做 golden 对照，oracle 固定为 `cel-js@0.8.2`。

两处已核实的引擎差异必须进 corpus，不能留到上线后发现：

1. `cel-js@0.8.2` **没有任何字符串方法**：`element.name.contains("x")`、`startsWith`、`endsWith`、`matches` 方法形式都会抛 "Unknown method"，上游靠两个注入的**全局函数** `contains(haystack, needle)`（大小写不敏感）与 `matches(value, pattern)` 工作（`server/src/computer/policy.ts`）。Rust 引擎必须注册同名、同签名、同大小写语义的两个全局函数；标准 CEL 方法形式（大小写敏感）允许作为超集存在。
2. 后果：一条在上游"求值出错"的规则（deny 出错 → 拒绝；allow 出错 → 不放行）在 Rust 里可能变成"正常求值"。迁移 preflight 对每条已持久化规则在两个引擎上各跑一遍 corpus context，**结果类别**（true / false / error）任一不同即在迁移报告高亮并要求管理员逐条确认后才导入；不确认的部署不切 policy writer。这是 §8.3 "不悄悄收紧或放宽"的机械执行面。

求值器的资源边界与失败记录方式同样是契约，不能留给实现自由发挥（§28.1 R26 / R27）：

- **解析前的输入闸门**：一次非递归线性扫描，拒绝超过 4096 字节或括号嵌套超过 8 层的表达式（扫描认字符串字面量，`contains(page.url, "((((")` 这类规则不被误伤）。**解析本身放在求值器自己拉起的、栈大小写死 16 MiB 的线程上并立即 join。** 理由是实测的：`cel 0.14.3` 的 antlr4rust 递归下降解析器每 MiB 栈约扛 6 层嵌套，~1 MiB 的线程第 6 层就把栈打穿，而 Rust 的栈溢出是 abort 不是可捕获的 panic —— 策略表达式来自管理员可写的列，于是"一条写歪的规则打死进程"是真实路径；而崩溃点随线程栈大小变化，等于让答案取决于跑在哪个线程上。求值侧不需要同样待遇（深度 64 的 AST 在 ~1 MiB 主线程上正常求值），也不能要：求值在工具调用热路径上。
- **失败不得携带 context 取值**：`cel` 的 `ExecutionError` 有若干变体把参与运算的值放进错误本体并由 `Display` 逐字打出（实测 `page.url + 1` 会把带 token 的 URL 打全）。失败在离开求值器的那一刻压成一组无载荷的封闭分类，分类靠**匹配变体**而不是读消息文本。表达式原文可以带（管理员写的规则），context 取值一律不带 —— 这与 §8.6 的 payload 字段 allowlist 是同一条要求的两个落点，也是上游 F-CEL-6 缺陷的 Rust 侧对应面。

规则固定：deny 先于 allow；missing/empty/broken policy fail-closed；`dry-run` 只改变执行拦截，不跳过 decision/audit；policy version 进入 approval 和 capability，版本变化后旧批准失效。多 replica 下 policy / grant / catalog 变更沿用上游 `policy-listener.ts` 的形态：PostgreSQL `LISTEN/NOTIFY` 只做唤醒，每个 replica 收到通知或重连后**整表重读**（NOTIFY 载荷 8000 字节上限，不用它带内容），并把 `policy_version` 写进每个 decision——一个 replica 用旧版本做出的 decision 在 audit 里可辨认。

新安装没有隐式 `allow: ["true"]`。首次设置 wizard 必须由本地 owner/admin 选择并保存一个有版本的 policy preset；在完成前所有 acting tool deny。旧部署中已持久化的显式 allow policy 原样导入并在迁移报告中高亮，不被 Rust 悄悄收紧或放宽。

### 8.4 Content governance

结构性权限仍是安全真源。内容检查固定为辅助层：

- 已知 API key/token 格式和高置信 secret canary 在发送到外部 MCP/provider 前默认 block；
- 信用卡号使用 Luhn 校验，其他 PII 默认 tag + audit-only，管理员可写 policy 提升为 deny；
- prompt-injection detector 只产生 risk signal，不能单独授予权限，也不能宣称消除注入；
- page/tool content 进入模型前标记来源和“不可信内容”，任何其中的授权指令无效；
- run 费用、token、tool step 和时长由独立 budget counter 强制，不依赖模型自律。

### 8.5 Approval

approval 绑定 `actor + bot + run + tool + canonical args hash + target + computer/catalog generation + policy version + expiry`。任一字段变化、角色撤销、页面导航、computer restart 或 catalog refresh 都使 approval 失效。

approval UI 展示真实 effect、target、diff/arguments 摘要和可能副作用；不得只展示模型生成的自然语言理由。

R77 的 production backend 将这条具体化为 native 0020 `tool_approvals`：binding 除本节原字段外再绑定 §6.2 的 `AuthGeneration`；pending 才保存由 first-party resolver 产生的 16 KiB bounded redacted arguments/change summary，grant/deny/expire/cancel 同事务清 NULL。TTL 固定 5 分钟（**新增**），DB clock 到点即失效；同一 run 只有 `once_per_run` 且全部 binding 逐字段相等时可复用，`every_call` 每个 call 都新问。请求与 requested audit 同事务后才可见；fresh same-origin grant/deny 只提交 decision enum，不接受任何 binding 字段。waiter 用同进程 notify + 1 秒 durable poll 跨 replica 醒来；run/member/lease/AuthGeneration 变化先转 cancelled。grant 后 application 再观察 current role/catalog/policy，journal 同事务只在 approval 仍 granted/未过期且 actor/Bot/run/tool/args/target/effect/class/catalog/policy 匹配时写 decision + approval id，否则零 effect。

当前只给 MCP/Drive target 提供 production observation（computer/document generation 均为非 computer 的零/None）；browser 接入时必须从 engine authority 重读两代际，不能把 request snapshot 原样当 current observation。`openbot-ui` 只有 authority-only card view model，尚无 Leptos component、焦点管理、键盘/读屏/倒计时与真实点击旅程；因此“approval backend/API”可勾，“approval GUI”仍不勾。

### 8.6 Audit

- 表语义保持上游现状，不重建：`audit_events` 由行级触发器拒绝 UPDATE，`0012` 起连 TRUNCATE 也拒绝；DELETE 只在声明了 retention 窗口的事务里、且只对窗口外的行放行。`AUDIT_RETENTION_DAYS` 原名原义保留（未设 = 永久；≥ 1 的整数；非法值拒绝启动），retention sweep 仍是"带锁的分批行删除"（上游 `audit-retention.ts`），只是改为由 Rust 用**独立 DB role** 执行；
- Server：业务 DB role 对 audit 只有 INSERT/SELECT，无 UPDATE/DELETE/TRUNCATE；migration 与 retention 各用分离角色；
- **不做表分区**：把既有 `audit_events` 改成分区表在 PostgreSQL 里等于建新表 + 搬行 + 换名，违反 §14.3 兼容期禁令，且上游的触发器语义已经给出同等保证。分区化列为 GA 后独立运维变更，自带 delta audit；
- hash chain 以**追加 nullable 列**落地（`prev_hash` / `row_hash`，首条 Rust 写入的行是 genesis，旧行 hash 为 NULL 并在 genesis checkpoint 里记录"链起点之前有 N 行未入链"）；周期 checkpoint 签名后写入本库 `audit_checkpoints` 表；**外部不可变存储是可选 sink**（S3 object-lock 或只追加文件），未配置时 readiness 不受影响——不把一项新基础设施写成上线前置；
- retention 删除窗口外的行之前，先为被删区间写一条包含首尾 `row_hash`、event count 的 closure checkpoint，链边界由此保留；
- Desktop：同样 append-only，但只承诺可追溯，不宣称抵抗设备所有者/root 篡改；
- payload 使用字段 allowlist，不保存原始 header/body、prompt、tool full result、screen frame、文件内容、secret 或可验证 secret hash；
- explicit `remember` 的执行前拒绝、committed success、确定 failed outcome 分别写新增 `memory.remember_refused` / `memory.remember_succeeded` / `memory.remember_failed`；unknown commit 不冒充 failed，而是 run reconciliation。三者 payload 只含 tool/target/args hash/policy/catalog/commit 等 allowlisted 事实，不含 memory content；
- human takeover 记录 request/taken/released，不记录每个键盘和鼠标事件；
- secret 输入只记录 secret ID、用途、目标字段和长度，不记录值。

## 9. MCP、Google Drive、OAuth、Skills 与 Grants

### 9.1 首版 MCP runtime 的精确范围

“Rust MCP runtime”在本次重写中固定表示：

1. RMCP 3.1.4 client；
2. MCP 2026-07-28 协议协商及现有兼容版本；
3. remote Streamable HTTP；
4. `initialize`、capability negotiation、`tools/list`、`tools/call`、cancel/timeout/progress；
5. OAuth 2.1/PKCE、catalog cache、result normalization；
6. grant、effect classification、policy、audit 和故障处理。

首版不向产品暴露 stdio、resources、prompts、tasks、elicitation，也不接受模型动态安装本机 MCP server。RMCP crate 可以包含这些类型，但运行时 capability 必须不声明，UI 不显示，测试确认无法调用。

单 server 固定上限：1,000 tools、每 tool description 4 KiB、input schema 256 KiB、单 call model-visible text 20,000 Unicode scalar values；超限 listing/call 显式失败或可见截断，不静默把任意 vendor payload塞入模型上下文。四个上限里只有最后一个是 parity（`server/src/plugins/mcp.ts::MAX_RESULT_CHARS = 20_000`，截断后附 `[truncated: the tool returned N characters]`），前三个是新增加固。计数单位是一处**有意的差异**：上游按 JS `.length` 数 UTF-16 code unit 并可能把一个字符从代理对中间切开，Rust 按 Unicode scalar value 数且永不切开字符；golden 对照里非 BMP 文本允许长度差异，fixture 注明。超时原名原值保留：`tools/list` 15 s、`tools/call` 60 s、OAuth token 换取 10 s（`LIST_TIMEOUT_MS` / `CALL_TIMEOUT_MS` / `TOKEN_TIMEOUT_MS`），并服从 §7.2 的 run 级预算。

### 9.2 连接生命周期

user-OAuth 和 bearer remote server 固定 per-call：创建 client → initialize → list/call → close。它消除跨用户 session 复用，符合现有 OpenBot 的谨慎语义。

v1 禁止任何跨 Bot、跨 actor MCP pooling。若远端协议返回 session ID，也只在本次 call 生命周期使用。连接 identity 至少包含：

```text
tenant_id
actor_id
bot_id
server_id
credential_generation
transport_fingerprint
protocol_version
catalog_generation
```

### 9.3 Catalog 与 stale grant

工具缓存记录 `server_id + name + schema_hash + effect + catalog_generation + first_seen + last_seen`。refresh 在一个事务中：

1. 写新 catalog generation；
2. 标记消失工具；
3. 将其 grant 改为 `suspended_missing`；
4. 写自动撤权 audit；
5. 发布 projection。

工具重新出现时，只有 schema hash、effect 和 vendor provenance 全部相同才显示为“可恢复”；仍需管理员显式启用。任何 tool 都不会因 transport 切换或 vendor 恢复而自动获得旧写权限。

### 9.4 OAuth

- PKCE S256、state、issuer/mix-up 校验、精确 redirect；
- RFC 8707 resource/audience binding；
- refresh rotation，旧 refresh token 在事务提交后失效；
- authorization server discovery 只接受与 protected resource metadata 一致的 issuer；
- 禁止 token passthrough 到模型、GUI、Electron 或另一个 MCP server；
- 401 进入一次受控 refresh，refresh 失败进入 `AuthRequired`，不无限重试；
- disconnect 立即停用本地 grant，随后调用 vendor revoke；vendor 失败不恢复本地访问。

OAuth callback 按发行物固定分离：Server 使用管理员登记的 HTTPS public callback；Desktop Local 使用单独的 installed-app OAuth client、system browser、PKCE 和仅监听 `127.0.0.1` 随机端口的短期 loopback callback；Desktop Remote 使用 Server callback。三个模式不复用 client secret 或 redirect URI，也不从 incoming Host header 推导 callback。

### 9.5 Google Drive REST 不是 MCP

当前 OpenBot 的 Google Drive 走 GA REST adapter，而不是 gated MCP endpoint。Rust 版保留封闭 `VendorTransport`：

```rust
#[async_trait]
pub trait VendorTransport {
    async fn list_tools(&self, ctx: &VendorContext) -> Result<Vec<ToolDescriptor>>;
    async fn call_tool(
        &self,
        ctx: &VendorContext,
        tool: &str,
        args: Value,
    ) -> Result<ToolOutcome>;
}
```

实现 `McpHttpTransport` 与 `GoogleDriveRestTransport`。Drive 使用 asker's per-user OAuth，scope 固定 read-only；search/recent/read/metadata 的结果带 vendor link 和 provenance，不缓存客户文档正文，不创建本地 ACL/index。

### 9.6 Skills 与 tool discovery

- personal skill 只属于作者，可附加到作者拥有的 Bot；
- deployment skill 只由 admin 管理；
- skill 是 instruction，不是 capability；其中提到一个 tool 不产生 grant；
- [当前上游提出的 skill 两阶段 tool retrieval](https://github.com/CopilotKit/openbot/issues/119) 尚未实现，Rust parity 不宣传已有；
- 首版继续只把已 grant 的 tool 暴露给模型，并限制 catalog/schema/context 大小；
- 未来 tool retrieval 另立产品变更，不在本次重写中暗中加入。

## 10. ComputerSecurityScope 与多 Bot/多用户隔离

### 10.1 为什么 `bot_id` 不够

一个 public Bot 可以被不同用户或 channel 调用。若它们只按 `bot_id` 复用 profile，用户 A 在网页中的登录 cookie、下载和页面状态会暴露给用户 B。正式模型拆为：

```text
ProfileScope
  = tenant_id + bot_id + credential_principal_id

WorkspaceScope
  = tenant_id + channel_or_thread_id

ComputerSecurityScope
  = ProfileScope + WorkspaceScope

ComputerInstance
  = ComputerSecurityScope + computer_id + generation

Viewer/InputLease
  = computer_id + tab_id + generation + actor_id + lease_epoch
```

`credential_principal_id` 只能是：

- user principal：个人登录，永不与其他用户共享；
- service principal：管理员显式创建并列出所有可使用者；
- local single-user principal：Desktop Local 当前 OS 用户。

public Bot 默认生成 per-user profile，不生成所有人共用 profile。共享 service profile 同时只允许一个 acting run；其他 run 明确返回 busy，不排队隐式操作。

共享 channel 中，user-principal computer 的 live screen、snapshot、download 和 human control 默认只对该 principal 可见；其他成员只看脱敏 activity/audit summary。发起者可以铸造一次性、可撤销、限时 screen-share grant。service-principal computer 才可按 channel membership 向全体成员展示，UI 必须明确标识“共享服务身份”。

每个 thread/channel 有独立 workspace/artifact root（R1 无下载落盘，§11.2；若将来加入，下载也落在这个 root 下）。相同 principal 的浏览器 profile 可以跨 thread 保留登录，但 profile 同一时刻只能被一个 ComputerInstance 持锁；切换 workspace 前必须结束前一 lease。

### 10.2 Engine 能看到什么

Browser Engine 必然能读取其 ProfileScope 内的网站 cookie/session，这是执行浏览器任务的必要能力。安全承诺限定为：

- engine 只获得自己的 profile/workspace；
- 不获得产品数据库、模型 key、MCP refresh token、OIDC/SAML secret、updater key或其他 scope；
- compromise 的最大数据域是当前 ComputerSecurityScope；
- profile reset 清除该 scope 登录；Desktop 依赖 OS full-disk encryption，Server 依赖加密 volume；不对同 UID/root 或物理设备所有者承诺密码学擦除。

### 10.3 Desktop 与 Server 安全表述

| 档位 | 保护目标 | 不能声称的能力 |
| --- | --- | --- |
| Desktop | 防止正常产品路径串 profile/workspace、限制 renderer/sidecar 权限、可靠清理进程 | 不抵抗同 UID 恶意程序、管理员/root、内核漏洞或敌对租户 |
| Server + runsc | 不可信用户/任务之间的租户隔离、独立网络/文件/进程/配额 | sandbox 不是漏洞为零；仍需 egress、凭据、审计和补丁 |
| Desktop engine 进程（R119，2026-08-28） | 由 Rust sandbox helper 启动并约束：只允许 profile / temp 目录读写、只允许经本机 loopback 代理出站、只允许执行自身 bundle 内的 helper。fidelity 分级：macOS `Enforced`（sandbox profile）/ Windows `Degraded`（Job Object kill-on-close + 进程数与内存上限 + restricted token + profile 目录 ACL + Chromium `--proxy-server` 指向本机代理且 `--proxy-bypass-list="<-loopback>"`）/ Linux Desktop tier-2（namespaces + seccomp，若可用）；fidelity 进入 readiness / diagnostics 与 UI | `Degraded` 不阻断 browser computer 与组件渲染（与下一段的 shell 高风险模式不同），但 UI 明示；`Unavailable`（helper 无法施加任何约束）= engine 不启动，走 RefusedCard / `engine_unavailable`。同样不抵抗同 UID 恶意程序 |

Desktop shell 只有平台 sandbox fidelity 为 `Enforced` 时才可启用高风险模式：Linux 使用 user/mount/pid/network namespace + seccomp；Windows 使用 AppContainer/restricted token + Job Object + scoped ACL；macOS 使用已验证的 sandbox profile/entitlement。fidelity 为 `Degraded/Unavailable` 时，任意 shell 默认关闭，只允许 Rust 内置文件操作与逐次用户批准的有限命令。

### 10.4 Server v1

- 独立 Rust Supervisor 是 Tier-0，不与公网 API 同进程；
- Docker/containerd socket 只在 Supervisor；API/Agent/Computer 无权访问；
- Supervisor 只接受 `ensure/stop/reset/list`，调用方不得自报 image、command、mount、network 或 env；
- computer image 固定 digest；rootfs read-only、non-root、drop all capabilities、no-new-privileges、seccomp、cgroup/pids/memory/disk quota；
- 每 scope 独立 network namespace、volume、IPC secret 和 generation；
- `runsc` 是 production mandatory；不能启动 runsc 时，multi-user server readiness 失败；
- SPIFFE/SPIRE 保留为可选 workload identity，未启用时使用 peer credential + short-lived capability，不退回开放端口。

### 10.5 Network egress

应用层 URL 检查不能覆盖子资源、redirect、WebSocket、WebRTC、QUIC 或 compromised engine。Server 固定：

1. computer namespace 默认无直连互联网；
2. HTTP(S)/WebSocket/DNS 只能经过 per-scope egress gateway；
3. 禁用 QUIC，WebRTC 默认禁用，避免绕过代理；
4. gateway 对 DNS A/AAAA、redirect 每跳、IPv4/IPv6 reserved/private/metadata、port、scheme 和 tenant allow/deny 执行策略；
5. 所有 iframe、script、image、font、XHR/fetch、worker、service worker 和 popup 都经过同一出口；
6. Desktop 做同样的应用级代理与 URL policy，但文档明确它不是 kernel-level tenant boundary。

### 10.6 EngineScope 与 ExecutionRealm（R118 / R120，2026-08-28）

```text
EngineRole
  = BrowserComputer(ComputerSecurityScope)      # §10.1；每 scope 一个 engine 进程实例
  | SandboxedComponent(ComponentRenderScope)    # §3.3；每 Desktop 应用实例一个 engine 进程实例

ComponentRenderScope
  = tenant_id + actor_id + desktop_window_session_id   # 无 ProfileScope：会话临时、无持久 profile

ExecutionRealm
  = HostLocal          # Desktop Local 的 file/shell；受 §10.3 fidelity 门控
  | ScopedContainer    # Server 与 Desktop Remote 的 file/shell；Supervisor + runsc（§10.4）
```

- role 由 Rust 在 boot handshake 里铸造（§11.2），不从 renderer、argv 自由字符串或页面 URL 得出；
- `ExecutionRealm` 两者之间**没有隐式 fallback**（G5C）：Desktop Local 没有 `ScopedContainer`，不引入 Docker Desktop 或任何本地容器 / VM（§2.3 条 13）；Server 没有 `HostLocal`；Desktop Remote 的 shell/file 是 Server 的 `ScopedContainer`；
- 组件 engine 的 `ComputerId` / `ComputerGeneration` 与 browser computer 同一套铸造与失效规则（§17.2 条 6）；render session 就是它的 `TabId`；
- Grok Bot 的 "local" / "box" 双执行域（`grok-bot/source/host/box/*`、`electron-main/box/*`）只映射到本节的两域，其远程云 box 与本地 Docker box 都不引入。

## 11. Browser Engine 与 OpenBot/CrabCode 复用

### 11.1 单一 engine

Desktop 与 Server 使用同一个最小 Electron/Chromium engine package、同一 shim、同一协议和同一 conformance suite；Server 将其置于 runsc container。engine 按 Rust 铸造的 role 工作（§10.6）：`BrowserComputer` 每 `ComputerSecurityScope` 一个进程实例（scope-bound persistent profile）；`SandboxedComponent` 每 Desktop 应用实例一个进程实例（临时、非持久 partition，§3.3）。两种 role 必须是不同的进程实例、不同的 profile / partition、不同的 egress 与资源预算，但共用进程类、shim、协议与 conformance（R118）。首版不维护 Playwright engine 与 CrabCode engine 两套生产实现，也不采用 standalone Chromium + 直连 CDP（用户裁决 D2；理由与被否决的备选见 §11.6）。当前 OpenBot 的 Playwright 代码是行为 oracle 与 fixture 来源。

### 11.2 Engine 协议

控制面继续采用 authenticated UDS/Named Pipe 上的有界 NDJSON 或等价 typed framing；screencast 使用独立二进制 framing。Browser Engine 只接收：

```rust
pub struct EngineCommand {
    pub operation_id: OperationId,
    pub computer_id: ComputerId,
    pub generation: u64,
    pub capability: OneShotCapability,
    pub operation: BrowserOperation,
}
```

它不接收 `actor_id`、role、policy、intent 或 `policy_decision_id`；这些留在 Rust。`BrowserOperation` 是封闭 enum，R1 成员**与固定 commit 的 agent-computer 浏览器面一一对应**：navigate、snapshot、read、click、type、key、scroll、screenshot、screencast（start/stop/ack）、human input（§12.5 的输入 union）、secret insert、profile lifecycle（ensure/stop/reset）。**没有 download、没有 upload**：上游 29 条手写路径里没有下载/上传/文件选择/对话框处理（`page.on`、`setInputFiles`、`filechooser` 全仓零命中）；Chromium 自发的下载事件由 engine 默认取消并上报一条规范化 `download_refused` 事件，弹出的 JS dialog 默认 dismiss 并上报。把下载落盘或上传文件做成工具是独立产品变更，届时才适用 §11.3 的 quarantine / artifact handle 规则。文件与 shell 不走 `BrowserOperation`，它们是 computer 的另一组封闭操作（§18 "File/shell" 行：files/list、files/read、files/write、exec）。禁止自由 CDP method、自由 HTTP passthrough、自由本机路径、任意 shell 或任意环境变量。

component role 的会话协议是第二个封闭 enum（R118，**新增**；不是 `BrowserOperation` 的成员）：

```rust
pub enum RenderSessionOperation {
    Start { render_id: TabId, html: String, css: String, js: String, args_json: String },
    Input(BrowserInput),   // §12.5 同一 union
    Stop,
}
```

`Start` 的四段内容由 Rust 从 published 列与已校验的 args 一次性注入（`window.__args` 语义与上游逐字相同，§3.3）；engine 不得读取任何文件、URL 或第二来源。

boot handshake（R119，三平台统一）：Rust 先创建 pipe endpoint（macOS/Linux 为随机路径、`0600` 的 UDS；Windows 为随机名、仅当前用户 SID 可访问的 Named Pipe），再 spawn engine，并向其 stdin 恰写入一行 ≤ 4 KiB 的 boot capability（pipe 名、`EngineRole`、protocol version、release epoch、一次性 128-bit token），随后关闭 stdin；engine 连接 pipe 后发送 `hello{token}`，Rust 校验 token **与 peer credential**（UDS：`SO_PEERCRED` / `getpeereid`；Named Pipe：`GetNamedPipeClientProcessId` 等于 spawn 得到的 PID 且进程创建时间一致），二者任一不符即 kill 并推进 `ComputerGeneration`。engine 二进制、shim ASAR 与协议 hash 的 digest 由 Rust 在 spawn **之前**校验（§16.2），engine 不自报。

### 11.3 Browser 安全配置

- 全部不可信 renderer（remote page 与 component render session）：`nodeIntegration=false`、`contextIsolation=true`、`sandbox=true`、`webSecurity=true`、`webviewTag=false`、无 preload、production 无 devtools；shim 在 `app.ready` 之前调用全局 sandbox（`app.enableSandbox()`），`--no-sandbox` / `sandbox:false` / `webviewTag:true` 在任何配置禁止（R119；参考源 `grok-bot/source/electron-main/main.ts` 三者俱全，见 §11.5，不得照搬）；
- Browser/Component 正向测试必须证明 renderer 进程**实际** sandboxed（macOS：helper 进程 `sandbox_check` 为真；Windows：renderer token 为 AppContainer / 低完整性；Linux：`/proc/<pid>/status` `Seccomp: 2` 且 `NoNewPrivs: 1`），而不是只检查配置文本；
- remote page 无 preload、无 Electron/Tauri API；
- permission request/check handler 默认拒绝 camera、microphone、screen capture、geolocation、USB、HID、serial、Bluetooth、notification 和 clipboard；
- popup/new-window 默认拒绝；外部打开只接受 Rust 重新验证的 URL；
- 不开放 remote debugging port，只通过 `webContents.debugger` 使用 CDP；正式帧源固定为 `Page.startScreencast`（§12.2），Electron offscreen `paint` 只作诊断 fixture（§12.6）；
- 启用 ASAR integrity，关闭未用 Electron fuses（`RunAsNode` / `EnableNodeCliInspectArguments` / `EnableNodeOptionsEnvironmentVariable` 关，`OnlyLoadAppFromAsar` / `EnableEmbeddedAsarIntegrityValidation` 开），禁止 `ELECTRON_RUN_AS_NODE`；fuses、ASAR、rebrand 与 ASAR integrity 值全部由 Rust `cargo xtask engine bundle` 写入，不用 npm 工具（R117）；
- R1 没有下载落盘与上传操作（§11.2）；若未来作为产品变更加入，download 进入 quarantine、校验名称/MIME/大小、不自动打开，file upload 只接受 Rust 铸造、scope 绑定的 artifact handle——两条规则此时即已写死，不随实现期再议；
- Electron/Chromium critical/high 修复在 72 小时内升级；无法及时升级时关闭受影响能力或停止发行；
- browser sidecar 不自更新，必须与 Rust/Tauri 原子版本、原子签名；engine 禁止 `autoUpdater`（§2.3 条 15）；
- component role 另加（R118）：session 级 proxy 指向 `127.0.0.1:1` + `webRequest` 对 `component://` 与 `data:` / `blob:` 之外全部 cancel + §3.3 CSP，三层缺一即红；permission request/check 全部拒绝；navigation 与 popup 全部拒绝；无 clipboard、download、file chooser；
- shim 约束（R117，`cargo xtask electron-shim-check` 判红）：来源 clean-room（只依据 Electron 公开 API 文档，不复制 CrabCode `browser-shell` 或 `grok-bot/source/electron-main` 的任何文本）；文件 allowlist 恰为 `crates/openbot-desktop/engine-shim/{package.json,main.mjs,generated/protocol.mjs}`；`package.json` 键集合恰为 `name` / `version` / `main` / `private` / `type`，零 `dependencies` / `devDependencies` / `scripts`，无 lockfile；非空 LOC ≤ 600（**新增**预算，超过即需在 PR 里逐行解释为何不能放进 Rust）；允许的 Electron API = `app` / `BrowserWindow` / `session` / `webContents` / `webContents.debugger` / `protocol` / 固定的 permission、navigation、crash handler；允许的 Node 内置模块只有 `net`（连 pipe）、`buffer`、`process`（读 stdin、退出）；禁止 `child_process`、`fs`、`http` / `https`、`dns`、`eval`、`executeJavaScript`、`sendSync`、`<webview>`、自由 method dispatcher、自由 CDP、自由 URL passthrough、renderer 自报 role / scope / generation；生成的 `protocol.mjs` 的 hash 必须等于 Rust 侧 `openbot-contracts` 生成物的 hash。

### 11.4 CrabCode 复用清单

| 资产 | 正确用途 | 禁止做法 |
| --- | --- | --- |
| `acosmi-supervisor` / daemon launcher / heartbeat | 进程 registry、PID identity、watchdog、shutdown、socket lock | 整 crate 无审计复制 |
| permission / shell parser / sandbox / exec | 平台 sandbox、command plan、process tree、fidelity | 把 CrabCode 单用户路径语义当 OpenBot ACL |
| `acosmi-cmd-browser` | Rust browser request adapter、explicit target、timeouts | 暴露自由 method/path |
| `components/browser-shell` | （R117 降级）只作可选的行为 fixture 来源：tab / snapshot / input / framing 的**可观察行为**记录；shim 本身 clean-room，不复制其任何文本 | 复制完整 desktop host、Design/账号/Office 能力；把它当 shim 的源码 |
| app-server protocol/transport | framing、origin、auth、thread/turn fixture | 复制 200+ method 的产品专属 dispatcher |
| 历史 Tauri host | path/artifact/log/menu/deep-link/single-instance/cleanup 模式 | 回滚整个已删除提交 |
| TS Agent/MCP | 行为 fixture、错误语义 | 进入最终生产控制面 |

所有行目前都是“权属清理后可用”，不是自动授权。每个复制文件须有 `SOURCE_PROVENANCE`：权利人、原路径、上游路径/commit、原/目标 hash、许可证、修改声明、书面授权编号。

### 11.5 `grok-bot/` 参考树：定位、方法与 census（R115 / R116，2026-08-28）

**它是什么**：`grok-bot/`（tree `86f5a85f560f721677fa7e587a67ac0ffc036cb5`，§1.2）是 Anysphere（Cursor）公开发行的 Grok Bot 0.18.0 macOS 应用的**反编译重建**（其 `README.md` / `PROVENANCE.md` / `NOTICE.md` 自述：bundle ID `com.anysphere.sand`、"Names and module boundaries inferred from a compiled application"、"No upstream source-code license is asserted or granted"）。它**不是** §1.2 / §23.1 条 3 的 xAI Grok Build，Apache-2.0 不覆盖它。本机复算：`source/` 1,722 个 `.ts`、全树 543,212 行、TS/TSX 493,338 行、`source/packages/{proto,redacted-protos}` 263,713 行；`source/` 与 `frontend/` 下 `*.test.ts(x)` = 0，`packages/agent/state.ts` 自注 "Intentionally partial recovery"，`frontend/` 是 partial reconstruction、shipped renderer 只有 minified bundle（命令见 §28.4）。

**它能做什么**（真源第 4 层，§28.5）：只提供架构 / 执行 / 状态机 / 协议语义的参考——coordinator 与 supervisor 的进程 identity、generation、backoff、resync、退役；runner 的 turn / stream attempt / retry / stall / checkpoint；`always/ask/never` 权限偏好状态机；local-exec / shell 的 cancel、deadline、output 上限；extension DAG 的启动、失败回滚与逆序 teardown。**它不提供产品行为**：产品 / API / schema / 旅程的 oracle 仍是固定上游 OpenBot（第 3 层）。

**唯一允许的方法 = 规格先行吸收**（类别 `A`）：读参考 → 在本文件对应章节写出状态机 / 不变量 / 错误与并发语义（新增即标 **新增**）→ 登记 `source_lineage`（`GRB-<family>-<n>`：`grok-bot/<path>::<symbol>` 作为证据锚点）→ Rust 实现 + 本项目自己的 fixture 与测试。**禁止**逐文件 / 逐函数翻译，禁止复制任何文本（含注释、字符串、标识符序列）；`T`（近机械翻译）与 `C`（产品能力候选）两类不存在。理由是技术的也是法律的：重建代码零测试、部分恢复、与原应用的等价性未被验证（其 `PROVENANCE.md` 说 "The immutable release is the product specification"），逐函数翻译会把未验证的行为与不可审计的来源一起带进 Rust；§23.3 的反编译规则与 §11.4 的 clean-room 规则对它同样成立。其余类别：`S`（engine shim 边缘，§11.3）、`R`（明确拒绝的实现，§2.3 条 13–16 与 §11.3）、`P`（partial / placeholder，evidence-only，不进任何范围）。

**census 只有 tier-1**：`cargo xtask grok-inventory` 生成 `inventory/grok/files.yaml`（每文件：path、family、非空 LOC、maturity 标记 `production | partial | generated | artifact-only`），机械生成、不做人工分类、不进任何分母；`--check` 模式与树同步是 G0 判据之一（§24）。`GRB-` lineage 行只在某个批次真正吸收一个模式时追加，不预先铺表。

**v5 候选（无承诺，仅登记；每项都依赖 OpenBot 没有的后端或属于新产品面）**：

| 家族 | 依赖 / 性质 | 若立项须先解决 |
| --- | --- | --- |
| `automations`（定时 / 重复 run） | 新 scheduler、新表、新 UI | §7.2 后台 durable run 之上的调度语义与配额 |
| subagent / background work 面板 | §7.2 已有 durable run，缺 UI 与治理 | 与 §8 管线的 acting 归属 |
| `permissions` 的 `always/ask/never` 用户偏好 | 位于 CEL policy 之下的用户偏好层 | 不得绕过 §8.1 审批与 §17.2 条 3 的 fail-closed |
| `terminal` / local-exec UI | §18 File/shell 只有封闭操作，无终端 | §10.3 fidelity 与 §10.6 realm |
| cloud agents / forever-box / box-store-sync / cross-user-sharing / teach-recording / managed-setup / webauthn-proxy / host-upgrade | 依赖 Cursor 云后端 | §2.3 条 16 已拒绝，除非本项目自建对应 Server 面 |

**原始安装包**：`research-archives/original/0.18.0/` 只保留 identity（`artifacts.json` / `SHA256SUMS`）；两个 LFS 指针于 R116 移除（对象从未上传，默认 `git clone` 恒红），目录内 `.gitignore` 禁止再加入。

### 11.6 D2 的备选与否决理由（ADR，R118）

用户裁决 D2：Browser Computer 与 Desktop sandboxed component 固定使用 Electron 内置 Chromium。被否决的备选逐一记录，避免实施期重议：

| 备选 | 否决理由 |
| --- | --- |
| Servo | 布局 / 样式是 Rust，但 JS 引擎是 SpiderMonkey（C++）；没有 CDP / screencast / partition 的稳定嵌入面，也没有 Chromium 那样经年对抗敌意内容的多层沙箱；用它反而要自建一层沙箱 |
| wry / 系统 WebView 独立进程 | WebView2 / WKWebView / WebKitGTK 三套引擎三套隔离语义；帧 / 输入 / egress 无统一控制；Server runsc 内不可用（§2.1 条 2、§2.3 条 11） |
| wasmtime / WASI | 要求把组件契约改成 WASM，破坏 §3.3 钉死的上游 HTML/CSS/JS 契约 |
| standalone Chromium headless-shell + Rust 直连 CDP（`--remote-debugging-pipe`） | 是唯一能让第一方 JavaScript 归零的方案；被否决的理由是工程而非原则：Electron 的 `session` / `webContents` / permission handler 与三平台签名发行结构是现成且 CrabCode 已验证的，headless-shell 的 macOS 公证、Windows 签名与自建发行结构需要独立 delta；本裁决不排除将来以 delta audit 重议 |
| **Rust 拥有的 OS 沙箱 helper 包住 Electron engine 进程** | **采纳**（R119，§10.3）：它不替代渲染引擎，但把"engine 主进程 = Node"的最大损害域限制在 profile 目录 + loopback 代理内 |

## 12. 实时 Screencast、Input 与 Human Lease

### 12.1 正确基线

OpenBot 当前已经通过 Playwright CDP 实现 `Page.startScreencast`、逐帧 ACK 和输入；不足之处是 JSON/base64 WebSocket、有限 metadata、单 viewer 语义和缺少统一 generation/ticket。Rust 版保留其可观察行为并加固。

### 12.2 数据路径

```text
Chromium Page.startScreencast
  → minimal Electron shim: decode base64 JPEG, attach engine generation
  → authenticated per-computer UDS/Named Pipe binary ingress
  → Rust ScreenIngress validation
  → ScreenHub latest frame（每 tab/generation 只保留最新值）
  → Desktop loopback binary WS / Server authenticated WSS
  → Leptos canvas + createImageBitmap
```

CDP `startScreencast` 当前只选择 JPEG（生产）与 PNG（诊断），不把 WebP 写成已支持格式。ACK 在帧成功进入 size-1 latest buffer 后发送；慢消费者只能丢旧帧，不能形成无界队列。component role 的帧走完全相同的路径（computer_id = 组件 engine 的 ComputerId，tab_id = render session），不设第二条帧路径（R118）。

### 12.3 Frame contract

```rust
pub struct FrameHeader {
    pub protocol_version: u16,
    pub computer_id: ComputerId,
    pub tab_id: TabId,
    pub generation: u64,
    pub seq: u64,
    pub captured_at_ms: i64,
    pub device_width: u32,
    pub device_height: u32,
    pub device_scale_factor: f32,
    pub page_scale_factor: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub mime: FrameMime,
    pub payload_len: u32,
}
```

帧头设置 magic/version/header length/payload length；Rust 在分配前校验最大尺寸。任何 computer/tab/generation 不匹配的帧直接丢弃并记 metric，不进入其他 viewer。

### 12.4 Viewer ticket

Desktop Leptos 先用 Tauri typed command 申请 `ScreenSession`。Rust 返回 loopback 地址和 128-bit、30 秒有效、一次性 ticket；WebSocket 通过 `Sec-WebSocket-Protocol` 携带 ticket，不放 URL/query/log。服务端同时校验精确 Tauri origin、window label、actor、computer、tab、generation 和 auth generation。

Server 使用同源 `wss` 与 session cookie/CSRF-style origin check；不能把 computer token交给浏览器。每个 viewer 有连接数、帧大小、带宽和 idle limit。

### 12.5 Input

坐标转换使用 frame metadata、DPI、zoom、scroll、canvas letterbox。输入 union 与上游 `/stream` 协议对齐：mouse（move / down / up，含 button 与 modifiers）、wheel、key（down / up，含 modifiers 与 `Input.dispatchKeyEvent` 所需的 keyCode 表）、insertText、secret insert；不提供自由 CDP。不设 "IME composition" 与 "drag" 两个独立变体：IME 合成发生在 viewer 自己的输入元素里，合成完成的文本走 insertText（上游 `/human/type` 正是这样做的）；拖拽就是 down → move → up 序列，engine 不需要知道"这是一次拖拽"。

```text
GUI input
→ viewer/window/session ACL
→ HumanLease(owner, computer, tab, epoch, expires_at)
→ generation + auth generation
→ policy/audit takeover state
→ BrowserInput union
→ CDP Input
```

lease transfer、release、expiry、navigation 或 computer restart 都递增 epoch。旧 input 即使在 socket buffer 中也会被拒绝。接管期间 Agent acting 立即返回确定性 refusal，不排队。

paste 使用 `Input.insertText`，不读取系统 clipboard；secret 使用独立 typed command，值不经过普通 key event、frame log 或 transcript。

### 12.6 性能目标与降级

- 目标：1280×800、JPEG quality 70（与上游 `screencast.ts` 的 `maxWidth 1280 / maxHeight 800 / quality 70` 逐值相同；上游不限 fps、每次变化一帧）；fps 上限是新增背压：10 fps passive / 15 fps driving；component render session 另限 ≤ 5 fps（R118，新增；组件多为静态，`Page.startScreencast` 只在重绘时出帧，静态组件零流量）；
- loopback capture-to-paint p95 ≤ 200 ms，p99 ≤ 400 ms；
- 每 viewer 最多 1 个待发 frame，ScreenHub 每 tab 最多 2 个 frame buffer；
- 最后 viewer 断开后 2 秒内停止 screencast；
- `Page.startScreencast` capability 不存在时，降级为 `captureScreenshot` 2 fps，并在 UI 明示“低频预览”；不称实时；
- Electron offscreen/beginFrameSubscription 不作为第二生产路径，只作为实验 fixture。

必须覆盖 DPI/zoom/scroll、resize、navigation、tab switch/close、frame corruption/order、慢消费者、ticket replay、engine restart、多 viewer、IME 合成文本经 insertText、down→move→up 拖拽序列、human lease race 和跨 scope frame 注入。

## 13. Tauri/Leptos 与 in-process transport

### 13.1 GUI

- Leptos CSR/WASM；不维护 React 第二 GUI；
- Server 由 Axum 提供相同静态 bundle；Desktop 由 Tauri custom protocol 提供；
- 主 WebView 只加载打包本地内容，拒绝 remote navigation；
- strict CSP，无 `eval`、无远程 script、无宽泛 `connect-src`；
- deep link、file association、clipboard 和 external URL 都当不可信输入；
- Tauri capability 按 window label 单独配置，禁止 `windows:["*"]`、宽泛 filesystem 和 remote API access；
- production 禁用 devtools，所有 command 枚举注册并生成审计清单；
- CSS 由 Tailwind v4 standalone CLI（钉 sha256 的单文件二进制）经 trunk `--offline` 编译，仓库零 `package.json` / Node（§0.1）；
- `index.html` 零内联脚本；主题 class 与 `lang` 在首帧由 Rust 改写（Axum 从 cookie、Tauri custom protocol 从本地设置），不靠 JavaScript 防闪烁；
- 字体（Inter Variable 4.1）与图标（Lucide 1.33.0 allowlist）随 bundle 打包，零远程资产；
- 视觉、token、布局、i18n、a11y 的全部契约见 `docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md`。

### 13.2 typed in-process，不复制 JSON-RPC

Tauri `setup` 创建一个 `Arc<dyn ApplicationService>`。普通 request 直接 typed 调用，server request/stream 使用有界 channel：

```rust
pub struct DesktopSession {
    pub window: WindowIdentity,
    pub auth_generation: u64,
    pub events: mpsc::Receiver<AppEventRef>,
    pub shutdown: CancellationToken,
}
```

默认队列：command 256；每窗口 critical event ref 256；token delta 每 50 ms/8 KiB 合并；progress/presence 使用 latest-value；shutdown deadline 5 秒。

投递等级：

- terminal、approval、policy decision、server request：不可静默丢；队列满即显式断开/失败，客户端从 durable cursor replay；
- text/reasoning delta：可合并，不改变最终文本；
- progress/presence：latest-value；
- screen：独立 binary channel、latest-frame；
- 任意丢弃或合并都产生 metric 和 sequence gap，不能让 GUI误以为完整。

Codex in-process 的借鉴限于 bounded queue、duplicate request rejection、critical notification、lag visibility、pending cleanup 和 finite shutdown；OpenBot 不复制它的 initialize/JSON-RPC DTO。

### 13.3 多窗口

一个 native broker 可以服务多个窗口，但 Rust 按 window label、actor、thread subscription 和 auth generation 过滤。window A 永远收不到 window B 的私有 thread、screen ticket 或 approval；过滤不能由前端自行完成。

### 13.4 Screen 不走 Tauri event

Tauri 官方说明 event 适合小量 JSON，Channel 针对有序流优化；持续画面仍属于高频大二进制。正式路线使用 loopback binary WebSocket。Tauri Channel 只承载结构化 Agent/tool/policy 事件。

## 14. PostgreSQL、Schema、Migration 与 Backup

### 14.1 单数据库裁决

Server 和 Desktop Local 都使用 PostgreSQL 17。理由：

1. 当前 28 张表和 13 条 migration 已是 PostgreSQL 语义；
2. audit、并发 lease、outbox、多 replica、JSON query 和约束依赖成熟事务；
3. 双 SQLite/PostgreSQL 会让 auth、array、JSON、lock、migration、backup 和故障恢复产生两套事实；
4. 用户已接受“数据库访问 Rust、数据库引擎不要求 Rust”的定义。

Desktop Rust supervisor 管理固定版本 PostgreSQL sidecar：仅 loopback/本机 socket、随机 SCRAM secret 存 OS key store、独立 data dir、启动锁、ready probe、graceful shutdown、backup 和 upgrade。PostgreSQL major upgrade不与 Tauri/Electron major upgrade同一 release。

**"迁到 0012"这条前置不能只靠 schema 判定。**13 条 migration 里 `0003_backfill_account_issuer.sql` 是**唯一一条纯数据迁移**（整条只有一句 `UPDATE "accounts" SET "issuer" = CASE … END WHERE "issuer" IS NULL`，不动任何结构），所以跑到 `0002` 未跑 `0003` 的库与完整迁到 `0012` 的库 **schema 事实逐字段相同**，纯 schema 的边界检查在构造上无法区分二者。

**但判据不能是"`accounts.issuer` 存在 NULL"** —— 上游 `server/src/db/schema/core.ts::accounts` 的 `issuer` 字段注释逐字写明该列"Nullable in the database, deliberately, even though every write fills it"，理由是滚动发布期新旧 replica 并存，旧 replica 会插入不带该列的行；`NOT NULL` 会让这些人的首次登录失败。也就是说 **NULL 在一个完整迁移过的库上是合法状态**，据此 fail-closed 会拒绝启动健康的库。（`0006` 里那句 `ALTER COLUMN "issuer" DROP NOT NULL` 是 no-op：全仓 migration 零处 `SET NOT NULL`，该列自 `0002` 加入起就是可空。）

正确的信号是**迁移账本而不是数据形状**：上游用 `drizzle-kit migrate`（`server/package.json::db:migrate`），`server/drizzle.config.ts` 未自定义账本位置，故账本落在默认的 `drizzle.__drizzle_migrations`，本地对照物是 `server/drizzle/meta/_journal.json` 的 13 条 `tag`。边界检查应当读该账本并要求条目数 ≥ 13；账本不存在时（例如库由本项目的 Rust baseline 直接建成）必须**如实报告"无法验证数据迁移是否执行"**，不得用数据形状去猜，也不得默认通过。此后若再引入纯数据 migration，同样只能由账本覆盖。

R1 不要求 pgvector，也不重建 customer document index。升级前先要求旧 OpenBot 把数据库迁到当前第 13 条 migration（`0012`）；Rust 不接收更早 schema。Fresh install 使用当前最终 schema 的 Rust baseline，不创建已删除的 document/vector 表。`vector` extension 的实况是：上游 `0010_drop_the_document_index.sql` 已 `DROP EXTENSION IF EXISTS "vector"`（默认 RESTRICT），所以迁到 `0012` 的库**通常已没有**该 extension；只有部署自行加过 vector 列导致 `0010` 失败、或手工保留的库里还有它。Rust 兼容 migration 对 extension 零操作——既不创建，也不再删一次；Server 镜像改用平装 `postgres:17`，不再依赖 `pgvector/pgvector` 镜像。

**0012 之后的 Rust-owned 增量不写上游 Drizzle 账本。** 固定形态是：`db::baseline` 只负责把空库建成固定上游 0012 oracle；`db::compat` 只读 `drizzle.__drizzle_migrations` 判断旧库是否到边界；`db::native` 施加本项目的 expand-only 增量。自有账本固定为 `openbot_internal.schema_migrations(version, name, checksum, applied_at)`，checksum 是 migration SQL 原文的 SHA-256；同版本名字或摘要漂移必须 fail-closed。施加前取 transaction-scoped advisory lock，多 replica 同启时恰好一个写 DDL；真实 migration SQL 不用 `IF NOT EXISTS`，因此“对象存在但账本缺失”会报 drift 并整体回滚，而不是静默伪装成已施加。Fresh install 的 `baseline 0012 + native 增量 + 自有账本` 必须在**同一外层事务**一起提交；后续启动以通过 checksum/空洞校验的 native ledger 识别 Rust-managed fresh 库，同时仍跑 schema boundary。已有 public schema 若既无完整 Drizzle 账本、也无 native ledger，继续 fail-closed，不从结构猜 0003 已执行。回滚应用仍只允许回到兼容 expanded schema 的上一签名 build，不提供 downgrade SQL。（§28.1 R35/R54）

当前 native 链为 0013 → 0014 → 0015。0014 只追加 `users.auth_generation`，0015 只追加 `sessions.auth_generation`，均为 nullable bigint + 非负 CHECK；PostgreSQL 17.11 post-0015 fixture 为 **31 表 / 250 列 / 95 约束 / 53 索引 / 4 触发器**。生产入口施加到最新版本；历史 fixture 测试用同一账本/锁/摘要路径固定各自边界，不复制 migration 执行器。（§28.1 R39 / R42）

### 14.2 28 表 parity ledger

Phase 0 为每张现有表记录：Rust aggregate/repository、主键、unique、foreign key、delete behavior、encryption、retention、API owner、migration、旧/新 fixture。当前域至少包括：

```text
users / sessions / accounts / verifications / user_roles / revoked_access
sso_providers
deployment_packages / agents / agent_profiles / agent_preferences
channels / channel_memberships / channel_agents / intelligence_channel_mappings
credentials / audit_events
action_policy / computer_snapshot
components / component_exclusions / component_functions / sandboxed_components
mcp_servers / mcp_tools / mcp_user_credentials / plugin_grants / skills
```

`intelligence_channel_mappings` 在 native thread 切换后迁为 legacy provenance，不再是 live truth；先保留，旧系统退役并完成审计保留期后再通过独立 destructive migration 处理。

Repository 必须与其物理表同批落地并跑真库，不允许先造零方法空 struct 把 `repo=` 名字“占上”。截至 0013，已有物理表对应的 **30** 个具名 repository 全部实现：上游 28 表各一个，另加 `ToolCallRepo` / `ToolAttemptRepo`；`audit_checkpoints` 与 `audit_events` 共用 `AuditEventRepo`。40 个规划落点中剩余 **10** 个恰好对应尚未建表的 thread/message/run/outbox/memory/import 面，归 G3 建表同批闭合；implemented 集合必须是 planned 集合的真子集且越界项为 0。基础 CRUD 只接收 typed row/typed key，表列来自编译期台账、值全绑定；`LegacyIntelligenceMappingRepo` 构造性只读；Vault 用 key-id CAS 轮换，Audit 用事务锁串行 hash chain/checkpoint，Tool receipt 只在 call+attempt 同事务 commit 后签发。（§28.1 R38）

People 的 role/access 不能拆成“先读 person → application 判定 → 多次 repo 写 → 最后另写 audit”：两个管理员并发互降时，各自会在陈旧快照上看见另一个管理员，且 audit 失败会留下已经提交的权限变化。固定边界是一个 `PeopleAdministration` typed port，由 PostgreSQL adapter 在 deployment-wide people advisory transaction lock 下同事务读取 subject / 其他有效 admin / generation，调用 domain 的 floor/self/last-admin 判定，写 role 或 deny/session/generation，并通过共享 audit transaction helper 追加事件；任一步失败整体回滚。Application 只做权威 `AuthContext` 的 admin gate、搜索归一与页长钳制；Axum/Tauri 不复制规则。公开 `Person` 逐字段保持固定上游形状，不暴露内部 auth generation。（§28.1 R40）

### 14.3 Expand/contract

兼容期 migration 只允许：新表、nullable column、backfill、index、非破坏性 constraint validation。禁止 drop、rename、类型收紧和 primary key 改写。

固定步骤：

1. expand schema；
2. Rust/旧系统都能读；
3. backfill + checksum；
4. 切唯一 writer；
5. 观察 30 天；
6. 另一个经过批准的 release 才执行 contract。

数据库不做 downgrade migration。回滚应用只回到仍兼容 expanded schema 的上一签名 Rust build。

### 14.4 Backup/Restore

- Server：PITR/WAL archive；标准部署 RPO≤5 分钟、RTO≤15 分钟；要求 committed-transaction RPO=0 的部署必须配置同步 standby 并通过故障切换演练；每日 full backup + 连续 WAL；
- Desktop：关闭前一致性 checkpoint，每日滚动 base backup，保留最近 7 份；用户可导出加密 backup；
- browser profile/workspace 与 PostgreSQL 分开备份，scope ID 和 generation manifest 保持一致；
- backup 中 secret 保持 envelope 加密，backup key 与数据库分离；
- 每个 release 候选对生产规模脱敏快照执行 3 次 restore drill；
- restore 后验证主键集合、外键、行数、canonical JSON hash、audit chain、credential decrypt canary 和 profile manifest。

## 15. API、Route 与行为兼容

### 15.1 Canonical inventory

不能用“95 个静态 route”冒充完整 API 数。构建期从 Axum router、Auth、AG-UI 和 migration/health adapter 生成 canonical inventory：method、full path、auth class、role、input/output schema、error status、owner、测试 ID。

现有 `/api/agents`、`/api/channels`、`/api/computers`、`/api/components`、`/api/sandboxed`、`/api/plugins`、`/api/admin/*`、`/api/threads`、`/api/route` 和 agent callback 行为均纳入。

### 15.2 CopilotKit endpoint

最终 Leptos GUI 不依赖 `@copilotkit/react-core` 或 `/api/copilotkit`。迁移期间保留一个只为旧 React 客户端服务的 Rust compatibility facade，输入输出由固定 trace 验证；最终 React 客户端退役后该 facade 从发行物删除。

AG-UI 是持续支持的开放协议边界；CopilotKit Intelligence 私有 wire protocol 不是最终产品 API。

### 15.3 错误语义

- 未登录 401；已登录但角色不足 403；Bot/channel 不可见统一 404；
- malformed payload 400，不产生 acting decision；
- policy refusal 403 + stable error code/rule ID；
- unavailable dependency 503；vendor failure 502/normalized tool error；
- stale snapshot/generation 409；request/idempotency binding conflict 409（R65）；lease conflict 409；
- unknown commit 202/409 对应 reconciliation，不伪装 500 或 success；
- 空、新 thread history 200 + empty list。

错误给用户的文本可本地化，但 stable code、HTTP status 和 audit event 类型不能随文案变化。

### 15.4 环境变量处置（本文件已裁决部分）

§21.1 条 6 要求所有变量在 `parity/env.yaml` 标记 preserve / rename / remove。其中会改变关联方行为的裁决不能留到 Phase 0，在此写死；Phase 0 只补齐表外的纯内部变量。上游 `docs/configuration.md` 记录 48 个，`server/src/config.ts` 读 32 个，agent-computer / supervisor 另读 22 个。

| 变量 | 处置 | 固定语义 |
| --- | --- | --- |
| `INTELLIGENCE_API_URL` / `INTELLIGENCE_API_KEY` / `INTELLIGENCE_GATEWAY_WS_URL` / `COPILOTKIT_LICENSE_TOKEN` | remove | 只有 §20.3 的导入工具读取；生产二进制出现其中任一变量即启动报错"已退役变量"，不静默忽略 |
| `BETTER_AUTH_SECRET` / `BETTER_AUTH_URL` | rename → `OPENBOT_SESSION_SECRET` / `OPENBOT_PUBLIC_URL` | 旧名在启动时给出一次性迁移提示后拒绝启动；`OPENBOT_PUBLIC_URL` 原本就存在（缺省回落 `BETTER_AUTH_URL`），成为唯一公共地址来源 |
| `KEY_ENCRYPTION_KEY` | preserve | v1 envelope 解密（§6.4）与 HMAC 标签派生（run assertion、thread 相关签名）都依赖它；base64 解出长度必须 ∈ {16, 24, 32}（上游 WebCrypto 接受这三种，硬要求 32 会让 16/24 字节 KEK 的部署起不来因而永远迁不出自己的密文，见 §28.1 R33），16/24 允许启动但告警建议轮换到 32；示例值在生产拒绝 |
| `DEPLOYMENT_ID` | preserve | thread id 的 6 字节指纹来源（§20.3）；改它等于放弃对既有 thread 的 `owns` 判定，迁移 preflight 拒绝与旧库不一致的值 |
| `OPENBOT_SINGLE_USER` / `INITIAL_ADMIN_EMAILS` / `TRUSTED_ORIGINS` / `OPENBOT_APP_URL` / `TENANT_PACKAGE_DIR` / `APP_DIST_DIR` / `PORT` / `DATABASE_URL` | preserve | 语义不变 |
| `NODE_ENV` | rename → `OPENBOT_ENV`（`production` / `development`，缺省 `production`） | 上游只用它做一件事：`NODE_ENV=production` 时拒绝示例 `KEY_ENCRYPTION_KEY`。Rust 版缺省即生产语义，只有显式 `OPENBOT_ENV=development` 才放行示例 key；它对单用户、cookie、policy 等一切安全判断仍然无效（§6.1） |
| `OPENBOT_DEV_NO_AUTH` | rename → `OPENBOT_SINGLE_USER` | Phase 0 的 70 条与本表原本都漏了它，而它是**上游活着的读取点**（`server/src/auth/dev-actor.ts::singleUserEnabled`，`trim()` 后恒等 `"true"`），是 `OPENBOT_SINGLE_USER` 的历史别名。旧名在启动时给出一次性迁移提示后拒绝启动，并**点名新变量** ——「不认它」会让靠它跑单用户模式的部署以「没有 IdP 也没有单用户旗标」这个看起来无关的理由失败（§28.1 R34） |
| `GOOGLE_OAUTH_*` / `MICROSOFT_OAUTH_*` / `OKTA_OAUTH_*` | preserve | 三家可同时配置（§6.2） |
| `AGENT_STALL_TIMEOUT_MS` / `AUDIT_RETENTION_DAYS` | preserve | §7.2 / §8.6 |
| `OPENBOT_RUN_DEADLINE_MS` | 新增 | §7.2 |
| `OPENBOT_PROVIDER_MAX_OUTPUT_TOKENS` | 新增 | §7.4 / R70；缺省 16384，只接受 1..=1000000，0 不关闭预算；当前为每 sampling output cap，不冒充 run-wide token/cost budget |
| `OPENBOT_PROVIDER_EGRESS_ALLOW_CIDRS` / `OPENBOT_PROVIDER_ALLOW_HTTP` | 新增 | §7.3 / §7.5；前者只收精确数值 CIDR，后者只有逐字 `true` 才允许 HTTP；二者互不替代，默认仍 HTTPS + 禁 private/special address |
| `AGENT_TOOL_TOKEN` | remove | §3.4；preflight 列出仍在用它的端点 |
| `MANAGED_AGENT_AG_UI_URL` / `MANAGED_AGENT_TOKEN` | preserve（可选覆盖） | §3.4；未设时 managed 插槽 = 进程内 built-in Agent |
| `BOT_PROVIDER` / `BOT_MODEL` / `BOT_RESPONSES_API` / `OPENAI_API_KEY` / `OPENAI_BASE_URL` / `ANTHROPIC_API_KEY` / `ANTHROPIC_BASE_URL` / `GOOGLE_API_KEY` / `GOOGLE_GENERATIVE_AI_BASE_URL` | preserve | §7.3；由 Rust built-in Agent 读取，原来读它们的 `agent-langgraph` 进程不再发布 |
| `OPENBOT_ACCESSIBILITY_DISABLED` | remove | 它只控制 CopilotKit runtime 自带的 Segment/Scarf 分析上报（`@segment/analytics-node`、`@scarf/scarf` 是 runtime 依赖）。Rust 版**没有任何第一方遥测外发**，变量无事可控；§16.4 据此写明"零 phone-home" |
| `AGENT_COMPUTER_URL` / `AGENT_COMPUTER_POLICY` / `AGENT_COMPUTER_ALLOW_PRIVATE_HOSTS` / `COMPUTER_SUPERVISOR_URL` / `COMPUTER_TOKEN` / `SUPERVISOR_TOKEN` | preserve（Server） | Desktop Local 不读它们：本机 engine 与 Supervisor 由 Rust 进程内管理 |
| `COMPUTER_RUNTIME` / `COMPUTER_SANDBOX` / `COMPUTER_IMAGE` / `COMPUTER_NETWORK` / `COMPUTER_NAMESPACE` / `COMPUTER_MEMORY_BYTES` / `COMPUTER_MAX_BROWSERS` / `COMPUTER_BROWSER_IDLE_MS` / `COMPUTER_SHELL_ENV` / `ACTION_TIMEOUT_MS` / `NAVIGATION_TIMEOUT_MS` / `DOCKER_SOCKET` | preserve（Server Supervisor / computer image） | `COMPUTER_RUNTIME=runsc` 在多用户 Server 由 §10.4 升格为强制；`COMPUTER_SHELL_ENV` 继续拒绝 `PATH` 等前置名；默认值逐个与固定 commit 相同（idle 30 min、pids 512、`/exec` 45 s backstop） |
| `SPIRE_*` / `SPIFFE_ENDPOINT_SOCKET` | preserve（可选） | §10.4 |
| `COMPUTER_BOT_ID` / `PROFILES_DIR` / `WORKSPACE_DIR` | rename → scope 化 | 容器内按 `ComputerSecurityScope` 注入，不再只有 `bot_id`（§10.1） |

未列出的变量由 Phase 0 归类；任何 remove 都必须在启动期被识别并报错，禁止"读不到就当没设"。

## 16. 部署、打包、更新与可观测性

### 16.1 Server 发行物

发布：

- `openbot-server` OCI image；
- `openbot-supervisor` OCI image；
- 固定 digest 的 `openbot-computer` image；
- PostgreSQL migration binary `openbot-migrate`（R51 当前只闭合
  `preflight-audit-retention`；其余 PostgreSQL/import readiness 子命令必须随 G8 逐项增加，不能用一个局部预检冒充整包）；
- Docker Compose production/dev profiles；
- SBOM、provenance、NOTICE、config schema 和 runbook。

all-in-one image 只允许 `OPENBOT_SINGLE_USER=true` 的 local/dev profile；multi-user production 未配置独立 Supervisor + runsc 时 readiness 失败，不能静默退回共享 browser。

### 16.2 Desktop 发行物

- macOS arm64/x64 signed + notarized（签名顺序：内层 Electron Framework / helpers → engine sidecar → Rust helper → 外层 Tauri app，最后 notarize）；
- Windows 11 x64 Authenticode installer（engine exe / dll、Rust sidecar 与 installer 全部签名）；
- Linux x64 AppImage/deb 为 **tier-2**（R122，2026-08-28）：三平台编译在 CI 必绿，但 golden / AX / 签名 / sandbox fidelity 证据不作为 G6 / G8 判据，不是 supported release target；升为 supported 需要独立 delta 补齐 GUI、sandbox、packaging 三套证据。Server（Linux）不受影响；
- Electron/Chromium、PostgreSQL、helper 与 Rust/Tauri 作为一个 release epoch 原子交付；PostgreSQL major 与 Electron major 不进入同一个 release；
- Electron 由 `cargo xtask engine fetch|verify|bundle` 按 `tools/engine-pins.toml` 组装：下载官方 zip → 校 sha256 → rebrand（bundle ID / Info.plist / 资源名不含 §23.4 禁用词）→ 打包 shim 为 ASAR → 写 fuses 与 ASAR integrity → 生成 sidecar manifest；全程 Rust，零 npm（R117）；
- 首次运行不下载 browser/database binary；
- sidecar manifest 记录 platform、arch、sha256、signing identity、version、minimum compatible core；
- 启动发现任何 sidecar digest、release epoch 或协议 version 不一致即拒绝。

Tauri updater 使用新项目独立 key、HTTPS 和签名。禁止复制 CrabCode 的 updater private/public key、bundle ID、deep-link scheme、证书、OAuth client 或发布账号。常规 downgrade 禁止；紧急 rollback 使用单独签名 authorization。

### 16.3 Supply chain

CI 固定执行：

```text
cargo fmt --check
cargo clippy --all-targets --all-features -D warnings
cargo test --locked
cargo deny
cargo audit / RustSec
cargo vet
OSV scan（Electron shim/packaged assets）
cargo xtask engine verify（engine-pins sha256 / --version / fuses / ASAR integrity / release epoch）
cargo xtask electron-shim-check（文件 allowlist / LOC ≤ 600 / API allowlist / forbidden import / 协议 hash）
cargo xtask grok-inventory --check（inventory/grok/files.yaml 与参考树同步）
仓内 package.json 恰一个且零 dependencies/scripts 的反向 grep
secret scan
license/NOTICE/provenance verification
CycloneDX/SPDX SBOM
reproducibility check
artifact signature/provenance verification
```

`Cargo.lock`、`tools/pins.toml` 与 `tools/engine-pins.toml` 提交（R117：engine 的 lockfile 就是 `engine-pins.toml`，不存在 npm lock）；git dependency 必须固定 commit。build.rs、proc macro、FFI 和 `unsafe` crate 单列审计；核心 crate `unsafe_code = "deny"`，确需 unsafe 的窄 crate 有 owner、测试和安全说明。

RUSTSEC-2023-0071 的 `rsa 0.9.10` 由钉版 `openidconnect 4.0.1` 非可选引入，advisory
无 patched 版本；本仓 RP 仅用 RSA 公钥验证 IdP RS256，不执行该通告前提中的
网络可观测私钥签名/解密。因此允许一条**窄豁免**：`deny.toml` 及 `cargo audit`
只忽略该 ID，并在它们之前必跑 `tools/check-rustsec-waivers.sh`。脚本锁死精确版本、
反向生产依赖链、openidconnect feature 零扩张与 RSA 私钥符号零命中；任一改变
必须先判红重审。RustSec 一旦出现 patched 版本，或 OIDC token endpoint / dynamic
provider 引入 private-key JWT，同 PR 必须删除或重写豁免。（§28.1 R44）

`cargo vet` 钉 `0.10.0`，数据固定放在仓根 `supply-chain/`，CI 只跑
`cargo vet --locked`。初始建立时只直接信任 Cargo Vet 官方 registry 登记的
Google exact/delta audits，`imports.lock` 锁定本轮实际覆盖的 14 个依赖；其余
350 个精确版本是 bootstrap exemptions，它们只表示“接入时已存在”，不表示
已审计。W-7 TLS 新增依赖时闸门精确列出 20/20 unvetted；Google/Mozilla 均无关键
rustls/ring exact/delta，因而另加 20 条**精确版本** exemption，每条写
`owner=security` 与 `not a full source audit`，当前合计 370，G2 外审仍不得跳过（R48）。
Cargo.lock 新增/升级未覆盖版本会直接判红，CI 不自动 regenerate；
更新 imports/exemptions 必须在同 PR 审查锁文件和差异。Mozilla 的 publisher/wildcard
动态信任和 Bytecode Alliance 在 0.10.0 下的 80 条无效审计告警均不被冒充为
已验证输入。workflow 同时把 checkout / rust-cache / install-action 从漂移 major tag
收窄到已实查的 patch tag。（§28.1 R45）

### 16.4 Observability

Rust 全链使用 `tracing` + OpenTelemetry；Server 暴露 Prometheus metrics；Desktop 默认只保留 7 天 redacted local ring buffer，不自动外传。**零 phone-home**：上游 `@copilotkit/runtime` 依赖 `@segment/analytics-node` 与 `@scarf/scarf`，默认向 CopilotKit/Segment 上报使用分析（`OPENBOT_ACCESSIBILITY_DISABLED` 关闭它）；Rust 版删掉 runtime 后没有任何第一方外发遥测端点，OTel exporter 只在管理员显式配置 collector 地址时才建连，supply-chain 闸门（§16.3）把"**第一方**二进制内出现非配置来源的外部分析域名"判为失败（R125 限定范围：Electron/Chromium 二进制内固有的 Google 域名字符串不在此闸门内，它的运行时出网由 §10.3 的 engine 约束与 §10.5 的代理兜底；`grok-bot` 的 `telemetry` / `codebase-telemetry` / `analytics-client` / `experiments` 家族一律 `R`，§2.3 条 16）。该变化写进 §22 的合规披露。

统一关联字段：

```text
deployment_id / tenant_id / request_id / actor_id / bot_id
channel_id / thread_id / run_id / tool_call_id
computer_id / generation / policy_decision_id
mcp_server_id / transport / release_sha
```

高基数 actor/thread 不进入 metrics label，只进入受控 trace/log。内部 HTTP/WS/UDS/Named Pipe 传播 `traceparent`；外部 provider/MCP 只有协议允许时才传播匿名 request correlation，不泄露内部身份。

必须指标：HTTP latency/status、active WS；Agent first-event/run/retry/token/cancel/stall；tool decision/outcome/commit unknown/approval wait；MCP/Drive OAuth/latency/truncation；computer start/crash/generation/resource/orphan；screen fps/frame age/drop/ACK/ticket；DB pool/query/outbox/audit；Tauri queue/lag/update/sidecar mismatch。

立即 P0：`audit_before_action_violation > 0`、`cross_scope_guard_failure > 0`、vault decrypt anomaly、SAML replay、sidecar signature mismatch。P1：unknown commit 非幂等 tool、OAuth refresh storm、computer restart storm、DB pool >80% 持续 5 分钟。

### 16.5 Retention

| 数据 | Server 默认 | Desktop 默认 |
| --- | --- | --- |
| operational logs | 30 天 | 7 天 |
| traces | 7 天 | 本地 24 小时采样 |
| metrics | 30 天 | 7 天聚合 |
| raw diagnostics | 关闭；启用后 24 小时 | 关闭；启用后 24 小时 |
| screen recording | 关闭 | 关闭 |
| audit | 管理员 policy；不得与 telemetry 共用 | 用户明确设置；默认无限直到空间阈值告警 |

## 17. 威胁模型与不变量

### 17.1 威胁主体

必须同时假设：恶意网页、prompt injection、恶意 remote Agent、恶意 MCP server、被攻陷 browser engine、被攻陷 Tauri renderer、普通用户越权、管理员误配、同主机其他进程、供应链包、数据库故障、网络中断和 provider 返回畸形流。"被攻陷 browser engine" 明确包括其**主进程**（Electron 主进程即 Node runtime）：Desktop 上它的最大损害域由 §10.3 的 engine 约束定义（profile / temp 目录 + loopback 代理），Server 上由 runsc 定义；G5E 的判据对主进程与 renderer 同时成立（R119）。

### 17.2 十二条发布级不变量

1. Rust 是 actor、target、policy、approval、capability 和 audit 的唯一铸造者。
2. 任一 acting effect 之前都有 durable decision + attempt。
3. deny 优先；空、坏、未知 policy fail-closed。
4. browser target 显式，不默认 active tab；snapshot ref 绑定 document generation。
5. profile 不跨 credential principal；workspace/download 不跨 thread/channel。
6. engine restart/reset 使旧 ref、ticket、approval、capability、lease 全失效。
7. human lease 期间 Agent acting 立即拒绝、不排队。
8. secret 不进模型、GUI state、browser event、普通日志、trace、screen URL。
9. non-idempotent unknown commit 不自动重放。
10. renderer/XSS 不能扩大 Tauri capability，remote content 无 Node/Tauri/Electron API。
11. Server browser 无直连互联网，只经 per-scope egress gateway。
12. 任一跨 scope 数据/帧/凭据泄漏是立即停发的 P0，不允许风险接受豁免。

### 17.3 Quota

按 tenant/user/bot/computer/run 同时限制：active run、browser process、tab、CPU、RSS、pids、disk、download、screen viewers/bandwidth、MCP calls/schema/result、provider token/cost、tool steps、wall clock、audit/log growth。配额拒绝有 stable code 和 audit，不表现成随机 timeout。

## 18. 全量实施矩阵

| 能力域 | Rust owner | 必须交付 | 完成证据 |
| --- | --- | --- | --- |
| Contracts | contracts | API/AG-UI/browser/MCP/component DTO、string ID、schema/version | canonical inventory；旧/新 golden 100% |
| HTTP/API | server/application | 用户/admin/agent/computer/plugin/component/channel/thread 全路由 | 每 endpoint happy/401/403-or-404/400/故障矩阵 |
| Database | infra | 28 表、13 migration 兼容；native thread/memory 新表 | 主键/外键/行数/JSON hash 零差异；restore drill |
| Auth/People | infra/application | single-user、Google/Microsoft/Okta、dynamic OIDC/SAML、role/revoke/group | OIDC/SAML 负向套件；撤权立即失效；外审 |
| Vault | infra | v1 envelope import、v2 AEAD、rotation/revoke/write-only | secret canary 全链零泄漏；rotation 原子 |
| Tenant package | application | 5 YAML、theme allowlist、引用与 group claim 校验 | 当前 package golden + 全部坏包错误 |
| Coworker/channel | domain/application | profile/visibility/preferences/delete/tombstone/membership/routing/activity | store/route/user journey；跨用户统一 404 |
| Native thread | domain/infra | message/run/event/lease/outbox/realtime/replay | crash/reconnect/fencing/cursor；无 Intelligence 配置完整运行 |
| Memory | domain/agent | explicit preference/fact（只有 `remember` tool 与 GUI 两个显式写入口）、provenance、delete | scope/recall/supersede/delete fixtures；无跨用户 recall；无后台抽取 job（正向对照：故意投喂应被抽取的对话，memory 行数不变） |
| Built-in Agent | agent | 3 provider families、stream/tool loop、8-step、cancel/budget/recovery | recorded stream、partial JSON、429/断流/unknown commit |
| Remote AG-UI | agent/server | endpoint safety、standing role、callback token/run assertion、stall | 官方 schema golden；SSRF/token/expiry/断流负向 |
| Tool/Policy/Audit | domain/application | schema/effect/CEL/approval/decision-attempt-outcome | corpus 对等；audit-before-action 违规 0 |
| MCP | agent | RMCP 3.1.4 Streamable HTTP tools/OAuth/per-call | 官方相关 client conformance 100%；恶意 server suite |
| Google Drive | agent | REST adapter、per-user OAuth、disconnect、read-only result | live sandbox tenant contract；撤权下一调用生效 |
| Skills/grants | application | ownership、grant、stale suspension、catalog generation | 授予/撤销/消失/重现全状态测试 |
| Components | ui/application | compiled Leptos、sandboxed HTML/CSS/JS、publish/withhold/data function/HITL | render/schema/golden 截图（设计系统文档 §10）/a11y；sandbox escape 0 |
| ComputerManager | computer | security scope、generation、driver、lease、quota、reconcile | 多用户同 Bot + 多 Bot 隔离；crash/reset/upgrade |
| Supervisor | computer | server runsc containers；desktop process tree | socket 不在 API；digest/namespace/resource/cleanup |
| Engine（两 role，R117–R119） | computer/desktop | 单一 engine package、clean-room shim、stdin boot capability + peer credential、role 铸造、component render session、OS 约束 fidelity、engine bundle | `electron-shim-check` / `engine verify` 绿；renderer 实际 sandboxed 正向证据；G5A–G5F 矩阵；conformance suite 两 role 各一份 |
| Browser actions | computer | navigate/read/snapshot/click/type/key/scroll/screenshot（R1 无 download/upload，§11.2） | OpenBot + CrabCode fixture；旧 ref 100% 拒绝；Chromium 自发下载被取消并上报 `download_refused` |
| Screen/input | computer/server/desktop | CDP、binary hub、ticket、coordinates、insertText（含 IME 合成文本）、human lease | latency/fps/backpressure/replay/race/跨 scope 注入 |
| File/shell | computer | canonical handle、symlink/hardlink、env、timeout/cancel | path corpus；cancel 5 秒内进程树归零 |
| Leptos GUI | ui | 31 route 对应旅程 + 1 个新增 memory 页（§3.1 条 7）、settings/admin/sign-in、web/desktop | route ledger 100%；golden 截图（设计系统文档 §10.1 矩阵 / §10.4 判据）/a11y（其 §9.2）/i18n（`xtask i18n-check`）/E2E（Desktop sandboxed component 的 a11y 豁免见 §3.3） |
| Tauri | desktop | capability、typed in-process、multi-window、update/sidecar | XSS 模拟；queue saturation；签名安装/升级/回滚 |
| Server/deployment | server/testkit | OCI/Compose/migration/health/readiness/multi-replica | clean checkout；8-Bot soak；backup/restore |
| Observability | 全部 | OTel/metrics/log/redaction/support bundle | run→decision→tool→computer 全链可追踪；无 secret |
| Release/legal | testkit/CI | SBOM/provenance/NOTICE/signature/brand separation | 未知 license/未登记复制/unsigned binary 构建失败 |

## 19. 实施顺序（阶段门；原 52 周日历已于 R125 作废）

### 19.1 阶段门（R125，2026-08-28；取代原 12 人 / 52 周日历）

不再给没有实证的周数。每个阶段只用入口条件、产物与退出证据控制；Engine 线是新插入的并行线，既有 G3 / G4 / G6 余项（§24.1 未勾项）按各自闸门继续推进，不被 Engine 线阻塞，反之亦然。

| 阶段 | 入口 | 产物 | 退出证据 |
| --- | --- | --- | --- |
| **P0-code** | 本轮 R115–R125 落档（docs PR） | ① `parity/overlay/v4.yaml` + `parity-check` 校验（R124 三行初值）；② `HumanLeaseEpoch` checked 递增 + poisoned 状态与测试（R124）；③ `cargo xtask grok-inventory` 与 `inventory/grok/files.yaml`（R116）；④ `cargo xtask engine fetch / verify`（消费 `tools/engine-pins.toml`，R117）；⑤ `cargo xtask electron-shim-check` 与 `crates/openbot-desktop/engine-shim/` 的 allowlist 规则（只锁规则，shim 代码属 P1）；⑥ 台账新增 T-ID：engine boot / role / render session / confinement / conformance（进 `browser-operations.yaml` 与 `components.yaml`，recount 同步） | 五个 xtask 子命令在干净 checkout 上绿；overlay 报告能机械打印 carry / revalidate / split / superseded 计数；`cargo test -p openbot-computer` 含 epoch poisoned 用例 |
| **P1 Engine 最小闭环** | P0-code 绿 | clean-room shim（≤ 600 LOC）；Rust spawn / stdin boot capability / peer credential / hello / ready / shutdown；`app.enableSandbox()` 与 §11.3 固定配置；Browser role 与 Component role 各完成一次 start → frame → stop；engine bundle（rebrand / ASAR / fuses / integrity）；两个 spike：runsc 内 Chromium 沙箱层（R121，钉 runsc 版本）与 Windows Named Pipe peer credential | 无 listening debug port；renderer 实际 sandboxed 的正向证据（三判据，§11.3）；malformed / stale frame 全拒绝；进程清理 0 orphan；`electron-shim-check` / `engine verify` 绿 |
| **P2 Browser / Screen / HumanLease** | P1 绿 | 全部 closed `BrowserOperation` → CDP 映射（T-BROP-0036–0044）；profile / tab / snapshot / ref / generation；ScreenIngress / ScreenHub / viewer ticket；坐标 / insertText / 拖拽序列 / HumanLease 接 engine；download / dialog / file chooser / popup / permission 的确定性拒绝 | G5A / G5B / G5E 与 G7 的协议 / 性能 / 跨 scope 矩阵（§12.6、§21.4、§21.5） |
| **P3 Desktop component** | P2 绿 | Component role render session；`DesktopSandboxCanvas` + 结构化参数 fallback；三层零 egress 与硬预算；T-CMP-0021 / 0022 与 R124 拆出的 Desktop T-ID | G5F：临时 process / partition、零 egress / callback 正向实测、预算与清理；component crash / DoS 不影响 GUI |
| **P4 Realm** | P1 绿（与 P2 并行） | `ExecutionRealm` 两域、file handle / shell helper / 进程树 / 资源 / cancel；Desktop engine 进程 OS 约束的 fidelity 实现（R119） | G5C / G5D；cancel 后 5 秒进程树 0；fidelity 进入 readiness |
| **既有余项** | 各自闸门 | G2 外审 / KMS / Windows 原生构建；G3 / G4 余项；G6 剩余 24 route journey、AppSidebar、完整 Composer、golden / AX；G8 发行 | §24 各闸门原判据 |

两次独立安全审计：第一次仍按 §24 G2；第二次在 G8 之前、P3 之后。

### 19.2 已作废的日历（历史）

原 v3 §19.1（12 人团队）与 §19.2（W1–W52 日历）于 R125 作废，不再作为承诺或估算依据；其内容可在 git 历史（`2a0c542` 及之前）查阅。范围的相对规模由台账机械给出：v3 剩余 todo 由 `cargo xtask recount` 复算，Engine 线新增 T-ID 在 P0-code 进入台账后同样由 recount 复算，不在本文件手填。

### 19.3 Phase 0 必做产物

Phase 0 不是“再研究一次”。必须生成机器可检查的：

```text
parity/api.yaml
parity/routes.yaml
parity/tables.yaml
parity/events.yaml
parity/env.yaml
parity/tests.yaml
parity/components.yaml
parity/browser-operations.yaml
provenance/sources.spdx.json
fixtures/agui/*.jsonl
fixtures/provider/*.jsonl
fixtures/mcp/*.json
fixtures/policy/*.json
fixtures/browser/*.json
parity/ui.yaml
fixtures/ui/seed.json
fixtures/ui/golden/MANIFEST.toml
fixtures/ui/golden/{web,macos-arm64,windows-x64}/*.png
tools/pins.toml
tools/engine-pins.toml                       # R117（2026-08-28，已落）
tools/electron-v43.3.0.SHASUMS256.txt         # R117：上游 SHASUMS256.txt 副本（已落）
parity/overlay/v4.yaml                        # R124（P0-code 建立）
inventory/grok/files.yaml                     # R116（P0-code 由 xtask 生成）
```

`parity/ui.yaml`（21 原语 + 45 业务组件 + 47 图标 + 6 运行时库 + 27 页，每项标 parity / 新增 / 替代）、`fixtures/ui/**` 与 `tools/pins.toml` 的 schema 与内容见设计系统文档 §11 / §12.1。

每个条目具有 owner、source link+commit、Rust target、test ID、migration rule、status。CI 拒绝未归类项和没有证据的 `done`。

## 20. Migration、Cutover 与 Rollback

### 20.1 不双写高权限 effect

- shadow Agent 只重放录制 provider stream，不执行 live tool；
- browser profile 同时只有一个 generation/engine 持锁；
- 同一 route/domain 同时只有一个 writer；
- audit、credential、profile、workspace 不做旧/新双写；
- tool/MCP/browser 外部副作用绝不为了对比执行两次。

### 20.2 产品数据库迁移

1. Rust shadow-read 现有 PostgreSQL，比较脱敏 canonical response。
2. expand migration，不改变旧写路径。
3. 按域切 read-only admin → coworker/channel CRUD → plugins/components → computer gateway → Auth。
4. 切 Auth 时清除 Better Auth session；保留 account/provider/issuer 与 user 关联，但不反向解密 Better Auth 的 access/refresh/id token，首次 Rust 登录后用新 token 替换；connector refresh token 位于 OpenBot Vault，原样保留。
5. 每个写域通过 route ownership flag 单写；出现差异立即回旧 owner。

### 20.3 Intelligence thread 导入

旧 OpenBot 在 maintenance 前运行固定 commit 的 legacy exporter，生成加密、签名的中立 bundle：thread、message、AG-UI semantic event、user/project mapping、cursor、hash。Rust importer：

1. 验证 bundle schema/signature/hash；
2. 映射 deployment/user/bot/thread ID。thread id 沿用上游 `thread-identity.ts` 的布局：RFC 9562 UUIDv8，前 6 字节 = `SHA-256(DEPLOYMENT_ID)` 指纹，其余随机。一个 Intelligence project 可能被多个部署共用（生产与其开发副本），导入只认指纹 `owns()` 为真的 thread，指纹不匹配或前缀期（部署尚无名字）铸造的 thread 列入报告由管理员逐个认领；Rust 之后继续按同一布局铸造 thread id，`owns()` 语义不断档；
3. 幂等导入 message/run event；
4. 对每 thread 重建 projection 和 full-text index；
5. 比较 count、ordered event hash、terminal state 和 sample render；
6. 写 `intelligence_import_cursors` 与 provenance；
7. 导入工具不包含在最终运行 image。

不导入未公开的 proprietary learning internal state。可观察 memory 只有在 exporter 能给出内容、scope 和 provenance 时导入；没有可验证来源的 hidden learning 不伪造。

### 20.4 Production cutover

1. 提前 7 天通知；
2. 进入 maintenance，拒绝新 run/tool/computer action；
3. drain active run，unknown commit 必须人工 reconciliation；
4. final Postgres backup + Intelligence export；
5. import/checksum；
6. 启动 Rust owner、验证 auth/thread/tool/screen canary；
7. 切入口；
8. 旧 TypeScript/React/Intelligence runtime 立即只读，不再接受请求；
9. 观察 30 天后删除旧 live secrets，保留法律/许可证允许的审计证据。

### 20.5 回滚边界

在 native thread cutover 前，每个已迁 Rust domain 可以按 route owner 回旧实现，数据库只含 additive schema。

native thread final cutover 是明确的 writer switch。之后不回到 Intelligence/TypeScript writer；运行故障回滚到上一签名 Rust build，仍读 PostgreSQL native thread 真源。原因是回写 proprietary thread 平台没有稳定公开双向合约，伪造可回滚会制造丢消息风险。

自动停止放量条件：

- 任意跨 tenant/user/bot/thread 数据、frame、credential 泄漏；
- 任意 acting effect 没有 durable decision；
- 数据 checksum 差异 > 0；
- 身份绕过、撤权不生效、P0/P1；
- 15 分钟窗口 5xx/崩溃率超过旧版 2 倍且绝对值 >1%；
- 至少 100 个可比 run 中完成率下降 >5 个百分点；
- profile 无法重新打开、single computer RTO 超过 2 分钟；
- sidecar/update signature 或 release epoch 不一致。

## 21. 测试与量化验收

### 21.1 Parity

1. 现有静态 route 与 Auth/legacy runtime 动态 route 全部进入 canonical inventory；每个覆盖 happy、401、403/404、400、dependency failure。
2. 31 个前端 route 文件全部映射；每个页面至少一个核心 journey 和权限可见性测试。
3. 28 张表、13 条 migration 全部映射；生产脱敏快照 primary key set、foreign key、row count、关键 JSON canonical hash 差异为 0。
4. 105 个现有测试文件以及 Phase 0 生成的全部 AST 级 test inventory 逐个标记 `ported`、`covered-by-golden` 或 `not-applicable-with-proof`；未分类为 0。1,007 个词法命中只用于交叉检查，不能替代 AST inventory。
5. 每个 compiled component 具参数/render/action golden；sandboxed component 具 publish/revision/security fixture。
6. 所有环境变量标记 preserve/rename/remove，并提供启动错误或 migration 文档；未知变量不静默忽略。影响关联方的裁决已在 §15.4 写死，Phase 0 只补表外的纯内部变量。

### 21.2 Agent/Protocol

- skeleton/late field/unknown event/rotating item ID；
- interleaved parallel tool calls、partial JSON、UTF-8 split；
- SSE/WS interruption、resume、429、401、timeout、cancel race；
- provider complete 但本地未 commit、本地 commit 但 UI 未收到；
- compaction 后 tool call/result、provenance、approval 不丢；
- official AG-UI golden 完整事件族；malformed event 不崩 UI；
- remote redirect/DNS rebinding/auth stripping/callback token/assertion expiry；
- thread fencing、duplicate event、cursor replay、multi-replica notification loss。

### 21.3 Tool/MCP/Policy

- deny-before-allow、empty/broken fail-closed、dry-run、policy version race；
- approval args hash/generation/expiry；
- non-idempotent unknown commit；
- RMCP 官方与产品相关 client conformance 100%，无 expected-failure baseline；
- HTTP redirect、OAuth state/PKCE/mix-up/resource、refresh rotation；
- catalog generation、withdraw/reappear、schema hash/effect change；
- malicious huge schema/result、tool name collision、unknown content type；
- Google Drive user A/B 结果和 token 绝不交叉；disconnect 下一调用 deny。

### 21.4 Computer/Screen/Sandbox

- 同 Bot 用户 A/B、Bot A/B、thread/channel A/B、generation old/new 全交叉矩阵；
- cookie/storage/service worker/cache/profile/download/workspace/artifact/IPC/secret/frame/audit/MCP token 交叉命中 0；
- symlink、hardlink、path traversal、malicious filename、socket抢占、PID reuse、profile lock；
- malicious engine 伪造 computer/generation/frame/outcome/peer/capability；
- navigation redirect、iframe、subresource、WS、WebRTC、QUIC、metadata、private/reserved IPv4/IPv6；
- DPI/zoom/scroll/letterbox/resize/tab close、ticket replay、多 viewer、IME 合成文本/拖拽序列；
- human lease 时 Agent input 100% 立即拒绝；
- Tauri XSS 不能扩大 command、读取 vault、订阅其他 thread 或取得 screen ticket；
- sandbox component XSS/top-nav/network/storage/MessageChannel replay/CPU-memory DoS。

### 21.5 性能/稳定性

- 不含外部 vendor 等待的本地 API p95 不得比旧版慢超过 10%；
- recorded provider stream 本地规范化与转发 p95 ≤50 ms；
- screencast 1280×800/10 fps loopback p95≤200 ms、p99≤400 ms；每 viewer ≤1 pending frame；
- Server 8 scopes、Desktop 4 scopes 连续 24 小时 soak；预热后 RSS 增长≤10%；orphan process/socket/profile lock=0；
- cancel 后高权限进程树 5 秒内退出；
- last viewer 断开 2 秒内停止 cast；
- thread reconnect replay 无丢失/重复 semantic event；
- migration/cutover maintenance window RPO=0；标准 Server backup RPO≤5 分钟、RTO≤15 分钟；single computer RTO≤2 分钟。

### 21.6 Security/Release

- 未豁免 critical/high dependency vulnerability=0；
- P0/P1 open finding=0；
- secret canary 在 transcript、AG-UI、provider diagnostic、audit、log、trace、metric、crash dump、backup index 中命中=0；
- audit-before-action violation=0；cross-scope guard failure=0；
- updater manifest 篡改、sidecar替换、partial update、old-version replay、wrong-platform artifact、key rotation 全拒绝；
- unknown license、missing NOTICE、unregistered copied file、floating git dependency、unsigned binary 均使 CI 失败。

评测同时运行 AgentDojo 间接提示注入、WebArena 浏览器任务和 SWE-agent ACI 类接口测试；报告任务成功率、越权率、拒绝率、人工审批率、恢复率、成本和 false positive，不能只报成功率。

## 22. 关联方影响

| 关联方 | 确定影响 | 固定处理 |
| --- | --- | --- |
| 普通用户 | Rust Auth 切换需重新登录；旧 thread 导入；browser login/workspace/connector token 应保留 | 提前 7 天通知；提供 import report；不要求重新连接 Drive，除非 token 本身失效 |
| Channel 成员 | 同一 public Bot 不再共享个人 profile；current run screen 绑定实际 credential principal | UI 明示当前 computer principal；只有发起者可默认接管，显式 transfer 才转移 |
| 管理员 | policy/grant/IdP/audit 语义保持；group control 从 no-op 变为强校验 | 提供 package/IdP preflight；不满足 group mapping 时拒绝启动而非静默忽略 |
| 外部 AG-UI Bot 开发者 | endpoint、standing role、tool schema、callback token/run assertion 需兼容 | 发布 protocol fixture、fake callback server 和兼容测试 image；不要求其改用 Rust |
| CopilotKit | Intelligence 从必需 live dependency 退出；OpenBot MIT attribution继续 | 仅通过合法 export/API 迁移；不抓取或复制未授权服务后端；不宣传得到 CopilotKit 背书 |
| Google Drive | OAuth callback、scope、revoke 和 REST quota | 新产品单独注册 OAuth client/redirect；per-user read-only；vendor revoke 可追踪 |
| MCP vendor | protocol pin、session、OAuth、schema/effect 可能变化 | nightly live contract + release conformance；vendor error 与 policy refusal 分开 |
| OIDC/SAML IdP | callback、issuer、claim/group mapping、session reset | 预发布 metadata validation；切换时统一 re-login；不复用 CrabCode/CopilotKit client ID |
| 运维/SRE | 新增 Rust server/Supervisor、computer image、bundled desktop Postgres/browser | 容量、backup、key、upgrade、egress、support bundle、PITR 和 incident runbook |
| 安全/合规 | SAML、browser、MCP、dynamic component、Agent content 都是不可信边界 | 两次外审；威胁模型/DFD/SBOM/签名/redaction/audit chain 证据 |
| 开发团队 | React/Hono/Bun 转为 Rust/Leptos/Axum/Tauri | owner map、Rust training、review checklist、unsafe/FFI gate、三平台 CI |
| CrabCode 权利人 | 专有代码可能进入新产品 | 每文件书面授权与 provenance；没有授权只按行为 clean-room 重写 |
| OpenAI/xAI/OpenCode/Steel 等上游 | Apache/MIT/NOTICE 与商标边界 | 回溯原始来源、保留修改声明；模型/API 使用权另行获得 |
| 最终客户/采购 | 依赖从 CopilotKit Intelligence 转为自有 Rust/Postgres，仍依赖模型/IdP/vendor | 数据流、subprocessor、retention、DPA、BYOK、region 和出网清单明确披露 |
| 仍以 deployment-wide `AGENT_TOOL_TOKEN` 回调的外部 Bot 运营方 | 共享 token 路径删除（§3.4）；未换发 per-agent token 的 Bot 在 cutover 后每次回调 401 | preflight 列出最近 30 天用共享 token 回调过的端点；cutover 前逐个换发并实测一次回调；换发未完成的部署不进入 §20.4 第 7 步 |
| 使用示例包 `allowed_groups: [all]` / 具名组的部署 | 包 channel 从"对所有人不可达"变为"按 §6.5 真正 provision"；具名组无 IdP mapping 的多用户部署启动被拒 | 包 preflight 报告逐 channel 给出"将被 provision 的受众"；拒绝启动的原因可操作（缺哪个 IdP 的 mapping） |
| 依赖 Intelligence 隐式记忆的用户 | 隐式跨会话记忆不可复刻（§4.3 条 8）；Rust 版只有显式 `remember` / GUI 记住 | 发布说明明写该差异；导入报告列出"可导入的显式 memory 数 = 0 或 N"，不伪造 |
| 合规 / 隐私（遥测） | CopilotKit runtime 自带的 Segment/Scarf 使用分析随 runtime 一并删除；Rust 版零第一方外发 | subprocessor 清单删去 CopilotKit/Segment/Scarf；新增项只剩管理员自配的 OTel collector |
| 依赖 `pgvector` 镜像或 `vector` extension 的运维 | Rust 版不需要 pgvector；Server 镜像改平装 `postgres:17` | 既有库里残留的 `vector` extension 零操作（§14.1）；runbook 写明可由运维自行决定是否清理 |
| plain HTTP（非 loopback）部署 | 仍可登录（§6.3），但 cookie 无 `Secure`、readiness 标 `insecure_transport` | 部署文档保持"把 TLS 放在前面"的建议，不新增拒绝开关 |
| Anysphere / Cursor（`grok-bot/` 反编译重建的权利人，R116） | 本仓 public 且保留重建源码树；用户裁决权利状态不作为技术计划的阻断项 | 只做规格先行吸收、不翻译不复制、每次吸收记 `source_lineage`；原始安装包不入仓；风险登记在 §23.1 条 8，不从本表消失 |
| Desktop 用户（R118 / R119） | Tauri GUI 保持轻量；Electron engine 只在 browser computer 或 sandboxed component 需要时启动：每 scope 一个 browser role + 每应用实例一个 component role | 状态与 fidelity 在 UI 明示；按需启动、零会话退出；无孤儿；原子更新；预算可见 |
| 组件作者（R118） | HTML/CSS/JS 契约（`window.__args`、零 callback、零网络）两宿主相同；Desktop 由 Electron 渲染成帧 | 预算与错误在文档写明；超限只终止本组件并显示 RefusedCard |
| 无障碍用户（R118） | Desktop sandboxed component 仍是帧画布 | 结构化参数 fallback、`Escape` 退出画布、具名说明（GUI 真源 §9.1） |
| Linux Desktop 用户（R122） | tier-2：编译必绿，但无 golden / 签名 / sandbox 证据，不是 supported release | 发布说明明写 tier-2；升级为 supported 走独立 delta |
| 最终客户 / 采购（出网清单，R125） | Electron/Chromium 进发行物后其二进制内固有的 Google 域名字符串不是第一方外发；运行时出网由 engine 约束 + 代理兜底；Grok telemetry / Statsig 家族全部 `R` | subprocessor / 出网清单只列管理员自配的 OTel collector 与用户自己浏览的站点 |
| SRE / 发布（R117 / R125） | 两个宿主 + Electron / PostgreSQL / helper sidecar 同一 release epoch；Chromium critical/high 72 小时升级（§11.3） | digest / signing / orphan recovery / rollback runbook；engine bundle 由 xtask 组装，零 npm |

## 23. 许可证、来源与品牌

### 23.1 代码许可

1. OpenBot 固定源码为 MIT。逐语言翻译仍按衍生实现治理，发行包保留 `Copyright (c) 2026 CopilotKit` 和 MIT 文本。
2. OpenAI Codex 是 Apache-2.0；复制/改造文件保留 SPDX、copyright、来源 commit、显著修改声明和适用 NOTICE。
3. Grok Build（xAI，§1.2 `19d42e35…`）第一方代码为 Apache-2.0，但部分工具来自 Codex/OpenCode；必须回溯原始来源和第三方声明，不能只记 xAI。**它与本仓 `grok-bot/` 是两个不同的东西**（条 8）。
4. AG-UI 为 MIT；RMCP/规范在许可证迁移过程中，按固定 commit 的实际文件 license 处理，不能给整个仓库套一个猜测。
5. Electron 自身 MIT，Chromium、Node、FFmpeg 等随其发行包的第三方 notices 必须原样交付。
6. Steel Browser/CDP 参考代码分别按其 Apache/BSD 等固定来源处理。
7. CrabCode 根 `THIRD_PARTY_NOTICES.md` 明示其为 closed-source proprietary；workspace `license = MIT` 只是一条 metadata，不能覆盖根声明或单文件来源。
8. `grok-bot/`（§11.5）是 Anysphere（Cursor）Grok Bot 0.18.0 的反编译重建，其 `NOTICE.md` 自述无上游源码许可、要求独立权利审查。用户于 2026-08-28 裁决：权利状态与独立权利审查**不作为本技术计划的阻断项**；本文件据此把它登记为**长期风险**而不是闸门，并固定使用方法（R116）：只做规格先行吸收，不逐文件 / 逐函数翻译，不复制任何文本，每次吸收登记 `source_lineage`，原始安装包不入仓。§23.3 的反编译句与 §11.4 的 clean-room 规则对它同样成立。

所有复用生成机器可读 provenance：source repo、commit、original path、destination、license、copyright、modified flag、source/target hash、authorization。

### 23.2 新项目发行许可

本次实施默认是内部、闭源、all-rights-reserved 的第一方新代码；MIT/Apache 等第三方代码按各自条款分区随包。该默认值避免在权利人尚未书面决定时擅自把 CrabCode 专有资产开放。若未来开源，必须另立书面发布决议并重新做 whole-tree license audit，不在本次重写中自动发生。

### 23.3 服务许可不等于源码许可

复用 Codex/Grok Build 开源代码不授予 OpenAI/xAI 模型、消费者订阅或 OAuth 使用权。模型只使用官方 developer/API credential；不得复制 Codex/Grok/CrabCode 的消费者账号桥或私有 token。

CopilotKit Intelligence、OpenAI API、xAI API、Google Drive 和 IdP 都有独立服务条款、数据处理与费用边界；Rust 代码许可证不能替代服务合同。

CopilotKit 当前服务条款把 managed services 与 open-source components 分开，并限制使用服务构建相似/竞争服务及逆向服务源码。因此 native thread/memory/realtime 只能依据 OpenBot MIT 源码、开放协议、自有需求与黑盒可观察用户契约做 clean-room 实现；不得把 managed Intelligence 私有响应、反编译结果或未授权内部资料当源码。旧数据导出必须使用客户账户依法可用的 export/API，并在迁移前取得合同/法务确认。同一条规则适用于 `grok-bot/`：它是反编译结果，只能作为行为 / 架构证据，不能作为源码（§11.5 / §23.1 条 8）。

### 23.4 品牌

MIT/Apache 不授予商标权。对外产品名称、bundle ID、domain、deep-link scheme、图标不得包含或仿冒 OpenBot、CopilotKit、Codex、OpenAI、Grok、xAI。内部仓库可以使用 `openbot-rs` 作为迁移代号；外部发行前必须使用完成商标清查的新品牌。法律 notices 和准确兼容性说明可以引用来源，并明确“无从属、认证或背书关系”。

## 24. Go/No-Go 闸门

### G0：Evidence 与权属

- 固定 source/provenance/SBOM/NOTICE；
- API/page/table/env/event/test parity ledger 未分类项=0；
- CrabCode 每个拟复制文件有授权或明确转 clean-room；
- 上游基线测试原始结果归档；
- `grok-bot/` 参考树可完整检出（LFS 指针 = 0，R116）且 `cargo xtask grok-inventory --check` 与树同步；
- `tools/engine-pins.toml` 的五个 sha256 与上游 `SHASUMS256.txt` 副本逐字相等（R117，§28.4 命令）。

### G1：Rust Core 与 PostgreSQL

- 10 crate workspace、toolchain、locked build；
- ApplicationService 经 Axum/Tauri 结果一致；
- 28 表/13 migration 映射；read checksum 0 差异；
- tracing/metrics/redaction 从首个 vertical slice 生效。

### G2：Auth/Vault/Policy/Audit

- OIDC/SAML/session/role/group/revoke 全矩阵；
- v1 credential/SSO decrypt + v2 rotation；
- CEL corpus对等；
- acting before durable decision=0；
- 第一次外部安全审计无 P0/P1。

### G3：Native Thread/Realtime/Memory

- 不配置 Intelligence 完整创建、运行、恢复、reconnect、list/read thread；
- thread fencing、outbox、cursor replay、multi-replica notification loss；
- memory scope/provenance/delete；
- legacy export/import checksum 0 差异。

### G4：Agent/AG-UI/MCP/Drive

- 三 provider adapter recorded trace；
- built-in tool loop、8-step、cancel/budget/unknown commit；
- full required AG-UI event golden、remote callback/stall/SSRF；
- RMCP relevant conformance 100%；Drive per-user/read-only/disconnect。

### G5：Computer/Isolation

- 同 Bot 不同用户、多 Bot、多 thread/channel、旧/新 generation 交叉=0；
- Server runsc mandatory，API 无 runtime socket；
- file/shell/path/process/network fault injection；
- engine compromise fixture 无法扩大 scope；
- 子闸门（R125）：**G5A ElectronEngine** —— 单一 shim（`electron-shim-check` 绿）、§11.3 固定配置、boot handshake / peer credential、CDP 映射与两 role 各一份 conformance、renderer 实际 sandboxed 正向证据；**G5B Scope** —— user / Bot / thread / generation / profile / partition 交叉 0；**G5C ExecutionRealm** —— `HostLocal` / `ScopedContainer` 无隐式 fallback；**G5D FileShell** —— 三平台 OS sandbox fidelity、TOCTOU / 进程树 / 资源 / cancel；**G5E EngineCompromise** —— malicious renderer **或主进程** / frame / outcome 不能扩大 scope，Desktop engine 进程约束 fidelity 进入 readiness（R119）；**G5F ComponentRuntime** —— 临时 process / partition、三层零 egress 与零 callback 正向实测、硬预算与清理（R118）；
- runsc 内 Chromium 沙箱判据（R121）：Server engine 的 renderer `/proc/<pid>/status` 必须 `Seccomp: 2` 且 `NoNewPrivs: 1`，且 layer-1（namespace 或 setuid helper）存在；`--no-sandbox` / `--disable-seccomp-filter-sandbox` 在任何配置禁止；若 runsc 不满足前提，修复只能在 runsc 版本 / 配置侧，永不在 Chromium flag 侧；P1 spike 在 Ubuntu 24.04 x86_64 + 钉版 runsc 上产出证据并把版本写入 §1.2。

### G6：GUI/Components/Tauri

- 31 route journey 100% + 新增 memory 页 journey；
- compiled gallery全部 Leptos；sandbox escape=0；
- multi-window ACL、Tauri XSS、queue saturation/shutdown；
- 视觉：设计系统文档 §10.1 矩阵的 golden 全部通过（Web 110 张、Desktop 每平台 54 张；判据 = 差异像素 ≤ 0.1% 且无 8×8 全差异块），三平台 bundle 摘要相等；**不做跨引擎逐像素比对**（这是 "web/desktop visual parity" 的可判定定义）；
- a11y：设计系统文档 §9.2 四项机械判据全绿（唯一豁免：Desktop sandboxed component，§3.3 已写死）；
- i18n：`xtask i18n-check` 绿（`en` / `zh-CN` 键集合逐字相等），`zh-CN` 27 页 golden 另录一套；
- `xtask design-lint` / `css-check` / `bundle-budget` 绿（`app.css` 上限 128 KiB、警戒 120 KiB，R123）；
- Linux Desktop 为 tier-2（R122）：其 golden / AX 不作为本闸门判据。

### G7：Screen/Handover

- 目标 fps/latency/backpressure；
- ticket/replay/origin/generation；
- coordinates / insertText（含 IME 合成文本）/ 拖拽序列；
- human lease 时 Agent acting 100% 拒绝；secret canary 0 泄漏；
- component render session 的帧走同一 ScreenHub 路径、≤ 5 fps、viewport 驱动启停（R118）。

### G8：Migration/Release

- 三次 production-scale backup/import/restore；
- RPO/RTO；
- signed OCI/installer/atomic sidecar update；
- Phase 0 AST 级 test inventory mapping 100%；
- 第二次外部安全审计无 P0/P1；
- 供应链、NOTICE、brand、runbook 全通过；
- engine bundle：`cargo xtask engine verify` 绿（sha256 / fuses / ASAR integrity / rebrand / release epoch），Electron `autoUpdater` 禁用，零 npm（R117）；Linux Desktop tier-2 不进入签名 / 更新判据（R122）。

任何闸门失败都只能修复后重跑，不能以“后续补齐”进入下一发布阶段。

### 24.1 实施状态勾选（2026-08-28；进度证据以机器台账为准）

- [ ] **G0**：Phase 0 证据产物已落；仍缺 §1.1 两份输入文档原件，故整关不勾。2026-08-28 R116 后 `grok-bot/` LFS 指针 = 0、可完整检出；`grok-inventory --check` 与 engine-pins 交叉校验待 P0-code。
- [x] **G1**：10 crate/locked build、Axum/in-process、28 表/13 migration/read checksum、tracing/metrics 四判据均已通过。
- [ ] **G2**：整关未通过；以下子项已经有本机机械证据：
  - [x] CEL 69 条 corpus 与固定 6 条差异台账；
  - [x] 通用 decision→attempt→capability→execute→outcome/audit 构造性边界；
  - [x] v1 credential/SSO 互操作与 v2 record rotation 代码路径；
  - [x] 环境/动态 OIDC、SAML、keyed session/group/replay 本机生产竖切；
  - [x] W-5 batch 1–6：identity / People / Audit / Policy durability+fanout+管理写面 / tool transcript projection / per-user credential 选择与退役；
  - [x] W-5 batch 7 + R73–R75：Tenant Package 五 YAML、environment allowlist、theme/brand 替代裁决、PostgreSQL Agent/Profile/Channel 原子同步与 §6.5 membership；callback lifecycle 1 条、真 server-side-tools 5 条与 Server MCP OAuth 连接/退役安全边界已闭合；G2 专项队列当前 **155 done / 79 todo / 234 total**；
  - [x] production session sign-out：`GET /api/me/session` 只投影revocable；`POST /api/auth/sign-out` 经已验ResolvedAuth+trusted Origin只删当前(session_id,actor)，清host-only HttpOnly/Lax cookie；single-user明确不可撤；PG17.11 SCRAM实得旧cookie401、其他session200。Better Auth wildcard仍因其余兼容路由todo而不勾整条；
  - [x] W-7c：Ubuntu 24.04 x86_64 自动 PR CI 首次真跑；Rust 1.98.0、workspace **1083/0/114**、native guards、parity、deny/audit/vet 全绿（run `32762651186`）。R63 后自动触发按用户额度指令关闭，但既有证据与本勾选不撤销；
  - [ ] 独立 SAML/XSW 与整体安全外审、Server KMS/HSM、Windows 原生构建；Desktop Local installed-app OAuth、browser/file/shell、approval critical realtime/真实 PG 浏览器竖切等 G4/G6 余面仍缺。
- [ ] **G3**：整关未通过；以下 native data base 已有机械证据：
  - [x] native 0016 十表、post schema fixture、40/40 真实 repository 与 staged tool→run FK；
  - [x] ThreadIdentity UUIDv8 deployment fingerprint 固定上游 8 条、Thread/Run/Lease/Memory 纯领域不变量；
  - [x] native thread `POST /api/threads/mint` / `GET /api/threads/{thread_id}`：typed ApplicationService、OS CSPRNG、PostgreSQL scope 与 Axum/Tauri 对拍；
  - [x] fencing takeover、single foreground、terminal exactly-once、cursor replay、replay-safe outbox、explicit memory scope/source/delete 真库矩阵；
  - [x] typed `BeginThreadRun`：thread/membership/message/running run/started event/lease/replay-safe outbox 同事务，run-id 幂等与末段失败全回滚；
  - [x] thread event durable replay→LISTEN/NOTIFY wake→live、丢通知周期 catch-up、双 replica、SSE `Last-Event-ID` reconnect 与撤权断流；
  - [x] scope-aware native history + compatibility facade：空/new/invisible/deleted 200 `messages:[]`，结构损坏仍 503；
  - [x] explicit memory backend：GUI user_action remember、list/correct/forbid/delete、user+exact Bot/thread FTS + structured-tag recall、无后台抽取；
  - [x] expected-sequence semantic chunk/terminal writer、assistant message materialize、dispatch outbox relay、lease renew/takeover 与 delivered-stale reconciliation；G4 consumer 未接时明确 failed terminal，不伪造回复；
  - [x] same-origin/read-only `openbot.thread-events.v1` WebSocket 与 SSE 共用 durable cursor stream，1KiB inbound cap；
  - [x] channel roster activity 生产闭环：channel-anchored user begin/assistant terminal 在原事务内单调更新
    `channels.last_message*` 并发 bounded PostgreSQL NOTIFY；typed subscription 每帧回查当前
    `channel_memberships`，`GET /api/channels/events` 固定 same-origin/read-only
    `openbot.channel-activity.v1`。socket 无 durable cursor，断线必须 refetch roster；通知/frame
    均不携 member IDs；
  - [x] channel detail/shared native thread读面：typed GET经同一ApplicationService，list/detail只投影
    deployment/tenant匹配的native channel thread且不读Intelligence mapping；channel-anchor的
    status/begin/history/realtime按当前channel membership，direct-bot仍按thread membership；撤权后
    stale thread membership不能扩大上述四面。完整tool/approval/screen control矩阵仍独立todo；
  - [x] user channel create + recipient routing + native first turn：`POST /api/channels`只收canonical
    Agent IDs，PG同事务按序锁profile并复核tenant/domain access，写channel/creator membership/
    channel_agents/deployment-owned native thread且零Intelligence mapping；`POST /api/route`显式选择零模型，
    inference以当前roster/active reach经package Chat模型建议，所有不确定结果回确定性default；serializable
    audit事务复读候选并只记ID/closed reason，候选变化409。新`POST /api/threads/{id}/runs`只封装既有
    BeginThreadRun，刚创建thread的真实PG桥接已通过；
  - [x] native channel conversation snapshot + realtime idle send：单条PG statement原子投影messages/
    foreground run/active sampling text tail/last event cursor，native GET只取path+AuthContext且no-store；
    EventSource从cursor接SSE durable replay/live，标准Last-Event-ID重连优先，gap/坏payload不显示而refetch。
    有thread直接BeginRun、无thread先mint再以channel anchor begin；terminal后PG history materialize并硬刷新恢复；
  - [x] durable actor-owned foreground cancel：typed request先按deployment/tenant/current anchor membership与
    run owner锁定，再写replay-safe internal outbox；LISTEN只作wake、100ms poll兜漏通知，lease owner/fencing
    跨副本消费。`Cancelling`保持foreground，只有child-stopped后才有唯一`Cancelled`；无local child时
    terminal+cancel+原dispatch outbox同事务收口，exact replay不重复行；
  - [x] Rust Intelligence importer：signed+encrypted neutral bundle、独立 target mapping/claim、逐 thread 原子 cursor/resume、DB 重算 ordered checksum、observable memory provenance 与 staged tool→run FK finalize；最终 runtime 零 Intelligence 调用；
  - [x] 50ms/8KiB accumulator 已接真实 Rust OpenAI Responses/Chat producer；normalized text/reasoning 以 expected sequence 写 `DurableTextRun`/journal，terminal 只物化 text；
  - [x] built-in `remember` backend：explicit prompt→provider call→唯一 tool pipeline→`origin=remember_tool` preference/fact+DB provenance→durable tool pair→第二次 sampling；无后台抽取；
  - [x] Memory GUI与全局写入控制：native 0022把tenant/actor runtime control独立于memory记录持久化；
    缺行默认enabled。disabled在同一事务拒绝GUI remember、correct与built-in `remember` tool，
    但list/recall/forbid/delete始终可用。`/settings/memory`以owner keyset展示status/kind/sensitivity/
    scope/provenance/tags，支持load-more、correct replacement、forbid/delete内容擦除与中英双语；
  - [ ] 实际 legacy exporter/production bundle 三次演练尚未闭合：固定上游公开源码只有按已知
    `(threadId,userId)`的`getThread`与单thread messages读取，没有thread枚举、semantic event或
    observable memory export；不得猜managed private endpoint，等待合同/法务许可的customer API/数据。
- [ ] **G4**：整关未通过；以下 Rust built-in Agent 子面已有本机机械证据：
  - [x] pure reducer + bounded dispatch consumer；reserve→durable ack→activate、activation 起算 absolute deadline/lease heartbeat、cancel 等 children stopped；
  - [x] OpenAI-compatible Responses + Chat adapter：safe dialer、SSE UTF-8/multiline、skeleton/延迟字段、partial JSON、交错 tool calls、未知扩展、真实 read-gap stall；
  - [x] Anthropic Messages adapter：system/messages/tools 分域、thinking/text/partial tool JSON/usage、固定 version + header-only key、未知事件隔离；
  - [x] Google streamGenerateContent adapter：systemInstruction/content/function call+result/usage、header-only key、无 vendor response id 时确定性 trace id；
  - [x] package `model.yaml` model/credential ref、每 run PostgreSQL active credential 精确选择、stored-first/env fallback/corrupt-no-fallback、standing prompt/provenance 与 Server production assembly；
  - [x] `providerSource=package|managed` 权威路由、managed 三家 production factory；缺 managed adapter 不回落 package key/provider；
  - [x] pre-stream unavailable/首事件 429/5xx 有界 retry + Retry-After，auth/schema/commit-unknown/mid-stream 永不重放；
  - [x] 每 sampling output token cap 与 usage 双校验；`agent.invoked` / `agent.stream_stalled` / 新增 deadline audit 均写 production hash chain 且先于普通 terminal；
  - [x] 真实 tool host loop：complete batch 按 stable index 串行、跨 sampling 8-step、三家 assistant/tool pair、durable checkpoint/context reload、Rust call identity；首个 production executor `remember` 经 CEL/decision/attempt/capability/outcome/audit，generation race fail-closed；
  - [x] run/user cancellation 的统一host入口：PostgreSQL control outbox跨副本到lease owner，built-in Agent
    watch token沿context/provider/tool child传播；真实PG证明active child先drop、再写唯一Cancelled terminal。
    RMCP/computer/file/shell各协议级notification/process-tree仍独立todo；
  - [x] 固定 `@ag-ui/core@0.0.57` 的 33 个 event literal、RunAgentInput、stateful lifecycle/text/tool/state/messages/activity/step/reasoning/raw/custom/interrupt/error decoder 与原子 RFC 6902；开放 payload 只保留为 bounded untrusted data；
  - [x] package-backed `remote_ag_ui` lifecycle/text 生产竖切：权威 route→唯一 SafeDialer POST/SSE→decoder→durable semantic chunk/assistant/terminal；RunAgentInput 以 DB clock 铸 10 分钟 assertion，并从 current grant 投影同一 whole tool set；
  - [x] per-Agent callback token issue/rotate/revoke：`obot_agt_`+32-byte CSPRNG、DB hash-only、fresh Origin、owner/admin/fresh generation、mutation+audit 同事务；callback 同验 token/assertion/Bot/actor/run/lease/current tool-set，并经同一 PostgreSQL sequence + ApplicationService 执行真实 RMCP outcome；共享 `AGENT_TOOL_TOKEN` 构造性不存在；
  - [x] native 0017：PostgreSQL durable per-run tool sequence、catalog generation/schema/effect/availability、grant state 与 endpoint+vendor+provenance fingerprint；missing/changed 同事务 suspended_missing+audit，重新出现不自动启用；
  - [x] pinned RMCP 3.1.4 / MCP 2026-07-28 client：SafeDialer-only Streamable HTTP、per-operation initialize/list/call/close、1000/4KiB/256KiB/20k limits、progress-aware timeout cancellation、live schema binding 与 commit-unknown reconciliation；
  - [x] server-side-tools 五条 production 竖切：无 grant 零工具、vendor 原 schema、official RMCP HTTP 真调用、Bot audit、CEL refusal marker；另有 definite failure、secret content block、acting approval refusal 与 two-replica sequence 证据；
  - [x] native 0018 + credential identity：OAuth client 登记/轮换推进 server credential generation，旧 grant 固定旧代际并在 refresh 转 suspended_missing，永不把权限静默搬到新 client；
  - [x] Server/Desktop Remote MCP OAuth：401 PRM→exact issuer/S256 discovery、RFC8707 resource、HMAC+AEAD single-use state、PKCE/code callback、v2 refresh pointer/rotation、actor catalog/runtime、401 单次 refresh/retry 与 typed HTTP 四面；
  - [x] local-first disconnect：本地 tombstone/join delete/audit 先 commit；RFC7009 失败进入 `revocation_pending`，SKIP LOCKED 周期 reconciliation 成功后才记 vendor 已撤权；
  - [x] native 0019 + Google Drive GA REST：closed transport identity、compile-time single-vendor catalogue、4 条 read-only static tools、asker per-user Google OAuth、SafeDialer REST、vendor link/provenance、Agent 401 单次 refresh/retry、local-first disconnect/revoke reconciliation；正文不入库且不建本地 ACL/index；
  - [x] native 0020 + durable approval backend：完整 binding（含 AuthGeneration）、pending-only 脱敏摘要、actor-only typed GET/decision、fresh Origin、grant/deny/expire/cancel hash-chain audit、跨 replica wait、once-per-run exact reuse；真实 acting MCP grant 后才写 approval-linked decision/attempt 并调用 vendor，deny 零 attempt/action；
  - [x] Agent profile权限与roster/detail读面：`canAccess/canRun/canManage`六条纯领域判据唯一化，
    `AgentReadScope`只从权威AuthContext取得tenant/actor/admin；PostgreSQL同时收紧package tenant、
    public/private、owner/admin、soft-delete与per-user hidden，SQL结果再过domain终判。GET list/detail
    只回closed secret-free DTO与`no-store`，missing/invisible/deleted/cross-tenant统一404；
  - [x] create-time routing provider：production main复用package model/每请求PostgreSQL credential/Vault/
    SafeDialer并固定OpenAI Chat Completions；模型只建议权威roster内ID，缺credential/transport/坏JSON/
    低confidence均由Application成功fallback，tool output拒绝，消息与模型理由不进hash-chain audit；
  - [ ] provider gate 要求的三家 recorded vendor trace 仍为 **0/3**，本批未使用 live vendor credential；human approval 的 Leptos/Axum 可点击竖切已落，但真实 PG 浏览器端到端、critical realtime/完整 thread 集成仍未闭合；完整 run-wide token/cost/并发/computer budget、Desktop Local installed-app client/system browser/random loopback callback、RMCP/computer/file/shell各自的协议级cancel notification/process-tree、MCP 专用 private egress与 admin custom/通用 refresh/grant/effect 完整 UI、用户创建 remote Agent lifecycle/customer auth、interrupt/resume 与其余事件 durable/UI projection、browser/file/shell executor 尚未闭合。Google `drive.readonly` restricted scope 的外部 verification/security assessment 也不是本机代码证据。
- [ ] **G5**：ComputerSecurityScope/runsc/fault injection/engine compromise 未完整实施。
- [ ] **G6**：整关未通过；以下 Web GUI 地基已有本机机械证据：
  - [x] 第一真源钉版 Leptos 0.8.19/router 0.8.13/meta 0.8.6/i18n 0.6.2；Tailwind 4.3.3、Trunk 0.21.14、Binaryen 132、wasm-bindgen 0.2.127 全部 exact hash/version，真实 offline/locked Trunk bundle A/B 字节一致；
  - [x] tokens.toml 单源生成 CSS/Rust、Inter 4.1 随包、74 项 icon manifest/SVG 双向闭合；i18n en/zh-CN 456 叶键及占位符 exact；WASM gzip/CSS/fonts 与零内联脚本预算绿；
  - [x] Axum `APP_DIST_DIR` 条件挂同一 bundle，唯一同源 external bootstrap、strict CSP/安全头、cookie/Accept-Language 首帧 `<html class lang>` Rust 改写、API/缺失 asset 不被 SPA fallback 隐藏；
  - [x] Approval 页面只展示服务端权威 effect/target/redacted arguments/change，GET poll + fresh POST grant/deny；真实 Chromium 已验证 APG ThemeToggle/LocaleSwitch、批准后 card 消失+status、1440×900 与 1024×640 无横向溢出、landmark/heading/id/name/remote-resource 审计；浏览器数据来自明确 test-only fixture，不冒充生产 PostgreSQL；
  - [x] Server 用户偏好经唯一 typed ApplicationService/PostgreSQL native 0021 持久化并镜像 closed `SameSite=Lax` cookie；Desktop Local closed file 原子写；Leptos startup read + serialized/coalescing partial PUT，失败显示本地化 `role=alert`；
  - [x] opt-in Tauri 2.11.5 production custom protocol：host-bound window label→AuthContext，未绑定连 asset 401；preferences/approval 只经 typed in-process；本地偏好/OS locale 首帧 Rust 改写、strict CSP、canonical asset/closed MIME/8MiB；Linux Server/Web 与 WASM graph 构造性无 Tauri/Wry/GTK；
  - [x] UI ledger 的27条primitive子账全done；46条Tabler→Lucide经第一真源→icons.toml→ledger三向join关闭；
    layout 组 detail-panel/page-shell/row-mark/stagger 四条业务、orb/ai-core→AgentPresence两条又有生产实现与本机证据。
    ComputerPlaceholder/Art两条又共享唯一中性线稿闭合；Batch30关闭独立ChannelRow；Batch31关闭
    abstract-avatar→统一Avatar与AgentCard；Batch32关闭RecipientField。当前UI=`85/67/152`；
    Google Drive brand、AppSidebar总项+其余32业务/runtime/golden保持todo；
  - [x] `design-gallery` compile feature 承载 `/_design` 状态/键盘/AX 样本；production feature 关闭且 bundle gate 直接要求 WASM `_design` byte=0。真实 Chromium 已验证十条基础原语的 Enter/Space、Field ARIA、focus-within、Textarea 十行 cap、separator/skeleton AX 与 DOM 五类零缺陷；截图只作目视 QA，不冒充 golden；
  - [x] Message/Bubble compound、platform-aware Kbd、SHA-256 deterministic Avatar、5s generation-safe polite Toast、400ms hover/focus/Escape Tooltip 已过 Rust/WASM/Chromium/AX；Avatar remote image、Item/Tooltip external link 构造性拒绝，Toast 仍不冒充所有 accepted:false 业务 use case 已接线；
  - [x] Dialog/Sheet 共享唯一 modal kernel：explicit ARIA、首焦点、Tab双向环、Escape/close/backdrop、return focus、body scroll lock 与 path-sibling inert/aria-hidden；Sheet top/right/bottom/left 四值不复制安全规则；
  - [x] Menu compound 以 closed `data-state` 映射 open/disabled，根/一层子菜单实现 APG ↑↓/Home/End/Enter/Space/Escape/→←、500ms 多字符 typeahead、disabled skip、exactly-once activation、outside dismiss；Tab/ShiftTab 离开菜单且焦点不落 body；
  - [x] MessageScroller 具 initial/following/free/anchored 三态：Resize/Mutation+rAF 跟 streaming，真实用户意图让出，append/prepend/resize 保持阅读项，new-user 48px anchor、same-count 不回旧 anchor，命名 region→log/live 与自管理 end button；
  - [x] Combobox/Select 共用唯一 listbox 内核：editable filter/empty 与 select-only 500ms typeahead，committed/active 分离，disabled skip、Escape cancel/Tab commit、exactly-once selection；owner focus+active-descendant、named listbox、Field ID/disabled/invalid/described 自动接线；
  - [x] Sidebar 具 lg expanded240/user rail48、md auto rail、compact shared Sheet 三态；Ctrl/Command+B、named nav/current、same-origin links、Footer bottom、external trigger controls/返焦、mobile inert/scroll cleanup 均有三 viewport Chromium 证据；
  - [x] 46条Lucide mapping的文档名、manifest upstream/name/usage、Rust enum target、SVG安全形状与done evidence逐条机器相等；IconBrandGoogleDrive必须继续跟brand manifest todo同步；
  - [x] layout 组四条：PageShell 只消费960/1200/768宽度与44px topbar token，已接入production Approval；
    PageBackLink 构造性同源，RowMark 为中性vendor tile，Stagger 为30ms/8cap纯CSS；DetailPanel 四态优先WAAPI尺寸联动、
    API不可用时同token CSS fallback、reduce=0ms，完成后卸载并返焦；
  - [x] AgentPresence 按§6.7把上游437+395行orb/shader收为20px四态Signal；完整环/单弧/双弧/danger环分形，
    thinking/speaking=1200ms、error=160ms×1均从token生成，本地化role=img名称，reduced-motion全局静止；
  - [x] ComputerPlaceholderArt 以唯一currentColor中性线稿替代两份彩色噪声SVG；
    ComputerPlaceholder只复用它，零gradient/filter/defs/ID/remote/字面色，两入口均aria-hidden/focusable=false；
  - [x] AppSidebar的production roster与独立ChannelRow已落：50条keyset页/load-more、只搜可见name/
    last-message、socket reconnect-refetch、current row、三断点同一children、current user/session/
    sign-out；真实data-backed `/channel/:id` 可直接硬刷新。ChannelRow已勾；Batch32接真实new-channel，
    Batch34/35再接plain conversation、durable Stop与mount-local queue；Batch36接Memory destination，
    Batch37接Settings Preferences destination。AppSidebar总项仍因skills/admin destinations缺失不勾，完整channel route仍因markdown/sources/附件/per-channel draft/
    steer/screen journey缺失不勾；
  - [x] data-backed `/agents` read surface：固定上游`mine`与`!mine && public`两组、144×180
    AgentCard、URL-owned只读profile DetailPanel、同源percent-encoded id、404错误态与AppSidebar
    Agents destination；头像在已有同名文字处AX隐藏，Inter最终产物从`/fonts/*`同源加载。只关闭
    T-UI-0029/0030；create/edit/duplicate/hide/unhide/delete/start-channel与正式route/golden仍todo；
  - [x] `/channel/new`真实首发route：static route先于dynamic channel id；无recipient发送禁用且刷新零
    channel；URL可恢复hidden但有权的Agent，RecipientField复用Combobox键盘模型；首发只按
    create channel→native BeginThreadRun→成功navigate，begin失败复用同一channel/run-id，create响应未知
    禁止二次提交。1440/1024/900/600 overflow0、landmark/h1/id/console与52→52→53浏览器证据成立；
  - [x] Composer纯状态地基：固定上游draft10+queue16逐条Rust移植；Segment→draft、single Agent、
    command prompt/action/chip与busy park/settle/remove/一次合并均由纯函数承担，`Cow`保留no-op identity，
    action只返回closed effect ID。queue明确只活在当前mount、不冒充durable outbox；Batch35已把text-only
    busy queue/remove/单次settle与Stop后drain接进production conversation，但sources/附件/per-channel draft/
    steer仍未落，故Composer业务/route仍不勾；
  - [x] channel plain-text conversation production slice：既有Message/Bubble/MessageScroller渲染durable
    user/assistant/tool activity，system prompt不进transcript；Started/text/terminal驱动busy/streaming/
    localized notice，Enter发送/Shift+Enter换行/IME Enter不提交，busy可写草稿但Send禁用。四视口X/Y
    overflow0；Batch35又接durable Stop→Cancelling→Cancelled与mount-local queue/remove/settle，hard reload
    queue丢失而foreground恢复；markdown/tool boundary/screen/sources与完整route保持todo；
  - [x] `/settings/memory`新增route：只消费typed no-store API；50→52 owner keyset、四种状态、scope/
    source/origin/tags、global writes switch、correct/forbid/delete均接production ApplicationService。
    写入关闭跨reload保持，只禁新增/纠正而不禁擦除；dialog取消返原按钮，成功权威refetch后聚焦
    replacement/变更行。release CSS 445规则真实加载，中英、1440/1024/900/600 overflow0、console0；
  - [x] `/settings` Preferences真实route：复用Batch16唯一preference context/API/native0021，不造第二
    store。ThemeToggle在页面与Sidebar均为system/light/dark APG radiogroup；LocaleSwitch由调用点提供
    唯一bounded ID，双实例duplicate=0。快速theme+locale连续更新显示当前页面唯一`role=status`；
    worker固定在AppShell稳定owner，locale重渲染不再取消receipt收尾，队列排空后status消失且reload
    保留合并值。Sidebar Settings真实导航、键盘、四视口overflow0、console0；settings二级layout仍todo；
  - [x] Settings secondary shell：只包裹`/settings`与`/settings/memory`，全局App shell继续拥有唯一main；
    named nav以`--size-subnav`实得200px，Back/General exact/Memory三条均同源且current恰1。固定上游
    Connected accounts/gallery因production route未实现而构造性不画断链；两route点击/硬刷新/返回app、
    1440/1024/900双列与600单列均overflow0、duplicate/alerts/console0；其余settings route仍todo；
  - [ ] reviewed 外部产品名/bundle id/deep-link 后的 `tauri.conf.json`/binary、真实 window lifecycle/multi-window integration、macOS arm64/Windows x64 原生发行构建，以及AppSidebar总项+其余32业务组件/28 route、1 brand icon、6runtime替代、110 Web + 两平台各54 golden、完整axe/键盘E2E尚未闭合；
  - [ ] Tauri target-aware bans/sources 已绿；macOS/Windows 各仍有 5 个 MPL-2.0、5 个 runtime UNIC unmaintained（无 patched 版），Cargo Vet 为 macOS **270** / Windows **269** unvetted（既有 target 基线 181，净增 89/88）；未改 license/advisory/vet policy，故供应链与 G6 整关均不勾。
- [ ] **G7**：Screen/Handover 性能、安全票据、human lease 未实施完成。
- [ ] **G8**：生产规模迁移演练、签名发布、第二次外审、brand/runbook 与全台账 100% 未完成。

当前总台账：parity **644/1678 done（1034 todo）**，fixtures **16/38 done（22 todo）**。勾选只表示整项判据已经通过；局部代码存在但整关未闭合时不得勾整关。

## 25. Definition of Done

只有同时满足以下条件，才能称“OpenBot 已完成全量 Rust 重写”：

1. 固定 OpenBot commit 的正式页面、API、数据、协议、治理、部署和用户旅程全部在 Rust-owned 路径有可重复证据。
2. 第一方 React/Hono/Bun/TypeScript Agent/MCP/Auth/DB 生产链清零。
3. 非 Rust 例外仅为最小 engine shim（两 role，clean-room，§11.3）、外部引擎/服务和受隔离的用户脚本数据；工作区零 npm（R117）。
4. Rust/PostgreSQL 是 thread、memory、realtime、run lock、policy、audit 和产品数据真源；无 CopilotKit Intelligence 许可/key 仍完整运行。
5. remote AG-UI、Google Drive REST、MCP tools、Generative UI、SSO、people、tenant package、computer 与 screen 没有因“重写”被删减。
6. 当前已知缺陷按本文件修正，未被机械照译。
7. 105 个测试文件、Phase 0 AST 级全部测试条目、31 route、28 表、13 migration 和所有动态路由均有归类与证据。
8. 跨 scope 泄漏、audit-before-action 违规、unsigned/unknown provenance、P0/P1 均为 0。
9. 安装、升级、备份、恢复、迁移、incident、key rotation、provider/MCP/browser outage 均有演练过的 runbook。
10. 第一真源保持不变；本文件及 evidence bundle 记录全部修订原因和来源。

## 26. 主要一手来源

### OpenBot 与 CopilotKit

- [OpenBot 固定源码](https://github.com/CopilotKit/openbot/tree/891df72f1827454d8b353d108fe5dd2313b7e30d)
- [OpenBot architecture](https://github.com/CopilotKit/openbot/blob/891df72f1827454d8b353d108fe5dd2313b7e30d/docs/architecture.md)
- [OpenBot screencast](https://github.com/CopilotKit/openbot/blob/891df72f1827454d8b353d108fe5dd2313b7e30d/agent-computer/src/screencast.ts)
- [OpenBot Supervisor](https://github.com/CopilotKit/openbot/blob/891df72f1827454d8b353d108fe5dd2313b7e30d/supervisor/src/docker.ts)
- [OpenBot MCP per-call client](https://github.com/CopilotKit/openbot/blob/891df72f1827454d8b353d108fe5dd2313b7e30d/server/src/plugins/mcp.ts)
- [OpenBot vendor transport / Drive REST](https://github.com/CopilotKit/openbot/blob/891df72f1827454d8b353d108fe5dd2313b7e30d/server/src/plugins/transport.ts)
- [CopilotKit Enterprise Intelligence architecture](https://docs.copilotkit.ai/premium/intelligence-platform)
- [CopilotKit self-hosting and enterprise license boundary](https://docs.copilotkit.ai/agno/premium/self-hosting)

### Agent 与协议

- [OpenAI：Codex as a platform / open agent harness](https://developers.openai.com/blog/codex-as-a-platform)
- [OpenAI：Codex App Server 官方文档](https://learn.chatgpt.com/docs/app-server)
- [OpenAI：Codex open-source components](https://learn.chatgpt.com/docs/open-source)
- [xAI：Grok Build is Now Open Source](https://x.ai/news/grok-build-open-source)
- [Grok Build 固定源码](https://github.com/xai-org/grok-build/tree/19d42e35c07a9c9244f03f6df0c4c353f970d4f9)
- [AG-UI 固定源码](https://github.com/ag-ui-protocol/ag-ui/tree/e42bdbedc27cdf982ed9b5de904215acd73a17fb)
- [AG-UI Events](https://docs.ag-ui.com/concepts/events)
- [Anthropic Messages streaming](https://platform.claude.com/docs/en/build-with-claude/streaming)
- [Google Gemini generateContent / streamGenerateContent](https://ai.google.dev/api/generate-content)
- [RMCP release 3.1.4](https://github.com/modelcontextprotocol/rust-sdk/releases/tag/rmcp-v3.1.4)
- [MCP conformance](https://github.com/modelcontextprotocol/conformance)
- [MCP 2026-07-28 authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)

### GUI、Browser 与隔离

- [Tauri + Leptos](https://v2.tauri.app/start/frontend/leptos/)
- [Tauri calling frontend / Channel](https://v2.tauri.app/develop/calling-frontend/)
- [Tauri capabilities](https://v2.tauri.app/security/capabilities/)
- [Tauri updater](https://v2.tauri.app/plugin/updater/)
- [Electron Security](https://www.electronjs.org/docs/latest/tutorial/security)
- [Electron Fuses](https://www.electronjs.org/docs/latest/tutorial/fuses)
- [Electron ASAR Integrity](https://www.electronjs.org/docs/latest/tutorial/asar-integrity)
- [Electron release timelines](https://www.electronjs.org/docs/latest/tutorial/electron-timelines)
- [CDP Page.startScreencast](https://chromedevtools.github.io/devtools-protocol/tot/Page/#method-startScreencast)
- [CDP Input](https://chromedevtools.github.io/devtools-protocol/tot/Input/)
- [gVisor Security Model](https://gvisor.dev/docs/architecture_guide/security/)
- [Electron release v43.3.0（2026-08-04；Chromium 150.0.7871.212 / Node 24.18.1；SHASUMS256.txt）](https://github.com/electron/electron/releases/tag/v43.3.0)
- [Electron process sandboxing](https://www.electronjs.org/docs/latest/tutorial/sandbox)
- [Electron webContents.debugger](https://www.electronjs.org/docs/latest/api/debugger)
- [Electron session（setProxy / webRequest / permission handlers）](https://www.electronjs.org/docs/latest/api/session)
- [Chromium Linux sandboxing（layer-1 / layer-2）](https://chromium.googlesource.com/chromium/src/+/main/docs/linux/sandboxing.md)

### 参考源（真源第 4 层，§28.5）

- [`grok-bot/README.md`、`PROVENANCE.md`、`NOTICE.md`（本仓，R116：Anysphere Grok Bot 0.18.0 反编译重建的自述）](../grok-bot/README.md)
- [Tailwind CSS standalone CLI v4.3.3](https://github.com/tailwindlabs/tailwindcss/releases/tag/v4.3.3)
- [trunk v0.21.14](https://github.com/trunk-rs/trunk/tree/v0.21.14)
- [leptos_i18n](https://github.com/Baptistemontan/leptos_i18n)
- [Lucide 1.33.0](https://github.com/lucide-icons/lucide/releases/tag/1.33.0)
- [Inter 4.1](https://github.com/rsms/inter/releases/tag/v4.1)

### 论文与评测

- [ReAct](https://arxiv.org/abs/2210.03629)
- [SWE-agent / Agent-Computer Interfaces](https://arxiv.org/abs/2405.15793)
- [AgentDojo](https://arxiv.org/abs/2406.13352)
- [WebArena](https://arxiv.org/abs/2307.13854)
- [Firecracker NSDI 2020（仅作为被排除的高隔离层研究依据）](https://www.usenix.org/conference/nsdi20/presentation/agache)

### 服务、许可与品牌

- [OpenBot MIT License](https://github.com/CopilotKit/openbot/blob/891df72f1827454d8b353d108fe5dd2313b7e30d/LICENSE)
- [CopilotKit Terms of Service](https://www.copilotkit.ai/terms-of-service)
- [OpenAI Brand Guidelines](https://openai.com/brand)
- [xAI Brand Guidelines](https://x.ai/legal/brand-guidelines)

## 27. 最终结论

两份第一真源的核心方向是正确的，但原补充方案只覆盖了几项困难基础设施，不能直接代表完整 OpenBot 重写。经本轮低修订后，正式路线是：

> Tauri 2 + Leptos、Axum、Rust Application Core、PostgreSQL 17、native Rust thread/memory/realtime、Rust built-in Agent、remote AG-UI、RMCP tools client、Rust Policy/Audit/Vault/Supervisor、CrabCode 受权基础设施，以及一个最小化、无业务裁决权的 Electron/Chromium browser engine。

该路线既保留 OpenBot 的完整产品契约，也消除了长期依赖 TypeScript 控制面、CopilotKit Intelligence 真源、跨用户 profile、MCP 过度实现、双数据库和多 driver 的结构性风险。按第 24 节逐闸门验收后，可以达到本项目约定的真正“全量 Rust”。

## 28. 前置审计修订记录（第二轮 2026-08-22；第三轮 2026-08-28 见 §28.5）

审计方式：本机独立克隆 `CopilotKit/openbot`（`git rev-parse main` = `891df72f…`，与 §1.2 钉死的 commit 逐字符相同，钉死之后上游零提交）、`acosmi/OpenBot`（只有 README 与本文件）、本机 CrabCode（`98f971bcf…` 存在）；每条断言亲自 `grep` / `read` / `gh api` 复核，未跑过的命令不写进本节。v2 → v3 只改本文件，不触碰任何输入文档。

### 28.1 修订清单（按严重度）

| # | 位置 | v2 表述 | 问题 | v3 修订 | 证据 |
| --- | --- | --- | --- | --- | --- |
| R1 | §7.2 | "默认 run absolute deadline 30 分钟"与"保持当前行为"并列 | 固定 commit 没有任何 run 级绝对期限；唯一 30 分钟常量是浏览器空闲驱逐 `COMPUTER_BROWSER_IDLE_MS`；真正的 parity 是 `AGENT_STALL_TIMEOUT_MS`（默认关） | 拆成三行表：8 步（parity）/ stall 看门狗（parity，原名原义）/ 绝对期限（**新增**，`OPENBOT_RUN_DEADLINE_MS`） | `grep -rn '30 \* 60' --include=*.ts .` 只命中 `agent-computer/src/profiles.ts`；`config.ts::agentStallTimeoutMs` 未设返回 0 |
| R2 | §2.4 / §6.5 / §3.2 | `allowed_groups` 只规定"无 mapping 则拒包" | 漏了三件事：#82 已于 08-21 关闭（上游改文档不改行为）；包 channel 在当前产品对**所有人**不可达（含单用户管理员）；随包示例用 `allowed_groups: [all]`，按 v2 规则会把官方示例包拒在门外 | 保留字 `all` / 具名组 / 空列表三档 + 单用户模式语义写死；§22 加关联方行 | `synchronizeTenantPackage` 只写 `channels` 与 `channel_agents`，全仓 `channelMemberships` 的 insert 只有 `channels/routes.ts` 创建路径一处；`examples/fintech/channels.yaml` |
| R3 | §7.3 | 三家 provider 无 parity 依据，与 §0.4 "新模型专用集成不得挤入"自相矛盾 | 包 Bot 被钉死 `model.provider must be openai`；但管理 Bot `agent-langgraph` 按 `BOT_PROVIDER=openai\|anthropic\|google` 选 provider，所以三家确是 parity | 补上依据，并把 provider 选择位置按上游分两层写死；参考 Agent 保持 OpenAI 单协议 | `tenant-package.ts:349`；`agent-langgraph/src/index.ts:59–119`；`.env.example` 118–150 |
| R4 | §8.3 / §1.2 | "固定版 `cel-rust`" | `cel-rust` 是仓库名，crates.io 没有这个 crate；同仓有新名 `cel`（0.14.3，2026-08-15 更新）与停更旧名 `cel-interpreter`（0.10.0）。另：`cel-js@0.8.2` 没有字符串方法，上游靠两个注入的全局函数工作，方法形式规则在上游是"求值出错" | 钉 `cel = 0.14.3`；写死两个全局函数的签名与大小写语义；preflight 对每条规则比对**结果类别**（true/false/error），变化即高亮待确认 | `cargo search`、crates.io API；`server/src/computer/policy.ts:166–197` |
| R5 | §8.6 | audit 表分区 + 周期 checkpoint **必须**签名写入不可变对象存储 | 过度设计：既有表改分区 = 建新表搬行换名，违反 §14.3；上游触发器（`0007`/`0012`）已给出 append-only + 窗口内删除的同等保证；"不可变对象存储"是一项新的上线前置基础设施，落到运维头上 | 表语义保持上游；`AUDIT_RETENTION_DAYS` 原名原义；hash chain 以追加 nullable 列落地；外部 sink 可选；分区化列为 GA 后变更 | `server/drizzle/0012_*.sql`；`audit-retention.ts` |
| R6 | §11.2 / §11.3 / §12.5 / §12.6 | `BrowserOperation` 含 download/artifact；upload 规则；输入 union 含 IME composition 与 drag；JPEG quality 65 | 上游 29 条手写路径没有下载/上传/文件选择/对话框处理（`page.on` / `setInputFiles` / `filechooser` 零命中）；`/stream` 输入 union 只有 mouse/wheel/key/text；screencast quality 是 70 | R1 enum 与上游一一对应；下载默认取消并上报；不设 IME/drag 变体；quality 70，fps 上限标为新增 | `agent-computer/src/screencast.ts:136–142`；`grep -rlE 'page\.on\(\|setInputFiles\|filechooser' agent-computer/src` 为空 |
| R7 | §3.1 / §4.3 / §18 | memory 三类含 "operational checkpoint"；memory 页未标注为新增 | 固定 commit 的 `app/src` 与 `server/src` 没有任何 memory UI/API（只有注释提到 Intelligence 持有它），memory 是替代面不是 parity 项；checkpoint 属于 run journal | 只保留两类、写入只剩两个显式入口、无后台抽取 job；route ledger 记 31 + 1；§22 加"依赖隐式记忆的用户"行 | `grep -rn -i '\bmemor' app/src server/src` 无生产代码命中 |
| R8 | §3.3 | 沙箱组件"通过 MessageChannel 返回渲染帧和交互事件"，未写运行时契约 | 上游沙箱脚本只拿到 `window.__args` 与自己的 DOM，没有 data function、网络或回调通道；Desktop 独立 renderer 的 a11y 代价未披露 | 契约写死为上游现状；Desktop renderer 两条后果（同一 engine 类；a11y 豁免）写死 | `app/src/lib/copilot/sandboxed-tools.tsx:110`；`admin/playground.tsx:42` |
| R9 | §3.4 / §7.1 / §15.4 | 未提 deployment-wide `AGENT_TOOL_TOKEN`、managed Bot 插槽（`MANAGED_AGENT_*`）、`unavailable` tombstone 类型 | 三者都是固定 commit 的生产行为，直接影响外部 Bot 运营方与"盒内 Bot" | 共享 token 路径删除 + preflight 清单；managed 插槽默认 built-in、变量保留为可选覆盖；tombstone 作为第三终态 | `agents/callback-token.ts:164–190`；`docs/configuration.md:30–35`；`copilot.ts:39–70` |
| R10 | §16.4 / §15.4 / §22 | 未提上游 runtime 自带的使用分析外发 | `@copilotkit/runtime` 依赖 `@segment/analytics-node` 与 `@scarf/scarf`，`OPENBOT_ACCESSIBILITY_DISABLED` 只为关它；删 runtime 后该变量无事可控，合规披露需同步 | 写死"零 phone-home"，变量 remove，subprocessor 清单更新 | `bun.lock` 中 `@copilotkit/runtime@1.68.3` 的 dependencies；`config.ts:566–569` |
| R11 | §6.3 | cookie 无条件 `Secure` | 上游支持非 loopback plain HTTP（文档只建议 TLS；CHANGELOG 专门修过 plain HTTP 发不出消息），无条件 `Secure` 等于让这类部署无法登录 | `Secure` iff `https`；readiness 标 `insecure_transport`；GUI 禁依赖 secure-context-only API | `docs/deployment.md:87–90`；`CHANGELOG.md:448–452` |
| R12 | §14.1 | "现有数据库中的 `vector` extension 原样保留" | 前提不成立：上游 `0010` 已 `DROP EXTENSION IF EXISTS "vector"`，迁到 `0012` 的库通常已没有它 | 改为"extension 零操作"；Server 镜像改平装 `postgres:17` | `server/drizzle/0010_drop_the_document_index.sql:26`；`docker-compose.yml:3` |
| R13 | §9.1 | 四个上限一并写成固定值；计数单位、超时未提 | 只有 20,000 是 parity（`MAX_RESULT_CHARS`），且上游按 UTF-16 code unit 数；三个超时常量未保留 | 标明 parity/新增；计数单位差异显式化；15 s / 60 s / 10 s 原名原值保留 | `plugins/mcp.ts:19–29`；`plugins/store.ts:248` |
| R14 | §20.3 / §15.4 | thread id 与 `DEPLOYMENT_ID` 的关系未写 | 上游 thread id 是 UUIDv8 + 6 字节 `SHA-256(DEPLOYMENT_ID)` 指纹，导入的 `owns()` 判定与后续铸造都依赖它 | 导入规则与铸造布局写死；`DEPLOYMENT_ID` preserve 且 preflight 校验一致 | `channels/thread-identity.ts` |
| R15 | §1.2 / §1.3 | oracle 运行时版本缺失；route 计数无复算口径；Leptos "稳定版" 已不是最新 | fixture/golden 需要锁版本；95 需要命令才能复现；0.8.20 已发布 | 加 oracle 版本表；写明 95 的口径与排除项；Leptos 注明 0.8.20 存在但不升 | §28.4 |
| R16 | §1.1 | 两份输入文档只有 SHA-256 | 仓内不存在原件，无法被任何人复核 | 要求 Phase 0 归档到 `docs/inputs/`；本文件为仓内唯一真源 | `gh api repos/acosmi/OpenBot/git/trees/main?recursive=1` 只有 2 个 blob |
| R17 | §8.3 | 多 replica 下 policy 传播未写 | 上游 `policy-listener.ts` 以 LISTEN/NOTIFY 唤醒 + 整表重读解决"规则只在 N 分之一 replica 生效"的已知事故 | 形态写死，`policy_version` 进每个 decision | `computer/policy-listener.ts`；`CHANGELOG.md:319` |
| R18 | §3.2 | routing 失败语义只写"失败用默认" | 上游 router 还在置信度低于阈值、id 不在 roster、JSON 不可解析时落默认，且 routing 发生在 channel 创建时一次性钉定 | 四种落默认情形与钉定时机写死 | `routing/classify.ts:1–46, 124–152` |
| R19 | §15.4（新增） | 环境变量处置全部推给 Phase 0 | 影响关联方的 remove/rename（Intelligence 四项、Better Auth 两项、`NODE_ENV`、`AGENT_TOOL_TOKEN`、遥测开关）不能等 Phase 0 | 新增处置表；`NODE_ENV` → `OPENBOT_ENV` 并钉示例 key 的拒绝语义 | `config.ts` 读 32 个变量；`docs/configuration.md` 记 48 个 |
| R20 | §0.1 / §1.2 / §3.1 / §13.1 / §18 / §19.2 / §19.3 / §24 G6 / §26（2026-08-22 第二次修订） | GUI 只有旅程与架构；视觉 / token / 字体 / 主题 / 响应式 / i18n / a11y 零定义（`样式` / `Tailwind` / `设计系统` / `字体` / `深色` / `响应式` / `i18n` 全文 0 次）；G6 的 "visual parity" 没有判据 | 上游 UI 栈实测：Tailwind `^4.3.3` + shadcn `base-nova`（`@base-ui/react`）+ 21 原语 + 45 业务组件 + 47 Tabler 图标 + 6 个运行时 JS 库（base-ui / motion / streamdown / prompt-area / boring-avatars / tw-animate-css）+ 手动两态主题不跟随系统 + 0 i18n 框架；v3 对它们既无 parity 裁决也无替代方案，且 `leptos_router` 最新版要求 `leptos ^0.8.20` 与钉死的 0.8.19 冲突 | 用户裁决：自有设计系统 / Tailwind v4 standalone CLI 零 Node / 中英双语带 i18n；新增 `docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md` 作为视觉真源；G6 的 visual 改为 golden 截图的可判定定义；§1.2 补 GUI 工具链、生态（`leptos_router =0.8.13`）与资产钉版；§19.3 补 `parity/ui.yaml` 等产物 | 设计系统文档 §1 / §17（上游计数命令）；`app/components.json`；`app/src/styles.css`；crates.io `leptos_router` 0.8.13–0.8.15 的依赖声明 |
| R21 | §1.2（2026-08-22 第三次修订） | Rust 工具链 `1.94.1`（2026-03-25 发布） | 该 pin 是从 CrabCode 基础设施继承的默认值，v3 正文没有为它写过任何理由；而 `1.98.0` 早在 v3 定稿前三天（2026-08-18）就已发布。更关键的是它**正在卡工具**：Phase 0 的 AST 级 test inventory 需要 `oxc_parser`，`cargo build` 在 1.94.1 上直接拒绝 `oxc_parser@0.146.0 requires rustc 1.95.0`，被迫退到 MSRV 台阶上的 `0.139.0` | 升 `1.98.0`；全 workspace 实跑回归，迁移影响 = 1 处新 clippy lint（`collapsible_match`）；oxc 随之升 `0.146.0`，两版 `xtask test-inventory` 产物剔除 parser 版本字段后**逐字节相同** | `rustup check` 实测 `1.94.1 -> 1.98.0 (88d9e12ae 2026-08-18)`；delta audit 全文见 `docs/2026-08-22-Rust工具链1.94.1升1.98.0-delta审计.md` |
| R22 | §2.4（新增行）/ §6.5 / §14.2（2026-08-22 G1 实施轮） | §2.4 收录了 7 条"不得照译"的上游缺陷，未收录 channel 可见性这一条 | 上游 `channels/routes.ts` 的 `list` 分两段查：分页段只 `innerJoin(channelMemberships)`，hydration 段额外 `innerJoin(intelligenceChannelMappings)`。两段判据不一致 ⇒ `nextCursor` 非空而本页可以为空。且全仓 `insert(channelMemberships)` 只有创建路径一处、`tenant-package.ts` 对两张表都零引用，所以 §6.5 给包 channel 补 membership 之后，它们仍会被那个 join 过滤掉 —— Intelligence 已退役，不会再有 mapping 行 | §2.4 加一行写死：运行时可见性只查 materialized membership；`intelligence_channel_mappings` 只读且不进任何可见性判据；分页与 hydration 共用同一判据 | `grep -c "channelMemberships\|intelligenceChannelMappings" server/src/tenant-package.ts` = 0，正向对照同文件 `grep -c "insert("` = 5（实际写 agentProfiles / agentTable / channelAgents / channelTable / deploymentPackages）；`grep -rn "insert(channelMemberships)"` 全仓仅 `channels/routes.ts` 一处 |
| R23 | §5.3（2026-08-22 G1 实施轮） | "所有 ID 是 string newtype"，十五个名字里含 `ComputerGeneration` 与 `DocumentGeneration` | 与本文件自身冲突：§11.2 的 `EngineCommand` 与 §12.3 的 `FrameHeader` 两个结构体都把 generation 写作 `pub generation: u64`。而且"旧 generation 失效"这条不变量依赖数值序，字符串的字典序会给出错误答案（`"10" < "9"`） | 裁决 D7：`ComputerGeneration` / `DocumentGeneration` 落 `u64` newtype 并派生 `Ord`，其余 13 个仍是 `String` newtype。§5.3 那条 string 规则给出的理由是"兼容端必须接受上游既有字符串"，而 generation 不是上游提供的标识符，是本系统自己单调递增铸造的，该理由不适用 | 本文件 §11.2 / §12.3 的 Rust 代码块原文 |
| R24 | §14.1 / §24 G1（2026-08-22 G1 实施轮） | "升级前先要求旧 OpenBot 把数据库迁到当前第 13 条 migration（`0012`）；Rust 不接收更早 schema" —— 未说明这条怎么判定 | 纯 schema 判定有结构性盲区：`0003_backfill_account_issuer.sql` 是 13 条里唯一的纯数据迁移，跑没跑过在 schema 上不留痕迹。**本轮第一次给出的判据（`accounts.issuer` 存在 NULL 即判红）是错的**，由实施子代理指出并经复核推翻：上游 `core.ts::accounts` 的 `issuer` 注释写明该列"deliberately"可空，滚动发布期旧 replica 会插入不带该列的行，所以 NULL 在完整迁移过的库上合法，fail-closed 会拒绝健康的库 | §14.1 改写：判据换成读迁移账本 `drizzle.__drizzle_migrations`（条目数 ≥ 13），账本缺失时如实报告"无法验证"而非猜测或默认通过；并记下 `0006` 的 `DROP NOT NULL` 是 no-op | `grep -n "issuer" server/drizzle/*.sql` 显示 `0002` 以 `ADD COLUMN "issuer" text`（无 NOT NULL）加入、`0006` 有一句 `DROP NOT NULL`、全仓 `SET NOT NULL` 零命中；`core.ts::accounts` 的 issuer 注释原文；`server/package.json::db:migrate` 用 `drizzle-kit migrate`，`server/drizzle/meta/_journal.json` 有 13 条 tag |
| R25 | §16.3 / §19.3 的闸门驱动器（2026-08-22 G1 实施轮） | `cargo xtask ci` 被定为"按 §16.3 顺序跑本机可执行的闸门段"的单一入口，驱动器是 workspace 自己的 bin target | 这条闸门在 Windows 上**构造性地永远跑不绿**：第 3 步 `cargo test --workspace --all-features` 会重新链接驱动器自己（xtask 的 `required-features = ["xtask"]` 恰好被 `--all-features` 满足），而 Windows 不允许删除正在运行的 exe，cargo 的 uplift 报 `failed to remove file target/debug/xtask.exe: 拒绝访问 (os error 5)`；同一条命令在 Linux 上恒绿 —— 答案取决于跑在哪台机器上的命令不是闸门。本轮先试的"把自己复制到临时目录再 re-exec"**无效并已撤回**：占住那个文件的是父进程自己，复制出一个子进程不释放父进程的镜像锁（实测第 3 步照旧 os error 5） | 结构性解法：驱动器与被驱动的构建落在两棵互不包含的 target 树。`.cargo/config.toml` 的 alias 加 `--target-dir target-xtask`，`cmd_ci` 再把子进程显式钉回 `<root>/target`（不靠继承）；不变量本身由 `xtask.rs::driver_conflicts_with_child_target` 承担，摆放错了当场拒跑并打印两条路径 | 修前实跑 `cargo xtask ci` 第 3 步 `error: failed to remove file D:\OpenBot\target\debug\xtask.exe / 拒绝访问 (os error 5)`；修后同机 `cargo xtask ci` = `5/5 全绿（recount 143 条全部实跑）`；负向对照 `cargo run -p openbot-testkit --features xtask --bin xtask -- ci` 被守卫当场拒跑并打印驱动器与子构建两条路径；单测 `driver_inside_child_target_is_detected`（正向）/ `driver_in_a_sibling_tree_is_not_a_conflict`（负向）/ `missing_child_target_is_not_a_conflict`（干净 checkout） |
| R26 | §8.3 / §8.6（2026-08-22 G2 实施轮） | CEL 求值的失败只被当成"结果类别 error"，没有规定失败**怎么被记录** | `cel 0.14.3` 的 `ExecutionError` 有若干变体把参与运算的 `Value` 放进错误本体，`Display` 逐字打出来。实测：context 里 `page.url = "https://example.com/order?token=SECRET123"`，求值 `page.url + 1` 得到 `Unsupported binary operator 'add': String("https://example.com/order?token=SECRET123"), Int(1)`。这与 corpus 记录的 F-CEL-6（cel-js 把整份 context 拼进 `Identifier not found` 消息、上游 `policy.ts::matches` 用 `console.error(String(error))` 原样打日志）是**同一族缺陷**，只是换了引擎；而 §8.6 要求审计 payload 走字段 allowlist | §8.3 补一条构造性约束：CEL 失败在离开求值器的那一刻压成一组**无载荷**的分类（`openbot_domain::policy::cel::failure::CelFailure`），`ExecutionError` 的 `Display` 不得出现在日志 / 审计 / 错误响应 / GUI。分类靠**匹配变体**而不是读消息文本；表达式原文可以带（它是管理员写的规则，不是被检查对象的数据），context 取值一律不带 | 泄漏样例见左；**正向对照**同一次测量里 `page.url < 1` 落 `NoSuchOverload`，输出只有 `No such overload` —— 说明泄漏是变体相关的，"小心别打印错误"这种纪律型防线必然漏掉恰好带值的那几个变体 |
| R27 | §8.3（2026-08-22 G2 实施轮） | 只规定了 CEL 的求值语义，没有规定**解析器的资源边界** | `cel 0.14.3` 的语法分析是 antlr4rust 递归下降，栈消耗随括号嵌套线性增长；而 Rust 的栈溢出是 **abort 不是可捕获的 panic**。策略表达式来自管理员可写的 `action_policy.deny` / `.allow`，于是"一条写歪的规则打死进程"是一条真实路径。更糟的是崩溃点取决于线程栈大小 —— 同一条表达式在不同线程上一个崩一个不崩，正是本文件反复判定为"不是闸门"的那种形态 | §8.3 补两条，缺一不可：① 解析**之前**做一次非递归线性扫描，拒绝超过 `MAX_EXPRESSION_BYTES = 4096` 或括号嵌套超过 `MAX_EXPRESSION_DEPTH = 8` 的表达式（扫描认字符串字面量，不误伤 `contains(url, "((((")` 这类规则）；② 解析放在**求值器自己拉起的、栈大小写死的线程**（`PARSER_STACK_BYTES = 16 MiB`）上并立即 join，把"调用者此刻还剩多少栈"移出等式。求值不需要同样待遇（实测见右） | 本机 debug 构建、表达式 `"(".repeat(n) + "true" + ")".repeat(n)` 逐档二分，第一个崩溃深度：~1 MiB（Windows 主线程）**6** / 2 MiB 12 / 4 MiB 28 / 8 MiB 64 / 64 MiB 不崩（深度 ≥ 100 时解析器自己报语法错误）。**正向对照**：64 MiB 线程上编译出的深度 64 AST 拿回 ~1 MiB 主线程执行，正常返回 `Bool(true)` —— 爆栈的是解析不是求值。corpus 69 条表达式实测最大嵌套深度 2、最长 143 字节，上限 8 / 4096 分别留了 4 倍与 28 倍余量 |
| R28 | §1.2 / §8.3（2026-08-22 G2 实施轮） | 只钉了 `cel = 0.14.3`，没有裁决它的 feature | `cel` 的默认 feature 是 `["regex", "chrono"]`，其中 `regex` 打开它自己那份**大小写敏感**的 `matches` 字符串方法。开着它，`element.name.matches("sub.*")` 求值成 `false`，而 oracle（`cel-js@0.8.2`）在同一条上抛 `Unknown method: matches` = `error` —— 多出一条要管理员在迁移 preflight 里逐条确认的语义翻转 | 钉 `default-features = false`。关掉之后同一条落到本项目注册的全局 `matches`，以方法形式调用时只收到 1 个实参，报 `Invalid argument count: expected 2, got 1` = `error`，与 oracle **同类**。§8.3 原文允许"标准 CEL 方法形式作为超集存在"，那是**允许**不是**要求**，所以按 parity 取窄的那侧 | 69 条 corpus 逐条实跑对照：开 `regex` 分歧 **7** 条，关掉 **6** 条（差的那条就是 `method-form-matches`）。剩下 6 条全部由 corpus 已记录的 F-CEL-1（方法形式，3 条）与 F-CEL-3（`&&` / `\|\|` 的交换律吸收，3 条）解释，写成 `tests/cel_corpus_parity.rs` 里的封闭台账，多一条少一条都判红。附带：`cargo tree -e normal` 实测依赖图 67 → 41 个 crate，chrono 一并出图 |
| R29 | §6.2 / §16.3（2026-08-22 G2 实施轮） | §6.2 钉 `samael 0.0.22` 做 SAML、并要求 OIDC discovery/JWKS 走"和 remote Agent/MCP 相同的 safe dialer"；§16.3 同时要求可复现构建且把 build.rs / FFI 单列审计 | 两条要求在 **TLS 与 XML 签名这一层相撞**，v3 没有记这次相撞：① `samael 0.0.22` 经 `openssl-sys 0.9.117` 要求一份 OpenSSL 安装，本机 MSVC 目标探测不到（`$HOST = $TARGET = x86_64-pc-windows-msvc`），且它另需 libxml2 + xmlsec —— 一整套非 Rust 工具链；② safe dialer 需要 TLS，而 `rustls` 的两个后端 `aws-lc-rs` / `ring` 都带 C / 汇编构建脚本，`native-tls` 在 Linux 上落回 OpenSSL。任何一条都会把 C 工具链拖进发行物的构建面，属于**独立的、要 delta audit 的决定**，不能作为"加一个库"的副作用悄悄发生 | G2 本轮按边界交付、不假装做了：OIDC 的协议实现落 `openbot-infra::auth::oidc` 并以 `default-features = false` 关掉 `openidconnect` 自带的 `reqwest` + `rustls-tls`，出网由调用方注入 `oauth2::AsyncHttpClient`（该 trait 对 `Fn(HttpRequest) -> F` 有 blanket impl，测试用确定性假实现驱动全流程）—— 于是"绕过 safe dialer 的出网路径"在构造上长不出来。SAML 的**校验规则**（签名覆盖 / Destination / Audience / Recipient / `InResponseTo` / 时间窗 / assertion replay / 拒 SHA-1 与外部实体）落领域层，XML 签名验证的落地与 TLS 后端选型各自另立 delta audit；在此之前不得宣称 §24 G2 的"OIDC/SAML 全矩阵"闭合 | `cargo build` 实测 `samael 0.0.22` 失败于 `openssl-sys 0.9.117` 的 "could not find directory of OpenSSL installation"；本机 `command -v nasm` 无、perl 是 msys 版（vendored 构建同样走不通）；`openidconnect 4.0.1` 的 `[features]` 段 `default = ["reqwest", "rustls-tls"]`；`AsyncHttpClient` 定义在 `oauth2-5.0.0/src/endpoint.rs` |
| R30 | §6.2 条 6 / §6.5（2026-08-22 G2 实施轮） | email 规范化只写"按上游 `trim().toLowerCase()`" | **`trim()` 在两种语言里不是同一个函数。** JS 的 `String.prototype.trim` 去的是 WhiteSpace ∪ LineTerminator，而 ECMAScript 的 WhiteSpace **包含 U+FEFF**（`<ZWNBSP>`）；Rust 的 `str::trim` 按 Unicode `White_Space` 属性去空白，U+FEFF 的类别是 `Cf` 不在其中。后果不是学术性的：一个带 BOM 保存的 `.env` 会让 `INITIAL_ADMIN_EMAILS` 的**第一项**以 U+FEFF 开头，于是 admin floor 上的第一个人**永远匹配不上** —— 而 floor 正是上游设计里"最后一个管理员误降权之后回来的那条路"，它静默失效意味着锁死时没有回来的路，且没有任何报错 | `NormalizedEmail` 的唯一构造入口显式把 U+FEFF 一并当首尾空白去掉（`is_boundary_whitespace` = Unicode `White_Space` ∪ U+FEFF），并配负向对照测试证明**裸的 `str::trim` 去不掉它**。同轮另一处不照译：上游 `auth/signed-value.ts::sign("")` 产出的串被它自己的 `verify` 判无效，Rust 版在**签发侧**就拒绝空值而不是产出一个注定验不过的串 | `node -e '"\uFEFFa@b.com".trim() === "a@b.com"'` → `true`（本轮实测，node v20.19.6）；Rust 侧 `'\u{FEFF}'.is_whitespace()` → `false`，负向对照测试 `rust_trim_alone_does_not_remove_the_bom`；`sign("")` 一条用 node 跑上游原码实测得 `".199bct…"` 而同一份 `verify` 对它返回 `null` |
| R31 | §6.3（2026-08-22 G2 实施轮） | "短 idle + 绝对期限"、"敏感 admin 写操作要求 fresh session" —— 三个时长一个数字都没有 | **上游给不出参照**：`server/src/auth/index.ts` 传给 `betterAuth({...})` 的选项里一个 `session` 配置都没有（该文件里唯一的 `session:` 在 `databaseHooks` 下），跑的是 `better-auth 1.7.1` 的库默认值；而本机没有 `node_modules`、不能联网，那三个默认值**本轮无法测量**。按本文件的纪律，未测量的值不得写成"当前行为" | 三个时长落成 `openbot-infra::auth::AuthConfig` 的具名常量并标**新增**：idle **8 小时**（覆盖一个完整工作日不用重登，同时把没锁屏的浏览器限制在一个工作日内）/ absolute **7 天**（与活跃度无关的硬顶，让凭据泄漏的可用窗口有上界）/ sensitive-write freshness **15 分钟**（够做完一批管理操作，短到走开的会话不能用来做高权限写）。领域层**不持有**这三个数字：`openbot_domain::identity::session::SessionLifetimePolicy` 只在构造期校验 `fresh < idle ≤ absolute`（`fresh >= idle` 会让敏感写闸门恒真，这条被做成构造期拒绝）。三者暂不引入环境变量 —— 那是一次 §15.4 修订 | `grep -n "session:" server/src/auth/index.ts` 唯一命中在 `databaseHooks` 段内；`bun.lock` 记 `better-auth@1.7.1`；上游仓无 `node_modules` |
| R32 | §6.2 条 7（2026-08-22 G2 实施轮） | "普通管理员不能撤销自己；**最后一个有效管理员不能被删除**"，与条 6 的 admin floor 并列陈述，读起来像 parity | 前半句是 parity，**后半句上游没有实现**：`server/src/app.ts` 恰有 4 处 409 —— 配置管理员不可降权、配置管理员不可撤销、不可自我降权、不可自我撤销；全 `server/src` 找不到任何"数一数还剩几个管理员"的逻辑。上游是**刻意**用 floor 代替计数（`auth/roles.ts::isConfiguredAdmin` 的注释把 `INITIAL_ADMIN_EMAILS` 说成"最后一个管理员误把自己降权之后回来的那条路"）。按 CLAUDE.md §4 与 §28.1 R1，把新增写成"当前行为"是最重的一类错误 | 条 7 后半句改标**新增**；实现落 `RoleChangeRejection::LastAdmin` / `AccessChangeRejection::LastAdmin`，文档里写清 floor 盖不住的两种处境（两个管理员并发互降 —— 自我检查看的是身份、计数看的是剩余数量；以及 floor 名单上的人可能已离职） | `grep -rniE "last admin\|only admin\|yourself\|self-demot\|own role" server/src --include=*.ts \| grep -v '\.test\.'` 只命中 `app.ts` 那两条自我保护与两条注释；**正向对照**同一批文件 `grep -rn requireAdmin` 命中 38 |
| R33 | §15.4 / §6.4（2026-08-22 G2 实施轮） | `KEY_ENCRYPTION_KEY` 的处置写作「preserve … base64 **32 字节**」 | 这个长度约束**没有上游依据，而且会锁死数据**。上游 `server/src/credentials.ts::aesKey` 把 base64 解出的字节原样 `crypto.subtle.importKey("raw", …, {name:"AES-GCM"})`，WebCrypto 因此接受 **AES-128/192/256 三种长度**。现网完全可能有一个用 16 或 24 字节 KEK 跑了很久的部署；Rust 侧若在启动时硬要求恰好 32 字节，那个部署起不来 ⇒ 它自己的 `credentials.encrypted_value` **永远迁不出来**。「更严」在这里等于数据丢失。同一轮还推翻了一个相关的实现前提：`aes-gcm 0.10.3` 虽只导出 `Aes128Gcm` / `Aes256Gcm` 两个别名，但它 `pub use aes;`，`AesGcm<Aes192, U12>` 可以正常构造，所以 192 位并非"Rust 侧做不到" | §15.4 该行改为：base64 解出长度必须 ∈ **{16, 24, 32}**，其余（含 0 / 20 / 31 / 33）拒绝启动，**不许截断、补零或就近取整**；16 与 24 允许启动但发结构化告警建议轮换到 32（轮换路径就是 §6.4 的 v1→v2 迁移）。示例值在生产拒绝这条不变 | `node` 实测 `crypto.subtle.importKey("raw", <n 字节>, {name:"AES-GCM"})` + 一次真实 encrypt/decrypt 往返：16 / 24 / 32 全部 OK 且明文逐字节回来；**正向对照** 20 与 31 报 `DataError: Invalid key length` —— 证明该探针会说「不」，不是恒真。本轮由实施子代理与主控**各独立跑一次**，结论一致 |
| R34 | §15.4 / §19.3（2026-08-22 G2 实施轮） | 环境变量处置表与 Phase 0 的 `parity/env.yaml`（70 条）都没有 `OPENBOT_DEV_NO_AUTH` | **它是活的读取点，不是死变量。** `server/src/auth/dev-actor.ts::singleUserEnabled` 现在就在读它，`server/tests/single-user.test.ts` 有对应用例，`CHANGELOG.md` 也提到它。之所以在 Phase 0 被漏掉，是因为 `parity/env.yaml` 的复算命令里那条覆盖 server 的扫描只扫 `server/src/config.ts`（本表 §1.3 口径），而这个变量的读取点在 `auth/dev-actor.ts` —— **一条按文件收敛的复算命令，天然照不到落在别处的读取点**，这是本轮值得记下的方法论缺口，不只是漏了一个名字 | `parity/env.yaml` 补第 **71** 条（`openbot-dev-no-auth`），三处 recount 期望值同步改为 entries 71 / rename 7 / 替代 14；§15.4 处置表补一行。裁决为 **rename → `OPENBOT_SINGLE_USER`** 而不是 remove：「不认它」会让一个靠它跑单用户模式的部署以「没有 IdP 也没有单用户旗标」这个**看起来与它无关**的理由启动失败，操作员不会想到是变量改了名 —— 给它一条自己的稳定 code 才能把人指到新名字 | `git grep -n "OPENBOT_DEV_NO_AUTH" -- server/src server/tests CHANGELOG.md` 三处命中；`grep -c` 于 `parity/env.yaml` 补入前为 **0**；**正向对照**同一条命令对 `OPENBOT_SINGLE_USER` 命中 **20** 处，证明该扫描面确实覆盖得到这类变量；补入后 `cargo xtask parity-check` = 通过（0 违反），`grep -c '^  - id: ' parity/env.yaml` = 71 |
| R35 | §14.1 / §14.3（2026-08-23 W-1 实施轮） | 只裁决了“fresh baseline 到 0012”与“旧库先到 0012”，没有规定 0012 之后 Rust-owned migration 的施加路径、并发与记账 | 若复用 `drizzle.__drizzle_migrations`，本项目会篡改只读上游证据；若只靠 `IF NOT EXISTS`，对象同名异形或半次施加会被静默伪装成成功；若没有全局锁，多 replica 同启会竞争 DDL | 补入上文固定形态：`db::native` + `openbot_internal.schema_migrations` 自有账本；version/name/SHA-256 三向绑定；transaction advisory lock；DDL 与记账单事务；真实 migration 无 `IF NOT EXISTS`；无 downgrade。0013 只追加 audit 两个 nullable text hash 列，并新建 audit_checkpoints/tool_calls/tool_attempts；`runs` 尚未存在，tool_calls.run_id 先落列，FK 到 G3 建 runs 时再以 expand-only 添加，不越界提前建 G3 表 | PostgreSQL 17.11 真库：`native_0013_is_idempotent_and_concurrent_callers_serialize` 得恰好 `Applied + AlreadyApplied` 且账本 1 行；`object_collision_rolls_back_every_0013_change_and_does_not_forge_ledger` 制造同名异形表后命中 42P07，audit hash 列/checkpoint/账本均 0 残留；`ledger_drift_is_refused_even_when_all_objects_exist` 命中封闭 LedgerDrift |
| R36 | §14.1 / §19.2 / §24 G1（2026-08-23 W-1 实施轮） | `fixtures/db/schema-0012.json` 在 PostgreSQL 18.1 生成，却把它当 PostgreSQL 17 的 baseline oracle；D-4 只登记“理论上可能不同”，未实跑 | PostgreSQL 17.11 实跑并非空 diff：28 张表都只差约束集合。18.1 fixture 为 `f=27/n=153/p=28/u=4`，17.11 为 `f=27/p=28/u=4`；153 条 `n` 恰好等于 153 个 `columns[].notnull=true`，是 PG18 对同一 NOT NULL 事实的重复暴露，不是 DDL 语义差异 | `schema_facts.sql` 显式排除 `contype='n'`，NOT NULL 仍由逐列 `notnull` 完整验证；fixture 在 PG17.11 重生成，约束数 212→59。去掉旧 fixture 的 153 条 n 后，与 PG17 活库整棵 JSON 结构化相等。另生成 post-0013 fixture（31 表/248 列/93 约束/53 索引/4 触发器），逐对象机械断言 0012 是 0013 子集 | `equal_after_removing_pg18_not_null_constraints True`、`actual_constraints 59 / expected_without_n 59`；`baseline_reproduces_the_reference_schema_exactly` 在 PG17 绿；`post_0013_fixture_is_exact_and_every_0012_object_survives` 绿；完整 infra 真库矩阵 235 passed / 0 failed / 0 ignored |
| R37 | §16.3 / §19.3（2026-08-23 W-1 实施前闸门轮） | 本机闸门被当成跨环境同一答案，但两条 recount 命令依赖 GNU 输出形态，日志格式测试又隐式继承操作者的 `RUST_LOG` | macOS 上 `grep -r -c <单文件>` 输出 `file:count`，BSD `wc -l` 带前导空格；同一上游 commit 因此 2 条失配。宿主 `RUST_LOG=warn` 时，格式测试发 `info!`，缓冲区必为空——生产过滤语义正确，但测试答案取决于 shell | 单文件计数去掉无意义的 `-r`；六库文件数经 `awk printf` 归一成 GNU/BSD 同字节；格式测试通过私有 `subscriber_with_filter` 注入固定 `info`，公开 `subscriber` 仍照常读取 `RUST_LOG`。recount 实际使用 `rg`，移交环境清单显式列为必需工具 | 修前实测：API 判据实得 `server/src/app.ts:1`、UI 判据实得带多段空格、两条 events stderr=`rg: command not found`；安装 ripgrep + 修订后 `cargo xtask recount --require-upstream` = 143/143；同一个 `RUST_LOG=warn` 下单测由稳定失败变为通过；完整 `cargo xtask ci` 5/5 全绿 |
| R38 | §14.2 / §8.1 / §8.6（2026-08-23 W-2 实施轮） | 台账规划了 40 个 `repo=` 落点，移交单却要求 W-2 一次全部实现；其中 10 个对应的 native 表要到 G3 才存在 | 对不存在的表先造同名空 struct 只能让 grep 变绿，既无 SQL 又不可能有真库测试；反过来把 G3 十张表提前塞进 W-2，会越过 §24 阶段依赖并迫使 schema 设计脱离 thread/run 第一条真实用例 | 固定 repository 与物理表同批：0013 当前 schema 先闭合 30 个，剩余 10 个随 G3 表同批；用 `IMPLEMENTED_REPOSITORIES` 与 parity planned 名单做四向 recount（40 planned / 30 implemented / 10 missing / 0 unplanned）。安全敏感面不用 generic 空壳：Vault CAS、Audit 链/checkpoint、Tool 双行事务各有专门 API；legacy mapping 无写方法 | `all_thirty_current_repositories_touch_their_real_tables` 在 PG17 逐个调用 30 个 repo；Tool 重复 attempt 命中 23505 且 call 零残留，receipt 在 commit 后才返回；Audit 倒序/伪 count/链后 unlinked 全部 fail-closed；Vault 错 key CAS=0、正确=1、撤销后 active=None；完整 infra 真库矩阵 241 passed / 0 failed / 0 ignored；严格 recount 147/147 |
| R39 | §6.2 条 10 / §14.1（2026-08-23 W-3 实施轮） | `AuthContext`、Desktop broker 与 domain access plan 都依赖 auth generation，但 0013 及以前没有任何数据库权威落点；若临时放内存，多 replica 重启后会各答各的 | generation 是跨 session/WS/ticket/capability 的共同失效轴，不能猜、不能只存在进程内；另一方面兼容期直接加 `NOT NULL DEFAULT 0` 会把历史未知值伪装成真实代际并收紧旧 writer | native 0014 在 `users` 末尾只追加 nullable bigint 与非负 CHECK；旧 NULL 读 0，真实 role/access 变化同事务 `coalesce+1`；生产 migration 链按顺序施加，历史 0013 fixture 仍经同一执行器固定边界；generation 只留服务端，不加入 people DTO | PostgreSQL 17.11：`post_0014_is_exact_expand_only_and_null_legacy_generation_is_zero_floor` 逐对象证明 0013 是 0014 子集、users 只多 1 个 nullable 列；负数命中具名 CHECK；双 replica 实得 Applied + AlreadyApplied 且账本恰 2 行；fixture = 31/249/94/53/4 |
| R40 | §5.2 / §6.2 条 6–10 / §14.2（2026-08-23 W-3 实施轮） | 固定上游把 people 的查、判、写与 audit 分在 route/store 两层；若 Rust 逐个复刻调用，last-admin 计数与权限写之间存在竞态，audit 写失败也无法回滚已提交的角色/撤权 | 第一真源新增的 last-admin 与 audit-before-action 要求比上游更强的事务边界；同时把内部 generation 塞进 `Person` 会静默新增公开 wire 字段，违反 parity | 增加 `PeopleAdministration` 原子 port 与五个 typed application command；adapter 用 deployment-wide 事务锁串行 role/access，同快照判 floor/self/last-admin，同事务写业务/generation/audit；provider 集合稳定排序，坏 cursor 按上游回第一页；公开 DTO 严格排除 auth generation，并按上游 `Date.toISOString()` 固定 UTC 毫秒。Axum 生产路由仍留 W-4，不在本条提前宣称闭合 | PostgreSQL 17.11 `people_application` 5/5：两管理员并发互降实得 1 success + 1 `RoleLastAdmin`、最终 admin=1；audit invariant 失败后 role/generation 零副作用；撤权 deny/session/generation/audit 同批、恢复不回退 generation；搜索 wildcard/cursor/provider 顺序真库通过。全 workspace 908 passed / 0 failed / 44 ignored；infra 真库 248/0/0 |
| R41 | §5.2 / §8.1 / §8.3 / §17.2 条 2、9（2026-08-23 W-3b 实施轮） | 领域已有十二段类型状态机，infra 已有 call+attempt/outcome repo，但全仓没有 application 调用点，`openbot-agent` 仍为空；另一个实测缺陷是“已成功落库的 `commit_state=unknown`”会走 `Completed` | 只靠“repository 已存在”不能证明 action 前一定调用；executor 若直接接 tool+args，仍能绕 capability。unknown 即使写成功也不代表成功，而是已知需要和解；把它回成 ToolResult 会诱导继续循环 | 新增封闭 `InvokeTool`、`ToolControlPlane` 与 `ToolJournal`；policy 结论只能由 domain 构造；raw args 只随字段私有的 `AuthorizedToolCall` 到 executor，report 必须带 redeemed proof；decision+attempt commit、capability CAS 均在 execute 前；outcome+audit 同事务；unknown 恒 halt。Agent gateway 铸 UUIDv7/per-run seq，actor 只取 AuthContext。生产 executor 仍属 G4，不提前宣称 G4 | application 7 条管线矩阵 + 1 条 approval 细分码：完整顺序、decision/attach 失败执行数 0、outcome 失败只执行 1 次、unknown 两态、enforce/dry-run、人拒绝、scope/malformed；Agent 3 条含 32 并发 sequence=0..31；PG17 `tool_application` 5/5：happy、refusal、audit rollback、unknown、duplicate，完整 infra 253/0/0；workspace 922/0/49 |
| R42 | §6.3 / §14.1 / §17.2 条 6（2026-08-23 W-4 实施轮） | 上游 `sessions.token` 是可直接使用的明文，且 session 行没有签发 generation；若只查当前 user generation，每次请求都会“自动升级”旧 session，角色/撤权变化无法使既有 session 失效 | 第一真源已要求 token 只存 keyed hash、refresh 不沿用旧代际、切换时旧 Better Auth session 全失效；没有 session 自己的代际就无法同时满足三条 | native 0015 追加 nullable `sessions.auth_generation` + 非负 CHECK；旧 NULL 不回填。production resolver 只认 `openbot_session`，HMAC 后查库并逐次校验 session/user generation、deny、role、absolute/idle；重复 cookie/明文旧行/NULL/撤权/过期均 fail-closed。敏感写带 live assurance + trusted Origin，四原因四码 | PG17 native0015 2/2（fixture 31/250/95/53/4、旧 6 行 NULL、负值 CHECK、双副本一次）；server `postgres_auth` 4/4：keyed hash、重复/旧 token/代际变化、deny/过期/缺角色、真实 cookie→HTTP→ApplicationService→generation/audit 竖切 |
| R43 | §5.2 / §6.3 / §16.1 / §16.4（2026-08-23 W-4 实施轮） | 全仓无 `main.rs`，唯一 AuthResolver 是 testkit；`/metrics` public；span/metrics 无 route；people 五条 application command 没有 HTTP 腿 | 没有可运行二进制就无法证明启动/migration/readiness/监听；给 metrics 单独 token 会造第二个认证脑；原始 URL path 进 label 会让对端制造无界 series；把 tenant package 目录路径当包 ID 会永久铸错 deployment/thread 归属 | 新增 `openbot-server` main：单一 DATABASE_URL parser 对不能兑现的 TLS/拓扑选项拒绝而不降级，fresh baseline/legacy boundary/native 链，显式单用户 loopback 或 PostgreSQL session resolver、DB readiness、graceful shutdown；tenant package loader 未落地前要求显式 `DEPLOYMENT_ID`。接 `/api/me` 与 admin status/people 三面，role/access 强制 fresh+Origin；metrics 共用 session。route 只取 Axum MatchedPath，未匹配统一 `unmatched`；非 loopback 明文 HTTP 在 readiness 投影 `insecure_transport:true`。多用户在 G5 isolation 未接前 readiness 刻意红；OIDC/SAML 登录/session 签发仍属 G2，不能冒充闭合 | server 单测含 HTTP framing/sensitive/route/metrics/readiness；DB parser 负例含 `sslmode=require`/read-write/channel-binding/hostaddr；testkit `transport_people_parity` 五命令同一 Arc 对拍；真实进程冒烟：fresh 库账本 3、single principal 1/roles 2，health/readiness/me/metrics 均 200，route label 静态，Ctrl-C exit 0；临时库已删除 |
| R44 | §16.3（2026-08-23 D-1 供应链裁决） | `cargo deny check` / `cargo audit` 唯一命中 RUSTSEC-2023-0071：`rsa 0.9.10` 由钉版 `openidconnect 4.0.1` 非可选引入，且 advisory `patched=[]` | 无说明地忽略是隐藏风险；无限期保留一条必红闸门则会训练所有人忽略红灯；换掉 openidconnect 又会偏离已钉选型 | 仅对该 ID 记一条 owner=security 的窄豁免：本仓只以 `CoreJsonWebKey`/`RsaPublicKey` 做 RP 公钥验签，无 RSA 私签/解密。`tools/check-rustsec-waivers.sh` 先锁死 rsa/openidconnect 精确版本、反向生产链、feature 零扩张和私钥符号零命中；版本/feature/消费者/private-key JWT 或 patched 状态任一变化必须重审 | 本轮 `cargo tree -i rsa -e normal --all-features` 精确四节；guard exit 0 且内置正向对照；`cargo deny check` = advisories/bans/licenses/sources 全 ok；`cargo audit --no-fetch --deny warnings --ignore RUSTSEC-2023-0071` 加载 1225 条 advisory、扫描 374 个 crate 依赖、exit 0 |
| R45 | §16.3（2026-08-23 D-5 供应链棘轮） | CI 固定要求 `cargo vet`，但本机无工具、仓内无 policy/audits/import lock；直接接线会让全部依赖恒红 | 一条恒红闸门等于没有闸门；反过来把 `init` 生成的 exemptions 称为“已审计”也是造假。正确起点是显式标记存量未审，但对任何增量立即判红 | 钉 `cargo-vet 0.10.0`；提交 `supply-chain/config.toml` / `audits.toml` / `imports.lock`。只导入 Google exact/delta audits 并锁定 14 个 fully audited；350 个精确版本保留 bootstrap exemption 且文档明说“不是审计结论”。CI 仅跑 `cargo vet --locked`、不自动 regenerate；checkout/rust-cache/install-action 也收窄到已实查 patch tag | `cargo vet --version` = 0.10.0；正向 `cargo vet --locked` = `14 fully audited, 350 exempted`；负向临时删除 `aead 0.5.2` exemption 实得 exit 255 / `Vetting Failed` / 精确 missing `safe-to-deploy`；exemption 数可由一条 tomllib 命令复算 |
| R46 | §6.4（2026-08-23 D-3 密钥擦除裁决） | `SecretBytes::drop` 只是 `Vec::fill(0) + compiler_fence`，模块文档自身也诚实标为“尽力”；`zeroize 1.9.0` 已经由既有密码学树锁定 | 手写普通写没有“不被优化掉”的稳定保证；但把 zeroize 说成“清除进程内所有历史副本”同样是造假 | workspace 显式钉 `zeroize 1.9.0` 且只开 `alloc`；`SecretBytes` 改持有 `Zeroizing<Vec<u8>>` 并标记 `ZeroizeOnDrop`，删除手写 Drop。保证只到当前 Vec length+capacity；历史扩容和调用方副本仍明示不保证 | `cargo tree -i zeroize -e normal --all-features` 证明 1.9.0 已在图；domain 325 unit + 6 integration + 7 doctest 全绿；trait 编译期断言 `SecretBytes: ZeroizeOnDrop`；`cargo vet --locked` 仍 14 audited / 350 exempted，没有绕过 R45 |
| R47 | §5.3（2026-08-23 D-2 类型所有权裁决） | `AttemptId` / `CapabilityId` / `CatalogGeneration` 已在 domain 定义却被 application/infra 消费；`AuthGeneration` 同时以 domain newtype 和 `AuthContext` 裸 `u64` 存在；`SecretId` / `CredentialGeneration` 则仍只在 vault domain | 六个全上收会把无跨层消费者的类型塞进 contracts；全不上收则让前四个跨层概念继续多真源。contracts 所有权应由真实消费者裁决，不是名字看起来像 ID 就搬 | 上收 Attempt/Capability/Catalog/Auth 四类型，保持原有窄 trait 面且全部不 serde；`AuthContext` 字段/构造/getter 全程改用 `AuthGeneration`，domain 旧路径只做 re-export。Secret/Credential generation 明确留 domain，第一条跨 crate 用例出现时同批移动 | 全仓 `pub struct` 复算实得六个名字每个恰一个定义；跨 crate 旧 domain path 零命中；contracts 负向测试锁死三个内部 ID 与 AuthGeneration 均不 serde、ActorId 正向仍可 serde；WASM contracts check 与 workspace 全闸门通过 |
| R48 | §6.2 / §7.5 / §10.5 / §16.3（2026-08-23 W-7 safe dialer/TLS） | R29 刻意留下真实 HTTP/TLS；若直接开 openidconnect/reqwest 默认 feature，会出现第二 resolver/proxy/redirect/retry 路径，DNS 检查后仍可能重绑；rustls provider 又不可避免扩大 C/汇编构建面 | 安全属性必须落在“连接参数就是已验证 SocketAddr”而非一组 client builder 约定；同时不能把 ring 的 Rust API 冒充纯 Rust 构建，也不能为变绿全局关闭 executable/script 闸门 | 唯一 `openbot-infra::net::safe_http`：每跳解析→IANA/IP/CIDR→直接 TcpStream，原 host 只用于 SNI/Host；最多 3 跳、跨 origin 剥 Authorization、secret POST 30x 收紧、总时限/流式大小上限。TLS 钉 rustls 0.23.43 + ring 0.17.14 + webpki-roots 1.0.9；cargo-deny 精确看见 38 Perl/17 `.o`，guard 锁 build.rs hash/feature/唯一调用面；CDLA 全文进 NOTICE/SPDX。Cargo Vet 新增 20 条均带 owner 且明说非 full audit，G2 外审仍硬前置 | 负向先得 build-script-not-allowed、38 个 detected-executable-script、CDLA rejected、20/20 unvetted；收口后 guard exit 0、deny licenses/bans 绿、vet=`14 audited/370 exempted`。本机网络矩阵覆盖 private/metadata/rebinding/redirect/header/size/time；真实 CA→leaf TLS 证明连接 127.0.0.1 而 SNI=idp.test，错 hostname 被拒。完整 delta 见 `docs/2026-08-23-W7-safe-dialer-TLS-delta审计.md` |
| R49 | §6.2 / §6.3 / §6.5（2026-08-23 W-7 环境 OIDC 登录竖切） | 旧实现只有进程内 attempt、GET metadata 与 claims 单元；无 code POST/callback/session。另一个实测冲突是 Microsoft 默认 `common` 的 metadata issuer 为 `{tenantid}` 模板，普通 exact issuer verifier 会把合法多租户 token 全拒；若直接放宽又会接受错 tenant/key | callback 必须跨 replica 单次消费，token/JWKS/claims/session 顺序唯一；Entra tenant-independent 验证必须同时绑定 canonical GUID `tid`、token `iss` 与**选中 JWK 自带 issuer**，且 subject 在多租户下必须带 issuer 作用域 | PostgreSQL `verifications` 存 HMAC state/PKCE/nonce，DELETE RETURNING 先烧后验；受约束 oauth2 POST；JWKS unknown-kid 冷却重拉并保留 per-key issuer；Google/Microsoft/Okta 可同时 discovery。`common/organizations/consumers/GUID` 由固定 Microsoft host 构造，authority/issuer 模板分离；claims/group mapping 后，一个事务链接 account、双 email revoke、admin floor、materialized membership、generation、keyed session 与 audit，commit 后才返回 host-only HttpOnly/Lax cookie。长寿命配置/HMAC/OIDC secret 均由 zeroize、不可 Clone 的 `SecretBytes` 持有，协议库 `ClientSecret` 仅单 callback 临时物化；IP/email bucket 只以 HMAC 入 PG；token transport 断连归 503，IdP 限流/5xx/坏响应归 502，均不回显远端载荷。动态 IdP 写面和 SAML 仍未闭合 | W-7a 收口时 PG17/SCRAM infra+server **435/0/0**（W-7b 后当前总数见 R50）。新增真库矩阵：两 replica state 恰 1 success；provider mismatch（含合法但未注册 ID）/expired 均烧 state；group 撤销同事务删 membership、generation+1、清旧 session；audit 链故障 user/account/membership/session 全回滚；限速并发恰 1 allow+1 deny且库内零原始 IP/email。真实本机 TLS IdP 逐次完成 discovery→PKCE token POST→动态 RS256→JWKS→claims/group→session，重放未产生第 4 次 IdP 请求。Microsoft 官方实时样本：common/organizations 模板、consumers 固定 GUID、common JWKS 9 keys（6 template/3 consumer） |
| R50 | §6.2 / §6.4 / §6.5 / §16.3（2026-08-23 W-7b dynamic SSO/SAML） | R29 写“规则先落 domain”，但源码复核没有 SAML 模块；`sso_providers` 也只有通用读 repo，无加密写/注册/路由/callback。直接启 `samael+xmlsec` 首先缺 `xmlsec1-config`，安装后真实 XMLDSig 又因进程同时加载系统 libxml2 2.9 与 Homebrew 2.15 **SIGSEGV**。同时复核 2026-06 Better Auth 高危公告，旧删除只删 provider 行会留下可被重注册继承的 account anchor | 不能关闭 xmlsec 冒充验签；必须让 Rust libxml 与 xmlsec 加载同一 libxml2。动态写面必须 fresh admin+Origin，且 guard 必须先于 body 解析；ID/domain 与环境/保留名隔离；update/delete 在释放 provider ID 前同事务删除关联 accounts、推进 generation、清 session。登录 Origin/cookie 策略独立于环境 OIDC coordinator，dynamic-only 装配不能恒拒匿名路由。SAML 必须让签名覆盖的根是 Response，逐项绑定 Destination/Audience/Recipient/`InResponseTo`，assertion ID 跨 replica 单次烧；外审仍不可免 | 固定 samael 0.0.22+xmlsec，新增 31-crate/8-build.rs/原生四包 delta guard；macOS target link-search 排除双 libxml。动态配置用 v2 record AEAD（AAD 绑定 tenant/provider/column/version），兼容 plaintext/v1 并同事务写/回读迁移。PG route ticket/RelayState/replay 均 HMAC-only；动态 OIDC 每次从 DB 新鲜构造 safe runtime，state 在 discovery 前烧。SAML strict XML 拒 DTD/ENTITY/PI，根级唯一签名 Reference 绑定 Response ID，只许 RSA/ECDSA SHA-256/384/512，多 AudienceRestriction 按 AND；SP-initiated Redirect/POST、metadata/ACS、session/group 全接线；不开放 SLO。OpenSSL 3.6.3 两条 2026-08 低危 QUIC/OCSP advisory 对本仓 XMLDSig/X509 面不可达，3.6.4 可得即升级 | 负向：安装前 xmlsec1-config 缺失；双 libxml 真 SIGSEGV；deny 先红 8 build script + 1 executable；vet 先红 31/31，Google refresh 后 30/31；HTTP 矩阵先抓到 delete-provider 400-before-auth 与 dynamic-only Origin 恒拒并修复。收口：真实 RSA-SHA256 XMLDSig 正向；unsigned/SHA-1/错 Destination/Audience/Recipient/request/time/DTD 负向；PG17 三条（SAML replay+account cleanup、动态 OIDC 跨 replica TLS、legacy→v2）与 server 真 Axum 管理路由 1 条全绿；当前 workspace=`991/0/64`，PG17 infra+server=`451/0/0`（314+137），严格 recount=`147/147`。`cargo deny` 四段 ok，vet=`15 audited/400 exempted`（新增 30 条非审计）。完整证据见 `docs/2026-08-23-W7b-SAML-xmlsec-FFI-delta审计.md`；**尚缺独立 SAML/XSW 外审、Linux CI 真跑、Windows 原生构建、Server KMS/HSM key ring，故 G2 仍不得标绿** |
| R51 | §8.6 / §15.4 / §16.1 / §20.4（2026-08-23 W-6 migration preflight） | `AUDIT_RETENTION_DAYS` 上游注释声称“refused rather than coerced”，实际生产代码却先做 `Number(raw)`，因此 `+7`、`0x10`、`0b101`、`0o10`、`1e3`、`7.0`、`1.` 等都能让旧部署正常启动；Rust 领域 parser 按 §8.6 只收十进制正整数。若只等新 server 启动时失败，切换窗口才第一次暴露不兼容。同期复核还发现 R30 的实现只补 U+FEFF 却仍用 Rust `White_Space ∪ U+FEFF`，会多裁 ECMAScript 不认的 U+0085；server/infra 又各有一份 `optional/commaSeparated`，规则并不唯一 | 变量名与“未设=永久”保持；数值语义明确标**替代**并继续取窄，禁止生产入口强转。发布 `openbot-migrate preflight-audit-retention`：切换前扫权威进程环境，旧版接受/新版拒绝时 stdout 只给稳定 JSON code 与规范十进制 `replacementDays`，绝不回显原值；需改配置 exit 2，兼容 exit 0，超 `u32` 不擅自取整而要求人工选策略。它当前只是 §16.1 migration binary 的第一个局部子命令，**不冒充完整 PostgreSQL/import readiness**。ECMAScript `TrimString` 封闭表下沉 `openbot_domain::text`，server/infra 配置、retention、email、地址与 HTTP parseInt 共用；U+FEFF 去、U+0085 留 | 固定上游 `config.ts::auditRetentionDays` 实读为 `Number`/`Number.isInteger`/`>=1`，全固定上游测试对该变量零命中；Node 20.19.6 对 19 个正反样本逐条实跑。Rust 定向：domain retention 12/0/0、email 10/0/0、infra auth config 36/0/0、server config 61/0/0、真实 CLI 2/0/0；手动 `0x10` 实得 exit 2 + replacement 16，`30` 实得 exit 0；输出均不含原值 |
| R52 | §19.3 / §21.1 条 4 / §24 G2、G6、G8（2026-08-23 W-5 台账复算） | 移交指南把“G2 相关上游测试 234 条”写成排期依据，却没有集合定义或一条复算命令；同一个数字最初只存在于文档，无法证明不是手算巧合。W-7b 又把“直接对应的 11 条 done”误写成“234 里的 11 条”，但机器台账实得其中只有 `encrypt-sso-config` 7 条属于该集合，另外 4 条是 §16 health/IdP registration | 阶段队列以 `test_inventory.rs` 的封闭常量定义：所有 `FILE_RULES.reason` 引用 §6 或 §8 的文件共 24 个/246 条；`audit-silence`、`auth-client`、`credential-form` 是 audit 页文案、浏览器登录 client、credential GUI form，12 条显式归 G6；余下 `G2_TEST_FILES` 21 个/234 条归 G2。`tool-name`/`tool-result` 留 G2，因为承载 §8.2 metadata 与 policy refusal 解码可识别性，不依赖 route/视觉。分区必须不交叠且并集逐文件等于引用全集；新增文件/改引用不显式改分区就判红 | `cargo test -p openbot-testkit --features xtask --bin xtask g2_test_inventory_is_exactly_234` = 1/0/0，同时断言 234+12=246；生成器向 `parity/tests.yaml::recount` 写入带 21 文件 JSON 的单条 `jq` 复算，固定 AST 实得 234；重放 `cargo xtask test-inventory --upstream <固定克隆>` 仍为 105 文件/229 describe/1047 test，并完整保留既有 11 条 done |
| R53 | §6.1 / §6.2 / §14.2 / §19.3（2026-08-23 W-5 identity ledger batch 1） | W-4 的本地 provisioner 自造 `single-user` / `single-user@localhost` / `Local Owner`，而固定上游 `DEV_ACTOR` 与已冻结 CEL fixture 都用 `dev-local-user` / `dev@openbot.local`；上游注释明确固定 id 是 thread/memory 跨重启归属键，改名会把既有数据变孤儿。同一段 Rust SQL 又 `ON CONFLICT(id) DO NOTHING`，不能恢复被改坏的 canonical identity，并插 admin+user 两行，违反已落 `RoleAssignmentPlan` 的“删旧+唯一目标” | 单用户持久化收口到 `openbot_infra::auth::single_user`：disabled 在取连接前返回；enabled 单事务按固定 id upsert canonical email/name、保留既有 generation，并复用 `plan_set_role(Admin)` 把 role 集合收敛为唯一 admin。canonical email 被另一 user 占用时保留 23505、整事务回滚且错误不回显地址。Server resolver 与 transport 测试全复用 `SINGLE_USER_ACTOR_ID`，旧 `single-user` 字面量清零。多用户新身份的 admin/user seed 另收口为 domain `seed_role` 并由 OIDC 生产路径调用 | 固定上游 `dev-actor.ts::DEV_ACTOR/initializeDevActorUser` 亲读；仓内 CEL fixture 对 `dev-local-user` 有 16 处正向命中；W-6 commit 的旧双引号 actor literal 8 处 + SQL 单引号 1 处，修后两类均 0。新 `dev_actor` 矩阵 PG17/SCRAM=3/0/0：disabled pool size=0、篡改后恢复且 generation=7 不回退、email 冲突 23505 + 全回滚；domain roles=17/0/0；OIDC 真库 seed admin/user=1/0/0；tests ledger 新闭合 29 条后为 40/1007，G2 队列 36/198，严格 recount 149/149 |
| R54 | §14.1 / §16.1（2026-08-23 W-5 batch 1 真实进程复核） | 同一命名 fresh 库首启成功、四 HTTP 面与 canonical user 都正确；Ctrl-C 后二启却稳定报 `legacy_data_migration_unverifiable`。原因是 main 对 compat 明确允许给 Rust baseline 的 `DataMigrationVerdict::Unverifiable` 一刀拒绝：fresh baseline 不伪造 Drizzle 账本，二启必然落该态。更深一层，baseline 与 native 原先分两个事务，进程若死在中间会留下“有 0012 schema、无任何来源账本”的永久歧义；两个 replica 还可同时在锁外看见空库并竞争 baseline DDL | 新增 `db::fresh`：先取同一 migration advisory lock 并在锁内重检 public 表，再把 baseline + 0013–0015 + native ledger 放入一个外层事务；等待者发现已初始化就转 existing 分支。native 施加器抽出事务内核心，既有独立调用仍自己开/提交事务。Server `database::initialize` 三分：空 schema→Fresh；有效 native ledger→schema + checksum/空洞校验后 RustManaged；完整 Drizzle ledger→LegacyUpgraded；两账本都无→拒绝。native ledger 只作来源证明，存在不等于通过 | 负向现场：同库首启 200/ready 后二启实得该稳定错误。修后 PG17/SCRAM `database_initialization`=4/0/0：fresh→二启且 Drizzle 表仍不存在、双 replica 恰 Fresh+RustManaged/账本 3、未知无账本 legacy 拒绝、同名异形 native ledger 失败后 public 表数仍 0；保留现场库重建二进制后二启正常监听，Ctrl-C exit 0；临时库已删除 |
| R55 | §6.1 / §6.2 / §19.3（2026-08-23 W-5 G2 ledger batch 2） | `people-paging.integration.test.ts` 9 条仍全是 todo，但既有一条聚合测试只显式覆盖页序、wildcard、坏 cursor 与部分投影，不能据此把“name/email 能搜到第一页外目标”、HTTP 端 200 上限、point lookup 不扫 deployment、missing person 零副作用等未行使判据批量算 done。复核同时发现 People 搜索归一仍调用 Rust `str::trim()`，违反 R51 的 ECMAScript TrimString 单一真源。`roles.test.ts` 最后一条则用 fake 制造“session 存在但 user 不存在”，而固定上游与 Rust 生产 schema 都有同一 `sessions.user_id → users.id ON DELETE CASCADE` 外键；复制一个生产不可达的 domain 死分支不是证据 | 搜索归一改为唯一 `openbot_domain::text::trim_ecmascript`，用 U+FEFF/U+0085 双向测试锁住。People 9 条各建独立机械落点：真 PG keyset 页/全量 walk/NULLS LAST/name+email server-side search/wildcard/坏 cursor/full Person/missing NotFound，HTTP→application→typed port 另证明任意大 limit 在碰库前钳到 200；point query 的生产 SQL 必须以参数化 `WHERE u.id=$1` 在聚合前定界且不得复用 list CTE。缺 user 的 session 以真库 23503、正向可插、删 user 后 cascade 三段证明结构不可达，不新增假业务分支 | `people_application` PG17/SCRAM=14/0/0；People application=5/0/0、point SQL shape=1/0/0、HTTP cap=1/0/0。生成器重放仍为 105 文件/229 describe/1047 test 且保留 done overlay；tests ledger=50/997、G2=46/188、parity 总计=145/1500。完整 workspace=`1003/0/79`；PG17 infra+server=`474/0/0`。详见 `docs/2026-08-23-W5-G2身份台账batch2.md` |
| R56 | §5.2 / §8.3 / §8.6 / §19.3（2026-08-23 W-5 G2 ledger batch 3） | `action_policy` 虽有通用 CRUD 名字，但没有一份 production store 能把 current 行、显式配置、未配置 default-deny 与热路径内存统一起来；没有 LISTEN/NOTIFY、重连 catch-up 或 Server 启动接线，故 durability/fanout 13 条全 todo。Audit 则只有 typed append/checkpoint，没有 ApplicationService 读命令、管理员 HTTP 或 keyset reader；更严重的是 test inventory 把 `audit.test.ts` 标成 parity，但 §8.6 已明确用字段 allowlist 推翻上游自由 JSON + 敏感键黑名单，分类本身失真 | 新增 `PolicyStore`：DB 行是权威记录，内存持有 `raw + Arc<CompiledActionPolicy>` 原子快照，load/set/reset/refresh 才编译，acting 读取只 clone Arc；同进程 operation lock 收敛 refresh/write 竞态，upsert/reset 与空 payload `pg_notify` 同事务。`PolicyListener` 用独占 tokio-postgres 连接，首次 LISTEN 与每次重连后整表重读，断线有界重试；Server main 启动加载并持有 listener 到 graceful shutdown。无显式配置继续是独立 Unconfigured/default-deny，不继承上游隐式 allow。Audit 新增 wasm-safe DTO、typed command/port/use case、PG keyset reader 与 admin GET，Axum/in-process 共用同一 Arc；自由 redactor 两条改记 `rename: ported`，inventory 文件级标签保守改为“替代”。真实 PostgreSQL 中 announcement 无接收者/订阅离线时 row 仍是记录，listener 建立即 catch-up；不靠 fake 推断网络断连下未知 commit | PG17/SCRAM `policy_store`=13/0/0、`audit_reader`=4/0/0；domain audit=53/0/0、application audit=2/0/0、server admin=1/0/0、transport audit=1/0/0。真实 Server fresh 四只读面 200；同库写入 dry-run 后二启日志实得 `origin=Database/mode=dry-run/configured=true` 与内容版本，SIGINT 正常退出、临时库 0。生成器重放 105/229/1047 且 overlay 保留；tests=69/978、G2=65/169、API=13/135、parity 总计=165/1480。完整 workspace=`1010/0/94`；PG17 infra+server=`492/0/0`。Policy 管理写路由与真实 G4 executor 仍未闭合，不借本条宣称 G2/G4 通过；详见 `docs/2026-08-23-W5-G2策略审计台账batch3.md` |
| R57 | §5.2 / §8.3 / §15.1 / §19.3（2026-08-24 W-5 G2 ledger batch 4） | R56 的 store 已在生产启动，但没有 ApplicationService policy command/port，产品无法经 Rust-owned API 读写；`computer-policy-route.test.ts` 4 条与 `computer-policy.test.ts` 31 条全 todo。若把 store 直接塞进 Axum 会违反唯一业务入口；照抄上游 PUT 只验 admin 又会让跨站请求改写全 deployment 边界。另：Hono `/:botId/*` 对 `/policy` 零段匹配导致 deployment route 被误当 Bot 的缺陷，不应在 Axum 复刻成 wildcard exemption | 新增 wasm-safe `ActionPolicyDocument`、Get/Set typed command、`PolicyAdministration` port 与 application admin use case；`PolicyStore` 实现 port，updated_by 只取权威 actor id并在 commit 后替换预编译快照。GET/PUT 以精确 `/api/computers/policy` 路由挂载，不设 wildcard bypass；未配置 GET 明示 `policy:null` 并保持 default-deny。PUT 归类为替代：fresh live admin + trusted Origin 在 body parse 前通过，缺 Origin/非 admin/非 fresh 零 port 副作用；合法文档持久化后回显实际生效值。固定上游 31 条中 28 条求值逐项落 `computer_policy_upstream`，3 条 JSON parser 复用唯一 Server parser，不复制第二实现；cel-js→cel 与结构化 Refusal 继续按既有替代裁决 | domain policy matrix=28/0/0、Server parser=7/0/0、application policy=2/0/0、Server route=4/0/0、transport policy=1/0/0；PG17 `policy_store`=14/0/0，Policy HTTP 真腿=1/0/0（GET null、缺 Origin 403、trusted PUT 200、updated_by=`dev-local-user`、新 store 从 DB 恢复）。生成器重放保留全部 overlay；tests=104/943、G2=100/134、API=15/133、parity=202/1443。完整 workspace=`1046/0/96`；PG17 infra+server=`498/0/0`。真实 G4 executor 与 Bot status/access 路由仍未交付；精确未知路径 404 只证明 policy route 没开洞，不冒充 computer surface 完成。详见 `docs/2026-08-24-W5-G2策略写面台账batch4.md` |
| R58 | §5.1 / §8.1 / §8.2 / §9.1 / §19.3 / §24 G2、G4（2026-08-24 W-5 G2 ledger batch 5） | 移交指南在未亲读源码前把 `tool-name` 5、`tool-result` 6 与 `server-side-tools` 5 条合成一批。固定上游实读表明前 11 条是不依赖 Leptos 的纯 transcript projection；后 5 条却要求真 MCP HTTP、Bot grant、vendor schema、policy 与 MCP audit。与此同时，现有 `mcp_servers/mcp_tools` 没有 §9.3 的 catalog generation/stale-grant 状态。用 fake fetch 或只为测试搭 runtime 会把 G4 生产 executor 冒充成 G2 台账证据 | batch 5 只闭合前 11 条，`server-side-tools` 5 条保持 todo，等 G4 同批落 RMCP 3.1.4、唯一 safe dialer 的 Streamable HTTP、catalog generation/stale grant migration、权威 grant/schema、`ApplicationService` decision/attempt/capability 与 MCP audit 真竖切。UI 只新增纯 Rust `tool_name/tool_result`，不引 Leptos；typed `ToolResult` 以稳定 `policy_refused` code 识别新拒绝，字符标记仅读 legacy transcript。R51 `TrimString` 实现上收 wasm-safe contracts，`openbot_domain::text` 保留原 API 重导出，全仓仍只有一份 U+FEFF/U+0085 封闭集 | `openbot-ui`=12/0/0（固定上游 5+6，另 1 条 BOM/U+0085 正负对照）；contracts 共享 text 测试与 domain 原路径测试均绿。Cargo.lock 只给 `openbot-ui` 记已有 `openbot-contracts/serde_json` 直接边，新 package=0；未引入 RMCP/JSON-schema 依赖。完整 workspace=`1059/0/96`；生成器重放 105/229/1047 且 11 条 overlay 保留；tests=115/932、G2=111/123、API=15/133、parity=213/1432。详见 `docs/2026-08-24-W5-G2工具呈现台账batch5.md` |
| R59 | §5.2 / §6.2 条 8 / §6.4 / §8.6 / §9.2 / §19.3 / §24 G2、G4（2026-08-24 W-5 G2 ledger batch 6） | `plugin-user-credential.integration.test.ts` 11 条全 todo，而既有 `McpUserCredentialRepo` 与 `CredentialRepo` 只能分别 CRUD 一张表：没有一条生产边界能证明 user-OAuth 精确按 `(server, actor)` 选择、空 actor/缺连接/撤销不 fallback deployment credential，也没有把 refresh exchange 与 vendor token 分型。更严重的是 people 移除已提交 deny/session/generation，却没有执行领域注释明确列出的第二阶段 credential retirement；`mcp_user_credentials.user_id ON DELETE CASCADE` 会先删 join，把仍 active 的 refresh token 留成不可见孤儿 | 新增 `CredentialRecordVault` 与 `PluginUserCredentialStore`：v1 兼容读/v2 六元组 AAD 解封，单条 LEFT JOIN 在任何网络前按 `(server_id, actor_id)` 同快照取得个人 token 与 deployment client，缺失/撤销/错绑定 fail-closed；`OAuthTokenExchanger` 只接窄 exchange material，只有非空且不同于 refresh 的返回值能铸 `VendorAccessToken`。新增 application-owned `OwnedCredentialRetirer` 与 PostgreSQL 实现，直接按 `credentials(kind='mcp_user_token',key_id=owner)` 找正常行和 orphan；revoked_at、join delete 与 allowlisted `mcp.account_disconnected` audit 同事务。People deny/session/generation 先提交，再调用幂等退役端口；即使 access 已是 revoked，重试仍执行第二阶段；Server production assembly 已注入。`SecretId/ServiceId` 只由 infra adapter 就地构造后传给 domain `RecordBinding`，未穿 application/transport port，按 D-2 继续归 domain。它不实现 token endpoint/vendor/RMCP executor，也不声称补齐物理 credential generation/resource/expiry 或 Server KMS/HSM | PG17/SCRAM 定向 11/0/0，含 deployment fallback 正负对照、两 actor token 摘要、refresh 空/echo 构造性拒绝、真实 `DELETE users` FK cascade orphan、People→retire→typed audit 与 2→0→0 幂等计数；credential vault 单测新增 2 条，v2 搬移四维负例与固定上游 v1/明文负例均绿。生成器重放 105/229/1047 且 11 条 overlay 全保留；完整 workspace=`1061/0/107`，PG17 infra+server=`511/0/0`；tests=126/921、G2=122/112、API=15/133、parity=224/1421，fixture=9/22。Cargo.lock 新 package=0、schema/migration 仍止于 0015。详见 `docs/2026-08-24-W5-G2个人凭据台账batch6.md` |
| R60 | §3.2 / §5.2 / §6.5 / §13.1 / §16.1 / §16.2 / §19.3 / §24 G2、G6（2026-08-24 W-5 G2 ledger batch 7） | `tenant-package.test.ts` 26 条与 fintech fixture 1 条全 todo；上游 runtime theme 允许包覆写 `:root/.dark`，与 GUI 第一真源 `tokens.toml` 单一来源正面冲突。上游 `synchronizeTenantPackage` 又只写 channels.allowed_groups，不写 membership，复制后仍会让包 channel 对所有人不可达；空 audience 也静默接受。另两条实际风险是 environment expander 默认看完整 `process.env`（包可把 secret 展开进 DB/UI），以及随包 brand 直接使用第一真源禁止对外复用的 OpenBot 标记 | Tenant Package 收口到 application 纯 parser/validator + infra 有界 loader/PostgreSQL adapter：输入类型恰为 brand/agents/channels/model/knowledge 五 YAML，单文件 1 MiB，checksum 只覆盖展开后的五文件；`skin.stylesheet` 只记 compatibility ignored，loader 不打开 theme.css。环境只从启动层 allowlist 投影，本批生产只放 `MANAGED_AGENT_AG_UI_URL`；browser configuration 只有 neutral brand。示例改为 fintech/Ledgerline 并保留 MIT provenance。§6.5 复用既有 domain audience：空列表拒绝、named group 要 IdP mapping、single-user group ignored、all 全量 provision；同步锁定 package 表，拒保留名/user/cross-package Agent/Profile/Channel collision，resync audience 收紧时 membership+generation+session 同事务。动态 OIDC/SAML 暴露非 secret provider/mapping 投影；Server 启动经 application use case 同步，未设 DEPLOYMENT_ID 时只用**校验后的 package tenant id**，绝不再用目录猜 identity。runtime theme 两条改记 not-applicable-with-proof | 固定上游三个文件 SHA-256 分别为 `15c08eb…` / `43617c…` / `44f90ac…`。application package=18/0/0；tenant loader+PG17/SCRAM=8/0/0；fintech fixture=1/0/0；SSO mapping 定向=1/0/0。真实 Server 同库二启：tenant=fintech、agents=2、channels=3、membership grants 3→0、health/readiness/me 全绿、库内 package/agents/channels/memberships=1/2/3/3、Ctrl-C 两次 exit 0。完整 workspace=`1083/0/114`，PG17 infra+server=`521/0/0`；生成器重放 105/229/1047 且 27 条 overlay 全保留；tests=153/894、G2=149/85、API=15/133、parity=251/1394，fixtures=9/22。Cargo.lock 新 package=0（只给 application 增加既有 serde_yaml 直接边），schema/migration 仍止于 0015。省略包实体的 soft-delete/removed-channel 生命周期不在固定上游这 27 条内，继续归后续 G3/G4，不借本批宣称 Tenant Package 全生命周期或 G2/G6 整关完成。详见 `docs/2026-08-24-W5-G2租户包台账batch7.md` |
| R61 | §16.3 / §19.2 / §24 G2（2026-08-24 W-7c Linux CI） | 移交长期把“GitHub Actions 额度耗尽、运行数 0”当静态事实，并进一步把“G2 未标绿不得发布晋级”误写成“不得继续实现 G3”。实际 API 已为 Actions enabled；首次对 batch 7 精确 head 派发 Ubuntu CI 后又连续暴露三条本机照不到的差异：runner 缺 `rg`；`set -o pipefail` 下 `printf | grep -q` 因 SIGPIPE 把已存在的 bindgen 误报成缺失；deadline 测试在客户端正确超时断开后仍要求测试服务端 write 必须成功，Linux 合法返回 BrokenPipe 时 panic | 两个 job 显式安装并打印 `ripgrep`；所有同形 guard 改为 here-string，精确版本/feature/调用面集合不变；测试服务端只接受 BrokenPipe/ConnectionReset/ConnectionAborted 三种“对端已完成”错误，客户端仍须精确得到 DeadlineExceeded。恢复 `pull_request` 自动触发且不限制 base（堆叠 PR 同样受检），`main` push 与精确 head workflow_dispatch 保留；自动事件默认跑 supply-chain。§24 是发布 Go/No-Go，不取消 §19.2 的 Computer/GUI/Agent 并行实施；未绿只禁止勾关和发布晋级，不构成停止写后续生产代码的理由 | 失败链：run `32761260527` 命中 `rg: command not found`；run `32761589342` 命中 SAML guard SIGPIPE；run `32762136217` 命中 safe_http deadline 测试 BrokenPipe。最终自动 PR run `32762651186`（head `1a401fa…`）在 Ubuntu 24.04.4 x86_64、Rust 1.98.0 下 gates=3m34s、supply-chain=42s 全绿；workspace 日志机械汇总 `1083/0/114`，parity=`251/1394`、0 违反，deny/audit/vet 与两份 native guard 全绿。Linux CI 子项据此勾选；独立外审、KMS/HSM、Windows 与真实 G4 executor 仍不冒充完成 |
| R62 | §4.1–§4.3 / §5.3 / §14.2–§14.3 / §20.3 / §24 G3（2026-08-24 G3 native data base batch 1） | §4.3 只列十张尚缺表和十一条不变量，Phase 0 ledger 对 thread delete/retention 仍写“待定/倾向”，也没有列级 DDL。若凭喜好补列会制造新真源；若只造十个空 repo 又违反 R38。另一个兼容风险是 0013 已允许 durable tool call 早于 native runs：直接 VALIDATE run FK 会让升级扫描失败；完全不加 FK 则新写继续悬空。Memory 的“删除/禁止”若只改状态却保留 content，也不满足用户可控数据边界 | 以第一性不变量裁决并记录 native 0016：threads 软删且不猜固定 retention 天数，最终物理清理由 G8 policy；messages 保存结构化 content+search_text，summary 是 role 不是 memory；runs 的 queued/running/reconciliation_required 共用 partial unique foreground slot，terminal_event_seq/status/time/started_at 同形；run_events 双序 + partial unique terminal；lease 过期接管单调推进 fencing，值域与 PG nonnegative bigint 一致、MAX fail-closed；outbox 只容纳 internal/idempotent_external，non-idempotent 构造性拒绝；memory 只有 preference/fact 与 user_action/remember_tool/verified_import，fact/import 强制 source，forbid/delete 同写 content=NULL，simple FTS+tags 不引 pgvector。tool_calls→runs 用 NOT VALID FK：不扫描历史但约束新写，后续 importer/backfill 后另批 VALIDATE。十表与十 repo 同批，Thread/Memory repo 不公开 hard delete。ThreadIdentity 在 WASM-safe contracts 纯实现 SHA-256 前六字节+UUIDv8；infra 唯一 issuer 用 OS CSPRNG 填满 entropy，随机源失败不退化 | PG17/SCRAM native0016=3/0/0：post fixture 与活库逐字段相等、0015 每个旧列保留、双 replica Applied+AlreadyApplied；行为实证第二 foreground/terminal 拒绝、lease 1→2、cursor replay、outbox claim/CAS、memory source/scope/delete 与新 tool FK。post fixture SHA-256 `3a9ca0e…`，41 表/351 列/268 NOT NULL/181 约束/80 索引/4 trigger。40/40 repo 真表 inventory 绿；contracts ThreadIdentity 固定上游 8/0/0，infra production issuer=1/0/0，domain=336/0/0；完整 workspace=`1101/0/117`，PG17 infra+server=`525/0/0`。generator 重放仍 105/229/1047 且 8 条 overlay 保留；tables=54/0、tests=161/886、parity=269/1376、fixtures=10/22。Cargo.lock 新 package=0（contracts 只增加既有 sha2 直接边）。本批不宣称 transactional append/live SSE/WS、chunk accumulator、memory journey/importer 或 G3 整关完成 |

| R63 | §16.3 / §24 G2（2026-08-24 GitHub Actions 额度操作覆盖） | R61 为取得首份 Linux 证据恢复了 `pull_request` / `main push` 自动触发；用户随后明确说明 Actions 额度不足，并下达“不要跑 CI，本地跑测试”的当前操作指令。继续自动触发或手动派发会消耗用户未授权额度；反过来把 R61 已获得的 Ubuntu 证据取消也不诚实 | `.github/workflows/ci.yml` 删除 `pull_request` 与 `push`，只保留 `workflow_dispatch`；未经用户重新授权不得派发，也不运行 `cargo xtask ci`。实施验证改为本机按变更面选择定向 Cargo/PG 测试并记录精确命令与计数。该覆盖只改变当前运行方式，不降低 §16.3/§24 的最终发布判据，不撤销 W-7c 已勾证据 | 覆盖时 Actions API 实查六个最近 run 均为 completed、无 running/queued，故没有可取消任务；修改后本机 ThreadIdentity=`8/0/0`、domain=`336/0/0`、production issuer=`1/0/0`、临时 PG17/SCRAM native0016=`3/0/0`，未派发新 run。最终发布前仍须按届时用户授权恢复外部环境验证或取得等价证据 |
| R64 | §4.1 / §4.3 / §5.2 / §15.3 / §20.3 / §24 G3（2026-08-24 G3 native thread routes batch 2） | 固定上游 `GET /threads/:threadId` 只有配置 Intelligence reader 才注册；reader 以 `(thread,user)` 查询，404→unknown、其余失败→502。照译会违反 §4.1“最终请求路径不连接 Intelligence、无配置完整运行”。另一个易错点是把 `ThreadIdentity::owns` 当可见性：foreign/legacy UUID 可能已迁入本 deployment，ownership fingerprint 不是 ACL | 增加封闭 `MintThreadId` / `GetThreadStatus` command/reply 与 application-owned `ThreadDirectory`；production `PostgresThreadDirectory` 的 mint 走唯一 OS CSPRNG issuer，status 同时绑定 AuthContext 的 deployment/tenant/actor 与 materialized thread membership，deleted/无权/错 scope/不存在统一 `known:false` 防枚举。GET 恒注册，不含 Intelligence 类型/配置/fallback；UUID 外形错误在碰库前 400，native store 失败按 §15.3 回 503 而不是已删除 vendor 后仍冒充 502。Axum 只做 framing，Desktop typed in-process 与它持有同一 Arc | 固定上游 thread-routes 8 条逐项移植/显式替代：Server=`9/0/0`（另含 malformed 负向），application=`4/0/0`，Axum/Tauri 对拍=`3/0/0`。PG17/SCRAM=`1/0/0`：legacy UUID owns=false 但正确 scope known=true；错 deployment/tenant/actor、无 membership、deleted、missing 六类全 false；production mint owns=true。API ledger `T-API-0035/0036` 与 tests `T-TEST-0986–0993` 同批 done；API=`17/131`、tests=`169/878`、总 parity=`279/1366`，0 violations。transactional append/live/history/memory/importer 仍未完成，不借 status 路由勾 G3 整关 |
| R65 | §4.3 / §5.2 / §7.2 / §13.2 / §15.3 / §24 G3（2026-08-24 G3 transactional append + replay/live batch 2） | 0016 只有表/repo，不存在一条 ApplicationService 能证明 thread state、message、run、semantic event 与 outbox 同 commit。若 run_id 重试只看主键冲突，会把相同请求误报失败；若不同内容复用同 ID 又返回旧 receipt，会把篡改当幂等。现有错误域只有 stale/lease，没有诚实表达 idempotency binding 冲突的 409。实时侧若“先 replay 再 LISTEN”会漏窗口，若把 NOTIFY payload 当数据又违背 8000 字节限制；通知/连接丢失后若没有 durable catch-up，cursor 真源名存实亡 | 新增 typed `BeginThreadRun`：输入只有 thread/run/bot/互斥 anchor/plain-text message，scope/fencing/time/sequence 只取 AuthContext+PG。新 thread 必须由当前 deployment identity 铸造；已存在 legacy/foreign UUID 仍按 ACL 可写。transaction 先 per-thread advisory lock，scope/membership/agent visibility 同快照验证，30s lease（新增、不是 run deadline）、single foreground、user message、running run、started 双序 event、internal `agent_run_dispatch` outbox、空 payload pg_notify 同批 commit。相同 run_id+相同内容返回原 receipt 且零写；不同内容用新增稳定 `request_conflict` 409/concurrency audit；commit 返回前断连映射 accepted reconciliation。订阅先建立 LISTEN 再按 cursor replay，通知只 wake；断线重连先补取，另以 1s 周期作丢通知/撤权兜底。SSE 只认标准 `Last-Event-ID`，Desktop durable/error frame 归 Critical。50ms/8192-byte UTF-8 accumulator 已实现纯边界，但未接 provider 前不勾生产子项 | 本机：contracts=`62/0/0`，application=`99/0/0`（含 chunk 5、thread use case 8、ApplicationService execute/subscribe），Server thread=`11/0/0`、error=`12/0/0`，Desktop budget=`6/0/0`，transport thread parity=`3/0/0`。PG17/SCRAM 五条：begin=`3/0/0`（七 surface、精确 replay、末段 outbox collision 全回滚、legacy/new-foreign），scope=`1/0/0`，live=`1/0/0`（订阅前丢 notify、双 replica、强制 `pg_terminate_backend` 两条 LISTEN 后无通知补回 event、cursor reconnect、撤权 error）。新增 SSE API `T-API-0149` done；API=`18/131/149`、tests=`169/878`、总 parity=`280/1366/1646`，0 violations。Cargo.lock 新 package=0，只给 infra/server 记录既有 futures-core direct 边。尚未完成 WebSocket parity、outbox relay/真实 Agent 消费、lease renew/stale-running recovery、terminal/chunk writer、history、memory/importer，G3 整关保持未勾 |

| R66 | §3.1 / §4.1–§4.3 / §5.2 / §13.2 / §15.3 / §24 G3（2026-08-24 G3 history + explicit memory batch 3） | 固定上游 history consumer 只认 `{messages:[AG-UI Message]}`，却把任何非 2xx/坏 JSON/网络失败全部降成空；照搬会让数据库故障与真空 history 不可区分。query 的 `agentId` 若进入 ACL，又会允许客户端自报另一个 Agent 改变 history。Memory 侧只有 0016 表/repo 与纯 domain：没有一条 ApplicationService/HTTP 用户旅程；若 body 可传 owner/origin/created_by，就能伪造 `verified_import` 或替别人写。Correct 若原地覆写会丢 provenance；forbid/delete 若只改状态仍保留 content 会继续泄漏；只用 exact scope recall 又会漏 user-scope，放宽成任意 scope 则跨 Bot/thread | 新增完整 history DTO/command/port：UUID shape 在 application 验，PG query 同时绑定 tenant/actor membership 并按 message seq；missing/invisible/deleted/空 thread 都是成功空页，DB/未知 role/坏 toolCallId 是 503；compat `agentId` 必填只维持 wire，不参与 scope。新增独立 `MemoryAdministration`：wire 没有 owner/origin/created_by，GUI remember 构造性固定 `user_action`；普通 authenticated user 写要求 trusted Origin 且 guard 先于 body parse。64KiB content、32 tags×64 bytes、4KiB recall query 与页长 100 均在 application；PG transaction 验证 Fact source/Thread/Bot 可见性，create+event；list owner keyset；correct 新建并 supersedes 旧行、两侧 event 同事务；forbid/delete 同事务 `content=NULL` 且重复零事件；末段 event 失败全回滚。Recall 只含 user scope + 当前 exact visible Bot/thread，active/未过期，simple FTS 再以全部请求 structured tags 作精确 AND 收窄并按 rank+recency 排序，不引 pgvector。Remember tool 仍必须经 §8.1，不能借 GUI port fake-close | 本机：contracts memory=`2/0/0`，application memory=`5/0/0`，Server memory=`4/0/0`；PG17/SCRAM memory=`2/0/0`（message-only memory count=0、fact provenance、跨 actor、两页 cursor、correct/supersede、forbid/delete/重复事件、user/thread FTS+tag recall、末段 trigger rollback）。History：Server thread=`14/0/0`、application thread=`10/0/0`、PG history=`1/0/0`（五 role 顺序、scope/empty/deleted、坏 toolCallId）。API `T-API-0109` + 新增 `T-API-0150–0155` done；API=`25/130/155`、tests=`169/878`、总 parity=`287/1365/1652`，0 violations。尚缺 WebSocket、真实 Agent/outbox/lease recovery/terminal writer、remember tool、Memory GUI、importer/checksum，G3 整关保持未勾 |
| R67 | §4.3 条 1–7 / §5.2 / §7.2 / §13.2 / §16.3 / §24 G3（2026-08-24 G3 run runtime + WebSocket batch 4） | R65 已 durable 写 started/outbox，却没有 consumer、renew、chunk/terminal writer；Server 返回 running 后会永久悬空。直接把 outbox 标 delivered 再启动 Agent 会在 crash gap 丢 run，先启动再 ack 又可能 at-least-once 重复 effect；commit-unknown chunk 若没有 expected sequence 会重复文本。另一个缺口是 SSE 已有而 WebSocket 未实现；若另写 producer 会产生第二份 cursor/ACL。Axum `ws` feature 又新增 SHA-1/tungstenite 供应链面，不能无 delta 证据放入锁图 | 增加非 serde `ClaimedRunDispatch/RunExecutionLease` 与 application `RunRuntime` port：consumer 先按 `(run,fencing)` 幂等 reserve，outbox durable ack 后才 activate，ack 失败 revoke。每个 chunk/terminal 带 expected run-local sequence；相同 seq+payload 精确 replay，不同内容 conflict，commit unknown 可安全核对。PG 每次写验证 owner+fencing+expiry，同事务分配 thread cursor；terminal 聚合 text chunks 为唯一 assistant message、写 terminal/run/thread/notify 并释放 lease。未 delivered 的过期 dispatch 可 fencing+1 安全重绑；已 delivered 的 stale run takeover 后只进 `reconciliation_required/runtime_lease_expired`，不自动续跑。100ms relay poll、10s claim（30s lease 的 1/3）与 busy 100ms→6.4s 指数退避均标新增。G4 未接时 production `NoRunDispatchConsumer` 明确写 `agent_runtime_unavailable` failed terminal，绝不伪造回复。WebSocket 强制 trusted Origin + `openbot.thread-events.v1`，query cursor 进入与 SSE 相同 subscription；socket 只出不进，Text/Binary 1008，frame/message cap 1KiB。Axum ws 锁入 sha1 0.10.7/tokio-tungstenite+tungstenite 0.29.0；SHA-1 只作 RFC6455 handshake，不作 credential/hash；三包 build.rs=0、unsafe token=4/0/5，三条 exact exemption 明示非完整审计 | application run runtime=`3/0/0`；PG17/SCRAM run runtime=`4/0/0`：claim/ack exact replay、chunk tamper conflict、terminal+assistant、旧 fencing 拒写、pending takeover 1→2、delivered stale 2→3 reconciliation、terminal/outbox 末段失败全回滚、production fail-closed relay。Server thread=`16/0/0`，其中真实 loopback WebSocket=`2/0/0`：Origin/subprotocol/cursor/frame/normal close 与 client-data 1008。`check-websocket-dependencies.sh` 绿；deny/audit/vet 绿，vet=`15 audited / 403 explicit exemptions`。API 新增 `T-API-0156` done；API=`26/130/156`、tests=`169/878`、总 parity=`288/1365/1653`，0 violations。真实 provider/Agent reducer/tool loop 尚未接，故 accumulator 的 provider producer 与 G4/G3 整关继续未勾 |
| R68 | §4.1 / §4.3 / §14.3 / §16.1 / §20.3 / §24 G3、G8（2026-08-24 G3 Intelligence importer batch 5） | §20.3 只写“加密、签名、中立 bundle / schema-signature-hash / cursor”，没有固定算法、canonical hash、target mapping 权威性或 crash cursor 事务。若 bundle 自带 target user/deployment 即可越权；若导入 queued/running run 会在 maintenance 后伪造仍在执行；若每表各自提交，失败恢复会留下 thread/message/event 半棵；0016 的 tool_calls→runs NOT VALID FK 也会永久不 validate。另一个安全坑是直接用同 AES master+随机 nonce 跨 bundle，exporter nonce 重用会破坏 GCM | 固定 `openbot-intelligence-bundle-v1`：外信封 format/bundle/source/key-id/nonce/ciphertext/plaintext SHA-256 全进 Ed25519 strict signature；AES-256-GCM key 由 32-byte migration master 以 payload hash 作 HKDF-SHA256 salt、同 header 作 info 派生并 Zeroizing，AAD 同 header，signature 先于 decrypt。信封 512MiB cap；CLI 只收 regular file，Unix key 要同 inode + 0600，key 不进 argv/stdout/stderr。Payload 只允许 maintenance 已 drain 的 terminal run；running/queued 在 enum 层无入口。Target deployment/tenant/user/bot/channel 与 foreign UUID claim 只来自独立 mapping；未 claim 先出 report 且 DB 调用 0。每 thread aggregate + 四类 cursor 同事务；source checksum 先验，写后从 PG 重建 thread/member/title/message/event/terminal/memory/sample projection 再算 length-framed SHA-256；existing ID 逐字段同才幂等，异值 conflict。只导 observable active/superseded memory，强制 source，落 `verified_import`。失败 cursor 可精确 resume，completed rerun仍全量 DB recheck。独立 finalizer 在四行 cursor 全 completed 且 orphan tool call=0 后才 `VALIDATE tool_calls_run_id_fkey`。Importer 只在 `openbot-migrate`，Server main guard 为零调用点 | application importer=`7/0/0`；crypto=`1/0/0`（header/signature/key/ciphertext tamper 全拒）；CLI unit=`3/0/0`、旧 preflight=`2/0/0`。PG17/SCRAM adapter=`3/0/0`：第二 thread 末段失败后第一 cursor committed+failed，修复后只续第二并 completed；existing binding conflict 全回滚+四行 failed/$none；incomplete+orphan 阻止 FK，修复后 convalidated=true。真实 `openbot-migrate` 子进程=`1/0/0`：独立 exporter 侧 HKDF/AES/Ed 实现，0644 key exit65、0600 import exit0、四 completed cursor、FK finalize exit0。import dependency guard、clippy、WASM contracts 绿；Cargo.lock 新 package=0、仍 428 packages，vet 仍 15/403。上游 thread-status covered-by-golden 5 条转 done：tests=`174/873`；API=`26/130/156`，总 parity=`293/1360/1653`，0 violations。尚未取得真实 Intelligence export/API 与 production bundle，三次 production-scale 演练/合同法务/实际 legacy exporter 留 G8，故 G3/G8 整关均不勾 |

| R69 | §4.3 / §6.4 / §7.2–§7.5 / §15.4 / §16.3 / §24 G3、G4（2026-08-24 Rust Agent + OpenAI provider batch 6） | R67 的 `NoRunDispatchConsumer` 虽能防永久 running，却证明真实 Agent/provider 仍不存在。第一版草稿又把 managed `BOT_MODEL` + 启动期静态 key 直接接到 package Bot，违反 §7.3 的两层选择与固定上游“每 request 重建 Agent/credential”；context 没带 `systemPrompt`/provenance，reasoning 虽解析却被 host 当坏响应。更隐蔽的时序洞是 30s lease heartbeat 在 context/provider connect 之后才启动，而 connect budget 本身恰为 30s，可能首次续租前过期。package 默认协议也不能凭印象选 Chat/Responses | 新增纯 `AgentState + reduce(state,event)->effects`，固定 Queued→Preparing→Sampling→tool/approval/human→Committing→四 terminal；DB/provider/tool 全为 effect，tool cap=8，cancel 只有 child-stopped fact 后 terminal，journal/lease unknown 进 reconciliation。`BuiltInAgentRuntime` 实现 bounded reserve/activate/revoke、并发上限、activation 起算 deadline/heartbeat、provider stream→50ms/8KiB `DurableTextRun`；text/reasoning 共用 expected sequence，reasoning 不进 assistant materialization。OpenAI adapter 同时实现 Responses/Chat，请求只经 SafeDialer；SSE 接受 UTF-8 分片/multiline、skeleton/延迟 name、partial JSON、交错 tool call、空 delta、未知扩展，限制聚合字段/总 body，并按真实 body read gap 判 stall。固定发布物 `@ai-sdk/openai@3.0.99` 的 `createLanguageModel` 实读直接返回 Responses，故 package Bot 固定 Responses；model/credential ref 只取 `model.yaml`。每 sampling 从 PG 精确选 active model credential（created_at/id 降序），stored 优先、无匹配才 env fallback、匹配密文坏不得 fallback；v1/v2 都经现有 Vault。context 每 run 重读 package Agent `systemPrompt`，前置 MIT provenance guidance。新增 private/self-hosted provider 的精确 CIDR 与 HTTP 双开关，HTTP 不代替 destination allowlist；Server main 已接真实 package Agent，缺失面不造假 | 本机：domain Agent=`3/0/0`，application run runtime=`4/0/0`，Agent host=`7/0/0`，OpenAI adapter=`6/0/0`，safe streaming=`2/0/0`，Server config=`65/0/0`；PG17.11 **trust Unix socket**（不是新增 SCRAM 证据）真实竖切=`2/0/0`：BeginRun→relay→scope context→HTTP/SSE→reasoning/text journal→assistant/completed；stored/env/corrupt/missing、active exact match、created_at/id tie-breaker、role 热更新均实得。`cargo clippy` 六 crate all-targets/all-features `-D warnings`、safe-dialer guard 绿；Cargo.lock 仍 428 packages。env 新增 2 条、上游 tests 转 done 10 条：env=`37/36/73`、tests=`184/863/1047`、API=`26/130/156`、总 parity=`305/1350/1655`，0 violations，fixtures=`10/22/32`，Actions 未派发。仍未完成 Anthropic/Google、三家 recorded/live trace、429/5xx retry、完整 usage/cost/deadline audit、真实 tool loop/remember、remote AG-UI/RMCP/Drive/browser/file/shell；因此 G4 整关不勾，G3 只勾真实 provider producer 子项。详见 `docs/2026-08-24-G4-Rust-Agent-OpenAI-provider-batch6.md` |

| R70 | §6.4 / §7.2–§7.5 / §8.6 / §15.4 / §24 G4（2026-08-24 三 Provider + retry/budget/audit batch 7） | R69 只有 package OpenAI；managed 插槽仍会缺 provider，且 Agent row 没有权威 route 时存在误触 package key 的风险。若凭当前 SDK 印象实现 Anthropic/Google，会把 Google API key 放 query、假设并不存在的 response id，或把 Anthropic system/tool/usage 形状翻错。原 retry 草稿若对已见 response identity/delta 的断流重放，会重复非幂等 sampling；token cap 若只发给 vendor 而 host 接受无 usage 的 Completed，则 adapter 漏报即可绕过。deadline/stall 只有 terminal/metric 没有 durable audit，也不能证明停止发生在记录之前 | 新增独立 Anthropic Messages 与 Google streamGenerateContent adapter，固定官方协议和锁定发布物 `@langchain/anthropic@1.5.6`、`@langchain/google-genai@2.2.0`、`@google/generative-ai@0.24.1`；Google key 只进 header，缺 response id 时以首 chunk SHA-256 合成 trace id。Tenant package 以封闭 `providerSource=package|managed` 写 Agent configuration，context 每 run 读取，`ProviderRouter` 精确选择且 managed 缺失绝不回落 package。Retry 固定 `@langchain/core@1.2.8` 的 6 次/1s×2/jitter/64s 与 Retry-After，只有 pre-send unavailable 或首事件明确 429/5xx 可重试，response/增量后与 commit-unknown 永不重放。新增 `OPENBOT_PROVIDER_MAX_OUTPUT_TOKENS` 每 sampling cap；三家请求传 cap、adapter 必须给单调 usage，host 再拒绝缺失/重复/不自洽/超限。Agent lifecycle audit 以 production `PostgresAgentAudit` 写既有 hash chain：invoked 在 context/provider 前，stall/deadline 在 child/session drop 后、terminal 前，失败转 reconciliation；payload 只允许稳定 code | 本机定向：Agent=`16/0/0`；Anthropic=`5/0/0`、Google=`4/0/0`、OpenAI=`6/0/0`，provider filter=`44/0/0`；SafeDialer=`14/0/0`；Server agent config=`6/0/0`、main factory=`7/0/0`；application tenant=`19/0/0`、fixture=`1/0/0`。PG17.11 trust 临时实例：Agent/provider/audit=`4/0/0`（package、fresh Vault OpenAI、managed Anthropic+Google 且 package call=0、真实 deadline/stalling SSE 的四行 hash chain），tenant sync=`8/0/0`；这不是新增 SCRAM 证据。六 crate targeted Clippy `-D warnings`、fmt/diff、safe-dialer guard 全绿，Cargo.lock 仍 428 packages。strict recount=`153/153/0`；env=`49/25/74`、events=`7/67/74`、总 parity=`320/1337/1657`、fixtures=`10/22/32`，0 violations/warnings。未使用 live vendor credential，三家 recorded vendor fixture 仍 0/3；完整 run-wide token/cost/并发/computer budget、真实 tool loop/remember、remote AG-UI/RMCP/Drive/browser/file/shell 仍未完成，故 G4 整关不勾。详见 `docs/2026-08-24-G4-三Provider与Retry审计-batch7.md` |

| R71 | §4.3 条 7–11 / §7.2–§7.4 / §8.1–§8.6 / §24 G3、G4（2026-08-24 真实 tool loop + remember batch 8） | R70 的 parser 能产 tool call，但 host 收到第一条就固定 `tool_loop_unavailable`；`PostgresAgentContextSource` 又明确拒绝任何 assistant/tool history，production `OpenBotApplication` 仍注入 `NoToolControlPlane/NoToolJournal`，所以 8-step 只是 reducer 单测，不是产品能力。若直接拿 provider call id 当 `tool_calls` 主键，模型/vendor 就能铸造 control identity；若 outcome 后只在内存 append result，crash 会丢 tool pair且 terminal 会重复 materialize 前一 sampling 文本。更隐蔽的是 fresh AuthContext 与真正 memory INSERT 之间仍有撤权竞态：capability mint 后 generation 改变时，单次前置查询不足以阻止旧权限写入 | 新增 first-party `remember` catalog：OpenAI strict-compatible closed schema 与 metadata 共用 schema hash，effect=write/non-idempotent/parallel_safe=false；参数只有 kind/scope class/content/tags/sensitivity，owner/Bot/thread/source/expiry 无自报入口。host 先收齐 complete batch，按 stable index 串行，跨 sampling 累计 8 步；provider id 只配对 transcript，`AgentToolGateway` 另铸 UUIDv7/per-run seq。Production main 注入 `PostgresBuiltInToolControlPlane + PostgresToolJournal + AuthorizedAgentToolGateway`；每次 effect 前从 run/lease/users/roles/revocation 重建不可序列化 AuthContext。application 在 capability 后把 tenant/run/thread/AuthGeneration 封进 `AuthorizedToolCall`；memory writer 同一 INSERT transaction 再比较 generation/revoked/role。success 写 `origin=remember_tool`，fact source 自动取当前 run user message；definite failure/refusal 与 success 分写三类 memory audit。每个 outcome 以两条 message + checkpoint 同事务写，前一 sampling text 只 materialize 一次；context 严格验证 callId/name 闭合 pair，三家 adapter 回注后再 sampling。exact replay 核对 message/payload，tampered result conflict；unknown/外层 tool cancellation 进 reconciliation | 本机：Agent=`18/0/0`（含 reverse-index 两 call 顺序、两次 context/provider）；Application 全包=`119/0/0`；provider filter=`45/0/0`，Server main/config=`7/0/0 + 6/0/0`。PG17.11 trust：Agent/provider/tool-loop=`5/0/0`；remember 单条真实 run 连续实得 committed success、capability 后 generation 变化的 `not_committed` failure、policy deny refusal，memory writer call=2、memory row=1、decision/attempt=2、durable tool pair/checkpoint=3、最终 completed；三 audit 顺序 succeeded→failed→refused，exact replay=true、tampered conflict。既有 PG tool journal=`5/0/0`、memory=`2/0/0`。六 crate targeted Clippy、fmt/diff、safe-dialer guard 全绿；Cargo.lock 仍 428 packages。strict recount=`154/154/0`；events=`10/67/77`、总 parity=`323/1337/1660`，0 violations/warnings，tests/fixtures 不变。provider 是 deterministic external test double，绝不冒充 live vendor trace；三家 recorded/live 仍 0/3，human proof-of-intent/approval GUI、run-wide cost budget、remote AG-UI/RMCP/Drive/browser/file/shell 仍未完成，G4 整关不勾。详见 `docs/2026-08-24-G4-真实ToolLoop与Remember-batch8.md` |

| R72 | §7.1 / §7.5 / §13.1 / §24 G4（2026-08-24 Remote AG-UI 协议与文本生产竖切 batch 9） | R71 后 remote route 仍完全不存在。若直接依赖漂移中的 community SDK，会把其类型带进领域；若只按事件名做无状态 JSON 转发，错误 thread/run、重复 terminal、半截 tool JSON、坏 patch 与 remote 自报身份都可能穿进 journal。更关键的是，第一真源要求 callback token + 10 分钟 signed run assertion 绑定 bot/run/actor/tool-set；在该链未落地前把本地工具发给 remote 就是越权。endpoint 若另造 HTTP client，还会绕过既有 DNS rebinding/redirect/peer/stall 边界 | 逐项读取固定 `@ag-ui/core@0.0.57` 类型产物，`openbot-agent::agui` 固定 33 个 event literal 与 exact RunAgentInput；stateful decoder 要求匹配权威 thread/run 的 RUN_STARTED 第一、唯一 success/interrupt/error terminal，显式/convenience text/tool/reasoning/step 配对，tool JSON object 完整，state/activity 的 RFC 6902 六操作原子；messages/open payload 只作 bounded untrusted data。application 新增不暴露 endpoint/assertion 的 typed remote route 与 raw transport port；infra 只用唯一 SafeDialer + SSE，并按真实 body read gap 测 stall。package-backed remote Agent 每 run 重读 Profile standing role，ProviderRouter 精确选 remote adapter，assistant text/visible reasoning 进入既有 expected-sequence journal/terminal。assertion 尚缺时 production 固定 `tools=[]`，adapter 对无 assertion 的非空工具在网络前拒绝。用户创建 `package_id IS NULL` Agent、customer auth header、callback/assertion/tool grant、interrupt resume 与其余事件 durable/UI projection 继续显式未完成 | 本机：Agent=`28/0/0`（AG-UI 8、remote adapter 2）；infra real loopback POST + 3-byte SSE=`1/0/0`；Server main=`7/0/0`。PostgreSQL 17.11 TCP SCRAM Agent suite=`6/0/0`：production relay/context/router/SafeDialer/任意 5-byte body 分片→reasoning/text journal→assistant/completed，package provider call=0、invoked audit=1。五 crate targeted Clippy、fmt/diff、safe-dialer guard 全绿；Cargo.lock 仍 428 packages。strict recount=`154/154/0`；events 只关闭 lifecycle/text 两条，=`12/65/77`，总 parity=`325/1335/1660`，0 violations/warnings。implementation `d7e99c42fe7631963d283c52a9e81a213db7040b` + exact-schema tightening `7bb240811cac06c0388e2b07fc70ad83c3895e0a`，PR #26 创建后 OPEN/CLEAN/MERGEABLE、Actions=0。完整 official golden/callback/SSRF 注册/恢复尚未闭合，故 G4 整关不勾。详见 `docs/2026-08-24-G4-Remote-AGUI协议与文本竖切-batch9.md` |

| R73 | §3.4 / §6.4 / §7.1 / §7.5 / §8.6 / §24 G4（2026-08-24 Remote callback 双凭据 batch 10） | R72 把 remote request 接通却因 assertion/callback 未实现而只能发空工具。固定上游 token 只有 prefix 的宽松长度判断，run assertion 只绑 bot/actor/run/exp 且仍接受 deployment-wide `AGENT_TOOL_TOKEN`；后者与第一真源 §3.4 删除共享凭据的裁决直接冲突。若 token hash update 与 audit 分事务，会出现可用 credential 没有 trail；若 assertion 不绑 whole tool set，remote 可在同一 run 借后来增加的 grant。另一个实现坑是把一次性 token 塞进可 Clone/默认 Debug 的通用 reply，会在 trace/test/GUI 中制造不可追踪副本 | 新增纯 deterministic remote credential 领域：token 精确 `obot_agt_` + 32-byte base64url-no-pad；hash-only + constant-time compare；canonical sorted/dedup/length-framed tool-set SHA-256；HMAC 机制与固定 Node 完全互操作，同时 payload 新绑 version/deployment/tenant/toolSetHash/iat，且 `exp=iat+600000`。DB clock mint 后写 RunAgentInput。一次性 response 无 Clone/Display、Debug redacted、drop zeroize。typed ApplicationService issue/revoke 要 fresh Origin；PG transaction 重读 owner/admin/type/tenant/deleted/AuthGeneration/revoke/role，hash mutation 与 issued/revoked audit 同事务。Production callback 同验 per-Agent token、assertion、token owner=Bot、active run/thread/member/lease/current role 与 current tool-set；credential/refusal audit 只存 stable code、无 actor/Bot/tool/token/run/args。共享 token 生产分支为 0。当前 authoritative tool set 仍 empty，故任意 callback 在 executor 前 404；`api-agent-tools-call-post` 与真实 success 保持 todo | 本机：fixed Rust/Bun HMAC 向量逐字符相等，Rust assertion 被固定上游 `readRunAssertion` 回读同 bot/actor/run。contracts=`67/0/0`、domain=`345/0/0`、application=`119/0/0`、Agent=`28/0/0`、Server lib=`168/0/0`、main=`7/0/0`、transport parity=`7/0/0`。PG17.11 TCP SCRAM：callback infra=`2/0/0`、real session→Axum→ApplicationService→PG callback HTTP=`1/0/0`、Agent runtime=`6/0/0`。forced audit failure rollback、owner/admin/cross scope/stale generation、unknown/expired/missing/mismatch/revoked、empty tool set 与 canary=0 全实得。七 crate Clippy、contracts WASM、fmt/diff/SafeDialer guard 绿；Cargo.lock 仍 428。strict recount=`154/154/0`；API=`28/128/156`、events=`15/62/77`、tests=`206/841/1047`、G2 子队列=`150/84/234`、总 parity=`352/1308/1660`，0 violations/warnings。implementation `2fd1c6ff04d9bb544f4765d6fb67291f982178e4`，PR #27 创建后 OPEN/CLEAN/MERGEABLE、Actions=0。RMCP/Drive、非空 grant/callback outcome、跨副本 call sequence、outbound auth header 与 GUI panel 仍未完成，G4 整关不勾。详见 `docs/2026-08-24-G4-Remote-Callback双凭据-batch10.md` |

| R74 | §7.4–§7.5 / §8.1–§8.6 / §9.1–§9.3 / §15.3 / §16.3 / §24 G4（2026-08-25 RMCP 生产工具面 batch 11） | R73 的空 tool set 与 404 是正确 fail-closed，但也证明 server-side tool parity、非空 callback success 和跨 replica sequence 都不存在。若直接用 RMCP 自带 reqwest/长驻 pool，会绕 SafeDialer 与 actor/Bot identity；只按 name 查 live list 会在 schema drift 后继续使用旧 grant；URL/vendor/provenance 改变若不进入 grant identity，可把旧权限移到新 endpoint。任意 JSON Schema 若开 HTTP/file resolver 或回溯 regex，会引入 SSRF/DoS；tools/call timeout 若一律说 NotCommitted，会把已发送非幂等 effect 自动重放。credential-backed server 在 OAuth/bearer 尚未接线时也不能因存在 grant 而带空 Authorization 出网 | 固定 rmcp 3.1.4 与 MCP 2026-07-28，production 只开 client Streamable HTTP；自定义 backend 只经 SafeDialer，per-operation initialize→list/call→close。15s list、60s call 由 RMCP cancellable RequestHandle 带 progress token；post-send timeout/transport 恒 CommitUnknown reconciliation。live list 同 session 比 reviewed schema hash。native 0017 expand-only 增 durable run sequence、catalog generation/schema/effect/availability、grant state 与 endpoint+vendor+provenance fingerprint；missing/schema/effect/transport 变化同事务 suspended_missing+audit，重新出现不自动启用，refresh 前 read 也会因 fingerprint drift 立即拒绝。jsonschema 0.51.0 关闭 HTTP/file resolver，固定线性 regex、blocking pool 与 4096 cache cap。Context、signed whole tool set、callback auth 共用同一 catalog；callback 与 built-in 共用 Postgres sequence、AuthorizedAgentToolGateway、ApplicationService decision→attempt→capability→execute→outcome。read 可执行；acting 工具无真实 human proof 时写 approval_denied 且不执行。known secret 形状出网前 content_secret_blocked；vendor content 标 untrusted provenance；audit 的 Bot 只取权威 decision。production 仅暴露 credential_id NULL 的 public HTTPS server，credential-backed OAuth/bearer 构造性不可见，不能冒充已接 auth | 本机最终直接证据：mcp_protocol=5/0/0，callback HTTP=2/0/0，native0017=2/0/0，既有 Agent runtime=6/0/0；infra+server PG17/SCRAM 完整矩阵在最终收紧前 619/0/0，收紧后 impacted 5+2 与 all-targets Clippy 再绿。strict recount=154/154/0；API=29/127/156、events=18/59/77、tests=211/836/1047、G2 子队列=155/79/234、总 parity=361/1299/1660，fixtures=11/22/33，0 violations/warnings。Cargo.lock=460、production reqwest=0；deny/audit/vet 全绿，vet=15 audited/435 explicit exemptions；RMCP/SafeDialer/SAML/其它 delta guards 全绿。implementation 09e708b5bb16d4a43345720c3c8d45fac1d08291，PR #28 OPEN/CLEAN/MERGEABLE、Actions=0。MCP OAuth/bearer、run/user cancellation notification、专用 private egress 管理配置、运行期 refresh/admin effect/grant UI、真实 approval、Drive/browser/file/shell、其余 AG-UI durable/UI 与 G5–G8 仍未完成，故 G4 整关不勾。详见 docs/2026-08-25-G4-RMCP生产工具面-batch11.md |

| R75 | §2.4 / §6.4 / §9.2–§9.4 / §14.3 / §15.3 / §24 G2、G4（2026-08-25 MCP OAuth 生产连接面 batch 12） | R74 正确隐藏 credential-backed server，但也证明生产只能匿名调用；若把上游静态 token URL/内存 signed state 直译，会缺 PRM、issuer mix-up、RFC8707 resource、跨 replica single-use、refresh rotation 与 401 上限。更危险的是 client rotation 若不进入 grant identity，会把旧权限原样搬到新 credential；disconnect 若先等 vendor 或把 503 当回滚，会继续保留本地可用 refresh token。Server callback 若从 incoming Host 拼 redirect 又产生开放重定向/错注册。 | 新增 SafeDialer-only `McpOAuthClient`：无凭据 initialize probe 优先读 401 `resource_metadata`，再走 RFC9728 path/root fallback；PRM exact resource/issuer → RFC8414/OIDC 三路径 discovery，逐字 issuer、S256、client auth、HTTPS、RFC8707 authorization/token resource。管理员 OAuth client 输入用共享 zeroizing secret DTO，fresh Origin+Application admin gate 后才 discovery/v2 Vault；同事务推进 native 0018 server credential generation、把既有 grant 固定旧代际、失效 catalog、退役旧 client/user connections并写 audit。Server/Desktop Remote begin 用 OS CSPRNG state/verifier；state 只以 tenant/deployment HMAC identifier + AES-256-GCM attempt 落 `verifications`，callback DELETE RETURNING 先烧、再验 DB clock/AuthGeneration/client id/resource/RFC9207 iss/PKCE/exact redirect。code response 必须含 refresh；pointer switch/旧 token tombstone/connected audit 同事务，短 access 只给同一 MCP catalog。每 operation refresh 必须返回不同 refresh，CAS+scope+rotated audit commit 后才释放 bearer；resource 401 只再 refresh/retry 1 次，403 不循环。disconnect 先 revoked_at+删 join+vendor_revoked=false audit，之后 RFC7009；失败保持 `revocation_pending`，multi-replica `FOR UPDATE SKIP LOCKED`/crash lease 周期重试，成功才写 vendor_revoked=true。People offboarding 同样进入 pending。typed GET/connect/callback/register 与新增 DELETE disconnect 已挂 Server；无 HTTPS public URL 只禁 begin，不禁注册/list/reconcile。 | 本机直接证据：contracts MCP=2/0/0、application MCP/admin=2/0/0、infra OAuth/discovery=3/0/0、redirect/expiry=3/0/0、Server plugins=3/0/0、main=7/0/0；PG17/SCRAM `mcp_oauth_runtime`=2/0/0（真实 PRM/AS/code/token/revoke/RMCP，tamper/wrong-key/replay/expiry/mix-up token call=0，四次 refresh CAS，401 恰一次 retry，pending→reconcile）；native0017/0018=4/0/0；既有 MCP=5/0/0、per-user credential=11/0/0、callback=2/0/0。六 crate all-targets Clippy、contracts/UI WASM、fmt/diff 与全部既有 guards 绿；`cargo xtask ci` 未运行、Actions=0。parity=`394/1267/1661`：API=`34/123/157`、events=`21/56/77`、tests=`236/811/1047`，G2 队列仍 `155/79/234`；fixtures=`12/22/34`。Desktop Local installed-app client/system browser/random loopback listener 尚未接；MCP private egress、admin add/refresh/grant/effect 完整面、run/user cancellation notification、真实 approval、Drive/browser/file/shell 与 G5–G8 仍未完成，G2/G4 整关不勾。详见 `docs/2026-08-25-G4-MCP-OAuth生产连接面-batch12.md`。 |

| R76 | §2.4 / §5.2 / §8.1–§8.6 / §9.2–§9.5 / §14.3 / §15.3 / §24 G4（2026-08-25 Google Drive REST 生产面 batch 13） | R75 已有 actor OAuth 与 RMCP，但 `google-drive-rest.test.ts` 14 条和目录判据仍全 todo，生产没有 Drive transport、Google OAuth 或静态 catalog；把 Developer Preview MCP 当 GA 会稳定遭项目级拒绝，把 Google refresh 强制当 RFC 风格轮换又会误拒其正常“只回 access token”响应。若 Drive 只在 adapter 单测存在而没进入同一 grant/policy/decision/audit/Agent 边界，仍是 test-only runtime；若 read 正文写 DB/索引，又违反 §9.5。 | native 0019 给 `mcp_servers` 追加 nullable closed transport，legacy NULL=mcp；compile-time 目录只认 `google-drive`→GA `www.googleapis.com/drive/v3`/Google/first-party/user-OAuth/drive.readonly。`GoogleDriveRestTransport` 只经 SafeDialer，静态 search/recent/metadata/read 四工具；q 双转义、25 条、Doc/Sheet/Slides export、文本 allowlist、二进制 metadata-only、8MiB wire/20k model cap、HTTPS vendor link与 adapter provenance，不缓存正文、不建 ACL/index。Google web-server OAuth 固定 auth/token/revoke、S256/offline consent、client_secret_post、exact scope；code 必须 refresh，refresh 可按 Google 正常语义保留旧 refresh，偏 scope 仍 fail-closed。一次性 attempt v2 绑定 transport；callback/catalog/broker/revoke/Agent 按 closed transport 分派，Drive 仍走同一 CEL→decision/attempt→capability→outcome/audit，首个 401 只 refresh/retry 一次。新增 typed curated POST 与 Drive refresh；body 无 URL。local-first disconnect/reconciliation 复用 R75。修复 catalog refresh commit 后仍占 pool connection、外层 add 又占一条导致小池第三次 acquire 死锁。 | 本机直接证据：`google_drive_runtime`=30/0/1 + PG17/SCRAM 端到端=1/0/0（真实 add/register/code/refresh、actor A/B、grant、Agent 第一次 401/第二次成功、vendor link/provenance、正文 DB canary=0、public drive relation=0、Google 无 refresh rotation、disconnect 立即 deny、503 pending→revoke）；native0019=2/0/0，fixture 41表/369列/200约束；Server plugins=4/0/0、application admin=2/0/0、infra Drive/OAuth=3/0/0。既有 MCP OAuth=2/0/0、MCP catalog/tool=5/0/0、per-user credential=11/0/0 全绿。五 crate all-targets Clippy、contracts/UI WASM、fmt/diff 绿；strict recount=154/154/0，parity=`424/1237/1661`（tests=`265/782/1047`、API=`35/122/157`），fixtures=`13/22/35`，0 violations/warnings；`cargo xtask ci` 未运行、Actions=0。未使用真实 Google credential，`drive.readonly` restricted-scope verification/security assessment 是外部发布前置而非本机通过项；Desktop Local OAuth、custom MCP/private egress、通用 refresh/grant/effect GUI、approval/cancel、browser/file/shell 与 G5–G8 仍未完成，G4 整关不勾。详见 `docs/2026-08-25-G4-Google-Drive-REST生产面-batch13.md`。 |

| R77 | §5.2 / §6.2 条 10 / §7.2 / §8.1–§8.6 / §13.2–§13.3 / §14.3 / §15.3 / §24 G4、G6（2026-08-25 Durable Approval 生产面 batch 14） | R76 后 acting MCP 的 production `approval()` 仍固定 Denied；领域虽能比较 binding，却没有 durable request、跨 replica waiter、actor-only decision API 或 GUI presentation。若只让 renderer 回 `{approved:true}`，actor/Bot/run/tool/args/target/generation/policy/expiry 都可被自报；若 approval 不进入 tool decision/audit，无法证明一次 action 用的是哪份批准；若 pending 长存 raw args/secret，又违反 GUI state 与 audit 红线。现有 binding 还漏了 §6.2 明定会使旧 approval 失效的 AuthGeneration。 | ApprovalBinding/Observation 新增 AuthGeneration 与专属 invalidation；ToolApprovalRequest 扩为 Rust call/thread/effect/class/computer+catalog+document generation 与 first-party presentation，raw args 仍只在 private executor envelope。native 0020 新建 `tool_approvals` 并给 `tool_calls` 追加 nullable approval_id；pending-only 16KiB redacted args/change summary，resolved 同事务清 NULL；state/shape/generation/hash/time/FK closed checks。TTL 5 分钟（新增），DB clock inclusive expiry；`once_per_run` 只复用全 binding exact match，`every_call` 不复用。Postgres coordinator 先重读 run/member/lease/AuthGeneration并把 request+requested audit 同 commit；同进程 Notify + 1s durable poll 跨 replica wait，scope 变化 cancel。typed GET/POST 经唯一 ApplicationService；POST fresh Origin 在 parse 前且 body 只有 grant/deny。grant 后再次观察 actor/catalog/policy；ToolJournal 在 decision+attempt 事务里要求 approval 仍 granted/未过期且 actor/Bot/run/tool/args/target/effect/class/catalog/policy 全匹配，写 approval_id 后才 mint capability。grant/deny/expire/cancel 五 audit 进入既有 hash chain，outcome audit 同带 approval id。Server main 注入同一个 Arc；`openbot-ui` 只新增 authority-only card projection，不把它冒充 Leptos GUI。 | 本机：native0020 PG17/SCRAM=2/0/0，fixture=42表/398列/291 NOT NULL/217约束/85索引；approval runtime=1/0/0（actor A/B、wait/wake、once reuse、every deny、expiry、AuthGeneration cancel、resolved summary=NULL、五 audit、payload canary=0）；MCP protocol=5/0/0，其中真实 acting call pending→grant→approval-linked decision/audit→vendor 恰一次，第二 call deny→attempt 0/vendor不增。Server approval framing=1/0/0；domain approval=7/0/0、audit payload=11/0/0、event=3/0/0；application tool=9/0/0；UI projection=1/0/0。七 crate all-targets Clippy与 contracts/UI WASM 绿；strict recount=155/155/0，parity=`432/1237/1669`（API=`37/122/159`、events=`26/56/82`、tables=`55/0/55`、tests仍`265/782/1047`），fixtures=`14/22/36`，0 violations/warnings；`cargo xtask ci` 未运行、Actions=0。实际 Leptos component/keyboard/AX/golden、critical realtime delivery、browser current computer/document generation、完整 run/user cancel 仍未闭合，故 approval GUI、G4/G6 整关不勾。详见 `docs/2026-08-25-G4-Durable-Approval生产面-batch14.md`。 |

| R78 | §5.2 / §6.2 条 10 / §7.4 / §13.1 / §16.3 / §24 G4、G6（2026-08-25 G6 UI 地基与可点击 Approval batch 15） | R77 只有 authority-only card projection，仍没有 Leptos App、真实 bundle 或静态宿主。直接使用 Trunk 默认 initializer 会生成内联 module script，违反 strict CSP；让 Tailwind 与 build.rs 并发生成 ignored tokens.css 会在 clean checkout 竞态；只抽 `wasm-opt` 又会漏 macOS 相邻 dylib。Axum 若对所有 404 回 index，会把拼错的 API/asset 隐成 200；若 static route 在统一 layer 后追加，会绕 request-id/tracing/metrics/body limit。浏览器实测还发现 Leptos bool ARIA 输出空值/省略 false、未来 expiry 被读成“已过期”、1024px DOM min-width 与经典滚动条槽形成 15px 横溢。这些都不能靠“页面能打开”裁决。 | 钉 Tailwind 4.3.3/Trunk 0.21.14/Binaryen 132/wasm-bindgen 0.2.127 的平台 URL、整包/成员 SHA 与 version regex；tools fetch 支持大文件分段后仍校整包，Binaryen 同取 bin+lib。build.rs 与 Trunk pre-hook 共用唯一 `build_support/assets.rs`，post-hook 从 hashed bindgen pair 生成固定 external `openbot-bootstrap.mjs`，bundle gate 拒 inline/remote/eval/storage。`StaticApp` 启动时 canonicalize/bounded UTF-8/固定 marker/恰一同源 bootstrap，Server 只在 APP_DIST_DIR 配置时挂 Tower ServeDir；index 由封闭 theme/locale enum 改写并发 strict CSP/security headers，SPA fallback 排除 API/health/metrics/fonts/.well-known/有扩展名路径，且先挂 static 再统一 layer。Leptos `/approvals` 每秒读 typed API，只画权威 effect/target/redacted summary/change，POST 只含 id+grant/deny；ThemeToggle 用 APG radiogroup roving tabindex，LocaleSwitch 用 APG menuitemradio，ARIA 一律显式 true/false。test-only fixture 只复用 production router/UI 做 golden/AX，不作为 backend 证据；R77 的真实 PG/MCP 证据仍单独承担生产决议。供应链对 30 build.rs、2 许可、2 Windows import archive、2 unreachable maintainer scripts 与 2 compile-time unmaintained advisory 做精确 hash/consumer guard；不自动生成 cargo-vet exemption。 | 本机：UI=`19/0/0`、Server static=`4/0/0`，UI WASM 与 UI/Server/fixture/xtask Clippy `-D warnings` 绿；tools verify 四项绿；i18n=`261`、design-lint=`18 Rust/74 icons`、css-check=`43 classes`、bundle=`wasm gzip 359136/3670016, css 27002/98304, fonts 740216/819200, external 1/inline 0`；同源二次 Trunk build 八文件 SHA 逐项相等。真实 Chromium：theme Arrow/Home/End + locale menu/焦点/lang、approval click→card 0+status、1440×900/1024×640（后者 scrollWidth=clientWidth=1009）、main/nav/h1=1/1/1、duplicate id/heading jump/unnamed focusable/remote resource=0；curl 实得 CSP 与全安全头。dependency guard 绿；cargo deny offline exit0（errors=0，30 duplicate+1 OFL warning）、cargo audit 1225 advisory/640 deps exit0；cargo vet **185 unvetted** 保持红且 config 未改。parity=`436/1233/1669`：API=`38/121/159`、UI=`3/149/152`，fixtures=`14/22/36`，0 violations/warnings；strict recount=`157/157/0`。implementation `6c595660758542969cce37f87f3637fe6c9fdf59`；`cargo xtask ci` 未运行、Actions 未派发。Desktop/Tauri、偏好持久化、完整 routes/components/golden/a11y 与 cargo-vet 审计仍缺，故 G4/G6 整关不勾。详见 `docs/2026-08-25-G6-UI地基与Approval可点击-batch15.md` 与 UI 供应链 delta 文档。 |

| R79 | §5.2 / §7–§8.4 / §13.1–§13.3 / §14.3 / §15.3 / §16.3 / §24 G6（2026-08-25 UI 偏好与 Tauri custom protocol batch 16） | R78 的主题/语言只能改当前 DOM，reload/跨设备不保留；Desktop 没有本地偏好或 Tauri production host。若用 localStorage，会违反首帧零 JS、Server 跨设备与默认 typed lane 无 codec；若 renderer 自报 actor/fresh 或直接调业务，又破坏唯一 ApplicationService/authority 边界。供应链初看 Cargo.lock 全联合给出 15 advisory、33 build.rs、367 unvetted，还把 Linux GTK 当 Desktop；但 GUI 第一真源 §10.1 只发行 macOS arm64/Windows x64 Desktop，Linux 是 Server/Web，按联合图裁决会把不可同时成立的 target edge 拼成伪风险。 | 新增 closed `UiPreferences/UpdateUiPreferences`、typed Get/Update command/reply 与唯一 port。native 0021 新建 deployment/tenant/actor 复合 PK，theme/locale independently optional + nonempty/closed CHECK、actor cascade；PostgreSQL COALESCE partial upsert 以 DB clock 原子合并。Server GET authenticated、PUT Origin-before-body、no-store，closed HttpOnly SameSite=Lax cookie；UI startup read 以 interaction revision 防旧读覆盖，partial write 单队列合并且失败显式 alert。Desktop Local 用 256-byte 三行 closed file、0600、temp+fsync+rename+dir fsync；默认 typed lane no JSON。opt-in Tauri 2.11.5 custom protocol 以 webview label 绑定 AuthContext，unbound asset=401，preferences/approval 复用 typed transport，fresh 为 monotonic deadline；本地设置/OS locale 首帧改写、canonical asset/8MiB/closed MIME/CSP。五条 host-only dep 与模块只在 macOS/Windows；`TEST_SWIFT_RS=false force=true` 封死 dormant toolchain。13 个真实 build.rs、9 个 WebView2 payload、MPL/UNIC record 全 exact guard；六 target bans/sources 独立扫描，避免 cargo-deny cross-target parent/child 假组合。 | 本机：contracts/application/Server=`1+1+1/0/0`，Desktop local/no-codec/Tauri=`2+1+2/0/0`；PG17/SCRAM native0020+0021=`4/0/0`，schema0021=`43表/404列/222约束/86索引`、SHA `fab4e148…e3531`；UI=`19/0/0`、WASM/Clippy 绿，i18n=`262`、design=`19 Rust/74 icons`、css=`44`、bundle wasm gzip=`364812`/CSS=`27137`/fonts=`740216`/external-inline=`1/0`。Chromium keyboard/reload 与 cookie/index loopback 绿。dependency guard、六 target bans/sources、fmt、Desktop Tauri Clippy 绿。macOS/Windows license 各剩 MPL×5，advisory 各剩 runtime UNIC unmaintained×5；Cargo Vet target 为 `270/269`（基线181，净增89/88），config/exemption 零改。parity=`440/1232/1672`：API=`40/121/161`、tables=`56/0/56`、UI=`4/148/152`；fixtures=`15/22/37`，0 violations/warnings；strict recount=`157/157/0`。WIP commits `17d43eb`、`9a485315`；未运行 `cargo xtask ci`、未派发 Actions。外部 identity/tauri.conf/binary/window lifecycle/两平台原生发行、MPL/UNIC/Vet、其余 route/component/golden 仍缺，G6 不勾。详见 Batch16 实施与 target-graph delta 文档。 |

| R80 | §5.2 / §9.2–§10.4（GUI 第一真源）/ §13.1 / §16.3 / §21.1 / §24 G6（2026-08-25 十个基础原语与设计画廊 batch 17） | R79 后 UI 仍只有四条 ledger 原语；Settings/People/Memory/Plugin 等 route 缺共同表单/列表地基。若组件 callback 暴露 MouseEvent，会把输入设备细节带入 feature；直接 `provide_context` 会让 sibling Field 覆盖 ID；自由 Item href/tag 会开同源/XSS 漂移；Textarea 若把“十行×line-height+padding+border”公式留在手写 CSS，会分裂 token 真源并无必要地假定三引擎 typed arithmetic。另一个缺口是第一真源要求 `/_design` 仅开发存在，但旧 design-lint 简单禁止源码出现该字面，构造上无法同时拥有 gallery 与 production exclusion。 | 实现 Button/Field/InputGroup/Input/Item/Label/Separator/Skeleton/Switch/Textarea 十条 production primitive。Button/Item activation 统一 unit callback，显式 Enter/Space 且 disabled/loading 前拒；Field 用 scoped Provider 铸 control/description/error ID，nested control 自动 aria/disabled；ItemAction 只允许 bounded same-origin absolute Link 或 Button，selected 自动 check；Separator semantic/decorative+双方向；Skeleton 三 shape 永久 aria-hidden；Switch explicit aria-checked/Space；Textarea scrollHeight autosize + tokens.toml 单源 218px 十行 cap。新增 compile-only `design-gallery` feature：cfg route 承载全部状态；design-lint 只允许精确 guard，bundle-budget 扫 production WASM `_design` byte 非零即红。Gallery 不入 production/golden。修复 `transport_parity` 无 wildcard match 漏登记 Batch12–16 十命令：逐个明确由专项证据承担，不用 `_`。 | UI all-features=`30/0/0`，WASM、UI/testkit all-targets Clippy、fmt 绿；i18n=`275`、design=`29 Rust/74 icons`、css=`67`，production bundle wasm gzip=`371205`/CSS=`36123`/fonts=`740216`/external-inline=`1/0`，production/gallery `_design` bytes=`0/1`。Chromium：Button/Item Enter+Space 各 `0→1→2`，disabled 不增；Switch `false→true`；Field label/described/error exact；InputGroup 真 focus；Textarea 16 行 `216/218/374` capped；decorative/双方向 separator、三 skeleton AX；duplicate ID/unnamed/nested interactive/remote/overflow=`0/0/0/0/0`，main/nav/h1=`1/1/1`，current bundle errors=0。parity=`450/1222/1672`，UI=`14/138/152`，fixtures=`15/22/37`，0 violations/warnings；strict recount=`157/157/0`。Cargo.lock package delta=0；implementation `1ebccb9c7d14fa39566884d514f6a8052ba2c0f0`；未运行 CI/Actions。其余13原语、45业务组件、31 routes、全 golden/AX 仍缺，G6不勾。详见 Batch17 文档。 |

| R81 | §5.2 / §6.4 / §9.2–§10.4（GUI 第一真源）/ §13.1 / §16.3 / §21.1 / §24 G6（2026-08-25 展示身份与反馈原语 batch 18） | R80 后仍缺 transcript 的 article/bubble、跨平台快捷键、确定性头像、非阻塞反馈与 tooltip。若 Avatar 允许 remote src 会绕 CSP/egress；若 palette 运行时随机，golden 不可复算；Primary modifier 硬编码 ⌘ 会在 Windows 错；Toast 单纯 setTimeout 会让旧 timer 关闭重开的新状态；Tooltip 只用一个 open bool 会在 hover+focus 重叠时 mouseleave 错关，且自由 trigger URL/tag 再开输入面。 | 实现 Message compound（article/group/avatar/content/header/footer，start/end）、User/Assistant neutral borderless Bubble；Kbd modifier/key 闭集，WASM platform 将 Primary/Alt/Shift 映射 Apple symbols 或非 Apple names，特殊键 i18n；Avatar 只接 same-origin path 或 initials，SHA-256 principal 前32bit mod8 选 tokens.toml 亮暗 palette，外层单次 role=img；Toast exact5s/status/polite/manual+generation 与 ToastViewport；Tooltip scoped Provider、bounded Link/unit Button、hover/focus 双 signal+400ms generation、Escape/blur/leave，compile preview。remote image/link 负向拒绝。 | UI all-features=`40/0/0`、WASM/Clippy/fmt 绿；avatar 16 个 contrast pair 绿；i18n=`297`、design=`36 Rust/74 icons`、css=`85`，production bundle wasm gzip=`371457`/CSS=`40781`/fonts=`740216`/external-inline=`1/0`。Chromium/AX：articles Ada:start/张:end；Bubble border0；avatars initials/sizes/palette=`AL32#1/张24#3/AL40#1`；Kbd=`⌘K→Command K`+本地化 special keys；Toast stale/new deadline=`2 still visible→3 hidden`；Tooltip immediate hidden→450ms role+described→Escape/blur hidden；duplicate/nested/remote/overflow=0，final bundle errors=0。parity=`456/1216/1672`，UI=`20/132/152`，fixtures=`15/22/37`，0 violations/warnings；strict recount=`157/157/0`。Cargo.lock packages仍822，仅加已锁 wasm-bindgen direct edge；implementation `6ea3151818c7ea815636aed5edb6f10d272a44b2`；未运行 CI/Actions。MessageScroller/六复杂原语、业务接线/routes/golden仍缺，G6不勾。详见 Batch18 文档。 |

| R82 | §5.2 / §9.2–§10.4（GUI 第一真源）/ §13.1–§13.3 / §16.3 / §21.1 / §24 G6（2026-08-25 Dialog/Sheet batch 19） | R81 后表单仍无 modal primitive。若 Dialog/Sheet 各写 focus trap，会出现近似不等价的 Escape/Tab/return/scroll；只写 aria-modal 不保证背景在所有 WebView/AX 中 inert；直接 inert app shell 又会把嵌套 modal 自己禁用。自由 side/tag 同样扩大输入面。 | 建唯一 modal kernel：root ID 派生 panel/title/description/close；button trigger explicit haspopup/expanded/controls；panel role dialog+aria-modal；首 focusable、Tab/ShiftTab 首末环、Escape、内置/compound close、backdrop 全走 idempotent close；body overflow 保存/恢复、close/cleanup return focus。背景隔离从 modal layer 沿祖先路径上行，只给每级 path-sibling 加 inert+aria-hidden+modal marker，modal路径不禁；关闭按 marker恢复，HEAD不标。Dialog centered/85svh body scroll；Sheet 只选 presentation 与 closed top/right/bottom/left side，共用同核。 | UI all-features=`42/0/0`、WASM/Clippy/fmt绿；i18n=`306`、design=`39 Rust/74 icons`、css=`93`，production bundle wasm gzip=`369319`/CSS=`44218`/fonts=`740216`/external-inline=`1/0`。Chromium Dialog focusables close/cancel/save，双向Tab环；Escape/Cancel/真实坐标backdrop均关闭、focus回trigger、scroll恢复；open marker16/sidebar inert/head false，close marker0。Sheet right=`360×720`、同焦点环，Escape/Done回focus；四side enum/CSS绿。closed DOM duplicate/unnamed/nested/overflow=0，final `931d30…` errors=0。parity=`458/1214/1672`，UI=`22/130/152`，fixtures=`15/22/37`，0 violation/warning；strict recount=`157/157/0`；Cargo package delta0；implementation `79f696ecdcb1a0da5e30bb14b7ef5e1430c6e968`；未运行CI/Actions。MessageScroller/Combobox/Menu/Select/Sidebar、业务routes/golden仍缺，G6不勾。详见Batch19文档。 |

| R83 | §5.2 / §6.1 / §9.2–§10.4（GUI 第一真源）/ §13.1 / §16.3 / §21.1 / §24 G6（2026-08-25 Menu batch 20） | R82 后仍无 Menu。直接让每层 keydown 冒泡会使 submenu Escape 同时关根层、子层 ArrowDown 又移动根项；item 的 keydown+native click 若不共享边界会执行两次。Tab 若在 keydown 内先隐藏 popup，Chromium 会失去 focus origin；反过来无条件 preventDefault+人工选目标又违反“正常页序”。另一个缺口是只读 aria-expanded/native disabled 会违反 GUI §6.1 的 data-state 单一视觉状态面。 | 建 root/trigger/content/item/separator 与一层 submenu compound；bounded root/sub ID 单点派生 controls/label。trigger native button explicit haspopup/expanded/controls；root/sub role=menu，item role=menuitem tabindex=-1；open/disabled 经 closed data-state helper 同步 native/ARIA/CSS。↑↓ wrap、Home/End、disabled skip、500ms 多字符 typeahead；Right/Left/Escape 逐层，子 content 与父 trigger 对已处理键 stop propagation。Enter/Space/click 单次 unit callback 后同一路径关 root/return trigger；outside dismiss 与 Escape 同 close。Tab 不 preventDefault，零延迟后只在焦点仍留 menu/body 时按方向恢复到 trigger 后一项或 trigger，既保留已有原生落点又保证不落 body。 | UI all-features=`44/0/0`、WASM/Clippy/fmt绿；i18n=`315`、design=`40 Rust/74 icons`、css=`101`，production bundle wasm gzip=`369935`/CSS=`46119`/fonts=`740216`/external-inline=`1/0`，production/gallery `_design`=`0/1`。Chromium root/子层全键位、`mo` 多字符与500ms reset、父项 Escape 隔层、root/child Enter/Space/click exactly once、真实 CUA outside 与 Tab/ShiftTab 全绿；AX 命名 menu/menuitem/disabled/separator exact，duplicate/unnamed/nested/remote/overflow=0，final gallery `42183490…f0daf8f` errors=0。parity=`459/1213/1672`，UI=`23/129/152`，fixtures=`15/22/37`，0 violation/warning；strict recount=`157/157/0`；Cargo package delta0；implementation `0de120f3e41cc120f27defdb2c02a6a0ddec9b25`；未运行CI/Actions。MessageScroller/Combobox/Select/Sidebar、业务routes/golden仍缺，G6不勾。详见Batch20文档。 |

| R84 | §5.2 / §6.1 / §9.2–§10.4（GUI 第一真源）/ §13.1 / §16.3 / §21.1 / §24 G6（2026-08-25 MessageScroller batch 21） | R83 后 Message/Bubble 已有但 transcript 没有滚动所有权。只写“新消息就 scrollHeight”会在用户读历史时抢位置，也跟不到同一末项 streaming growth；prepend 若按 height delta 手补，Chromium native anchoring 已先补时会双移。实测还发现旧流式行收缩+append 的宿主 clamp scroll event 会被误当用户。固定 `@shadcn/react@0.3.0` 的 parent-realm HTMLElement filter 与 mount anchor 未登记形状也不能机械继承；anchor spacer 若每次先清零再设回会自激 ResizeObserver。 | 建 root/viewport/content/item/end-button compound 与 production `scroll_to_end` handle；FollowingBottom/FreeScrolling/AnchoredToMessage 三态。ResizeObserver 管尺寸、MutationObserver 管直接 child-list，rAF 合并；content-change pending + 180ms generation-safe programmatic settling 区分宿主 clamp 与 wheel/touch/scroll-key。稳定态记首 visible item+viewport offset，prepend 只补实际差；mount anchors 全登记、全 IDs replacement 才重置。new user anchor 保留前项48px；spacer 从总高减已知 spacer 得 natural height且同值不写。item 直接用 HtmlCollection Element，不做 cross-realm instanceof。viewport named region/tabindex0；content named log/live polite；button inactive即hidden。 | 锁定 tgz SHA-512 与 bun.lock integrity exact。UI=`47/0/0`、WASM/Clippy/fmt绿；i18n=`327`、design=`41 Rust/74 icons`、css=`109`，bundle wasm gzip=`370410`/CSS=`48046`/fonts=`740216`/external-inline=`1/0`，production/gallery `_design=0/1`。Chromium `9/9+8/8`：initial end、真实wheel yield、free append/prepend/resize position、following shrink+append/stream end、PageUp intent、48px anchor、same-count no jump、named AX/button、DOM五类0、errors0；final gallery `bf75ce97…ce458c2`。parity=`460/1212/1672`、UI=`24/128/152`、fixtures=`15/22/37`，0 violation/warning；strict recount=`157/157/0`；Cargo package delta0，只扩web-sys API feature；implementation `521873c3449a8bd68d541383eb385cc063348897`；未运行CI/Actions。Combobox/Select/Sidebar、ChatTranscript/routes/golden仍缺，G6不勾。详见Batch21文档。 |

| R85 | §5.2 / §6.1 / §9.2–§10.4（GUI 第一真源）/ §13.1 / §16.3 / §21.1 / §24 G6（2026-08-25 Combobox/Select batch 22） | R84 后三原语中 Combobox 与 Select 都依赖 listbox；各写一套会让 disabled skip/Escape/Tab/typeahead/selection/AX 漂移。editable 若拦 Left/Right 等系统键会破坏 IME/文本编辑；Select 若 navigation 即改 committed，Escape 无法取消。实测还发现 owner 自加 `-input/-trigger` 会断开 Field label，aria-labelledby owner 在 Chromium AX 中仍给 unnamed listbox；共享 root 固定180px又会把 channel recipient 输入锁窄。固定上游虽导出 chips/multi-select/scroll arrows，但第一真源只定义 single-value 两条，不能反向扩输入面。 | 建唯一 ListboxContext：open/value/query/committed label/active/empty、Field state、owner/content refs、500ms buffer与callback；visible/enabled discovery、↑↓wrap/Home/End、disabled skip、active scroll containment、click/keyboard single commit、outside/cancel共用。Combobox native input+contains filter/empty，不捕获文本编辑键；Select button-like owner+Space+prefix typeahead+Tab commit。owner保持DOM focus，option tabindex-1，active-descendant/aria-selected；popup接同一reactive owner label。owner直接用root ID，嵌Field要求ID exact并合并disabled/invalid/described。closed data-kind让editable width100%、select-only紧凑；active只滚popup。 | UI=`49/0/0`、WASM/Clippy/fmt绿；i18n=`342`、design=`44 Rust/74 icons`、css=`126`，bundle wasm gzip=`375269`/CSS=`51695`/fonts=`740216`/external-inline=`1/0`，production/gallery `_design=0/1`。最终当前bundle Combobox=`13/13`、Select=`14/14`、smoke=`10/10`；English `pu`/500ms reset、中文contains/prefix纯helper、Enter/Space/click once、Escape/outside/Tab语义、Field exact、named AX、四controls/active targets、DOM五类0/errors0；final gallery `5d871919…123ca7a`。parity=`462/1210/1672`、UI=`26/126/152`、fixtures=`15/22/37`，0 violation/warning；strict recount=`157/157/0`；Cargo dependency delta0；implementation `088625c78a97dc83f8a9190d38fdee09b450b3e1`；未运行CI/Actions。Sidebar、业务接线/routes/golden仍缺，G6不勾。详见Batch22文档。 |

| R86 | §5.1–§5.3（GUI 第一真源）/ §6.1 / §9.2–§10.4 / §13.1–§13.3 / §16.3 / §21.1 / §24 G6（2026-08-25 Sidebar batch 23） | R85 后primitive只剩Sidebar。照搬上游cookie/mobile hook/两份DOM会违反本仓typed preference与duplicate-ID边界；只用CSS复制nav会让mobile/desktop同时存在，Children内Menu IDs冲突。md若直接写collapsed会污染lg偏好。复用Sheet但外部SidebarTrigger不接modal trigger ref时，Escape会把focus丢body；open时resize若只换branch还会残留inert/body lock。gallery在700px又被尚未迁移的旧app.rs sidebar盖住pointer，不能把宿主旧壳误判成新原语失败。 | 建SidebarProvider唯一context：real viewport/user collapsed/mobile open/external trigger/shortcut。DocumentElement ResizeObserver按1024/768选Large/Medium/Compact；lg 240↔48、md强制48不改偏好、compact同一ChildrenFn单挂载进既有left Sheet。Ctrl/Meta+B只在lg/compact prevent+toggle，md no-op；listener cleanup。Trigger动态controls/expanded/disabled；Sheet close/shortcut close/resize exit都经保存NodeRef返焦。Header/Content/Footer/Group/List/Link closed compound，Footer底置；same-origin href、named nav/current/check；rail隐藏visible label但保留aria-label。 | UI=`51/0/0`、WASM/Clippy/fmt绿；i18n=`350`、design=`45 Rust/74 icons`、css=`142`，bundle wasm gzip=`371519`/CSS=`55073`/fonts=`740216`/external-inline=`1/0`，production/gallery `_design=0/1`。Chromium lg/md=`8/8`、compact=`8/8`、Meta+B另测：240/48、md preference、mobile panel240/dialog/nav、markers16→0、scroll restore、Escape/shortcut/resize返焦；AX/controls/current/DOM/scope overflow/errors绿；final gallery `6a7765ad…ffff10d`。parity=`463/1209/1672`、UI=`27/125/152`，27 primitive全done；fixtures=`15/22/37`，0 violation/warning；strict recount=`157/157/0`；Cargo package delta0，只扩web-sys events；implementation `d71347b37c6f581bbdf04e3329dbf670d844a45b`；未运行CI/Actions。旧app.rs shell、45业务组件/31routes/icon/runtime/golden/Tauri release仍缺，G6不勾。详见Batch23文档。 |

| R87 | §4.6.1–§4.6.3（GUI第一真源）/ §9.3 / §10.3 / §12.6 / §16.3 / §19.3 / §21.1 / §24 G6（2026-08-25 图标映射join batch24） | R86后74项manifest/SVG与源码allowlist已绿，但47条UI图标ledger仍todo：既有design-lint没有把§4.6.2的两对/行Markdown、icons.toml upstream_tabler/name/usage与ledger target逐条join，故“46名字已核实”仍只是散文。批量直接打勾又会把唯一brand例外一起误关。旧notes还声称zip缺失/禁网，与Phase0 provenance和已落资产状态漂移。 | design-lint新增确定性关系校验：解析第一真源47映射（单测覆盖一行两对+brand）；校source zip SHA跨文档/manifest声明一致；46 Lucide要求document name=manifest name、ledger Rust enum target=icon_variant、首upstream path∈usage、status done且evidence含`icon-mapping-join=46/46`，并继续执行74 manifest/SVG/currentColor/1.75/unsafe shape。Google Drive精确join到brand/google-drive.svg与brand::GoogleDrive，但brand manifest/ledger必须todo，提前打勾即红。46条旧未验证notes同批改为已验证。 | current rebuilt xtask design-lint输出`46/46 Lucide done; brand1/1 todo; exact`；xtask bin tests=`78/0/0`，openbot-testkit xtask Clippy/fmt绿。UI=`73/79/152`，全parity=`509/1163/1672`、fixtures=`15/22/37`、0 violation/warning；strict recount=`157/157/0`。Cargo/package/production bundle delta0，故未把Trunk/浏览器旧结果冒充本批证据。ENOSPC只清可重建target-xtask1.3GiB+incremental3.2GiB后从当前源码重建。implementation `f23595b0d3b112226167443faee5eec25bfd4f45`；未运行CI/Actions。Google Drive及三sign-in brand assets/条款/provenance、45业务/6runtime/27golden仍缺，G6不勾。详见Batch24文档。 |

| R88 | §4.5 / §5.1–§5.3 / §6.2 / §9.2–§10.4（GUI第一真源）/ §13.1–§13.3 / §16.3 / §21.1 / §24 G6（2026-08-25 layout业务组件 batch25） | R87后primitive/icon地基已闭，但45业务仍全todo，旧Approval又直接手写页框。上游detail=400px、stagger=40ms/12cap与本仓真源token 360px、30ms/8cap冲突，不能照搬。只隐藏detail会跳过尺寸联动且让焦点落body；各写一份像素时长又会分裂tokens.toml。实测还发现精确1024px时经典滚动条占15px，固定两列gallery会在无页面横滚时静默裁360px panel。 | 新建features::layout四模块。PageShell只允许960/1200/768三宽与44px topbar，stable h1/h2、same-origin back、Rows/Empty，并接入production Approval。RowMark收为36px中性vendor tile。StaggerItem只用static class + data-stagger0..8、token 30ms与全局reduce。DetailPanel以present/phase/generation管opening/open/closing/closed，宽度/时长/easing直读生成Rust token，优先WAAPI width/flex-basis/opacity，不可用则同token CSS transition，reduce=0ms；完成后卸载并返焦，Enter/Space preventDefault exactly-once。gallery用sidebar+detail token auto-fit，不增断点。 | UI=`56/0/0`、WASM、UI all-targets Clippy/fmt绿；i18n=`360`、design=`50 Rust/74 icons`、css=`159`，production bundle wasm gzip=`375251`/CSS=`60332`/fonts=`740216`/external-inline=`1/0`。Chromium 1024×640：Approval h1/main/nav=`1/1/1`、960/44px、overflow/duplicate/remote=`0/0/0`；layout panel=360、WAAPI path exit/enter=`160/240`、Enter/Space/click close count=`0→1→2→3`且返焦、named complementary/region/link各1、RowMark=36×36、stagger=`0/ob-list-stagger`、console0。parity=`513/1159/1672`、UI=`77/75/152`、0 violation/warning；strict recount=`157/157/0`。Cargo.lock/package delta0，web-sys只扩MediaQueryList，UI dependency guard绿。implementation `1689471ca65615d6ff332a770f5bfa6f03808666`；未运行CI/Actions。其余41业务/31routes/brand/6runtime/27golden/Tauri release仍缺，G6不勾。详见Batch25文档。 |

| R89 | §4.1 / §4.5 / §6.2 / §6.7 / §9.2–§10.4（GUI第一真源）/ §13.1 / §16.3 / §21.1 / §24 G6（2026-08-25 AgentPresence batch26） | R88后 agents 组仍有两份orb文件todo。照搬437行orb+395行ai-core的canvas/shader/音频会违反§6.7与原则7；只画一个旋转圈又会让thinking/speaking只靠动画区分，reduce后语义消失。1200/160ms若只写CSS会再次分裂tokens.toml。同时app-sidebar虽有channels/me API，但production sign-out仍属Better Auth wildcard todo，roster realtime也未对等；用`/sign`只清界面会冒充session revoke，故本批不勾app-sidebar。 | 新建reactive `AgentPresenceState` 闭集与20px AgentPresence，固定track+primary/secondary arcs DOM；idle完整环、thinking单弧、speaking双弧、error danger完整环，形状+颜色+本地化role=img名称同时传信。tokens.toml新增agent_presence_cycle=1200ms/error=160ms，CSS只消费生成变量；thinking spin infinite、speaking双弧alternate±、error单次位移。全局reduce闸门强制animation:none，静态形状/颜色/名称仍在。compile gallery同屏四态。 | UI=`57/0/0`、WASM、UI all-targets Clippy/fmt绿；i18n=`361`、design=`52 Rust/74 icons`、css=`163`，production bundle wasm gzip=`375421`/CSS=`62691`/fonts=`740216`/external-inline=`1/0`。Chromium 1024×640实得四态均20×20；thinking=`spin/1.2s/infinite`，speaking双弧=`1.2s/infinite/alternate±`，error=`160ms/1`；四个本地化AX name各1，DOM/overflow/remote/console全0。浏览器为no-preference；reduce只记全局CSS+单测构造性证据，不写成media实跑。parity=`515/1157/1672`、UI=`79/73/152`、0 violation/warning；strict recount=`157/157/0`。Cargo.lock/package delta0；implementation `5bb9d1ffe7ca8e0a023a5b07739584deee49cc26`；未运行CI/Actions。motion总项、app-sidebar、其余39业务/31routes/brand/27golden/Tauri release仍缺，G6不勾。详见Batch26文档。 |

| R90 | §4.1–§4.6 / §6.2 / §9.2–§10.4（GUI第一真源）/ §13.1 / §16.3 / §21.1 / §24 G6（2026-08-25 ComputerPlaceholderArt batch27） | R89后computer/placeholder与settings/background仍todo，固定上游两文件却是同职责162行彩色gradient+噪声/filter SVG。照搬会违反中性底色/无彩色背景/无阴影裁决，复制两份又会把defs ID与漂移加倍。装饰图若自报status/live又会伪造运行时语义。 | 新建唯一 `settings::ComputerPlaceholderArt`，保留1200×800/3:2坐标，改为`xMidYMid meet`、fill=none、stroke=currentColor的中性线稿；表面/线条只消费bg-subtle/border/fg-muted/secondary。`computer::ComputerPlaceholder` 只负责3:2 frame并复用Art，wrapper零第二份SVG。零gradient/radial/filter/noise/shadow/defs/DOM ID/style attr/remote/字面fill·stroke色；Art与wrapper均纯装饰AX隐藏，不进Lucide allowlist。 | UI=`59/0/0`、WASM、UI all-targets Clippy/fmt绿；i18n=`362`、design=`56 Rust/74 icons`、css=`168`，production bundle wasm gzip=`375910`/CSS=`63205`/fonts=`740216`/external-inline=`1/0`。源码反向单测实得Art唯一1个`<svg>`、wrapper零`<svg>`，禁止marker全0。Chromium 1024×640：两实例同viewBox/preserve/currentColor、实测约324.5×215.7；defs/gradient/filter/style/id/remote/literal-color=0，AX img/focusable=0，duplicate/nested/overflow/console=0。parity=`517/1155/1672`、UI=`81/71/152`、0 violation/warning；strict recount=`157/157/0`。Cargo.lock/package delta0；implementation `aa2c0a480009c54fd08b9db7210f7e3e483e9a94`；未运行CI/Actions。Computer其余四组件与Screen/G5、其余37业务/31routes/brand/runtime/golden/Tauri release仍缺，G6不勾。详见Batch27文档。 |

| R91 | §5.2 / §6.1–§6.5 / §13.1 / §15.3–§15.4 / §16.3 / §21.1 / §24 G2、G6（2026-08-26 production session sign-out batch28） | R90后app-sidebar的登出仍只能跳`/sign`；生产Server没有sign-out，这只会清界面而不撤数据库session，是安全假完成。同一bundle又服务multi-user与single-user，后者无可撤session；无可判定投影就会显示一个永远无效的登出。直接让handler接token/session id/actor body则重开自报身份与撤别人会话的输入面。 | ResolvedAuth只新暴露has_revocable_session bool，session id仍私有。AuthResolver新增revoke_session；Postgres实现只按已验`(session_id, actor)` DELETE，零wire token/id/actor。新`GET /api/me/session` 只回closed `{revocable}`/no-store，不改`/api/me {user}`；`POST /api/auth/sign-out` 先认证+验trusted Origin，后删当前session，回204与host-only HttpOnly/Lax Max-Age=0 cookie，Secure沿用public URL策略。single-user status=false、POST409，不写假成功。UI WASM helper只解closed DTO/接受204。 | Server framing=`2/0/0`；contracts=`1/0/0`，UI API host=`1/0/0`，contracts/UI WASM与contracts/server/UI all-targets Clippy/fmt绿。临时PostgreSQL **17.11** host SCRAM真库=`1/0/0`：坏Origin两session仍在，正确204后只删session-1，旧cookie请求channels=401、session-2 cookie=200，clear cookie exact。临时集群仅监听127.0.0.1:55428，测后停止并删除。production bundle wasm gzip=`374903`/CSS=`63205`/fonts=`740216`/external-inline=`1/0`。API=`41/121/162`、tests=`266/781/1047`、parity=`519/1154/1673`、0 violation/warning；strict recount=`157/157/0`。G2专项仍155/79/234；T-TEST-0728与新T-API-0162 done，T-API-0107 wildcard仍todo。Cargo.lock/package delta0；implementation `36be5747da2e5501fe2d5103635dd63df03668d0`；未运行CI/Actions。channel events/AppSidebar与其余G2/G6边界仍缺。详见Batch28文档。 |

| R92 | §3.2 / §4.1–§4.3 / §5.2 / §6.2 / §13.2–§13.3 / §15.1–§15.3 / §21.1 / §24 G3、G6（2026-08-26 Channel Activity/WebSocket batch29） | R91后roster仍只能轮询，固定上游`/api/channels/events`与10条activity/event判据全todo。若把memberIds放进NOTIFY/frame，会把完整channel成员表复制到每个renderer；若NOTIFY当真源，断线即永久漏更新；若writer先通知后commit，回滚也会显示幽灵消息；若只在upgrade验一次成员，撤权后的长连接继续收消息。直接落AppSidebar又会把channel行链接到尚不存在的真实destination route。 | `ChannelActivityEvent`为closed bounded projection；channel user begin与assistant terminal只在各自既有PG事务内、且`last_message_at`严格前进时更新`channels.last_message*`并`pg_notify(openbot_channel_activity)`。通知不携memberIds；每条subscription在每帧发送前按已验actor回查当前`channel_memberships`。NOTIFY仅优化，roster为真源，LISTEN失败发稳定错误后断流，UI重连必须refetch。`GET /api/channels/events`要求session+trusted Origin+`openbot.channel-activity.v1`，1KiB inbound，只读Text/Binary=1008，依赖错=稳定frame+1011。Desktop将channel event归Critical，压力时断开/refetch而非静默丢。AppSidebar/channel row继续todo，等真实channel route/journey同批。 | 本机：contracts/application=`71+123/0/0`；Server真实TCP/Axum WebSocket=`4/0/0`（401前置、Origin/protocol、typed frame、1008、error+1011）；infra preview=`1/0/0`、Desktop budget=`7/0/0`。临时PostgreSQL **17.11** host SCRAM真库=`1/0/0`：两member+outsider、同actor双连接、4→3→2 LISTEN清理、撤权后零帧、assistant roster exact、stale零覆盖/零通知、非member/未挂Bot写拒绝；实例只监听127.0.0.1，测后停止并精确删除。五crate all-targets Clippy、contracts WASM、fmt/diff绿；API=`42/120/162`、tests=`276/771/1047`、UI=`81/71/152`、parity=`530/1143/1673`、0 violation/warning；strict recount=`157/157/0`。T-API-0030、T-TEST-0391–0395/0398–0402 done；0396/0397及其余channel route/UI保持todo。implementation `572bb3c81e76dd18867b7925f3e947b45dcf7e38`；未运行`cargo xtask ci`、未派发Actions。详见Batch29文档。 |

| R93 | §3.1–§3.2 / §4.1–§4.3 / §5.2 / §6.2 / §9.2–§10.4（GUI第一真源）/ §13.1–§13.3 / §15.1–§15.3 / §21.1 / §24 G3、G6（2026-08-26 Channel Detail/ChannelRow batch30） | R92后roster realtime已闭，但G1时代`ChannelRepo`仍把thread_id恒置None；GET detail、ChannelRow与destination route缺失。若直接join legacy Intelligence mapping会重新隐藏刚provision的channel；若channel thread沿用direct-bot私有thread_membership，共享channel其他成员能看roster却不能看history/realtime。UI若先画全部AppSidebar链接会生成五类断链；nested route硬刷新又实测bootstrap把`./wasm`按document path解析成`/channel/*.wasm` 404。 | 新增closed GetVisibleChannel/ChannelDetail/Response与ChannelReadScope；list/detail只按current channel membership并投影deployment/tenant/anchor匹配的最新native thread，threadId在尚未开thread时为null。channel-anchor的status/begin/history/replay改按current channel membership，direct-bot保持thread membership；member首次begin补downstream run membership，撤权后stale行不能扩大四面。Axum GET只回`{channel:{id,name,agentIds,threadId,active}}`，missing/outsider统一404。Leptos App shell接真实50条keyset/load-more、visible-field search、localized ChannelRow、socket全量refetch、current user/revocable/sign-out与data-backed `/channel/:id`；完整chat/control不画。bootstrap保持JS module-relative但WASM改root-absolute同源。AppSidebar总项只实现已存在destination，余nav继续todo。 | 本机：contracts/application=`73+125/0/0`；Server channel=`6/0/0`；Axum/in-process parity=`8/0/0`；UI=`63/0/0`；xtask=`78/0/0`。PG17.11 host SCRAM：channel_repo=`7/0/0`、channel_detail/shared thread=`1/0/0`，覆盖foreign scope/deleted、shared member history/event0、revoke error+empty、stale thread membership零扩权。七crate all-targets Clippy、contracts/UI WASM、fmt/diff、dependency guard绿；i18n381、design62 Rust/74 icons、CSS178；bundle wasm gzip544139/CSS65607/fonts740216/external-inline1/0。真实Chromium：50→52、两类search/no-match、socket generation1→4、nested hard reload/current/h1/no fake composer、1440 240↔48、900 auto48、600 Sheet240+3 inert+Escape返焦、overflow0、204→sign且session401、final console0。API=`43/119/162`、tests=`290/757/1047`、UI=`82/70/152`、parity=`546/1127/1673`，0 violation/warning；strict recount=`157/157/0`。T-API-0034、T-UI-0038与T-TEST-0385–0390/0396–0397/0411/0416–0420 done；AppSidebar总项/T-ROUTE-0009/create/activity/control仍todo。Cargo.lock package delta0，只增既有futures-util target edge；implementation `33273d135ee16e70519f727a1d863c229567a0e7`；未运行CI/Actions。详见Batch30文档。 |

| R94 | §3.2 / §5.2 / §6.6（GUI第一真源）/ §13.1–§13.3 / §14.1 / §15.1–§15.3 / §21.1 / §24 G4、G6（2026-08-26 Agent Roster/Agents route batch31） | R93后`agents/agent_profiles/agent_preferences`已有表与callback写面，但list/detail API、纯profile权限判据与`/agents` destination仍todo。只把public/private写进SQL会让runtime与roster判据漂移；只带actor/admin又违反tenant是最外层scope，可能暴露另一tenant的package Agent。浏览器实测还发现第一真源示例的`../assets/fonts/*`在Trunk最终根CSS中请求成不存在的`/assets/fonts/*`。若先画create/edit/hide/delete按钮，只会把尚无生产API的动作伪装成可用。 | 新增closed `AgentProfile(s)Response`、typed list/get command与`AgentDirectory`；`openbot_domain::agent::profile_policy`唯一实现access/run alias/manage六条判据。Application只从AuthContext铸tenant/actor/admin；PG以compile-time SQL收紧package tenant、visibility/owner/admin、deleted/hidden并让domain复核，flags与has-*只从权威列/config投影。Axum GET list/detail均`no-store`，unknown query/malformed 400，missing/invisible/deleted/cross-tenant统一404。Leptos `/agents`按固定上游mine/explore分组，AgentCard+URL profile只读，AppSidebar接Agents；mutation/start按钮全部不画。字体URL修为根绝对同源`/fonts/*`并同步GUI第一真源。 | 本机：contracts agent=`2/0/0`、domain profile=`6/0/0`、application agents=`2/0/0`、Server agents=`4/0/0`、UI agents=`3/0/0`；PG17.11 host SCRAM=`1/0/0`，覆盖owner/other/admin list+get、system、hidden、deleted、cross-tenant、endpoint/auth/callback secret-free flags。七crate all-targets/all-features Clippy `-D warnings`、UI WASM、fmt/diff绿；i18n391、design65 Rust/74 icons、CSS193；bundle wasm gzip600783/CSS68843/fonts740216/external-inline1/0。真实Chromium：4张144×180卡、mine2/explore2、list/detail 200+no-store、URL硬刷新/404/close返焦、1440/1024/900/600 overflow0、Inter loaded、external/duplicate/fake actions/forbidden wire keys均0、合法route error0；保留Chromium preload SRI warning与尚未提供brand favicon的已知todo，不冒充全局warning0。API=`45/117/162`、tests=`300/747/1047`、UI=`84/68/152`、parity=`560/1113/1673`，0 violation/warning；strict recount=`157/157/0`。关闭T-API-0019/0020、T-TEST-0298–0303/0305/0328–0329/0332、T-UI-0029/0030；T-TEST-0306及Agent lifecycle、T-UI-0032、T-ROUTE-0007、T-UI-0126与AppSidebar总项继续todo。Cargo.lock/package delta0；implementation `55bc2f18d1f6108272864dee8fba6d22c637305a`；未运行CI/Actions。详见Batch31文档。 |

| R95 | §3.1–§3.2 / §4.1–§4.3 / §5.2 / §6.6（GUI第一真源）/ §8.6 / §13.1–§13.3 / §15.1–§15.4 / §21.1 / §24 G3、G4、G6（2026-08-26 Channel Create/Routing batch32） | R94后Agent可读但用户仍不能创建channel；固定上游create在一个事务内锁profile、建channel/member/agent mapping，而Rust若继续依赖只读legacy Intelligence mapping会让新channel没有native thread。`/api/route`的模型建议若直接当权限、把message/model reason写audit、或候选变化后仍commit，会产生越权与隐私/陈旧裁决。GUI若刷新即create会制造空channel；若只画发送按钮而没有native BeginRun HTTP path则是fake runtime。 | 新增纯routing11条+closed reason、typed create/route命令与ports。Application只从AuthContext铸scope，canonical Agent IDs；PG在单连接事务内按canonical顺序`FOR UPDATE` profile并复核tenant/domain access，写channel/creator membership/channel_agents/deployment UUIDv8 thread，零Intelligence mapping。route显式recipient零模型；inference取visible roster、active MCP reach与first-public default，经package OpenAI Chat/SafeDialer/每请求credential建议，失败/坏JSON/roster外/低confidence全fallback。serializable audit事务复读完整roster，变化409；payload只存IDs/closed reason。Axum接create/route与新增native `POST /api/threads/{id}/runs` framing。Leptos `/channel/new` URL-owned recipient、hidden direct/hard reload、刷新零create、首发create→begin→navigate；create未知禁止重发、begin失败同run-id重试。未实现的Enter/IME提示已删除，不冒充完整Composer。 | 本机：contracts=`1/0/0`；domain routing/channel/audit=`12+2+12/0/0`；application=`2+6/0/0`；infra provider=`2/0/0`；Server create/route/begin=`3+6+2/0/0`、main protocol=`1/0/0`；UI=`68/0/0`。PG17.11 host SCRAM=`2+1/0/0`：max_pool=1、独立identity、六surface原子、Unicode120、四类denial零残留、profile delete lock、created thread→real BeginRun membership/message/run/activity；active reach、hash chain、message/model prose canary0、candidate409/audit count1。七crate Clippy、UI WASM、tools/fmt绿；i18n396、design67 Rust/74 icons、CSS200；bundle wasm gzip669241/CSS70248/fonts740216/external-inline1/0。浏览器：refresh 52→52、hidden URL/hard reload、Combobox keyboard、first send 52→53、1440/1024/900/600 overflow0、1 main/nav/h1、duplicate IDs0、console error/warn0。API=`48/115/163`、tests=`334/713/1047`、routes=`1/31/32`、UI=`85/67/152`、parity=`599/1075/1674`，0 violation/warning；strict recount=`157/157/0`。关闭API3/tests34/route1/UI1；完整channel/home Composer、T-UI-0129 golden、G3/G4/G6整关继续todo。Cargo.lock package=`822→822`，只增既有uuid direct+WASM js CSPRNG edge；implementation `f9eb1594a5aad634be992c3f24f6dc1d21e2f806`；未运行CI/Actions。详见Batch32文档。 |

| R96 | §3.1条4 / §6.5（GUI第一真源）/ §13.1 / §21.1 / §24 G6（2026-08-26 Composer Draft/Queue batch33） | R95后`/channel/new`能首发，但固定上游Composer的draft/queue两份纯状态共26条仍todo。若直接在Leptos事件handler里拼规则，single Agent、command side effect时序、busy边缘与stop后drain会分散；若把queue写PG则又把上游明确“当前mount内存、reload丢失”的意图偷换成durable outbox。用闭包承载action还会把不可审计副作用藏进纯transform。 | 新增纯Rust `Segment/ComposerDraft`与`reduce_queue`。`@`/`/`trigger闭集；plain text复用ECMAScript TrimString；multiple Agent只保留最新；prompt展开、chip保留，action转closed deferred effect ID。Queue action闭集submit/settle/remove；idle空队列borrow原draft，busy append，settle以换行合成一个turn并按首次顺序去重commands；drain Agent ID固定None；caller ID区分同文消息。`Cow`构造性保留上游same-array/object no-op identity。模块尚无production consumer，用显式non-test dead-code allow标明分阶段地基；release optimizer证明确实不进现有UI产物。 | 固定上游draft=`10/0/0`、queue=`16/0/0`；UI全包=`94/0/0`，all-targets/all-features Clippy与WASM绿；i18n396、design70 Rust/74 icons、CSS200。release hashed asset名与R95逐字相同，bundle仍wasm gzip669241/CSS70248/fonts740216/external-inline1/0。tests=`360/687/1047`、parity=`625/1049/1674`，0 violation/warning；strict recount=`157/157/0`。关闭T-TEST-0131–0156；T-UI-0043/0123、T-ROUTE-0009及production Composer/stop/cancel/realtime仍todo。Cargo/lock/dependency/CSS/locale delta0；implementation `a34f68337ecfdc0cbed42637285a56263617520d`；未运行CI/Actions。详见Batch33文档。 |

| R97 | §2.4#44 / §3.1条4 / §4.1–§4.3 / §5.2 / §6.4–§6.5（GUI第一真源）/ §13.1–§13.3 / §15.1–§15.4 / §21.1 / §24 G3、G6（2026-08-26 Channel Transcript/Idle Send batch34） | R96只有纯draft/queue，channel detail仍画“conversation unavailable”。若GUI先GET history再另查run/cursor，查询间事件会让busy与replay起点漂移；若从最新cursor连流又不带active chunks，会漏掉尚未terminal materialize的半段回复。沿用上游module seed/stash会在reload/双mount制造第二真源；直接显示watchdog/provider Error文字又违反stable code+i18n边界。Stop若只调用本进程consumer，HTTP落另一副本时会静默失效，故本批不能顺手画假Stop。 | 新增closed `ThreadConversationSnapshot(messages,activeRunId,activeRunText,lastEventSequence)`与typed command/port/native GET。PG单statement按deployment/tenant/current anchor membership投影；active tail只聚合最后tool checkpoint后的text，active>1 fail。SSE允许一次性query cursor，标准Last-Event-ID优先；Leptos snapshot后接EventSource durable replay/live，sequence去重、gap/坏thread/payload refetch或断流，terminal后再取PG。user/assistant/tool-call/tool-result用既有Message/Bubble/Scroller plain-text投影，system不显示、DOM id hash；raw error改fieldless terminal enum→en/zh-CN。Idle send有thread直接Begin、无thread先mint；同run-id重试，busy可写草稿但Send禁用。Textarea实现Enter/Shift+Enter/IME。真实浏览器发现Chat shell min-height+100%导致约96万px，改固定viewport flex后四视口X/Y overflow0。 | contracts=`1/0/0`；Server conversation/SSE=`2+1/0/0`；UI=`101/0/0`；PG17.11 host SCRAM=`1/0/0`：snapshot active tail/cursor1→subscribe after1→live event2/terminal3→history materialize、outsider empty。六crate Clippy、UI WASM绿；i18n410、design71 Rust/74 icons、CSS204，bundle wasm gzip781045/CSS71409/fonts740216/external-inline1/0。浏览器：durable initial/history、ShiftEnter、Enter busy、stream marker、terminal refetch、busy draft preserve、hard reload、thread-null mint/begin/hard reload；1440 transcript578、1024/900/600 transcript318，overflow0，avatar AX重复0、duplicate IDs/alerts/console0。API=`49/115/164`、tests=`372/675/1047`、parity=`638/1037/1675`，0 violation/warning；strict recount=`157/157/0`。关闭new API1+fixed tests12；完整Chat/Composer/markdown/tool boundary/Screen/queue/stop/cancel/route仍todo。Cargo.lock/package delta0，仅web-sys EventSource/MessageEvent feature；implementation `6013072529f22054599264296552fd474a9e6bf1`；未运行CI/Actions。详见Batch34文档。 |

| R98 | §3.1条4 / §4.3 / §5.2 / §7.2 / §7.4 / §13.1–§13.3 / §15.3 / §21.1 / §24 G3、G4、G6 / §6.5（GUI第一真源）（2026-08-27 Durable Cancel/Production Queue batch35） | R97已接durable conversation与idle send，但Stop若直接调用当前Server进程的consumer，HTTP落另一副本会静默失效；若request ack直接画Cancelled，会在child仍运行时制造假终态；若无local child只写terminal而不收口原dispatch，会留下可再次claim的陈旧工作。Queue若写PG又会把固定上游“当前mount内存、reload丢失”偷换成durable outbox；在任意reactive Effect owner里直接发drain又会因busy写回dispose自己的异步send。 | 新增closed CancelThreadRun/reply与foreground queued/running/cancelling/reconciliation投影。Application只取AuthContext scope；PG锁run并验deployment/tenant/current anchor membership/run owner，先写唯一`agent_run_cancel` internal outbox再返回。RunRelay用LISTEN wake+100ms poll、lease owner/fencing claim且cancel先于dispatch/recovery；consumer返回ChildSignalled/NoLocalChild，前者等runtime child token真正停后journal terminal，后者同事务写Cancelled并逐字段校验/收口cancel+dispatch outbox。Axum POST只收path thread/run、trusted Origin先行，202不冒充terminal。Leptos空draft时Stop替Send，请求中/durable Cancelling可见但inert；terminal由SSE/snapshot决定。Batch33 reducer接production busy park/remove、busy→idle单次合并send、stop后同一路drain；组件owner承载异步send，hard reload清queue但恢复durable foreground。 | 首次PG实跑捕获`cancelled/delivered/pending/1`并据此补原子dispatch收口；tampered-dispatch恢复又暴露SQL三值逻辑会把NULL owned claim静默过滤，改用`IS NOT DISTINCT FROM $4`并验证claim后崩溃的新relay重放；修后PG17.11 host-SCRAM poll-only=`1/0/0`、cross-replica active child-drop=`1/0/0`。contracts/application/Agent/Server/UI=`76/137/28/202/105`全绿，infra=`306/0/0`、transport parity=`8/0/0`；六crate Clippy、contracts/UI WASM绿。i18n417、design71 Rust/74 icons、CSS206；bundle wasm gzip801086/CSS73154/fonts740216/external-inline1/0。浏览器实得Cancelling visible+disabled→Cancelled、queue remove 1→0、两条合成1 turn/standalone0、stop后drain1、reload queue1→0且foreground Stop恢复；1440/1024/900/600 overflow0、main/nav/h1各1、duplicate IDs/alerts/console0。API=`50/115/165`、tests=`372/675/1047`、UI=`85/67/152`、parity=`639/1037/1676`、fixtures=`15/22/37`，0 violation/warning；strict recount=`157/157/0`。只关闭新增T-API-0165；sources/附件/per-channel draft/steer、markdown/tool boundary/Screen、T-UI-0043/0123、T-ROUTE-0009与RMCP/computer/file/shell协议级cancel保持todo，G3/G4/G6整关不勾；implementation `86370626cebf3f78e87ac5b5a87e223377ff69ff`；未运行CI/Actions；详见Batch35正式文档。 |

| R99 | §3.1条7 / §4.1–§4.3 / §5.2 / §6.1–§6.7（GUI第一真源）/ §13.1–§13.3 / §14.1–§14.3 / §15.3 / §21.1 / §24 G3、G6（2026-08-27 Memory Controls/GUI batch36） | R98后explicit memory已有六条API、PG事务与remember tool，但没有global write control和任何用户页面；用backend存在冒充Memory GUI会是安全假完成。控制若塞进`user_ui_preferences`会混淆runtime数据治理与渲染偏好；伪造成特殊memory又会进入list/recall。关闭写入若连forbid/delete一起禁，会让用户失去擦除既有数据的能力；只在GUI禁按钮则tool/correct仍可绕过。 | native 0022新建`user_memory_controls(tenant_id,actor_user_id,writes_enabled,updated_at)`，缺行默认enabled且FK actor cascade；它不是memory记录。新增closed MemoryControl command/reply/port与GET/PUT `/api/memories/control`，只从AuthContext铸scope、no-store，PUT trusted Origin先于body。PG在GUI remember、correct和remember tool各自原事务内复读control；disabled统一投影`policy_refused/memory_writes_disabled`，但list/recall/forbid/delete始终可用。Leptos `/settings/memory`只消费typed DTO，首屏50+keyset load-more、四状态/kind/sensitivity/scope/source/origin/tags、switch、correct replacement、forbid/delete；不optimistic改row，成功后权威refetch。取消返原按钮；correct后旧按钮消失，故聚焦新replacement行。AppSidebar只新增真实Memory destination。 | implementation `bec30ec52e3fbaabe3aa3f08a5de0d1e7bd4f991`。PG17.11 SCRAM验证password_encryption与role hash均SCRAM；schema0022 regeneration开/关各`1/0/0`，44表/408列/299 NOT NULL/225约束/87索引、ledger10、SHA `f7dfda29…c5ee25`；memory真库`3/0/0`覆盖GUI/tool/correct拒绝、delete仍擦除、重新启用与跨tenant隔离。contracts/application/Agent/Server/UI=`77/138/28/203/108`，infra=`306/0/0`，transport=`8/0/0`+memory parity=`1/0/0`；七crate Clippy、contracts/UI WASM、fmt/diff绿。i18n452、design72 Rust/74 icons、CSS215；最终bundle wasm gzip870212/CSS73367/fonts740216/external-inline1/0。浏览器先因用户清理dist实证旧CSS 404，重建并重启后HTTP CSS=200/text-css、445规则；最终50→52、disable跨reload、47 correct全disabled而48 forbid/49 delete可用、correct/supersede与两类焦点、forbid/delete擦除、中英、四视口overflow0、duplicate/visible-alert/console0。API=`52/115/167`、routes=`2/30/32`、parity=`642/1036/1678`、fixtures=`16/22/38`，0 violation；strict recount=`157/157/0`。T-ROUTE-0032、T-API-0166/0167、T-FIX-0038关闭；正式golden T-UI-0152、legacy production drills、AppSidebar其余destination与G3/G6整关保持todo。未运行`cargo xtask ci`/Actions；详见Batch36正式文档。 |

| R100 | §3.1条7 / §5.2 / §6.1、§6.2、§7–§9（GUI第一真源）/ §13.1–§13.3 / §15.3 / §21.1 / §24 G6（2026-08-27 Settings Preferences batch37） | R99后theme/locale只有Sidebar入口，`/settings`仍落404，不能用native0021/API存在冒充settings route。第一真源又要求ThemeToggle同时存在settings页与Sidebar；LocaleSwitch原实现使用全局固定`locale-switch-label/current`，双实例必然duplicate ID。浏览器快速连续theme+locale还暴露更深缺口：server已提交`dark/en`，但`Saving preferences`永久不消失；worker由触发ThemeToggle的child owner创建，locale重渲染会取消receipt后的`set(false)`，固定sleep会把这个竞态掩掉。 | 新建`SettingsPage`，保留上游Preferences/General与Theme journey，并按第一真源增加system第三态/locale；description按native0021真实scope写“当前deployment跨设备”，不伪称every deployment。页面与Sidebar复用同一preference context/API；LocaleSwitch改为调用点传入bounded ID前缀，label/current/menu family两实例不交叉。保存worker在AppShell提供context时捕获stable Owner，所有child event只在该owner下启动serialized/coalesced loop；locale重渲染不再取消收尾。`PreferenceSaveStatus`在pending期间显示localized`role=status`，`/settings`由页面唯一播报、Sidebar抑制重复；fixture固定1s延迟构造性覆盖pending与两次PUT。AppSidebar只在route存在后新增Settings destination；settings二级layout不冒充完成。 | implementation `5babb78483d0083085047b21760dbc963a418383`。UI=`110/0/0`；UI+fixture Server bin Clippy、UI WASM、fmt/diff绿。i18n455、design73 Rust/74 icons、CSS221；bundle wasm gzip885190/CSS74670/fonts740216/external-inline1/0。release浏览器CSS455规则：Sidebar→Settings真实导航/current；双实例2 radiogroup+2 locale switch、duplicate ID0；主题/语言即时同步两处，End/ArrowDown/Enter/返焦通过；快速system+en时页面唯一`Saving preferences`，stable owner排空后status0/alerts0，hard reload保持2个System/2个English；1440/1024/900/600 overflow0、main/nav/h1各1、console0。routes=`3/29/32`、parity=`643/1035/1678`，0 violation；strict recount=`157/157/0`。只关闭T-ROUTE-0026；settings layout T-ROUTE-0005、正式golden T-UI-0150、connected accounts/gallery/computer、AppSidebar skills/admin与G6整关保持todo。未运行`cargo xtask ci`/Actions；详见Batch37正式文档。 |

| R101 | §3.1条7 / §5.2 / §5.3（GUI第一真源）/ §6.1–§6.2 / §9.2 / §13.1 / §21.1 / §24 G6（2026-08-27 Settings Secondary Shell batch38） | R100关闭Preferences page，但`settings/route.tsx`对应layout仍不存在；General与Memory直接落全局main，第一真源固定的200px secondary nav未实现。机械复制上游SettingsSidebar又会画`/settings/connected-accounts`与`/settings/components-gallery`两个当前不存在的断链；用placeholder吞掉点击同样是假完成。 | 新增`SettingsShell`，仅包裹真实`/settings`和`/settings/memory`，不嵌套第二个main。用aside+named nav与`--size-subnav`单源200px；Back to app、General exact、Memory三条bounded same-origin link，current只精确命中一条。Memory属于R7新增页，进入secondary nav明确为新增。Connected accounts/gallery在production route存在前构造性不渲染。≥768px为200px+content双列/sticky边界；<768px堆叠单列、nav横向换行，继续要求X overflow0。纯helper测试锁真实destination集合恰2且General不前缀误选Memory。 | implementation `c1cf2f073e445803f536e3f9c0b75d0404fa48a1`。UI=`111/0/0`，UI Clippy/WASM/fmt/diff绿；i18n456、design74 Rust/74 icons、CSS227；bundle wasm gzip892016/CSS76263/fonts740216/external-inline1/0。release浏览器CSS463规则：`/settings` subnav width200、断链0、General current1；Memory点击+hard reload保持shell/current/h1与50条memory，再回General；Back到`/`后shell0。1440/1024/900实得200px双列，600单列；两页面overflow0、main1/nav2/h1 1、current1、duplicate/alerts/console0。routes=`4/28/32`、parity=`644/1034/1678`，0 violation；strict recount=`157/157/0`。只关闭T-ROUTE-0005；connected accounts/gallery route、settings-sidebar业务组件与formal golden保持todo。未运行`cargo xtask ci`/Actions；详见Batch38正式文档。 |

| R102 | §3.1条7 / §5.2 / §6.1–§6.7（GUI第一真源）/ §9.2–§9.5 / §13.1–§13.3 / §15.3 / §21.1 / §24 G4、G6（2026-08-27 Connected Accounts batch39） | R101后actor OAuth/Drive/list/disconnect后端已存在，但`McpConnections`只有已连接行，无法呈现固定上游“管理员已add、本人尚未连接”的Not connected条目；若把所有`mcp_servers`或自定义connection直接画进个人页，会把未reviewed connector冒充用户OAuth目录。authorization URL若只做字符串前缀判断可接收userinfo/fragment/非HTTPS；disconnect pending若写成vendor revoked又会篡改事实。直接复制上游SettingsSidebar还会画尚不存在的Gallery；复制Google品牌图则会伪造未取得的品牌资产。 | contract新增只传stable id的`availableServerIds`，不传title/URL/credential；PG在AuthContext actor连接行之外，只查询编译期reviewed `google-drive`，DB行的url/vendor/provenance/transport必须逐字段等于固定identity，否则`Corrupt(reviewed_server_identity)`失败关闭。GET list、POST connect、DELETE disconnect均no-store，写请求继续trusted Origin。UI transport限定64字节server id、集合唯一与scope/redirect bounds；authorization receipt只接受bounded同源根路径，或无userinfo/fragment的HTTPS绝对URL，Server receipt后才full-page navigation。新增index/detail route并把Connected accounts加入真实SettingsShell；UI只join reviewed id，unknown/custom不显示且detail无action。connected页显示vendor实际scope与RFC3339时间；APG Menu断开后权威refetch，严格区分Revoked/Pending。异步connect/disconnect都绑定detail stable Owner；断开重渲染后先解除pending再聚焦新Connect。fixture只模拟available→callback→connected→disconnect pending，不冒充Google网络；品牌继续用中性Plug/RowMark。 | implementation `52b2c4f59906da58ae5d2d7db62adfcce90f9af5`。PG17.11 host SCRAM=`1/0/0`，证明启用前空、启用后唯一Drive、原OAuth/Agent/401 retry/local-first revoke全回归且tampered identity失败关闭；contracts/application/Agent/Server/UI=`78/138/28/203/114`，infra=`306/0/0`，transport=`8/0/0`，plugins HTTP=`4/0/0`；五crate Clippy `-D warnings`、UI WASM、fmt/diff绿。i18n473、design75 Rust/74 icons、CSS233；最终bundle wasm gzip1053824/CSS78190/fonts740216/external-inline1/0。release浏览器实得Not connected→full-page fixture callback→Connected→hard reload，scope/time、失败canary=0、unknown零action、中英；Menu ArrowDown/Escape、pending disabled、Pending真文案与Connect返焦；1280 overflow0、main1/nav2/h1 1/current1、duplicate/alerts/console0。routes=`6/26/32`、parity=`646/1032/1678`，0 violation/warning；strict recount=`157/157/0`。只关闭T-ROUTE-0029/0030；Google brand、formal page golden、通用MCP admin/private egress、Desktop Local OAuth、真实Google restricted-scope发布验证与G4/G6整关保持todo。未运行`cargo xtask ci`/Actions；详见Batch39正式文档。 |

| R103 | §3.1条10 / §3.3 / §5.2 / §6.1–§6.7（GUI第一真源）/ §8.5–§8.6 / §13.1–§13.3 / §14.3 / §21.1条5 / §24 G4、G6（2026-08-27 Components Gallery/Quote batch40） | R102后SettingsSidebar最后一个上游destination仍是断链；本仓只有components兼容表row/repo，没有ApplicationService、HTTP、renderer或Gallery route。固定上游index/detail都渲染真实ComponentPreview，只画元数据卡会是假完成。上游又让任意已登录browser announce自由name/title/kind/description；照搬会让renderer自报能力/模型说明。把全部13个名字先登记却只实现Quote同样会广告不可用能力；build sync若upsert已有row会覆盖管理员draft/publish/grant。 | 新增WASM-safe完整ComponentRecord与closed manifest；当前manifest/renderer registry双向恰`showQuote`，另外12个不伪报。announcement保留原wire，但application只接受与Server manifest逐字段相等的unique entries；Axum PUT trusted Origin。PG list按kind/title/name列全部治理row并排序聚合withheld/functions；sync只INSERT missing、首次默认published，existing零改；insert与`component.published` hash-chain audit同事务。GET/PUT在Axum与Tauri typed custom protocol都no-store。UI新增中性无边框GalleryFrame、四值Tone/文字+点Badge、共享RefusedCard、exact Quote schema/renderer与unknown fallback；index先best-effort announce再权威GET，只列published；detail显示真实preview/kind/called-as，stale published诚实fallback，unpublished/unknown按不存在。Gallery真实后SettingsShell补齐上游General/Connected/Gallery顺序并保留新增Memory。 | implementation `d5ca010231d1cfb6cc0950bddb84e7be7f651abf`。PG17.11 host SCRAM=`1/0/0`：强制audit失败row0、成功added1/audit1、重复added0、tampered拒绝、admin unpublish/draft不被覆盖、unknown kind失败关闭。contracts/application/Agent/Server/UI=`80/140/28/204/118`、infra/Desktop=`306/78`、transport=`8/0/0`；七crate Clippy、contracts/UI WASM、fmt/diff绿。i18n491、design81 Rust/74 icons、CSS251；bundle wasm gzip1099850/CSS84507/fonts740216/external-inline1/0。浏览器实得published stale+Quote两tile、unpublished0、真实Quote/fallback/detail facts/hard reload、showNotice no-such、篡改400/重复added空、中英；1280 subnav200、main1/nav2/h1 1/current1、nested interactive/overflow/duplicate/alerts/console0。API=`54/113/167`、components=`1/21/22`、routes=`8/24/32`、UI=`86/66/152`、parity=`652/1026/1678`，0 violation/warning；strict recount=`157/157/0`。只关闭T-API-0037/0038、T-ROUTE-0027/0028、T-CMP-0005、T-UI-0066；Quote runtime grant/decision仍使T-CMP-0007 todo，另12 renderer、Refused生产双接线、admin/sandbox/Desktop renderer与formal golden均未完成，G4/G6整关不勾。未运行CI/Actions；详见Batch40正式文档。 |

| R104 | §3.1条10 / §3.3 / §6.1–§6.7（GUI第一真源）/ §8.5 / §13.1 / §21.1条5 / §24 G6（2026-08-27 Gallery Cards batch41） | R103只实现Quote；固定上游`cards.tsx`另有showRecord/showMetrics/showChecklist/showNotice四个独立name。若合成一个`showCard(kind=…)`会同时改掉tool/catalogue/grant三种身份；若只加manifest不加renderer会广告不可用能力。上游Checklist用可视checkbox形状但明确read-only；照搬可点击控件会制造无法回传的假action。其tone又把emerald/amber/red落背景/边框，与GUI第一真源“语义色只落文字/图标/状态点”冲突。 | manifest/schema/renderer registry同批从1扩为5并保持稳定name排序。四schema逐层additionalProperties=false；Record required title/fields且value不截断；Metrics required title/metrics、maxItems6；Checklist required title/items与nested text/done；Notice required title/body/有序points；tone恰四值。Record/Metrics/Checklist/Notice全部复用GalleryFrame；semantic badge只用文字+点，Checklist只读glyph+删除线且零button/input/checkbox，Notice tone本地化。fixture把unpublished负例移到未实现future chart，使五真实renderer首次sync而existing未发布仍不被覆盖。T-CMP-0002仍不勾，因为conversation registration/per-Bot withholding/data grant/call-time decision未落。 | implementation `3173354d895110363850a4d8dcf6679fc90c332b`。PG17.11 host SCRAM五entry=`1/0/0`：forced audit failure五row0、成功added5/audit5、重复0、tamper/admin治理/unknown kind边界保持。contracts/application/Agent/Server/UI=`80/140/28/204/119`、infra/Desktop=`306/78`、transport=`8/0/0`；七crate Clippy、contracts/UI WASM绿。i18n496、design82 Rust/74 icons、CSS257；bundle wasm gzip1138033/CSS86968/fonts740216/external-inline1/0。release浏览器实得五renderer+stale六tile、unpublished0；Record三字段完整、Metrics三项/tone、Checklist2/3且interactive0、Notice两points，semantic badge background0；Metrics detail/hard reload、overflow/alerts/console0。parity仍`652/1026/1678`且0 violation/warning，strict recount=`157/157/0`。Charts/Decisions/Activity、runtime授权、admin/sandbox/Desktop renderer与formal golden继续todo；未运行CI/Actions。详见Batch41正式文档。 |

| R105 | §3.1条10 / §3.3 / §6.1–§6.7（GUI第一真源）/ §13.1 / §21.1条5 / §24 G6（2026-08-27 Gallery Charts batch42） | R104后还缺固定上游五个chart name。若引入JS图表库会违背全量Rust/零新增runtime；若Line/Area各抄schema/scaler会漂移。series若允许模型选色会混入拒绝/成功语义；空数据画空axis又会像renderer失败。 | manifest/registry从5扩为10，五name独立；Bar/Pie point与Progress target nested schema closed，Line/Area共用同一schema与`plot_geometry`。纯Rust/Leptos+SVG实现Bar/Donut/Line/Area/Progress；palette只循环chart-1..5 token；图形AX隐藏而文本DOM保留；total<=0/空集合显示本地化空态。PG sync自动扩为十entry，existing治理继续零覆盖。T-CMP-0003因runtime授权链未落仍todo。 | implementation `18a080cd2749cb958adcbfd12d06af11468d8ae8`。PG17.11 SCRAM十entry=`1/0/0`；contracts/application/Agent/Server/UI=`80/140/28/204/120`、infra/Desktop/transport=`306/78/8`；七crateClippy、WASM绿。i18n498/design83/CSS265，bundle=`1169672/91428/740216/1/0`。浏览器11 published/unpublished0；Bar三高度与token色、Donut48/26/26、Line无polygon/Area有polygon、Progress90/100，nested interactive0；detail/hard reload/overflow/alerts/console绿。parity仍652/1026/1678，strict157/157/0。Decisions/Activity/runtime授权/admin/sandbox/Desktop/formal golden继续todo；未运行CI/Actions。详见Batch42文档。 |

| R106 | §3.1条10 / §3.3 / §5.2 / §6.1–§6.5 / §8.5–§8.6 / §13.1–§13.3 / §15.3 / §21.1条5 / §24 G6（2026-08-27 Compiled Component Runtime Authorization batch43） | R105已有十个真实renderer与治理读面，但生产仍没有`for-agent` grant snapshot和每次tool-call decision。只在会话开始时读published/withholding会让随后撤权的旧snapshot继续渲染；只查component不查本次data functions会先告诉模型“已显示”再在组件读取时失败；若handler接actor/tenant/role或数据库里任意published stale row，renderer/调用方就能扩大身份与build能力；拒绝若先返回再另写audit，审计失败会留下无法追责的拒绝事实。 | 新增closed runtime grant/decision contracts与纯domain publication/description/withholding/function判定。Application只从AuthContext铸tenant/actor/admin，按build manifest限定renderer，function names按既有component语法排序去重。PG list在repeatable-read内复用唯一`can_run_agent`并投影published+description+非exclusion；decision在serializable事务内复核Agent、current build、component与全部function grants，拒绝和hash-chain audit同事务，payload只含权威bot、stable error_code与可选function。Axum POST trusted Origin-before-body，Axum/Tauri/UI均typed/no-store；domain/application零用户文案。 | implementation `15fdb401851f0fca666399e25270fc98cdd4a381`。定向contracts/domain/application/Server/UI/Desktop=`3+(2+12)+4+2+1+1/0/0`；PG17.11 host SCRAM=`1/0/0`，覆盖public/private/admin/deleted/cross-tenant、unpublished/null-description/withheld/stale renderer、function grant/missing、撤权后旧snapshot再调用；4条拒绝audit无自由reason，强制audit失败零decision且零新增row。八crate all-targets/all-features Clippy `-D warnings`、contracts/UI WASM、fmt/diff、UI/Tauri/release-target guards绿；i18n/design/CSS=`498/83/265`，Cargo/package delta0。API=`56/111/167`、parity=`654/1024/1678`、strict recount=`157/157/0`，0 violation。只关闭T-API-0039/0040；production conversation registration、Activity `/call`/data registry、Decisions HITL、Refused生产双接线、admin/sandbox/Desktop renderer/formal golden继续todo，全部T-CMP保持原状态；本批无视觉变化，未重建bundle/浏览器/golden，未运行CI/Actions。详见Batch43文档。 |

| R107 | §3.1条10 / §3.3 / §5.2 / §6.1–§6.5 / §8.3 / §8.5–§8.6 / §13.1–§13.3 / §15.3 / §21.1条5 / §24 G2、G6（2026-08-28 Gallery Activity/Data Functions batch44） | R106只查component/function grant，尚无build data registry、`/functions`/`/call`或Activity renderer。若照搬上游“任意能用Bot的session+function row即可读deployment audit trail”，会绕过本仓同数据源admin ACL与default-deny action policy；若decision让browser任意声明function或省略Activity真实读取，会先向模型确认已显示再失败；若read成功后另写audit，audit故障会把无痕数据交给renderer；若把失败混成refused，policy审计不再可信。 | 新增WASM-safe Activity/schema、两function typed registry/report/call。Application固定renderer→function映射：Activity恰一项两选一，其余current renderer零项；days/limit按上游默认/截整/clamp，unknown key在port前400。PG serializable顺序复核Agent→component/build→function identity→admin audit ACL→hot compiled action policy（空page/ReadTool）→grant；bounded SQL最多12/50行。success data+function.called同事务；read在savepoint失败后写function.failed并回502；refused三路继续同事务，payload只含allowlist facts。Activity Settings preview专用不可预览分支，零runtime mount。 | implementation `c742adbfb23d1bdf03b36ffb09ce9dac2d696e2b`。contracts/domain/application/Server/UI/Desktop=`4+(2+12)+5+3+2+1/0/0`；PG17.11 SCRAM runtime+11-entry catalogue=`1+1/0/0`，覆盖ACL/default policy/allow/grant/missing、两真实report、called/refused/failed、append-only与tamper/savepoint、forced-audit rollback、added11/audit11。八crate Clippy、contracts/UI WASM、fmt/diff/tools/UI/Tauri/release-target guards绿；i18n/design/CSS=`514/84/271`；bundle=`1174338/93646/740216/1/0`。release浏览器12 tile、Activity1/figure0、index/detail/hard reload不可预览、overflow/duplicate/nested/console0。API=`58/109/167`、events=`29/53/82`、components=`2/20/22`、parity=`660/1018/1678`、strict=`157/157/0`。关闭T-API-0041/0042、T-EVT-0025/0026/0028、T-CMP-0006；T-CMP-0001仍因production conversation registration/args projection/runtime mount/follow-up ask缺失而todo，Cards/Charts/Quote同理不勾。CSS仅余4658B预算；未运行CI/Actions。详见Batch44文档。 |

| R108 | §3.1条10 / §3.3 / §5.2 / §6.1–§6.5 / §7.2–§7.5 / §8.3 / §8.5–§8.6 / §13.1–§13.3 / §15.3 / §21.1条5 / §24 G4、G6（2026-08-28 Compiled Components in Conversation batch45） | R107后11个renderer、for-agent/decision与Activity data call虽已存在，但provider sampling仍只注入remember/MCP，conversation把component call/result画成通用tool文本。若只在启动时缓存grant，撤权后模型仍可继续调用；若让模型自报Activity function会扩大数据面；若UI拿频道第一个Agent猜调用者，多Agent频道会对错Agent授权/读取；若仅凭tool call参数画组件，拒绝、错配或坏history会把未授权内容显示出来。 | contracts建立ordinary 11项schema/title/confirmation/validator单源，Activity function只由report枚举推导。PG context每次sampling复用同一ComponentAdministration按fresh actor role/Agent/current build列definition；Agent gateway在generic acting tool前验证args并用fresh AuthContext走DecideComponent，成功/拒绝各形成一个durable tool reply。history DTO补Server-derived agentId与authoritative toolName/errorCode，PG以message.run_id关联runs.bot_id；UI只有call id+name+Agent三者一致且result无error、args过closed validator才逐字段mount renderer，否则共享RefusedCard。Decisions/HITL不走ordinary自动确认。 | implementation `b28801d0a007274376fef069be9f9d72f47f6d59`。contracts/gateway/UI conversation/runtime/Server thread=`5+5+11+1+21/0/0`；PG17.11 host SCRAM provider-context/thread-conversation/thread-history=`1+1+1/0/0`，证明published/withholding撤权下一次load生效与run-linked Agent identity exact。八crate all-targets/all-features check+Clippy、contracts/UI WASM、fmt/diff绿；i18n/design/CSS=`515/85/271`；bundle=`1247562/93646/740216/1/0`。release API六message均agentId=bot-0；Quote figure/text各1、Refused1、拒绝正文0、普通assistant保留，hard reload与1440/1024/600 overflow0、duplicate/nested0。应用异常0；另诚实记录既存Chromium preload-SRI警告1与favicon404 1，未削弱SRI。components=`5/17/22`、parity=`663/1015/1678`、strict=`157/157/0`，0 violation/warning。只关闭T-CMP-0002/0003/0007；Activity follow-up ask、Decisions两renderer、Refused sandbox共用、admin/sandbox/Desktop/formal golden继续todo；CSS余4658B。未运行CI/Actions、未push。详见Batch45文档。 |

| R109 | §3.1条10 / §3.3 / §5.2 / §6.1–§6.5 / §7.2 / §13.1–§13.3 / §15.3 / §21.1条5 / §24 G3、G6（2026-08-28 Activity Follow-up Ask batch46） | R108后Activity能在conversation读两种真实report，但固定上游还让非空结果发起两个新user turn。本仓若让component调用当前selected/roster第一Agent，多Agent频道会把追问发给错Bot；若当前run未结束就begin会撞唯一foreground约束。进一步审计发现`resumable`同时让send callback提前return并把Retry按钮disable，首次begin失败后虽显示Retry文案却永远无法重放。 | Activity follow-up与Decision HITL分离：前者走新的普通BeginThreadRun，后者仍待durable suspend/respond。两条模型prompt逐字固定且不i18n；按钮只在真实非空data出现。conversation把配对message的Server-derived Agent与busy signal传入组件；busy/resumable/submitting明确disabled。PendingTurn新增Agent，composer/component复用唯一send callback；resumable锁编辑但不锁Retry，并重放exact thread/run/Agent/message。fixture两种typed report与两组durable exchange，busiest首次固定503作正向retry证据。 | implementation `3a277568fa8706300ac7ba6d0c93b1d255adec22`。UI conversation/Activity=`12+1/0/0`；UI+Server all-target check/Clippy、最终fixture Clippy、contracts/UI WASM、fmt/diff绿；i18n/design/CSS=`517/85/271`；bundle=`1254517/93646/740216/1/0`。release两report/action各1；503后Retry enabled而action/textarea锁定，第二次与第一次runId/botId=bot-0/anchor/message逐字相同且201；Refusals另201，两个durable user/assistant均Agent exact。hard reload后两prompt/button/report各1，1440/600 overflow0、duplicate/nested0；正常应用error0，retry仅预期503 record+既存SRI warning。components=`6/16/22`、parity=`664/1014/1678`、strict=`157/157/0`。只关闭T-CMP-0001；Decisions/Refused sandbox/admin/Desktop/golden继续todo，CSS余4658B。未运行CI/Actions、未push。详见Batch46文档。 |

| R110 | §3.3 / §5.2 / §7.2–§7.5 / §8.5–§8.6 / §13.1–§13.3 / §14.3 / §15.3 / §21.1条5 / §24 G4、G6（2026-08-28 Durable Component Human Decisions batch47） | R109后`askApproval`/`askChoice`仍只有固定上游的内存`respond`语义。本仓若复用acting `tool_approvals`，会把自然语言问答错误绑定到外部effect授权；若只在Leptos内存等待，刷新/跨副本会丢pending且无法证明exactly-once；若先把两个name加入provider再补durability，模型可调用不可恢复的半成品。 | Decisions另立surface/HITL durable状态，不复用acting approval。native0023只expand新增`component_human_decisions`，绑定deployment/tenant/thread/run/actor/AuthGeneration/Agent/provider call/component/canonical args hash，唯一`(run_id,provider_call_id)`；Approval/Choice参数、答案、state/time由closed contract+DB CHECK封闭。Application限定30分钟且不越run deadline、64KiB args/100 choices/16KiB string/4KiB note；PG request/list/resolve/wait每次复核running lease、fresh generation、membership、Agent policy、current build/grant，同request/answer幂等而异值conflict，跨副本以1秒PG poll为真源；request/answer/expire/cancel audit同事务且不含问题/答案正文。Axum/Tauri接typed list/answer、Origin-before-body/no-store。manifest/provider仍保持ordinary 11项，Agent AwaitingHuman与两个renderer留下一批，故T-CMP-0004不勾。 | implementation `b0e7e7f28d103e226d1e1c0a8ee543d4954b0cc1`。PG17.11 SCRAM native fixture regeneration开/关=`1+1/0/0`、decision=`1/0/0`；schema0023=`45表/428列/316 NOT NULL/243约束/91索引/4触发器/4枚举/1函数/0扩展`，SHA-256=`489c0ac781baf4efc12e4a23bd28a1d37a716b54a996fcbe7737dc7acb376e5b`。contracts/application/domain/native/tables/Server/Desktop定向=`6+6+3+3+23+4+1/0/0`；affected compile、七crate Clippy、contracts/UI WASM、fmt/diff绿。API=`60/109/169`、events=`33/53/86`、tables=`58/0/58`、components=`6/16/22`、parity=`672/1014/1686`、fixtures=`17/22/39`，0 violation/warning；strict passed/mismatch/skipped=`158/0/0`。关闭T-API-0168/0169、T-TBL-0057/0058、T-EVT-0083–0086、T-FIX-0039；其中T-TBL-0057是补回Batch36已存在但历史漏记的`user_memory_controls`。本批无production UI/CSS/manifest变化，未重建bundle/浏览器/golden；未运行CI/Actions、未push。详见Batch47正式文档。 |

| R111 | §3.1条10 / §3.3 / §5.2 / §7.2–§7.5 / §8.5–§8.6 / §13.1–§13.3 / §15.3 / §21.1条5 / §24 G4、G6（2026-08-28 Component Decisions Runtime batch48） | R110只有durable request/list/answer/wait，两个name尚未进manifest/provider，reducer的`HumanReleased`却直接回`Sampling + StartProvider`；若照此接线，会在人的answer写成durable tool exchange之前开始下一次sampling。普通工具cancel时drop future→reconciliation是acting effect的正确保守语义，但同样drop human wait会留下state=pending且没有cancelled audit。上游被撤权Decision还必须`respond`，只画RefusedCard会把run永久挂住。 | manifest/provider/schema/Leptos registry同批从ordinary11扩为总13项。runtime把provider call id原样交gateway，Rust UUIDv7另作decision/internal identity；HumanRequired进入AwaitingHuman，HumanReleased只回ExecutingTools且零effect，exchange checkpoint后ToolResultCommitted才load context。human invoke用detached JoinHandle：cancel先正常提交Cancelled，waiter随后从PG观察terminal并退休/audit；ordinary effect仍drop-first/reconciliation。gateway对拒绝/坏参数返回单个error tool result。conversation 1秒读actor pending并按active run过滤，answer防重复，checkpoint reload authoritative pair；Approval/Choice同一renderer回读closed recorded result，Choice id+label exact、Approval note4KiB；默认文案与Input placeholder reactive i18n，Choice显式Enter/Space。 | implementation `b7652c4af39e905a1a65adeb9f5c1072a3d0e2e8`。Agent/contracts/domain/application/UI/Server/Desktop/transport=`33/84/369/6/128/4/78/8`且0失败；PG17.11 catalogue/human/provider-context=`1+1+1/0/0`，新增provider→pending→跨Application answer→exchange→resample完整竖切=`1/0/0`，answer前run running/tool result0，后answered+两audit各1、第二次context exact pair。九crate Clippy、WASM、fmt/diff与tools/i18n/design/CSS=`529/86+74/281`绿；bundle=`1322323/96050/740216/1/0`，CSS余2254B。已提交版本release浏览器Approval approve/decline、Choice Enter、pending→complete→hard reload、中英、四视口、Gallery14 tile/2 Decision全绿；duplicate/nested/alerts/external/console0。components=`7/15/22`、UI=`87/65/152`、parity=`674/1012/1686`、fixtures=`17/22/39`，strict=`158/0/0`。关闭T-CMP-0004/T-UI-0056；Refused sandbox共用、admin/sandbox/Desktop sandbox renderer/formal golden仍todo，G4/G6整关不勾。未运行CI/Actions、未push。详见Batch48文档。 |
| R112 | §3.3 / §5.2–§5.3 / §8.6 / §13.1–§13.3 / §14.1 / §15.3 / §17.2 / §21.1条5 / §24 G6（2026-08-28 Sandboxed Component Governance batch49） | 固定上游有draft/published/revision/sample生命周期，但`save`、`publish`、`remove`把source、共享governance与audit分成多次独立数据库调用；任何中途失败都会留下半发布或orphan。其路由还把email写成作者，published读对部分NULL以空串降级。若Rust照译，就会让audit失败后的业务写存活、compiled名字被沙箱面接管，或让draft/残缺published进入renderer。另一方面，擅自把`hasUnpublishedChanges`扩成比较描述/schema又会偏离固定上游明确的HTML/CSS/JS三列语义。 | 新增closed sandbox contracts与独立无data-function port；exact 2–40字节slug统一服务端加`custom_`，schema/sample结构上只能是object。save/publish/delete各用一个SERIALIZABLE事务同时写source、`components`与allowlisted hash-chain audit；publish双行锁、单DB时间、复制描述+四类源并checked revision+1；delete双重校验namespace与`kind=sandboxed`，可清治理orphan而拒绝compiled。published以FULL JOIN检查双bit、双行和四个非NULL列，任何漂移503且零draft fallback。作者改用AuthContext actor id；Axum parts-only fresh-admin/Origin在body前，Tauri用host window authority；五接口穿同一Arc对拍。`hasUnpublishedChanges`仍只比较HTML/CSS/JS。 | implementation `9e46e128c572ee76c0585da1075e3195b0fdcbdf`。contracts/application/domain/Server/Desktop=`86/148/369/209/79`且0失败；既有transport8+新增sandbox同Arc1均绿。PG17.11 SCRAM lifecycle/collision/orphan/三类audit故障回滚=`1/0/0`，实得revision0→1→2、双表published/空source负例fail-closed、published audit revision `[1,2]`、失败时source/governance/revision/delete全回滚。九crate Clippy、contracts/UI WASM、fmt/diff绿；API=`66/103/169`、components=`8/14/22`、parity=`681/1005/1686`、fixtures=`17/22/39`、strict=`158/0/0`。只关闭T-API-0049–0053/0103与T-CMP-0019；T-CMP-0020因playground/production wrapper未落仍todo，其余sandbox renderer/CSP/nonce/channel/Desktop/a11y同样todo。无UI变化，不冒充browser/golden/bundle。未运行CI/Actions、未push；详见Batch49文档。 |
| R113 | §3.1条10 / §3.3 / §5.2 / §7.2–§7.5 / §8.5–§8.6 / §13.1–§13.3 / §15.3 / §18 / §21.1条5 / §24 G4、G6（2026-08-28 Web Sandboxed Component Runtime batch50） | R112后沙箱只有治理面：published定义未进入provider，Agent把`custom_*`当generic acting tool，会话无production renderer，Playground也未迁移。若只在sampling时检查grant，旧schema/撤权可继续调用；若给沙箱复用compiled data-function会新增上游没有的能力。Web若用`srcdoc`会继承父CSP；只写`sandbox=allow-scripts`又不足以给作者JS一个受控启动序。Desktop若在主Tauri WebView直接创建iframe则违背独立renderer边界。 | dynamic sandbox定义与call-time authorize共用权威PG adapter：每次复核Agent、published、withholding、当前JSON Schema/args，外部`$ref`拒绝；任何`component_functions`行以corruption fail-closed。provider/gateway/durable history接exact `custom_`与上游confirmation。Web用同源普通导航`/sandbox/runner?render=<random>#<local payload>`，iframe policy恰`allow-scripts`；32-byte per-response nonce与exact CSP由Server生成，bootstrap先closed decode/清fragment/写`window.__args`再挂作者script。production与Playground复用同一frame；custom/Tauri scheme零iframe并复用RefusedCard。一次性MessageChannel设计为第二次load转移、2秒无ready即销毁并拒绝。 | implementation `c3e59a8663ba13d9644b3cad4e2599c64151bee0`。contracts/application/Agent/UI/Server/Desktop=`87/149/34/133/210/79`，transport=`8+1/0/0`；PG17.11 SCRAM lifecycle/runtime=`1+1/0/0`；九crateClippy、contracts/UI WASM、fmt/diff、tools绿。i18n/design/CSS=`560/89+74/292`；bundle=`1400622/97848/740216/1/0`，CSS余456B。release IAB实得Playground invalid sample→iframe0、恢复→1，conversation shared Refused2、sandbox exact、srcdoc0、query nonce跨reload不同、两页DOM/overflow/console审计绿；但该IAB不提供可用postMessage/addEventListener/MessageChannel且Chrome不可用，故只证明2秒fail-closed，绝不冒充args注入、作者JS、无网络/回调或channel正向执行。只关闭T-CMP-0008/0011/0015–0017；0009/0010/0012–0014/0018/0020–0022与admin route/UI formal golden保持todo。components=`13/9/22`、parity=`686/1000/1686`、strict=`158/0/0`。未运行CI/Actions、未push；详见Batch50正式文档。 |
| R114 | §5.3 / §10.1–§10.2 / §11.1–§11.3 / §12.4–§12.6 / §13.3–§13.4 / §17.2条4–12 / §18 / §24 G5、G6（2026-08-28 HumanLease与Browser Input Protocol batch51） | R113后若直接实现Desktop component-only renderer，会在尚无`openbot-computer` engine的仓内造出第三种生产渲染器，违背§11.1单一engine。固定上游`control.ts`只有holder/request/secret状态，没有actor/computer/tab/auth generation/epoch/expiry；旧input在take/navigation/restart后仍可能从socket buffer落地。另一方面，§12.5虽写`expires_at`却没有给默认HumanLease TTL，擅自选常量会把猜测写成产品契约。 | 先实现所有browser/component renderer共用的control/HumanLease与closed input protocol。help request逐字保留严格`age>10min`的读取时过期；HumanLease有效期必须由authority caller显式传入，不设猜测默认。take只接不可由外部deserialize的AuthContext，绑定actor/auth generation/computer/tab/computer generation/epoch/expiry；take/transfer/release/inclusive expiry/navigation/restart均推进epoch，fresh authorize逐字段拒绝旧ticket。secret state只存label/ref/权威DocumentGeneration，value复用zeroizing、non-Clone、redacted SecretBytes并走独立typed command。BrowserInput恰八变体，无IME composition/drag；BrowserOperation无自由CDP/upload/file chooser。实际CDP映射/engine reject仍待真实Electron conformance。 | implementation `9d027b22712982546be9cf18d957c730b10c4f67`。`openbot-computer` tests=`8/0/0`，all-target/all-feature Clippy `-D warnings`与fmt绿；Cargo.lock新package0，只新增既有contracts/domain/thiserror/time直接边。parity-check 0 violation/warning，browser-operations=`7/39/46`、components=`13/9/22`、总parity=`693/993/1686`、fixtures=`17/22/39`；strict recount=`158/0/0`。只关闭T-BROP-0005–0009/0045/0046；0036–0044、Electron进程/authenticated framing/CDP/ScreenHub/viewer ticket/Desktop sandbox renderer/a11y豁免全部保持todo。本批无UI/bundle/browser/golden，不运行CI/Actions、不push；详见Batch51正式文档。 |
| R115 | §0.4 / §1.2 / §2.3 条 16 / §3 / §11.5 / §24 G0 / §28.5（2026-08-28 第三轮：范围与真源优先级） | 旧 v4 用户裁决版把 `grok-bot` 置于固定上游 OpenBot 之上（第 3 层高于第 4 层），"GUI interaction/journey：Grok 为主要交互参考"，35 个 extension family 与 21 个 GUI family 全部进入 census，`C` 类"候选"无上限 | 产品的 parity oracle 是固定上游 OpenBot（1686 条台账与 §25 DoD 全部对它定义）；Grok Bot 的产品依赖 Cursor 账号 + 云端 box + `aiserver`（`grok-bot/README.md`），其 GUI 真源是 minified renderer；把它放在 OpenBot 之上会静默改写 32 route 的可观察旅程，且没有任何 oracle 能判 done；在 993 条 todo 未清时扩范围是重写的最大风险 | 五层权威改为：用户裁决 → 本文件 + GUI 真源 → **固定上游 OpenBot** → 参考源（只在点名吸收点上、只提供架构 / 执行语义）→ 实现证据；v4 产品范围 = v3 范围，Grok 产品能力 0 项进入，候选只登记 §11.5 无承诺表；§2.3 追加条 13–16 并声明对参考源派生项同样生效 | 本轮复算 grok-bot 家族计数 185 / 16 / 24 / 471 / 165 / 852 / 6 / 251（与旧 v4 §1.3 相等）、`frontend/src/recovered/features` 目录 21 个（旧 v4 写 20，其 §4.10 表自己列了 21 个名）；`grok-bot/README.md` "Inference Router / remote box / Cursor session"；台账 693 / 993 / 1686 全部定义在上游 `891df72f` |
| R116 | §1.2 / §11.5 / §23.1 条 3、8 / §23.3 / §28.4（2026-08-28 第三轮：`grok-bot` 定位与方法） | 旧 v4：D3 "主要架构、执行与交互参考"，D4 "大面积 TypeScript → Rust 语义翻译"，`T` 类 "近机械翻译 + differential fixture"，census 7 份 inventory 每项 15 字段作为 P0 退出条件；§23.1 条 3 与 `CLAUDE.md` §9 只写 "Grok Build 为 Apache-2.0" | `grok-bot/` 是 Anysphere Grok Bot 0.18.0 的反编译重建（README / PROVENANCE / NOTICE 自述，bundle `com.anysphere.sand`，"No upstream source-code license is asserted"），不是 Apache-2.0 的 xAI Grok Build；§23.3 "不得把反编译结果当源码" 与 §11.4 clean-room 规则被 D4 直接违反而未 supersede；重建代码 `source/`、`frontend/` 零测试，`state.ts:111` 自注 partial recovery，`T` 类的 differential oracle 根本不存在；2,111 文件 / 49 万行的人工分类挡在不依赖它的 Engine 线之前；两个原始安装包只有 LFS 指针、对象 404，默认 clone 恒红 | 定位为第 4 层参考、只提供架构 / 执行 / 状态机语义；唯一方法 = 规格先行吸收（读 → 写规格 → 记 lineage → Rust + 自有 fixture），禁止逐函数翻译与文本复制，`T` / `C` 类取消；census 只有 tier-1 文件级 inventory（xtask 生成，不进分母）；两个 LFS 指针移除、`grok-bot/.gitattributes` 去 LFS 行、目录 `.gitignore` 禁再加入、`research-archives/README.md` 改为 identity-only，tree hash `b68f2497…` → `86f5a85f560f721677fa7e587a67ac0ffc036cb5`；§23.1 条 8 登记为长期风险（用户裁决：不阻断），§23.3 补句；权利人进 §22 | `git lfs logs last` 404 两次（merge 与 archive 各一次）；`find grok-bot/source grok-bot/frontend -name '*.test.ts*' \| wc -l` = 0；`grok-bot/package.json` scripts 只有工具链测试 `node --test tests/*.test.mjs`（8 个文件）；`git rev-parse <staged-tree>:grok-bot` = `86f5a85f560f721677fa7e587a67ac0ffc036cb5`；§28.4 新增命令 |
| R117 | §0.1 / §1.2 / §11.3 / §11.4 / §16.2 / §16.3 / §19.3 / §25 条 3（2026-08-28 第三轮：Electron 获取、shim 谱系、零 npm） | 旧 v4 §3.5 "engine-shim/package.json + package-lock.json 只钉 Electron 与打包闭包"；v3 §1.2 只写 "CrabCode browser kernel Electron 43.3.0"，`tools/pins.toml` 无 Electron 条目，43.3.0 无处获取；§11.4 表把 CrabCode `browser-shell` 列为 shim 复用来源；仓内已因 `grok-bot/` 出现 2 个 `package.json` | npm lock = `npm install electron` = postinstall 从 GitHub 下载二进制，违反 §1.2 "缺工具即红不下载"、可复现构建与 2026-08-22 零 Node 裁决，并引入无闸门的 npm 供应链面；shim 谱系未定（CrabCode 需书面授权，`grok-bot` 是反编译） | Electron 43.3.0（2026-08-04 发布，Chromium 150.0.7871.212、Node 24.18.1）官方 release zip 五平台 sha256 钉入新建 `tools/engine-pins.toml`，上游 `SHASUMS256.txt` 副本入库 `tools/electron-v43.3.0.SHASUMS256.txt`；`cargo xtask engine fetch / verify / bundle`（P0-code）负责下载 / 校验 / rebrand / ASAR / fuses / integrity，零 npm；工作区唯一允许的 `package.json` = shim app manifest（五个键、零 dependencies / scripts / lockfile）；shim clean-room、文件 allowlist、非空 LOC ≤ 600、Electron/Node API allowlist、零 `child_process`，由 `electron-shim-check` 判红；§11.4 `browser-shell` 行降为可选 fixture 来源；`grok-bot/` 内 `package.json` 显式不参与构建 | `curl` GitHub API `releases/tags/v43.3.0`：`published_at` 2026-08-04T18:52:37Z、`prerelease` false、body "Updated Chromium to 150.0.7871.212"；`SHASUMS256.txt` http 200、74 行，五 zip 摘要逐字入 pins；五 zip URL HEAD 最终 200；`raw…/v43.3.0/LICENSE` 首行 "Copyright (c) Electron contributors"（MIT）；本机真下载 win32-x64 zip 144,396,349 B，sha256 与上游逐字相等，解压后 `electron.exe --version` 实跑结果记在 `engine-pins.toml`；`tools.rs` 硬编码四个 tool id，故另建 engine-pins 不影响 `tools verify`；`git ls-files \| grep -c '/package\.json$'` = 2 |
| R118 | §0.1 / §3.3 / §10.6 / §11.1 / §11.2 / §11.3 / §11.6 / §12.2 / §12.6 / §24 G5F、G7 / GUI 真源 §9.1（2026-08-28 第三轮：双 role engine、组件渲染模型、ADR） | v3 §3.3 已写死 Desktop 组件用同一 Electron engine 帧流回 GUI，但没有 role 模型、进程基数、预算、egress 层数、a11y fallback；旧 v4 §3.3 "与 Browser Computer 使用不同 Electron process instance" 未说 N 个组件是 N 个进程还是一个；D2 没有记录被否决的备选 | 没有基数，"CPU/RSS/DOM 硬预算" 无从定义，一条 transcript 5 个组件 = 5 个 Electron 应用进程不可接受；旧 v4 §12.4 把 a11y fallback 留作待明确；D2 与 §10 "shim 每多一行都要解释" 原则之间的张力（standalone Chromium + 直连 CDP 本可零 JS）没有 ADR | `EngineRole::{BrowserComputer, SandboxedComponent}`、`ComponentRenderScope`；组件 engine 每 Desktop 应用实例一个，render session = TabId，独立 in-memory partition 与 `component://<render-id>` opaque origin；预算（≤ 8 活跃 / 256 MiB / 5 s / 5 fps / 100 console error，新增默认值，policy 只能收紧）；三层零 egress（黑洞 proxy + webRequest cancel + CSP）缺一即红；帧 / 输入 / HumanLease 复用同一路径；`RenderSessionOperation` 封闭 enum；a11y fallback = 结构化参数 `<dl>`；§11.6 ADR 记录 Servo / wry / wasmtime / standalone Chromium 的否决理由与 "OS 沙箱包 engine" 的采纳 | v3 §3.3 末段与 T-CMP-0021 `migration_rule` 原文（"§3.3 末段写死的必然后果"，即 v3 从未有过 Tauri renderer 目标，旧 v4 §7.4 的说法不准确）；`crates/openbot-computer/src/browser/protocol.rs` 的 `BrowserOperation` 无 render session 成员；Batch 51 文档 "所有 browser/component renderer 共用 HumanLease/closed input"；`crates/openbot-ui/src/features/gallery/sandboxed.rs` 是唯一 Web frame |
| R119 | §0.1 条 6 / §10.3 / §11.2 / §11.3 / §17.1 / §24 G5A、G5E（2026-08-28 第三轮：engine 进程 OS 约束与 boot handshake） | v3 §10.3 只对 **shell** 高风险模式要求平台 sandbox fidelity；§11.3 只约束 renderer；旧 v4 §3.1 "通过继承的私有 pipe/handle 完成 boot handshake" 且允许 shim "启动 Rust 明确指定的 child/helper" | Electron 主进程即 Node，renderer 逃逸 = 当前用户全部权限，Desktop 上 G5E 只有 renderer 一层；Windows 上向 Node 传继承句柄要走 CRT fd 继承块，三平台不统一；shim 没有任何需要 spawn 子进程的职责 | Desktop engine 进程由 Rust sandbox helper 启动并约束（只允许 profile / temp 读写、loopback 代理出站、自身 helper 执行），fidelity 分级 macOS Enforced / Windows Degraded / Linux tier-2 进入 readiness，Degraded 不阻断但明示，Unavailable 不启动；boot capability 统一经 stdin 一行 + pipe，Rust 校 token 与 peer credential（`SO_PEERCRED` / `getpeereid` / `GetNamedPipeClientProcessId` + 进程创建时间），二进制 digest 在 spawn 前校验；shim 零 `child_process`；正向 sandboxed 判据三平台各一条 | Electron 官方 sandbox 文档（主进程不在沙箱内）；v3 §10.3 fidelity 表原文；`grok-bot/source/electron-main/main.ts:245/308/309` 的 `no-sandbox` / `sandbox:false` / `webviewTag:true` 作为 "不得照搬" 的正向对照 |
| R120 | §2.3 条 13 / §10.6 / §24 G5C（2026-08-28 第三轮：ExecutionRealm） | 旧 v4 G5C "HostLocal/IsolatedComputer 无隐式 fallback"、§4.8 "box → runsc/fixed digest"，未定义 Desktop 的 IsolatedComputer | Grok 的 box 默认是 Cursor 云端沙箱，重建版加了本地 Docker 替代（`grok-bot/README.md` "Local Docker sandbox"；`electron-main/box/local-docker-host-connector.ts`）；macOS/Windows Desktop 没有 runsc；Docker Desktop 是 §0.1 允许列表之外的新外部引擎 | `ExecutionRealm::{HostLocal, ScopedContainer}`；Desktop Local 只有 HostLocal（§10.3 门控）；Server 与 Desktop Remote 的 shell/file = Server 的 ScopedContainer；不引入 Docker Desktop / 本地容器 / VM；两域无隐式 fallback | `git ls-tree` 列出 `grok-bot/source/host/box/*`、`electron-main/box/{remote-connector-egress,local-docker-host-connector,egress-tunnel-wiring}.ts`；v3 §0.2 表 Desktop 列 "每 scope 一个受监管 Electron/Chromium 进程"，无容器 |
| R121 | §1.2 / §10.4 / §11.3 / §24 G5（2026-08-28 第三轮：runsc 内 Chromium 沙箱判据） | v3 §10.4 "runsc production mandatory"，§11.3 "sandbox=true"，旧 v4 §3.4 "禁止 --no-sandbox 且正向证明 renderer sandboxed"；runsc 未钉版 | Chromium layer-1（namespace / setuid）在无 userns 的容器与 gVisor 内常不可用，这正是容器镜像普遍加 `--no-sandbox` 的原因；两条规则会在实施期互相判红；本轮无法在本机验证 gVisor 行为 | 判据固定为 renderer `Seccomp: 2` + `NoNewPrivs: 1` 且 layer-1 存在；`--no-sandbox` / `--disable-seccomp-filter-sandbox` 任何配置禁止；不满足时只改 runsc 版本 / 配置，永不改 flag；P1 spike 在 Ubuntu 24.04 x86_64 + 钉版 runsc 上产证据并把版本写入 §1.2 与 `engine-pins.toml` | Chromium `docs/linux/sandboxing.md`（layer-1 / layer-2 定义）；v3 §28.2 记录上游 Supervisor 已有可选 `Runtime: runsc`；本轮无 Linux 实测，故只定判据不定结论 |
| R122 | §16.2 / §22 / §24 G6、G8（2026-08-28 第三轮：Linux Desktop tier-2） | v3 §16.2 "Linux x64 AppImage/deb 为 supported desktop"；旧 v4 §12.2 "待明确" | golden 矩阵与 Cargo 历史只覆盖 macOS/Windows（GUI 真源 §10.1）；R63 后 Actions manual-only，没有 Linux desktop 机器在环；"待明确" = 静默回退 | tier-2：编译必绿，golden / AX / 签名 / sandbox fidelity 不作 G6 / G8 判据，不是 supported release；升级走独立 delta；Server Linux 不受影响 | GUI 真源 §10.1 平台行；`CLAUDE.md` §10 R63 段；`fixtures/ui/golden/{web,macos-arm64,windows-x64}` 无 linux 目录 |
| R123 | GUI 真源 §10.5 / §15 / v3 §24 G6 / `crates/openbot-testkit/src/xtask/ui_gates.rs`（2026-08-28 第三轮：CSS 预算） | GUI 真源 §10.5 `app.css` ≤ 96 KiB（2026-08-22 设计期契约） | Batch 50 实测 97,848 B，余 456 B；R103 → R111 → Batch 50 三点 93,646 → 96,050 → 97,848（7 批 +4,202 B ≈ 600 B/批）；剩余 24 route + Composer / AppSidebar 估 +18 KiB；固定成本已在内，`css-check` 的字面量规则也不允许动态拼类名来压缩 | 上限 128 KiB，120 KiB 警戒只 warning；`ui_gates.rs` 的 `CSS_LIMIT` / `CSS_WARN` 同批落地；再放宽只能 delta audit 且先证明复用已做尽 | `grep -n CSS_ crates/openbot-testkit/src/xtask/ui_gates.rs`；`CLAUDE.md` §1 "CSS余456B"；R103 / R111 行内数字；本机验证方式记在 §28.5（Windows 原生构建受 samael/xmlsec 与 openssl-sys 阻塞，是 §24.1 G2 已登记的未闭合项） |
| R124 | §19.1 P0-code / §19.3 / `parity/components.yaml` / `parity/browser-operations.yaml`（2026-08-28 第三轮：台账 overlay 与两条 target 修正） | 旧 v4 §6.12 为 693 项各写 disposition + lineage + affected_symbols + affected_test_ids + revalidation（7 种 disposition × 4 种状态），三处台账目录；T-CMP-0021 target 在 `openbot_ui`；T-BROP-0046 target `openbot_computer::screen::lease::HumanLease` 不存在，`HumanLeaseEpoch::next` 用 `saturating_add` | v4 尚未改动任何符号，此刻 693 项除 4 条外全是 carry，全量分派是空转；schema v1 顶层键固定，新 schema 文件放 `parity/` 顶层会被 `parity-check` 判红；饱和后 epoch 不再推进、栅栏失效且被断言固定 | exception-only 的 `parity/overlay/v4.yaml`（P0-code 建立 + xtask 校验；disposition 收敛为 carry（隐含）/ revalidate（可带 `defect`）/ split / superseded）；初值三行：T-BROP-0046 revalidate + defect、T-CMP-0015 split web、T-CMP-0018 split web；批次按 `git diff` 命中的 `target` 前缀自动追加 revalidate 行并要求重跑；本 PR 改 T-CMP-0021 target/owner → `openbot_computer::component::runtime` / openbot-computer，T-BROP-0046 target → `openbot_computer::control::ControlService`；`HumanLeaseEpoch` 改 checked + poisoned 为 P0-code 项 | `parity/README.md` 规则 1 / 4 / 5；`control.rs:50` 与断言 `HumanLeaseEpoch::new(u64::MAX).next().get() == u64::MAX`；`grep -n "pub struct" control.rs` 无 `HumanLease` 类型；`parity-check` 本机运行方式见 §28.5 |
| R125 | §0.4 / §16.4 / §18 / §19 / §22 / §24 / §28.5 / 旧 v4 文档 / GUI 真源 §2（2026-08-28 第三轮：闸门、阶段与其它） | v3 §0.4 / §19 12 人 52 周；旧 v4 §8 重写 G0–G8、§9 P0–P8、§13 "下一批先完成 P0 census"；§16.4 零 phone-home 闸门 "二进制内出现外部分析域名" 未限定第一方；旧 v4 提交 `eb68406` 顺带带入 4 个无引用品牌文件 | census 挡路（见 R115 / R116）；Electron 二进制含 Google 域名字符串会让闸门恒红；两个自称权威的文档并存（`CLAUDE.md` 只认 v3）；品牌资产违反 GUI 真源 §2 "待商标清查后另立文档" | 日历作废改阶段门（§19.1，Engine 线与既有余项并行）；G0 / G5 子闸门 / G6 / G7 / G8 增补；§16.4 限定第一方并把 Grok telemetry 家族判 `R`；§18 加 Engine 行；§22 加八行；旧 v4 文档加 superseded banner，`CLAUDE.md` / README 指针；品牌概念稿在 GUI 真源 §2 登记为候选稿、不进 bundle；§28.5 记录本轮修订方法与五层权威 | `git show --stat eb68406`（4 个 brand 文件）；`grep -rln brand-concept docs CLAUDE.md README.md` = 0；v3 §24.1 1 勾 / 8 未勾 |

### 28.2 复核通过、原样保留的断言

§1.3 八个静态数字（504 / 72,000 / 28 / 13 / 31 / 95 / 105 / 1,007）逐个相等；§1.2 四个上游 commit（AG-UI `e42bdbed…` 08-21、RMCP `4a738b9d…` = tag `rmcp-v3.1.4` 解引用、Codex `4f39251a…` 08-22、Grok Build `19d42e35…` 08-19）与 `rmcp 3.1.4` / `tauri 2.11.5` / `openidconnect 4.0.1` / Electron `43.3.0` + Chromium `150.0.7871.212`（CrabCode `kernel-pin.json`）/ MCP `2026-07-28` 规范 URL 全部存在；§2.4 的 #36 / #44 / #53 / #72 / #106 全部 open，#119 两次 API EOF 未能复核状态（GitHub 作用域代理间歇握手失败，不构成 DIFFERS）；"Disconnecting is not built yet" 原文在 `connected-accounts/$key.tsx:167`；MCP 允许调用的 audit 确实发生在 vendor 调用之后（`plugins/store.ts` 的 `mcp.call_succeeded/failed` 在 `vendor()` 之后，`call_rejected` 在之前）；`knowledge.sources` 只在 `tenant-package.ts` 解析、全仓无消费者；per-call MCP client（`mcp.ts:211–227`）；`VendorTransport` 两实现（`transport.ts`）；worker 只返回 `{status:"idle"}`；AES-GCM v1 envelope 12 字节 IV、无 `additionalData`，`sso_providers` 两字段同用该 envelope；Entra claim 顺序 `email → upn → preferred_username`；SSO 三条注册路由管理员前置守卫；`TOOL_STEPS = 8`；run assertion `RUN_TTL_MS = 10 分钟`；默认 policy `allow: ["true"]`；CrabCode `in_process::embed()` 仍是 stub、生产 Agent 仍启动 Bun worker（与 §2.1 条 5 一致）；CrabCode 根 notices 文件明示 closed-source proprietary（与 §2.2 / §23.1 条 7 一致）；Supervisor 现状已有 `no-new-privileges` / `CapDrop ALL` / `PidsLimit 512` / 可选 `Runtime: runsc`，§10.4 是在其上加固而非从零。

### 28.3 审计后仍然成立的裁决（不改）

- Desktop sandboxed component 用独立 renderer（§0.1 条 3 / §2.3 条 11）：本轮只补代价披露，不推翻——推翻需要三平台 WebView 子帧脚本注入行为的实测证据，本轮没有。
- 12 人 / 52 周：第二轮时无法用源码证伪而保留；2026-08-28 R125 作废，改为 §19.1 阶段门。
- 新增的限额、egress gateway、Vault v2、approval 绑定等加固项：均有上游缺陷或威胁模型支撑，且都已标注"parity / 新增"。

### 28.4 计数复算命令（在固定 commit 的干净克隆根目录执行）

```bash
git rev-parse HEAD                                                   # 891df72f1827454d8b353d108fe5dd2313b7e30d
git ls-files | wc -l                                                 # 504
git ls-files '*.ts' '*.tsx' | xargs cat | wc -l                      # 72000
cat server/src/db/schema/*.ts | tr '\n' ' ' | grep -oE 'pgTable\(\s*"[a-z_]+"' | wc -l   # 28
ls server/drizzle/*.sql | wc -l                                      # 13（0000–0012）
git ls-files 'app/src/routes/*' | wc -l                              # 31
grep -rnE '^\s*(app|routes)\.(get|post|put|patch|delete|all)\(' server/src --include=*.ts | grep -v '\.test\.' | wc -l   # 95
grep -rnE '^\s*app\.route\(' server/src --include=*.ts | grep -v '\.test\.' | wc -l       # 9（模块挂载，不计入 95）
grep -cE '\.(get|post)\("/' supervisor/src/index.ts                  # 5
grep -oE '"/[a-z][a-z0-9/:_-]*"' agent-computer/src/index.ts | sort -u | wc -l   # 29
git ls-files '*.test.ts' '*.test.tsx' | wc -l                        # 105
git ls-files '*.test.ts' '*.test.tsx' | xargs grep -hoE '\b(test|it)\(' | wc -l   # 1007
grep -oE '"[A-Z][A-Z0-9_]{3,}"' server/src/config.ts | sort -u | wc -l            # 32
grep -oE '^\| `[A-Z][A-Z0-9_]+`' docs/configuration.md | sort -u | wc -l           # 48
grep -c 'const TOOL_STEPS = 8' server/src/copilot.ts                 # 1
grep -c 'RUN_TTL_MS = 10 \* 60 \* 1000' server/src/agents/callback-token.ts   # 1
grep -c 'MAX_RESULT_CHARS = 20_000' server/src/plugins/mcp.ts        # 1
grep -c 'quality: options.quality ?? 70' agent-computer/src/screencast.ts      # 1
grep -c 'DROP EXTENSION IF EXISTS "vector"' server/drizzle/0010_drop_the_document_index.sql   # 1
grep -rn 'knowledgeSources' server/src app/src --include=*.ts --include=*.tsx | grep -v test | wc -l   # 2（定义 + 赋值，零消费者）
grep -rlE 'page\.on\(|setInputFiles|filechooser' agent-computer/src | wc -l     # 0（无下载/上传/对话框处理）
grep -c 'legacyToken && sameToken' server/src/agents/callback-token.ts        # 1（共享 token 旧路径仍在）
# R20（2026-08-22）GUI 基线，全文见设计系统文档 §17
( cd app/src && find routes -name '*.tsx' | wc -l )                                                   # 31
( cd app/src && find routes -name '*.tsx' | grep -vE '(__root|_authed|_app|route)\.tsx$' | wc -l )    # 26（页面；其余 5 个是 layout）
( cd app/src && ls components/ui | wc -l )                                                            # 21
( cd app/src && find components -name '*.tsx' -not -path 'components/ui/*' | wc -l )                  # 45
( cd app/src && grep -rhoE '\bIcon[A-Z][A-Za-z0-9]+' --include=*.tsx --include=*.ts . | sort -u | wc -l )   # 47
( cd app/src && grep -c 'prefers-color-scheme' lib/theme.ts components/theme-provider.tsx styles.css )        # 各 0（主题不跟随系统）
( cd app/src && grep -rlE 'useTranslation|i18next|react-intl|next-intl|<Trans\b' --include=*.tsx --include=*.ts . | wc -l )   # 0（零 i18n 框架）
grep -cE '"@shikijs/core@' bun.lock                                                                   # 1（streamdown 的高亮器）
```

R116 / R117（2026-08-28）参考树与 Electron pin 的复算命令（在本仓根执行）：

```bash
git rev-parse HEAD:grok-bot                                                                                # 86f5a85f560f721677fa7e587a67ac0ffc036cb5
for d in source/electron-main source/electron-preload source/node-agent-coordinator source/host source/shared source/packages frontend/src/recovered/features; do printf '%6d %s\n' "$(find grok-bot/$d -type f | wc -l)" "$d"; done   # 185 16 24 471 165 852 251
find grok-bot/source/local-exec-daemon grok-bot/source/box-exec-daemon -type f | wc -l                    # 6
find grok-bot/source/packages/proto grok-bot/source/packages/redacted-protos -type f -print0 | xargs -0 cat | wc -l   # 263713
find grok-bot -type f \( -name '*.ts' -o -name '*.tsx' \) -print0 | xargs -0 cat | wc -l                    # 493338
find grok-bot/source grok-bot/frontend -type f \( -name '*.test.ts' -o -name '*.test.tsx' \) | wc -l         # 0（重建代码零测试）
ls grok-bot/frontend/src/recovered/features | grep -vc '\.ts$'                                              # 21（GUI feature 目录）
grep -rl '^version https://git-lfs' grok-bot | wc -l                                                        # 0（R116 后无 LFS 指针）
git ls-files | grep -c '/package\.json$'                                                                    # 2（均在 grok-bot/，不参与构建；P1 落 shim manifest 后为 3）
for p in darwin-arm64 darwin-x64 linux-x64 linux-arm64 win32-x64; do a=$(grep "electron-v43.3.0-$p.zip" tools/electron-v43.3.0.SHASUMS256.txt | cut -c1-64); b=$(grep -A3 "asset = \"electron-v43.3.0-$p.zip\"" tools/engine-pins.toml | grep -oE '[0-9a-f]{64}'); [ "$a" = "$b" ] && echo "$p MATCH" || echo "$p DIFFERS"; done   # 五行 MATCH
```

对不上 = 上游 commit 变了或本文件漂了 → 先核 `git rev-parse HEAD`，再按 §1.2 走 delta audit。

### 28.5 第三轮就地修订（2026-08-28，v3 → v4）：方法与真源优先级

审计方式：在 `origin/main`（`2a0c542`）的干净 worktree 上读本文件相关章节、GUI 真源、`CLAUDE.md`、九份台账、Batch 49–51 文档、移交指南与 `grok-bot/` 元数据；旧 v4 用户裁决版的每个计数亲自复算（§28.4 新增命令），每条 "参考源里有 X" 的断言 `git grep` 到符号；Electron pin 经 GitHub release API 与 `SHASUMS256.txt` 实取，并真下载 win32-x64 zip 校 sha256。R115–R125 只改本文件、GUI 真源、`CLAUDE.md`、README、移交指南、两份台账的 target / notes、`ui_gates.rs` 两个常量、`grok-bot/` 的三个元数据文件与两个 LFS 指针、`tools/` 的两个新文件；不动任何业务代码。

本机验证边界（如实）：本轮在 Windows 11 上进行，工作区完整 `cargo test` 因 `openbot-infra` 的 `openssl-sys` / `samael`（xmlsec FFI）在 MSVC 上无法原生构建（§24.1 G2 已登记的 "Windows 原生构建" 未闭合项），因此只能：① 编译 `openbot-testkit` 的 `xtask` bin（不含 dev-dependencies，覆盖 `ui_gates.rs` 改动）；② 用该 bin 运行 `cargo xtask parity-check`（覆盖两份台账的 target / notes 改动）。结果记在本节末尾与本 PR 描述；`cargo test -p openbot-testkit`、`recount`、`bundle-budget` 的完整实跑留给拥有 macOS / Linux 环境的 P0-code 批次，不冒充已跑。

真源优先级（五层，冲突时高层胜出）：

1. 用户裁决（本表的 R 行与 §0）；
2. 本文件（后端）与 GUI 真源（视觉 / 主题 / i18n / a11y）；
3. 固定上游 `CopilotKit/openbot@891df72f…`：产品 / API / schema / 旅程 / 迁移兼容的 oracle；
4. 参考源（`grok-bot/`、Codex、Grok Build、CrabCode）：只在本文件点名的吸收点上有效，只提供架构 / 执行 / 状态机 / 协议语义，不提供产品行为（§11.5）；
5. 实现与机械证据（Rust 代码、台账、fixture、测试、签名产物）。

旧 `docs/2026-08-28-OpenBot-TauriGUI-ElectronChromium-GrokBot大面积Rust迁移-v4修订计划-用户裁决版.md` 已被本轮吸收并加 superseded banner；与本表不一致处以本表为准。同日的只读前置审计原文归档为 `docs/2026-08-28-v4修订计划-前置审计.md`（证据记录，不是规范）。

本轮实跑结果（2026-08-28，Windows 11，worktree 基于 `2a0c542`）：

- `cargo build -p openbot-testkit --features xtask --bin xtask --locked`（`CARGO_TARGET_DIR=target-xtask`）：`Finished dev profile … in 19.84s`，覆盖 `ui_gates.rs` 的 `CSS_LIMIT` / `CSS_WARN` 改动；
- `xtask parity-check`（同一二进制，在本 worktree 根运行）：`parity-check: 通过（0 违反）`，合计 entries=1686、done=693、todo=993，fixtures 39/17/22——两份台账的 target / notes 改动不改变任何计数；
- §28.4 R116 / R117 段的每条命令逐一实跑：家族计数 185 / 16 / 24 / 471 / 165 / 852 / 251 与 daemon 6、proto 263,713、TS/TSX 493,338、测试 0、feature 目录 21、LFS 指针 0、`package.json` 2、Electron 五平台 `MATCH`；`tools/engine-pins.toml` 经 Python `tomllib` 解析通过，其四条 `[[recount]]` 分别得 5 / 5 / 74 / `MATCH`；
- Electron win32-x64 zip 本机真下载 144,396,349 B，`sha256sum` 与上游逐字相等；解压后 `./electron.exe --version > ver.txt 2>&1` 得 `v43.3.0`；
- **未跑**（如实）：`cargo test`（含 `openbot-testkit` 单测）、`cargo xtask recount`、`bundle-budget`、`clippy`、`fmt`——全部因 Windows 原生构建被 `openssl-sys` / `samael` 阻塞（`cargo tree -i openssl-sys` 显示经 `openbot-infra` 的直接依赖与 `samael 0.0.22` 两条路径进入 testkit 的 dev-dependencies）；`cargo fetch --locked` 经 GitHub 作用域代理成功，说明缺的不是 crate 而是 C 工具链。P0-code 批次在 macOS / Linux 上补齐这些实跑，不冒充已跑。
