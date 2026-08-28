# CLAUDE.md

OpenBot 全量 Rust 重写 —— 仓库级 AI 协作指引，入仓**首读这一份**。本仓 **public**。

> 真源 = `docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md`（v3，架构 / 能力 / 旅程）+ `docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md`（GUI 视觉 / token / 布局 / 主题 / i18n / a11y / 视觉闸门）。本文件只摘约束与理由，细节一律以两份方案章节为准；两者冲突时以方案为准（视觉归设计系统文档、架构归 v3），并同 PR 修订本文件。

---

## 1. 真源与现状

- **唯一实施真源**是上述方案 v3（§28 为第二轮审计修订记录）。两份输入文档仓内不存在，只登记了 SHA-256；在 Phase 0 把原件归档到 `docs/inputs/` 之前，**不得**以"输入文档里写过"作为依据（§1.1）。
- 阶段进度：**Phase 0（Evidence Freeze）产物已落地** —— `parity/*.yaml`（9 份；条目数随实施推进增长，真源是各台账自己的 recount，由 `cargo xtask recount` 逐条实跑，**不在本文件钉死**）、`provenance/sources.spdx.json`、`fixtures/**`、`tools/pins.toml`、10 crate 骨架、`cargo xtask parity-check`（§19.3）。CI 必须拒绝未归类项与没有证据的 `done`。
  **G0 仍有一项未闭合**：§1.1 要求把两份输入文档原件归档到 `docs/inputs/`，仓内与本机都不存在原件，只有 SHA-256 —— 在补齐之前不得宣称 G0 通过。
- **G1（Rust Core 与 PostgreSQL）四条判据本轮全部达成**（§24，四条缺一不可）：① 10 crate workspace + `cargo build --workspace --all-features --locked` 绿；② 同一个 `Arc<dyn ApplicationService>` 经 Axum 与 in-process 两条 transport 结果一致（当前 `cargo test -p openbot-testkit --test transport_parity` = 8 passed，Batch30新增channel detail与port scope逐字段对拍）；③ 28 表 / 13 migration 映射对走完 13 条 migration 的真参照库逐字段相等，read checksum 168/168 行逐字节相同；④ tracing span + 关联字段 + 脱敏 + Prometheus metrics 从首个 vertical slice 生效。
  **W-4 已关掉四项旧缺口**：production PostgreSQL/单用户 `AuthResolver`、独立 `main.rs`、`/metrics` session 访问控制、MatchedPath route span/metrics 均已落地；people/audit/policy 命令另有同一 Arc 的 Axum/in-process 对拍；非 loopback 明文 HTTP 同时打启动告警并在 readiness 投影 `insecure_transport:true`。W-5 batch 7 已补 production Tenant Package loader/sync。G3 batch 1–5 依次落 native DB、thread/live、history/memory、run runtime/WS 与 importer。G3/G4/G6 batch 6–41 已把 pure Rust Agent、三provider、RMCP/OAuth/Drive/approval、GUI地基/Tauri/27primitive/46图标/13业务组件、session sign-out、channel activity/detail/shared thread、ChannelRow、Agent roster/detail、channel create/routing/native first turn、`/channel/new`、Composer draft/queue纯状态、channel snapshot/SSE/plain transcript/idle send、durable跨副本Stop+production mount-local queue、actor-scoped Memory Controls/GUI、Settings Preferences稳定队列、完整Settings 200px secondary sidebar、actor Connected Accounts与Components Gallery治理读面/Quote+Cards五renderer落成。**以下仍未闭合，不得算进去**：read checksum/Drizzle 账本深校验；三家 recorded/live vendor trace、Approval真实PG浏览器/critical realtime/thread集成、完整run-wide budget；Desktop Local OAuth、MCP private egress/admin UI、完整AG-UI事件；browser/file/shell及各自协议级cancel；actual customer export/legacy production drills（固定上游只有known-thread读取、无合法枚举/event/memory export面，等待经许可API/数据）；G5前多用户readiness；package removed；AppSidebar剩余skills/admin，完整channel/home Composer（sources/附件/per-channel draft/steer/markdown/tool boundary/Screen）与Agent lifecycle route、external identity/Tauri binary/window lifecycle、其余route/component/brand/runtime/golden/a11y。Batch41 targeted：contracts/application/Agent/Server/UI **80/140/28/204/119**、infra/Desktop **306/78**、PG17.11 SCRAM **1**、transport **8**、Clippy/WASM/bundle/浏览器/parity/recount均0失败；i18n/design/CSS **496/82/257**，bundle **1138033/86968/740216/1/0**。Cards conversation runtime grant/decision、剩余8个renderer、formal golden与sandbox/Desktop renderer仍未完成；没有运行`cargo xtask ci`。fixture/native latest仍为 **0022**，见 R42/R43/R49–R104。
