# CLAUDE.md

OpenBot 全量 Rust 重写 —— 仓库级 AI 协作指引，入仓**首读这一份**。本仓 **public**。

> 真源 = `docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md`（v3）。本文件只摘约束与理由，细节一律以方案章节为准；两者冲突时以方案为准，并同 PR 修订本文件。

---

## 1. 真源与现状

- **唯一实施真源**是上述方案 v3（§28 为第二轮审计修订记录）。两份输入文档仓内不存在，只登记了 SHA-256；在 Phase 0 把原件归档到 `docs/inputs/` 之前，**不得**以"输入文档里写过"作为依据（§1.1）。
- 当前仓库处于**方案归档阶段，零实现代码**。第一批代码 = Phase 0 的机器可检查产物：`parity/*.yaml`、`provenance/sources.spdx.json`、`fixtures/**`（§19.3）。CI 必须拒绝未归类项与没有证据的 `done`。
- 上游对照固定在 `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`，不引用会漂移的 `main`（§1.2）。

## 2. 目标定义（为什么是这条线）

"全量 Rust"的定义固定为：GUI、业务、内置 Agent、策略、数据库访问、线程与记忆、实时事件、认证、凭据、审计、Supervisor 与全部高权限控制面由 Rust 实现（§0.1）。

允许的非 Rust 例外**只有**：Leptos/WASM 由系统 WebView 渲染；PostgreSQL / Chromium / Electron / OS keychain 作为外部引擎；用户自己的 HTML/CSS/JS 组件作为**不可信数据**在零权限沙箱里跑；用户接入的远程 AG-UI Agent 任意语言；第一方非 Rust 源码只剩**最小 Electron browser-engine shim**（只管 Chromium 生命周期、CDP、画面帧、封闭输入，无任何业务裁决权）。

理由：TypeScript 控制面、CopilotKit Intelligence 真源、跨用户 profile、MCP 过度实现、双数据库、多 driver 是上游的结构性风险（§27）；Rust 不做唯一控制面就不叫重写。

两个发行物共用同一 Rust core 与同一份 Leptos GUI：`openbot-server`（Axum，多用户）与 `openbot-desktop`（Tauri typed in-process；远程模式走同一 Axum API）（§0.2）。

## 3. 固定基线（改任一项 = 新建 delta audit，禁止静默升 lockfile）

| 项 | 钉死值 |
| --- | --- |
| Rust | `1.94.1`，edition 2024 |
| Tauri / Leptos | `2.11.5` / `0.8.19`（0.8.20 已存在，不升） |
| RMCP | `3.1.4` |
| CEL | crate **`cel`** `0.14.3`（"cel-rust"是仓库名，不是 crate）；oracle = `cel-js@0.8.2` |
| OIDC / SAML | `openidconnect 4.0.1` / `samael 0.0.22` |
| Browser kernel | Electron `43.3.0` / Chromium `150.0.7871.212` |
| 数据库 | PostgreSQL 17，**唯一**语义；Desktop 由 Rust 监管本机 sidecar；不需要 pgvector |

上游 oracle 运行时版本（copilotkit 1.68.3、ag-ui 0.0.57、better-auth 1.7.1、mcp sdk 1.30.0、playwright 1.62.1 …）以 §1.2 表为准，fixture 与 golden 只认这些版本。

## 4. 架构约束