- **G2（Auth/Vault/Policy/Audit）已从纯规则推进到真实 OIDC/SAML 网络/FFI 竖切**，四条判据（§24 G2）逐条如实：
  ① **CEL corpus 对等 —— 达成**：`fixtures/policy/cel-corpus.json` 的 69 条逐条在 Rust `cel 0.14.3` 上实跑，与 `cel-js@0.8.2` oracle 的分歧集合**恰好等于**一张写死的 6 条台账（`BTreeMap` 双向相等 + `evaluated == 69` 防跳过：多一条 = 既有规则语义悄悄翻转，少一条 = `cel` 改了行为）。6 条全是 `error → 非 error`，deny 侧放宽 2 条、allow 侧放宽 4 条。
  ② **acting before durable decision = 0 —— 通用 application 边界与真实 MCP/Memory/Drive 集合成员达成，整项仍未闭合**：`InvokeTool` 走 domain 十二段状态机；policy 结论只能由 domain 构造；decision+attempt commit 与 capability CAS 都发生在 `ToolControlPlane::execute(AuthorizedToolCall)` 之前；raw args 只能随字段私有信封到 executor；outcome+audit 同事务。Batch 11–15 把 official RMCP、actor OAuth、Drive、durable approval 与可点击 Web presentation 接入；acting MCP 实得 pending→human grant→approval-linked decision/attempt→vendor success，以及 human deny→attempt/vendor 0。**仍不得宣称整项/G4 通过**：browser/file/shell、Approval 真实 PG 浏览器/critical realtime/thread 集成与第一次外部安全审计尚未闭合（R41/R74–R78）。
  ③ **v1 credential/SSO decrypt + v2 rotation —— 代码路径达成，Server KMS/HSM 尚未闭合**：v1 解密由**真跨语言互操作**证明（逐字节复制上游 `credentials.ts` 跑 node 产出 7 条信封 → Rust 解开明文逐字节相等 → 测试里内联的 fixture 再喂回上游 `decryptSecret`，双向闭合），每条正向断言配错密钥 / 翻密文位 / 翻 tag 位 / 翻 IV 位四种负向对照。W-7b 又把 `sso_providers` plaintext/v1 同事务迁成 AAD 绑定 v2 并回读校验；当前 key ring 仍由 `KEY_ENCRYPTION_KEY` 单版本提供，未冒充 §6.4 的 Server KMS/HSM。
  ④ **OIDC/SAML/session/role/group/revoke 全矩阵 —— 生产代码、本机矩阵与 Linux CI 历史证据已接通，整项仍因外审/Windows/KMS 未闭合**：W-7a 有唯一 safe dialer、环境三家 OIDC、跨副本 state/token/JWKS/group/keyed session；W-7b 已接动态 admin 注册/更新/删除、匿名 HMAC routing ticket、动态 OIDC 跨 replica、SAML SP metadata/Redirect+POST/ACS、根级 XMLDSig、SHA-2 allowlist、Destination/Audience/Recipient/request/time/replay、account-anchor cleanup 与 v2 SSO config。W-7c 首次真实 Ubuntu 24.04 x86_64 CI 连续暴露缺 `rg`、pipefail/SIGPIPE 与 deadline 测试服务端错误假设，逐项修复后自动 PR run `32762651186` 的 gates/supply-chain 全绿，workspace **1083/0/114**。R63 按用户额度指令重新关闭自动 Actions；这不撤销既有 Linux 证据，也不允许未经重新授权派发。**仍缺第一次独立 SAML/XSW 与整体安全外审、Windows 原生构建、Server KMS/HSM；需要 SP private key 的 signed AuthnRequest 也必须与 KMS 同批，不在当前输入面假装支持**，不得宣称 G2 全矩阵/整关闭合（R48–R50/R61/R63）。
  **W-1/W-2 已闭合**既有 schema/repository 地基；**W-3a/W-3b/W-4/W-6/W-7a/W-7b/W-7c**证据保持。Batch 11 后 G2 专项仍 **155/79/234**；Batch12–41新增面不改变该四元组。当前 tests **372/675/1047**、env **49/25/74**、events **26/56/82**、tables **56/0**、API **54/113/167**、components **1/21/22**、routes **8/24/32**、UI **86/66/152**、整个 parity **652/1026/1678**，fixtures **16/22/38**。R64–R68固定G3 data/realtime/migration；R69–R104固定Agent/G4与GUI批次、session/channel realtime/detail/shared thread/ChannelRow/Agent roster/create/routing/Composer/conversation/durable cancel/Memory Controls/Settings/Connected Accounts/Components Gallery及未完成边界。仍未闭合三家trace、Approval完整集成、run-wide budget、Desktop Local OAuth、MCP admin/private-egress、完整AG-UI事件、用户创建Agent lifecycle、完整channel/home Composer/AppSidebar、其余8 compiled renderer/runtime grant、GUI/golden/Tauri binary/window与经许可legacy drills。**其它未闭合项**：G4余项/G5/G7、G2外审/KMS/Windows、G6/G8（R52–R104）。
  **D-1 已正式裁决**：RUSTSEC-2023-0071 只做一条窄豁免；`tools/check-rustsec-waivers.sh` 锁死精确四节生产依赖链、openidconnect feature 零扩张和 RSA 私钥符号零命中。Batch 15 另有两条仅 `informational=unmaintained`、`patched=[]` 的编译期宏窄豁免（paste/proc-macro-error2），由 `tools/check-ui-dependencies.sh` 锁 proc-macro 消费边界与修复状态；它们不冒充漏洞修复或维护性消失。Batch 16 的 Tauri release graph 另有 runtime UNIC unmaintained×5，尚未 ignore；target-aware advisories 正确保持红。Cargo.lock-only 多报的 Linux GTK/proc-macro-error/glib 十条在六个发行 target 均不可达，由 `check-tauri-dependencies.sh` 负向锁定。见 R44/R78/R79。
  **D-5 已建立棘轮**：CI 钉 `cargo-vet 0.10.0` 并跑 `cargo vet --locked`；Google exact/delta import 锁定 15 个 fully audited。350 个是 R45 bootstrap exemptions；W-7 TLS 新增 20 个、W-7b SAML 新增 30 个、G3 WebSocket 新增 3 个、G4 Batch 11 RMCP/schema 新增 32 个精确 exemption，逐条带 `owner=security` 与 `not a full source audit`，合计 **435**，不冒充审计。Batch 15 GUI baseline 在当前 target-aware no-all-features 口径为 **181 unvetted**；Batch 16 Tauri all-features 为 macOS **270** / Windows **269**（净增 89/88）。没有批量补 exemption，故 cargo-vet 明确红；新增/升级未覆盖版本仍当场判红。见 R45/R48/R50/R67/R74/R78/R79。
  **D-3 已正式采用 `zeroize`**：`SecretBytes` 内层是 `Zeroizing<Vec<u8>>` 并标记 `ZeroizeOnDrop`，drop 清除当前 length+capacity；历史扩容 allocation 与调用方副本仍不冒充可擦除。见 R46。
  **D-2 已按真实契约消费者裁决**：Attempt/Capability/Catalog/Auth 四类型穿 application/infra 边界，收口到 contracts 且不 serde，`AuthContext` 不再用裸 `u64` 表示代际；CredentialGeneration 尚未穿 port。SecretId/ServiceId 在 W-7b/R59 后由 infra adapter 就地构造并只传给 domain `RecordBinding`，不是 application/transport contract，继续留在 vault domain；第一次穿 port 时必须同批上收。见 R47/R59。
- v3 §28.1 当前 **104** 条（R1–R104）。实施时除既有安全/G2裁决外，必须读R61–R104：Linux历史证据、0016–0022、Actions manual-only、native thread/live/history/memory/run runtime/importer、Rust Agent/G4、GUI地基/Tauri、27primitive/46图标、layout/AgentPresence/线稿、session sign-out、channel activity/detail/shared thread/ChannelRow、Agent profile/roster、channel create/routing/native first turn、Composer draft/queue、channel snapshot/realtime/idle send、durable跨副本cancel/production queue、actor memory write control/GUI、Settings偏好/stable owner/secondary shell、reviewed Connected Accounts，以及compiled manifest/PG治理/Quote+Cards Gallery slice；AppSidebar/完整Composer/其余renderer/runtime/golden/经许可legacy drills仍未完成。
- 上游对照固定在 `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`，不引用会漂移的 `main`（§1.2）。

## 2. 目标定义（为什么是这条线）

"全量 Rust"的定义固定为：GUI、业务、内置 Agent、策略、数据库访问、线程与记忆、实时事件、认证、凭据、审计、Supervisor 与全部高权限控制面由 Rust 实现（§0.1）。

允许的非 Rust 例外**只有**：Leptos/WASM 由系统 WebView 渲染；PostgreSQL / Chromium / Electron / OS keychain 作为外部引擎；用户自己的 HTML/CSS/JS 组件作为**不可信数据**在零权限沙箱里跑；用户接入的远程 AG-UI Agent 任意语言；第一方非 Rust 源码只剩**最小 Electron browser-engine shim**（只管 Chromium 生命周期、CDP、画面帧、封闭输入，无任何业务裁决权）。构建期工具（Tailwind CSS standalone CLI、trunk、wasm-bindgen CLI、wasm-opt）是钉 sha256 的二进制，不进发行物；**仓库零 `package.json` / Node**（§0.1，2026-08-22 裁决）。