- **10 crate workspace**（§5.1）：`contracts / domain / application / infra / agent / computer / server / ui / desktop / testkit`。建新 crate 只有四个理由：独立安全边界、独立发布单元、明显不同的 feature graph、可单独复用的纯协议；其余用 module。理由：细粒度 crate 放大编译、循环依赖与重构成本。
- **唯一业务入口** = `openbot-application::ApplicationService`（`execute` / `subscribe`）。Axum、Tauri、测试、迁移工具只做认证、framing、大小限制、错误映射，**不得**各自实现业务规则，不得接受自由 method string、renderer 自报角色或任意 SQL（§5.2）。
- **ID 一律 string newtype**，不限定 UUID；兼容端必须接受上游既有字符串。`AuthContext` 只能由 Rust 从 session / peer / DB ACL 构造，外部传来的同名字段都是不可信输入（§5.3）。
- **Agent reducer 必须 pure**：`reduce(state, event) -> (state, effects)`；DB、provider、MCP、browser、file、shell 都是 effect。每 thread 一个 foreground actor 串行处理；后台工作是独立 durable run（§7.2）。
- **工具只有一条执行管线**（§8.1）：validation → 权威 actor/target → effect 分类 → CEL + 内容策略 → 审批 → **事务写 decision + attempt** → 单次 capability → 执行 → outcome + commit_state。decision 写失败即不执行；执行了但 outcome 写不进去 → `ReconciliationRequired`，不自动重试。
- **parity 与新增必须分开标注**。示例：tool step cap = 8（parity）、`AGENT_STALL_TIMEOUT_MS`（parity）、`OPENBOT_RUN_DEADLINE_MS` 默认 30 min（**新增**）（§7.2）；MCP 四个上限只有 20,000 字符是 parity（§9.1）；memory 页是 31 route 之外的 +1（§3.1）。理由：把新增写成"当前行为"是 v2 审计里最重的一类错误（§28.1 R1）。
- **数据真源**：Rust/PostgreSQL 是 thread、message、run、memory、realtime cursor、run lock 的唯一真源；未设任何 `INTELLIGENCE_*` / `COPILOTKIT_*` 变量时产品必须完整运行；Intelligence 只用于一次性导出导入，不做双写、不留隐藏 fallback（§4.1）。
- **Schema 兼容期只允许 expand**：新表、nullable column、backfill、index、非破坏性 constraint；禁止 drop / rename / 类型收紧 / 主键改写；无 downgrade migration（§14.3）。审计表不做分区，hash chain 以追加 nullable 列落地（§8.6）。
- **环境变量三档** preserve / rename / remove 已在 §15.4 裁决；被 remove 的变量出现在生产配置里必须**启动报错**，禁止"读不到就当没设"。
- **错误语义固定**（§15.3）：未登录 401；角色不足 403；资源不可见统一 404（防枚举）；policy refusal 403 + stable code；stale generation / lease 冲突 409；空 thread history 200 + 空列表。文案可本地化，code / status / audit 类型不变。

## 5. 发布级不变量（任一违反 = P0，立即停发，不允许风险接受豁免；§17.2）

1. Rust 是 actor、target、policy、approval、capability、audit 的唯一铸造者。
2. 任一 acting effect 之前都有 durable decision + attempt。
3. deny 优先；空 / 坏 / 未知 policy fail-closed。新安装没有隐式 `allow: ["true"]`。
4. browser target 显式，不默认 active tab；snapshot ref 绑定 document generation。
5. profile 不跨 credential principal（`ProfileScope = tenant + bot + credential_principal`，`bot_id` 不够，§10.1）；workspace 不跨 thread/channel。
6. engine restart/reset 使旧 ref、ticket、approval、capability、lease 全失效。
7. human lease 期间 Agent acting **立即拒绝、不排队**。
8. secret 不进模型、GUI state、browser event、普通日志、trace、screen URL。
9. non-idempotent unknown commit 不自动重放。
10. renderer/XSS 不能扩大 Tauri capability；remote content 无 Node/Tauri/Electron API。
11. Server browser 无直连互联网，只经 per-scope egress gateway；`runsc` 在多用户 Server 是强制项，起不来 readiness 就失败。
12. 任一跨 scope 数据 / 帧 / 凭据泄漏是 P0。

## 6. 明确不做（已裁决删除的过度设计，§2.3；提出即需重新立项）

Firecracker / youki · Restate / Temporal 等 durable execution 平台 · Codex AppServer JSON-RPC 全协议复制 · MCP stdio / resources / prompts / tasks / elicitation 与常驻 ConnectionSet · SQLite 第二数据库 · xAI 专用 crate · ACP / Rig / Goose · 全域事件溯源 · Electron + chromiumoxide 两套 driver · 用户 JS 进主 Tauri WebView · 持久化原始 provider stream / HTTP body / screen frame / secret · Playwright 与 CrabCode engine 两套生产实现。

同样**不复活**上游已否决的 customer document index / ACL 索引；`knowledge.sources` 只解析、不执行（§2.2）。

## 7. 上游缺陷不得照译（§2.4）