理由：TypeScript 控制面、CopilotKit Intelligence 真源、跨用户 profile、MCP 过度实现、双数据库、多 driver 是上游的结构性风险（§27）；Rust 不做唯一控制面就不叫重写。

两个发行物共用同一 Rust core 与同一份 Leptos GUI：`openbot-server`（Axum，多用户）与 `openbot-desktop`（Tauri typed in-process；远程模式走同一 Axum API）（§0.2）。

## 3. 固定基线（改任一项 = 新建 delta audit，禁止静默升 lockfile）

| 项 | 钉死值 |
| --- | --- |
| Rust | `1.98.0`，edition 2024（2026-08-22 由 `1.94.1` 升级，delta audit 见 `docs/2026-08-22-Rust工具链1.94.1升1.98.0-delta审计.md`） |
| Tauri / Leptos | `2.11.5` / `0.8.19`（0.8.20 已存在，不升） |
| Leptos 生态 | `leptos_router` **`=0.8.13`**（0.8.14+ 要求 `leptos ^0.8.20`，升 Leptos 必须同 PR 升 router）；`leptos_meta 0.8.6`；`leptos_i18n 0.6.2` |
| GUI 构建工具 | Tailwind CSS standalone CLI `4.3.3`（sha256 表在设计系统文档 §12.1）；trunk `0.21.14`（`--offline`，缺工具即红不下载）；binaryen `version_132`；wasm-bindgen CLI = `Cargo.lock` 版本 |
| GUI 资产 | Inter Variable `4.1`（OFL-1.1）；Lucide `1.33.0`（ISC AND MIT，Feather 衍生子集），只随包 allowlist 里的图标 |
| RMCP | `3.1.4` |
| CEL | crate **`cel`** `0.14.3`（"cel-rust"是仓库名，不是 crate）；oracle = `cel-js@0.8.2` |
| OIDC / SAML | `openidconnect 4.0.1` / `samael 0.0.22` |
| HTTP TLS | `rustls 0.23.43` + `ring 0.17.14` + `webpki-roots 1.0.9`；ring 非纯 Rust，38 Perl/17 预生成对象与 20 条非审计 exemption 由 R48 guard 锁定 |
| WebSocket | Axum `0.8.9` `ws` + `tokio-tungstenite/tungstenite 0.29.0` + RFC6455-only `sha1 0.10.7`；3 条非审计 exemption、build.rs=0、unsafe token=4/0/5 由 R67 guard 锁定 |
| Intelligence bundle | AES-256-GCM `aes-gcm 0.10.3` + HKDF-SHA256 `hkdf 0.12.4` + Ed25519 `ed25519-dalek 2.2.0`；新 package=0，one-shot CLI/最终 runtime exclusion 由 R68 guard 锁定 |
| Browser kernel | Electron `43.3.0` / Chromium `150.0.7871.212` |
| 数据库 | PostgreSQL 17，**唯一**语义；Desktop 由 Rust 监管本机 sidecar；不需要 pgvector |
| 数据库驱动 | `tokio-postgres 0.7.18` + `deadpool-postgres 0.14.1` + `postgres-types 0.2`。**不用 `sqlx`** —— 它的 `query!` 宏让 `cargo build --locked` 的答案取决于跑在哪台机器上（构建期连库或 `.sqlx` 离线元数据二选一）。SQL 手写，由对真库的集成测试验证（G1 裁决 D3） |
| ID 类型 | §5.3 的十五个核心 ID 里，`ComputerGeneration` / `DocumentGeneration` 是 **`u64` newtype**，其余 13 个是 `String` newtype 且**不做 UUID 校验**（R23）。另有四个不 serde 的内部跨层 contract：Attempt/Capability/Catalog/Auth generation（R47） |

上游 oracle 运行时版本（copilotkit 1.68.3、ag-ui 0.0.57、better-auth 1.7.1、mcp sdk 1.30.0、playwright 1.62.1 …）以 §1.2 表为准，fixture 与 golden 只认这些版本。

## 4. 架构约束

- **10 crate workspace**（§5.1）：`contracts / domain / application / infra / agent / computer / server / ui / desktop / testkit`。建新 crate 只有四个理由：独立安全边界、独立发布单元、明显不同的 feature graph、可单独复用的纯协议；其余用 module。理由：细粒度 crate 放大编译、循环依赖与重构成本。
- **唯一业务入口** = `openbot-application::ApplicationService`（`execute` / `subscribe`）。Axum、Tauri、测试、迁移工具只做认证、framing、大小限制、错误映射，**不得**各自实现业务规则，不得接受自由 method string、renderer 自报角色或任意 SQL（§5.2）。
- **ID 默认 string newtype**，不限定 UUID；唯一例外是 `ComputerGeneration` / `DocumentGeneration`，二者是派生 `Ord` 的 `u64` newtype（R23，旧 generation 失效依赖数值序）。兼容端必须接受其余上游既有字符串。`AuthContext` 只能由 Rust 从 session / peer / DB ACL 构造，外部传来的同名字段都是不可信输入（§5.3）。
- **Agent reducer 必须 pure**：`reduce(state, event) -> (state, effects)`；DB、provider、MCP、browser、file、shell 都是 effect。每 thread 一个 foreground actor 串行处理；后台工作是独立 durable run（§7.2）。
- **工具只有一条执行管线**（§8.1）：validation → 权威 actor/target → effect 分类 → CEL + 内容策略 → 审批 → **事务写 decision + attempt** → 单次 capability → 执行 → outcome + commit_state。decision 写失败即不执行；执行了但 outcome 写不进去 → `ReconciliationRequired`，不自动重试。
- **parity 与新增必须分开标注**。示例：tool step cap = 8（parity）、`AGENT_STALL_TIMEOUT_MS`（parity）、`OPENBOT_RUN_DEADLINE_MS` 默认 30 min（**新增**）（§7.2）；MCP 四个上限只有 20,000 字符是 parity（§9.1）；memory 页是 31 route 之外的 +1（§3.1）。理由：把新增写成"当前行为"是 v2 审计里最重的一类错误（§28.1 R1）。
- **数据真源**：Rust/PostgreSQL 是 thread、message、run、memory、realtime cursor、run lock 的唯一真源；未设任何 `INTELLIGENCE_*` / `COPILOTKIT_*` 变量时产品必须完整运行；Intelligence 只用于一次性导出导入，不做双写、不留隐藏 fallback（§4.1）。
- **Schema 兼容期只允许 expand**：新表、nullable column、backfill、index、非破坏性 constraint；禁止 drop / rename / 类型收紧 / 主键改写；无 downgrade migration（§14.3）。审计表不做分区，hash chain 以追加 nullable 列落地（§8.6）。
- **环境变量三档** preserve / rename / remove 已在 §15.4 裁决；被 remove 的变量出现在生产配置里必须**启动报错**，禁止"读不到就当没设"。
- **错误语义固定**（§15.3 + R65）：未登录 401；角色不足 403；资源不可见统一 404（防枚举）；policy refusal 403 + stable code；stale generation / request-idempotency / lease 冲突 409；空 thread history 200 + 空列表。文案可本地化，code / status / audit 类型不变。