#36 redirect/DNS rebinding · #44 malformed AG-UI content 崩 UI · #53 credential rotate 孤儿 · #72 空 history 500 · #106 stale grant 复活 · Drive disconnect 未实现 · **`allowed_groups` 从 no-op 变为真控制**（`all` / 具名组 / 空列表三档，单用户模式语义见 §6.5；上游包声明的 channel 对所有人不可达，官方示例用的就是 `[all]`）· MCP 审计在 vendor 调用之后。每条的 Rust 版确定语义以 §2.4 表为准，实现时不得"先照译再修"。

## 8. 证据与文档纪律

- 进入代码、commit、文档的每个判断都要有**本轮亲自跑出来的证据**；"应该 / 大概 / 按理说"不是依据。subagent 报的结论必须自己 grep / read 复核并再追一层。
- **每个计数都必须能被一条命令复算**（§28.4 就是范式）；复算不上的数字不写。
- 位置引用用符号名（函数 / 常量 / 文件），方案正文禁止裸行号（§28 审计表里的行号是证据记录，不是契约）。
- "当前行为"断言必须 grep 到上游常量或调用点；"上游没有 X"必须配正向对照（例如 `grep` 命中为空 **且** 同一命令在存在 X 的文件上能命中）。
- 新文档落 `docs/<YYYY-MM-DD>-<主题>-<形态>.md`（日期取本机当天，机器在 America/Los_Angeles）；修订方案必须在 §28 风格的修订记录里写"v_n 表述 / 问题 / 修订 / 证据"四列。
- 本仓 public：写入任何文档前确认不含 CrabCode 内部路径、错误字符串字面量、私有 URL 或凭据；CrabCode 相关表述的粒度不得超过方案 §1.2 / §11.4 已有水平。

## 9. 来源、许可与品牌（§23）

- OpenBot 上游 MIT，衍生实现保留 `Copyright (c) 2026 CopilotKit` 与 MIT 文本；Codex / Grok Build 为 Apache-2.0，复制文件保留 SPDX、来源 commit、修改声明；Grok Build 里源自 Codex/OpenCode 的工具必须回溯原始来源。
- **CrabCode 是闭源专有软件**：每个复制文件须有 `SOURCE_PROVENANCE`（权利人、原路径、上游 commit、原/目标 hash、许可证、修改声明、书面授权编号）；workspace 里的 `license = MIT` 是元数据，不等于授权。无授权只能按行为 clean-room 重写（§11.4）。
- 新项目**默认闭源、all-rights-reserved**；开源须另立书面决议 + whole-tree license audit（§23.2）。
- native thread/memory/realtime 只能依据 OpenBot MIT 源码、开放协议与黑盒可观察契约 clean-room 实现；不得把 Intelligence 私有响应或反编译结果当源码（§23.3）。
- 对外名称、bundle ID、domain、deep-link scheme 不得含 OpenBot / CopilotKit / Codex / OpenAI / Grok / xAI；内部代号 `openbot-rs`。禁止复用 CrabCode 的 updater key、bundle ID、证书、OAuth client（§16.2）。

## 10. 闸门

CI 固定（§16.3）：`cargo fmt --check` · `cargo clippy --all-targets --all-features -D warnings` · `cargo test --locked` · `cargo deny` · `cargo audit` · `cargo vet` · OSV / secret scan · license / NOTICE / provenance 校验 · SBOM · 可复现构建 · 签名校验。`Cargo.lock` 与 engine lockfile 提交；git 依赖必须钉 commit；核心 crate `unsafe_code = "deny"`。

Go/No-Go 走 G0–G8（§24），**任何闸门失败只能修复后重跑，不能以"后续补齐"进入下一阶段**。DoD 十条见 §25——没有 parity ledger 100% 归类、跨 scope 泄漏 = 0、audit-before-action 违规 = 0，不得宣称"全量完成"。

## 11. 协作约定

- **中文**沟通、报告、commit 主题；标识符 / 路径 / 命令原样。commit 主题 `type(scope): 一句话 —— 根因或理由`。
- 分支 `docs|feat|fix/<YYYY-MM-DD>-<主题>`；交付 = push 分支 + 开 PR + 停在移交；合并用 **merge commit**（不 squash / rebase），保留原 commit 可追溯。push 前 `git remote -v` 确认目标是 `acosmi/OpenBot`。
- 实施型任务做到底；合法停止只有两种：用户叫停、撞到需用户裁决的真分歧（设计多选一 / 不可逆或对外动作 / 超出授权）。
- 子代理只写码、不碰 git；其结论主控亲自复核。