## 4a. GUI 视觉约束（真源 = 设计系统文档；2026-08-22 三条裁决：自有设计系统 / Tailwind v4 standalone 零 Node / 中英双语）

- **视觉不是 parity 对象**：旅程 / route / 组件行为对上游 parity，外观是本项目自有设计系统；视觉 oracle = 自家 golden 截图（设计系统文档 §10），不是上游截图。v3 G6 的 "web/desktop visual parity" 指同一 bundle 在两宿主一致，已改写为可判定定义（同 bundle 摘要 + 各平台各自 golden，不做跨引擎逐像素比对）。
- **7 条设计原则**（设计系统文档 §3）：chrome 恒中性，零彩色背景 / 边框，唯一实心按钮 = primary；语义色只落文字 / 图标 / 状态点；选中态 = 文字色 + 对勾；卡片无边框不上浮，阴影只有 popover / dialog 两级；图标一律 Lucide 矢量（品牌标唯一例外）；密度偏紧（正文 14/20）；动效只解释状态变化，`prefers-reduced-motion` 下全静止。
- **token 单一来源** `crates/openbot-ui/design/tokens.toml` → 生成 CSS 三块与 Rust 常量；组件只用 token utility，禁止字面颜色 / 任意值 / `dark:` 变体；改 token 必过 `token_contrast_wcag_aa`（文字对背景 ≥ 4.5:1，ring / chart ≥ 3:1）。
- **主题三态** `system`（默认，新增）/ `light` / `dark`：`<html class>` 由 Rust 在首帧改写（Axum 读 cookie、Tauri 读本地设置），`index.html` 零内联脚本。
- **i18n**：`leptos_i18n`，`en` 为源、`zh-CN` 首版；缺键在库里只是 warning，闸门是 `xtask i18n-check`（两份 locale 键集合逐字相等）；文案不进 domain / application，错误以 code 穿越边界后在 GUI 本地化；术语表 `locales/GLOSSARY.md`。
- **a11y** WCAG 2.2 AA 的机械子集（对比度单测 + CDP AX 树检查 + 键盘旅程 + reduced-motion 终态相等）；唯一豁免 = Desktop sandboxed component（v3 §3.3）。
- **上游的 6 个运行时 JS 库**（base-ui / motion / streamdown / prompt-area / boring-avatars / tw-animate-css）全部有替代方案（设计系统文档 §6.3），新增第 7 个即需修订该表。
- 反向 grep 闸门 `xtask design-lint`（禁 `dark:`、禁字面色、语义色不落背景 / 边框、阴影只两级、图标 allowlist 两向零漂移、生产无 `/_design` 画廊）、`css-check`（class 必须是源码字面量）、`bundle-budget`（wasm gz ≤ 3.5 MiB / css ≤ 96 KiB / 字体 ≤ 800 KiB）。
- Batch 15–35 已完成 Web 地基、Approval 可点击竖切、Server/Desktop 偏好、opt-in Tauri、
  27条primitive、46条Lucide mapping、4条layout业务、2条AgentPresence、2共享线稿、生产session
  sign-out/status、channel realtime/detail、独立ChannelRow、Agent roster/AgentCard、RecipientField、
  plain conversation、durable Stop与mount-local queue；UI ledger当前85/67。浏览器
  fixture 只作 AX/键盘/视觉/host framing 证据，不能替代 Batch 14 PostgreSQL approval 或 Batch 16
  PostgreSQL preference 生产证据；compile-only `/_design` 也不算生产 route/golden。外部 identity/tauri.conf/binary、其余 route/component/golden 与
  MPL/UNIC/Vet 仍红，完整 G6 仍未勾。

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

#36 redirect/DNS rebinding · #44 malformed AG-UI content 崩 UI · #53 credential rotate 孤儿 · #72 空 history 500 · #106 stale grant 复活 · Drive disconnect 未实现 · **`allowed_groups` 从 no-op 变为真控制**（`all` / 具名组 / 空列表三档，单用户模式语义见 §6.5；上游包声明的 channel 对所有人不可达，官方示例用的就是 `[all]`）· MCP 审计在 vendor 调用之后 · **channel 可见性不得 join `intelligence_channel_mappings`**（上游 `list` 的分页段只 join membership、hydration 段多 join 它，两处判据不一致；Intelligence 退役后这个 join 会把 §6.5 刚补上 membership 的包 channel 原样过滤回不可达，等于静默撤销 §6.5 的修复。Rust 版只查 materialized membership，且分页与 hydration 共用同一判据 —— §28.1 R22）。每条的 Rust 版确定语义以 §2.4 表为准，实现时不得"先照译再修"。

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

完整本机闸门的单一入口 = `cargo xtask ci`（fmt → clippy → `cargo test --locked` → safe-dialer guard → SAML/xmlsec guard → parity-check → recount，7 段）。驱动器**必须**建在 `target-xtask/`（`.cargo/config.toml` 的 alias 已配 `--target-dir`），与子构建的 `target/` 互不包含：否则第 3 步会去重链正在运行的驱动器自己，Windows 报 `os error 5` 恒红、Linux 恒绿（§28.1 R25）。摆放错了 `cmd_ci` 当场拒跑并打印两条路径，不会退化成"某台机器上能过"。**R63 当前操作覆盖：GitHub Actions manual-only 且未经用户重新授权不得派发；也不运行 `cargo xtask ci`，改为按变更面执行本机定向测试。**

GUI 另加（设计系统文档 §15）：`cargo xtask tools verify` · `cargo test -p openbot-ui` · `xtask i18n-check` · `xtask design-lint` · `xtask css-check` · `xtask bundle-budget` · `bash tools/check-ui-dependencies.sh` · `bash tools/check-tauri-dependencies.sh` · `bash tools/check-deny-release-targets.sh` · golden 截图（Web 110 张 / Desktop 每平台 54 张，差异像素 ≤ 0.1% 且无 8×8 全差异块；更新只能随 PR 附 diff 图人工批准）· CDP AX 树检查。Batch 16 Cargo Vet target-aware 为 macOS 270 / Windows 269 unvetted，未获明确授权不得批量 exemption；MPL/UNIC 红灯也不得写绿。

Go/No-Go 走 G0–G8（§24），**任何闸门失败只能修复后重跑，不能以"后续补齐"进入下一阶段**。DoD 十条见 §25——没有 parity ledger 100% 归类、跨 scope 泄漏 = 0、audit-before-action 违规 = 0，不得宣称"全量完成"。

## 11. 协作约定

- **中文**沟通、报告、commit 主题；标识符 / 路径 / 命令原样。commit 主题 `type(scope): 一句话 —— 根因或理由`。
- 分支 `docs|feat|fix/<YYYY-MM-DD>-<主题>`；交付 = push 分支 + 开 PR + 停在移交；合并用 **merge commit**（不 squash / rebase），保留原 commit 可追溯。push 前 `git remote -v` 确认目标是 `acosmi/OpenBot`。
- 实施型任务做到底；合法停止只有两种：用户叫停、撞到需用户裁决的真分歧（设计多选一 / 不可逆或对外动作 / 超出授权）。
- 子代理只写码、不碰 git；其结论主控亲自复核。
