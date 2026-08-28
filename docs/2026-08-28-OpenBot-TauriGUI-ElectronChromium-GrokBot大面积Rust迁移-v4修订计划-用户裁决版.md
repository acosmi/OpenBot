# OpenBot Tauri GUI + Electron Chromium + Grok Bot 大面积 Rust 迁移：v4 修订计划（用户裁决版）

> 日期：2026-08-28（America/Los_Angeles）
> 状态：**架构方向已由用户裁决；正式第一真源与机械台账尚待按本文件迁移**。
> 基线：OpenBot `56f35a563e0e3fca907cc6c2a12ee8299f1fc89d`；`grok-bot` tree `b68f24972427952c4934e4364736fec62661044f`。
> 本文件是本轮审计与用户裁决后的唯一有效 v4 修订计划；先前关于 standalone Chromium 和将 `grok-bot` 仅作 forensic reference 的临时提案已清理，不再作为实施依据。
> 范围说明：按用户决定，本技术计划不把权利状态或独立权利审查列为阻断项；全文只处理产品、架构、实现、测试与发行工程。

---

## 0. 用户已经裁决的五项

### D1：主 GUI 保持 Tauri + Leptos/WASM

- Desktop 第一方 GUI 继续由 `openbot-desktop` 的 Tauri host 承载；
- GUI 产品代码继续是 Rust/Leptos 编译出的 WASM；
- Server Web 与 Desktop 继续共享同一 GUI bundle；
- 现有 window authority、typed in-process transport、有界队列、multi-window ACL、主题/i18n/a11y 与设计系统均保留。

Tauri 底层必然使用系统 WebView，但该 WebView **只加载第一方本地 GUI bundle**。用户所说“不要用 WebView”在本方案中的准确含义是：

> 不可信网页、Browser Computer 和用户 authored HTML/CSS/JS component，均不得进入 Tauri/system WebView。

### D2：Browser Computer 与 Desktop sandboxed component 固定使用 Electron 内置 Chromium

- Electron/Chromium 是这两个不可信执行面的唯一生产 engine；
- 不采用 standalone Chromium + Rust direct-CDP；
- 不采用 Tauri/Wry/system WebView 承载不可信内容；
- 不维护 Playwright、Electron、standalone Chromium 三套生产 driver；
- Browser Computer 与 sandboxed component 共用同一 Electron engine **进程类、协议与 conformance suite**，但必须使用不同进程实例、profile/partition、egress、resource budget 与 generation。

### D3：`grok-bot` 是 v4 主要架构、执行与交互参考

`grok-bot` 不再被限制为只读反例。它在以下领域具有主要参考优先级：

- Electron/host/coordinator/process ownership；
- Agent runner、tool、subagent、background work 与 scheduler；
- local computer 与 isolated box 的双执行域；
- permissions、approval、generation/epoch、重连、恢复与退役状态机；
- Browser/Computer、窗口/tab、VNC/Screen 交互模型；
- conversation、roster、agent-info、automations、settings、plugins、permissions、computer、terminal 等产品旅程；
- runtime composition、artifact verification 与 evidence-driven reconstruction 方法。

### D4：允许大面积 TypeScript → Rust 语义翻译

目标不是逐字符改语法，而是大面积迁移成熟职责：

- 纯函数、validator、normalizer、state machine、projection 可近机械翻译；
- authority、持久化、网络、文件、进程与框架相关部分按其不变量重写；
- Electron API 不可替代的最小边缘保留极薄 JS shim；
- partial recovery、空壳、generated code 和明确缺陷不得用行数冒充成熟实现。

### D5：“全量 Rust”的新精确定义

必须由 Rust 实现：

- GUI 产品逻辑与全部 Leptos components；
- domain/application、Agent/runner、tool、policy、approval、audit、vault；
- thread/message/run/memory/realtime 与 PostgreSQL access；
- Desktop authority、window/session ACL、coordinator、supervisor、update verification；
- Browser/Computer scope、engine lifecycle、ScreenHub、HumanLease；
- file/shell/box、平台 sandbox helper、egress、process tree；
- provider、MCP、OAuth、attachments、automations、sharing、notifications 与所有高权限控制面。

允许的第一方非 Rust 生产源码只剩 Electron API 的最小 shim。Electron 本身包含 Chromium 与 Node runtime，这是选用 Electron 的技术事实；“全量 Rust”不再解释为字面上的零 JavaScript runtime，而是：

> **零 JavaScript 业务控制面，零 JavaScript authority，零 Node Agent/host/coordinator/local-exec；只有无法绕开 Electron API 的机械 shim。**

---

## 1. v4 真源优先级

### 1.1 五层权威

1. **用户裁决**：本文件 §0 及后续明确裁决；
2. **v4 规范性真源**：待由本文件生成的正式后端真源与 GUI 真源；
3. **Grok Bot 主要架构/执行/交互参考**：固定 `grok-bot` tree；
4. **OpenBot parity 参考**：固定 `CopilotKit/openbot@891df72f…` 的 API、schema、既有旅程与迁移兼容；
5. **实际实现与机械证据**：Rust code、ledger、fixture、test、signed artifact。

### 1.2 冲突裁决

| 冲突领域 | 优先规则 |
| --- | --- |
| Desktop GUI 宿主 | Tauri + Leptos/WASM 固定，不因 Grok 的 Electron GUI 改写 |
| Browser/Component engine | Electron/Chromium 固定，不使用 Tauri WebView 或 standalone Chromium |
| process/coordinator/runner/execution | 优先吸收 Grok 的职责分离与状态机，翻译成 Rust |
| API/schema/legacy migration | OpenBot parity 保留，变化必须标 additive/replace/supersede |
| authority/security | 以 v4 Rust 不变量为准；参考源里的放宽实现不得覆盖 durable policy/capability/audit |
| GUI interaction/journey | Grok 为主要交互参考，OpenBot 既有可观察旅程继续兼容 |
| 视觉/token/品牌/i18n/a11y | 自有 GUI 设计系统为规范，Grok 布局与交互需映射到自有 token |
| 数据所有权 | PostgreSQL 与既有 event/outbox 继续唯一真源，不引入 Grok 的第二套 transcript/SQLite truth |

### 1.3 大面积迁移不等于整目录可信

迁移分类单位固定为“职责/函数/状态机”，不是目录或 LOC。审计规模如下：

| 家族 | 文件数 | 说明 |
| --- | ---: | --- |
| `source/electron-main` | 185 | Electron API、Desktop UX、coordinator、account、box 等混合 |
| `source/electron-preload` | 16 | 主 GUI bridge 为主，迁移后大多由 Tauri typed command取代 |
| `source/node-agent-coordinator` | 24 | lifecycle、gateway、renderer relay、supervisor |
| `source/host` | 471 | extensions、runner、box、session、Agent 与产品能力 |
| `source/shared` | 165 | contract、不变量、Node transport 混合 |
| `source/packages` | 852 | Agent/exec/proto/utility；含大量 generated/partial |
| `source/local-exec-daemon` + `box-exec-daemon` | 6 | 执行器与协议入口 |
| `frontend/src/recovered/features` | 251 | 20 个 GUI feature family |

其中 generated proto/redacted tree 约 263,713 行；`packages/agent/state.ts` 明示 partial recovery，若干 subagent state/queue 文件只是数行空壳。因此 v4 不以“翻译了多少行”计进度，而以 production capability 与证据闭环计数。

---

## 2. 最终产品拓扑

```text
┌─────────────────────────────────────────────────────────────┐
│ Tauri Desktop Host（Rust）                                  │
│  └─ Leptos/WASM 主 GUI：只加载第一方本地 bundle             │
│       ├─ typed in-process command / structured event        │
│       └─ ScreenHub canvas / one-shot viewer ticket          │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│ Rust ApplicationService / DesktopCoordinator                │
│  ├─ Auth / session / window authority / preferences         │
│  ├─ Thread / run / transcript / memory / realtime           │
│  ├─ Agent host / extensions / runner / tools / MCP          │
│  ├─ Policy → approval → decision/attempt → capability       │
│  ├─ ComputerManager / ScreenHub / HumanLease                │
│  └─ FileBroker / SandboxExec / BoxSupervisor                │
└──────────────────────────┬──────────────────────────────────┘
                           │ authenticated bounded UDS/Named Pipe
                           ▼
┌─────────────────────────────────────────────────────────────┐
│ purpose-built Electron engine shim（极薄 JS，无业务）       │
│  ├─ role=BrowserComputer                                   │
│  │    └─ scope-bound Electron/Chromium process + profile    │
│  └─ role=SandboxedComponent                                │
│       └─ ephemeral Electron/Chromium process + zero egress  │
└──────────────────────────┬──────────────────────────────────┘
                           │ CDP/frame/input/crash only
                           ▼
                 Rust ScreenIngress / audit outcome
```

Server 继续复用同一 Rust core 与 Leptos GUI；Server 的 Browser Computer 使用相同 Electron engine package 和协议，置于 runsc container。Tauri 只存在于 Desktop GUI，不进入 Server。

### 2.1 三条绝对边界

1. Tauri GUI 永远不加载 remote page 或用户脚本；
2. Electron engine 永远不接收 actor、role、policy、DB query、provider/MCP/Vault secret 或任意 shell；
3. GUI 永远不直接持有 Electron engine token；所有 frame/input/control 经 Rust ComputerManager 与 ScreenHub。

---

## 3. Electron/Chromium engine 的精确职责

### 3.1 唯一 engine package，两种 role

同一份 Electron 发行物、同一 shim 与同一协议，按 Rust authority 铸造的启动 role 工作：

```rust
enum EngineRole {
    BrowserComputer(ComputerSecurityScope),
    SandboxedComponent(ComponentRenderScope),
}
```

role 不从 renderer、argv 自由字符串或页面 URL 得出。Rust supervisor 通过继承的私有 pipe/handle 完成 boot handshake；engine 未通过 binary digest、release epoch、protocol version 与 one-time boot capability 校验时，不创建任何 renderer。

### 3.2 Browser Computer role

- 每个 `ComputerSecurityScope` 独立 Electron process instance；
- scope-bound persistent profile，tab 可共享同一 principal 登录态；
- profile/workspace 不与其他 user/Bot/thread 或 component 共享；
- Rust 只发送 closed `BrowserOperation`；
- shim 只把 closed operation 映射到固定 `webContents.debugger` CDP method；
- screencast、frame ACK、input、dialog/download/file chooser/popup 处置走同一 adapter；
- URL/redirect/subresource/WS/DNS/egress 仍由 Rust policy 与 gateway控制；
- crash/restart 必须推进 `ComputerGeneration` 并使旧 ref/ticket/lease/capability 全失效。

### 3.3 Sandboxed component role

- 与 Browser Computer 使用不同 Electron process instance；
- 临时、非持久 partition/profile；
- 无 Browser profile、workspace、clipboard、download、popup、permission、MCP、data function；
- 无 preload、无 Node、无 host callback；作者脚本只看到 DOM 与 `window.__args`；
- deny-all egress，HTML/CSS/JS/args 只由 Rust 一次性 render session 注入；
- frame 经 ScreenHub 回 Tauri GUI，输入是 closed pointer/wheel/key/text；
- CPU、RSS、wall-clock、frame、DOM 与 console/error 都有硬预算；
- 结束、超限、crash 或 Tauri viewer离开后清除完整进程树与临时目录。

### 3.4 Electron 安全配置

engine shim 必须在 `ready` 前调用全局 sandbox；禁止参考源中的 `--no-sandbox`。所有不可信 renderer 固定：

```text
sandbox = true
contextIsolation = true
nodeIntegration = false
webSecurity = true
webviewTag = false
preload = none
devTools = false (production)
```

另外：

- 不使用 `<webview>`；
- permission request/check 默认拒绝；
- navigation、popup、download、dialog、file chooser 都有明确 handler；
- 不开放 remote debugging TCP port；
- 正式帧源固定为 `webContents.debugger` 的 `Page.startScreencast`；Electron offscreen `paint` 只作诊断/可行性 fixture，不形成第二生产帧路径；
- Electron fuse、ASAR integrity、`ELECTRON_RUN_AS_NODE`、Node CLI inspect 均由发行 gate 锁定；
- Browser/Component 正向测试必须证明 renderer 进程实际 sandboxed，而不是只检查配置文本。

### 3.5 最小 JS shim 预算

建议只保留 purpose-built engine 目录：

```text
engine-shim/package.json
engine-shim/package-lock.json
engine-shim/src/main.mjs
engine-shim/generated/engine-protocol.mjs
engine-shim/build/fuses.mjs
```

`package.json`/lock 只钉 Electron 与打包闭包，不恢复通用 Node 应用栈；生产没有 React、Vite、Node coordinator、MCP、Agent 或 local-exec npm dependency。

shim 允许：

- Electron `app` 与 engine process lifecycle；
- `BrowserWindow` / `session` / `webContents` / `webContents.debugger`；
- fixed protocol handler、permission/navigation/crash hooks；
- binary frame decode/forward；
- 只启动 Rust 明确指定、签名与摘要匹配的 child/helper。

shim 禁止：

- DB、settings、account、secret、OAuth、provider、MCP、policy、approval、audit；
- 自由 method dispatcher、自由 CDP、自由文件路径、自由 URL passthrough、自由 spawn/env；
- `eval`、`executeJavaScript`、`sendSync`、第三方 npm runtime dependency；
- renderer 自报 role/scope/generation；
- Node coordinator/host/agent/local-exec/box daemon。

`cargo xtask electron-shim-check` 锁定：文件 allowlist、非空 LOC 预算、Electron API allowlist、forbidden import/string、生成协议 hash 与产物闭包。

### 3.6 签名、启动与更新

- Tauri Rust 在启动 engine 前校验 Electron Framework/helpers、shim/ASAR、protocol hash、签名 identity 与 release epoch；
- macOS 按内层 Electron Framework/helper → engine sidecar → Rust/helper → 外层 Tauri app 的顺序签名并最终 notarize；
- Windows engine exe/dll、Rust sidecars 与 installer 全部 Authenticode；Linux 包含签名 manifest 与启动时 digest校验；
- Electron engine 禁止 `autoUpdater` 和自更新；整包只由 Rust/Tauri updater 更新；
- Tauri、Rust、Electron/Chromium、shim、PostgreSQL 与 sandbox helper 必须属于同一 release epoch；
- PostgreSQL major 与 Electron major 不进入同一 release；
- 任一 digest/signing/protocol/epoch attestation 不一致时拒绝启动 engine，不回落 Tauri WebView、standalone Chromium 或裸执行。

---

## 4. `grok-bot` → Rust workspace 大面积迁移地图

### 4.1 分类代码

| 代码 | 含义 | 完成条件 |
| --- | --- | --- |
| `T` Translate | 成熟纯逻辑/状态机近机械翻译 | differential fixture + Rust unit/property test + production call point |
| `A` Adapt | 吸收职责与不变量，按现有 Rust authority/PG/Tauri 重写 | 目标架构 integration + fault/security test |
| `S` Shim | 仅 Electron API 无法替代的机械边缘 | shim allowlist/LOC/generation/security gate |
| `C` Capability | 新产品能力进入 v4 scope 候选 | 用户旅程、数据/API/事件/UI/测试全量台账 |
| `R` Reject implementation | 产品能力可保留，但明确拒绝参考实现方式 | 负向 guard，禁止进入 build/runtime |
| `P` Partial/placeholder | 恢复不完整，不能冒充成熟源码 | 补行为 oracle或重新设计后才能进入 T/A |

### 4.2 十 crate 总映射

| Grok Bot 家族 | Rust 落点 | 默认分类 |
| --- | --- | --- |
| `shared/**` 与实际消费的 wire DTO | `openbot-contracts::{desktop,host,execution,agent,automation,media,permissions}` | T/A |
| extension DAG、scheduler、epoch/retry/approval 不变量 | `openbot-domain::{runtime,agent,automation,sharing,transcript,tool}` | T |
| host use case、groups/workflows/automation/component orchestration | `openbot-application::{agents,runs,automations,groups,plugins,computer,attachments,search}` | A |
| storage/auth/provider/OAuth/artifact/search/update metadata | `openbot-infra::{repo,provider,oauth,artifact,search,notification,update}` | A |
| `packages/agent*`、runner、prompt、summarization、MCP/hooks | `openbot-agent::{host,runner,actions,context,prompt,tools,subagents,automation,mcp}` | T/A/P |
| shell/local-exec/box/browser/computer | `openbot-computer::{realm,supervisor,protocol,browser,screen,file,shell,box,process_identity}` | T/A |
| HTTP/SSE/WS 与多用户 transport | `openbot-server`，仍只调用 `ApplicationService` | A |
| frontend product features | `openbot-ui` Leptos/WASM modules | T/A/C |
| Desktop UX、Tauri lifecycle、Electron engine supervision | `openbot-desktop::{lifecycle,window,menu,deep_link,notification,update,engine}` | T/A/S |
| differential oracle、fault、runtime/build closure | `openbot-testkit` 与 `cargo xtask` | T/A |

现有十 crate 不因大面积迁移自动扩张。需要独立进程时，优先在既有 crate 增加 bin target；只有符合“独立安全边界/发布单元/feature graph/纯协议复用”才新增 crate。

### 4.3 `electron-main/**`

#### 翻译为 Rust/Tauri Desktop host

以下目录的产品行为、state machine 与 policy 大面积翻译；Electron API 调用改成 Tauri Rust API或 Rust application port：

- `window-state-*`、`window-chrome.ts`、`window-shortcuts.ts`、`host-window-chords.ts`；
- `application-menu.ts`、`deep-link/**`、`downloads/**`、`notifications/**`；
- `startup/**`、`update/**`、`prefs/**`、`feedback/**`；
- `account/**`、`auth/**`、`secrets/**`、`mcp/**`、`models/**`；
- `coordinator/**` 的 ownership/restart/resync/generation；
- `local-exec/**` 与 `box/**` 的 supervisor/status/recovery；
- `process-metrics/**`、`telemetry/**` 中能映射到 Rust tracing/metrics 的部分。

主窗口、菜单、通知、deep-link、updater 的**用户旅程**参考 Grok；最终执行归 Tauri/Rust。Electron reference main 不再作为 Desktop 主入口。

#### 只留在 Electron engine shim

- engine worker 的 `app` lifecycle；
- per-scope/per-render `BrowserWindow`；
- `session`、`webContents`、debugger、frame/input/crash；
- permission/navigation/popup/download/dialog/file chooser handler；
- authenticated Rust pipe 的 hello/ready/cancel/shutdown。

### 4.4 `electron-preload/**`

参考源 preload 暴露了大量 Desktop 业务对象。v4 的处理是：

- 主 GUI bridge 的产品能力全部转为 Tauri typed command/event；
- contract、payload、订阅与取消语义转到 `openbot-contracts` / `openbot-desktop`；
- Browser Computer 与 component renderer **没有 preload**；
- 因此参考源 preload 没有生产 JS 直译物，只有其 API inventory 作为 Rust/Tauri coverage oracle。

### 4.5 `node-agent-coordinator/**`

直接翻译或适配：

- carrier/bootstrap、hello/ready/shutdown；
- request id、duplicate、wrong-direction、cancel；
- renderer relay、gateway reconnect、heartbeat、backoff；
- host/local-exec supervisor、PID/start-time/generation identity；
- old attempt retirement、resync 与 account transition cleanup。

目标是 Rust `DesktopCoordinator` / `AgentHostSupervisor` / `ComputerSupervisor`。不保留 Node coordinator binary，不保留自由 `method:string + args:unknown`；frame 改为 closed enum、大小上限与严格方向。

### 4.6 `host/extensions/**`

35 个 extension family 全部进入 v4 census：

- 核心首批：auth、settings、inference、turn-execution、transcript、session、MCP、local-tool-permission、local-exec、attachments、secrets、memory、action-audit；
- 第二批产品能力：automations、cross-user-sharing、cloud-agents、auto-review、managed-setup、content-search、teach-recording；
- 运行运维：box-lifecycle、forever-box、box-store-sync、state-backstop、host-upgrade、notifications、notify-bus、telemetry、trays、wallpaper、webauthn-proxy；
- experiments 翻译成 typed config/admin policy，不搬 Statsig authority；
- source-map/codebase-telemetry 只保留本地诊断或明确 opt-in 行为。

extension DAG、循环检测、失败回滚、逆序 teardown 可直接翻译；每个 extension 的 IO 通过现有 application port/infra adapter，不建立 service locator 或 global mutable context。

### 4.7 `host/runner/**` 与 `packages/agent*`

可大面积迁移：

- runner state、turn shape、stream attempt、retry/stall/checkpoint；
- tool call identity、context processing、prompt assembly、redaction；
- local/box/browser/file/shell effect identity；
- subagent/background work、auto-review advisory、usage/budget；
- summarization、large-output spill、hook、MCP meta tools；
- conversation outline、reaction、state、send-message protocol。

拆层规则：

- reducer/state/validator/retry → domain；
- loop/tool/context/prompt/subagent → agent；
- IO/provider/storage → application ports + infra；
- browser/file/shell → computer；
- 每个 acting call 重新进入 OpenBot durable execution pipeline。

标注 partial/placeholder 的文件先归 `P`，不因路径存在就算迁移项完成。

### 4.8 local-exec / box / shell

直接翻译：

- process identity、generation token、adopt/replace/quarantine；
- cancel/control protocol、deadline、output/file limits；
- `always/ask/never` 的 UI状态、epoch、stale/abandoned/refusal 语义；
- local 与 box 显式区分；
- owner label、content-addressed artifact、readiness 与 recovery。

按 Rust authority 重写：

- permission → policy/decision/attempt/capability；
- path → root handle/no-follow/TOCTOU-safe；
- shell → OS-enforced helper/process tree；
- box → runsc/fixed digest/resource/egress；
- credential → Rust Vault/proxy，禁止整目录透传；
- local/box 不可用时不互相隐式 fallback。

### 4.9 protocol/shared/generated

- `shared/**` 按纯 contract / domain invariant / Node IO 拆分；
- generated proto 不逐行手译，只为实际消费 wire 生成 canonical `prost`/Rust DTO；
- 保留 version、hello/ready/shutdown、request id、cancel、epoch、sequence；
- free method/family 改为 closed enum；
- unknown、duplicate、wrong direction、stale generation、oversize/depth 全 fail-closed；
- 每个协议有独立 fuzz corpus 和 golden trace。

### 4.10 frontend → Leptos/WASM

每个 feature 固定转换规则：

- `model/controller/store/provider/projection` → Rust/WASM state；
- TSX → Leptos view；
- CSS → 自有 token/design-system class；
- Electron desktop bridge → Tauri typed command；
- remote/host state → `ApplicationService`；
- recovered selector/string/interaction 必须绑定 production call point 或 artifact fixture。

映射范围：

| Grok frontend family | OpenBot UI 落点 |
| --- | --- |
| `conversation/**` | channels/threads/composer/cards/replies/search/outline/viewers |
| `agent-info/**`、`roster/**`、`org-chart/**` | agents/groups/network |
| `automations/**` | automations/routines/history/editor |
| `computer/**`、`terminal/**` | computer/ScreenHub/terminal；VNC webview淘汰 |
| `plugins/**`、`settings/**`、`permissions/**` | plugins/settings/approvals |
| `access/**`、`account/**`、`onboarding/**` | auth/access/onboarding |
| `root-resilience/**`、`error-boundary/**` | recovery/error shell |
| `window-chrome/**` | Tauri window shell |
| about/feedback/deep-links/update/hidden-chats | 对应 Desktop/UI feature |

PDF、spreadsheet、rich editor、voice、Mermaid、KaTeX 等涉及新运行依赖的能力进入独立技术 delta；不顺手把原 JS package 复制进发行物。

### 4.11 tests/build

- 每个 `T` 函数先冻结 TS input/output/exception fixture，再做 Rust differential/property test；
- 每个 `A` capability 必须有真实 PG/network/process/renderer integration；
- runtime composition、renderer closure、publication verification 判据转进 Rust `xtask`；
- bootstrap extraction、opaque renderer overlay、Node tree-sitter patch、artifact fallback 不进入目标构建；
- Electron runtime、shim、WASM、Rust sidecars 由 xtask 按固定 manifest 组装、签名、生成 SBOM 与 release epoch；
- ASAR 只作容器格式，不作安全边界。

---

## 5. 现有 OpenBot 实现如何处理

### 5.1 保留

- 十 crate workspace；
- `ApplicationService`、Axum 与 Tauri typed in-process；
- `openbot-desktop` broker/window/event/session/transport；
- Tauri 2.11.5 host、主 GUI bundle 与当前视觉系统；
- Rust Agent/provider/MCP/OAuth/Drive/approval、PG thread/run/memory/realtime；
- sandboxed component governance、Web iframe runner；
- `ControlService`、closed BrowserInput、HumanLease；
- 当前 693 条历史 done evidence。

### 5.2 保留行为、替换或扩展 adapter

- `tauri_host.rs` 继续服务第一方 GUI，不承担不可信 component；
- Desktop component 当前 custom-scheme RefusedCard 分支改为申请 Electron render session + ScreenHub canvas；
- BrowserOperation 的目标 adapter 固定为 Electron `webContents.debugger` shim；
- Tauri ScreenSession 仍向 Rust申请 ticket，Rust再连接 Electron frame ingress；
- 现有 Web sandbox MessageChannel 只保留 Server Web；Desktop 不复用该 iframe路径；
- Grok 的 main/coordinator UX 进入 Rust/Tauri，不把完整 Electron desktop host带回来。

### 5.3 仍需先修的协议缺口

1. `HumanLeaseEpoch::next` 的 `saturating_add` 到 `u64::MAX` 不会再失效旧 ticket；改成 checked/poisoned 状态；
2. `BrowserOperation::Key` 与 `Scroll` payload 可直接构造空 key/NaN；改为私有 validated type；
3. `ComputerGeneration` 需要 durable supervisor authority；
4. Electron protocol 需 closed hello/ready/capabilities/cancel/shutdown 与 malicious frame decoder；
5. Web CSP/iframe 不冒充完整零网络/抗DoS，相关 todo 继续保持。

### 5.4 明确拒绝迁入的实现

- `--no-sandbox`、`sandbox:false`、`webviewTag:true`；
- `<webview>` / VNC 作为主 Screen UI；
- free `browser_cdp` + denylist；
- Node host/coordinator/Agent/local-exec/box daemon；
- production mock permission、缺 sandbox 时 `insecure_none`；
- mutable `latest` image、固定 bearer token、消费者认证目录挂载；
- renderer URL 自举 trusted origin、隐式 clipboard；
- SQLite/JSON/blob mirror 成为第二业务真源；
- opaque shipped renderer、artifact fallback、ad-hoc production signing；
- partial/empty module 被标成已迁移。

---

## 6. 已完成 693 项如何修订

### 6.1 总原则：历史 done 不抹除，v4 有效性另记

`693 done` 是 V3-B51 在当时源码、契约与证据下已经完成的历史事实。引入 Grok 架构和 Electron engine 后，不能把 693 项全部改回 todo，也不能假设它们自动适用于新实现。

每个既有 done 增加一层 v4 disposition：

| disposition | 含义 | 历史 status | v4 合并要求 |
| --- | --- | --- | --- |
| `carry` | 契约、实现与证据都不受影响 | 保持 done | 不要求额外复验 |
| `carry_extend` | 旧能力完整保留，只在其上新增 Grok 能力 | 保持 done | 新能力另建 delta T-ID |
| `internal_refactor` | 外部契约不变，内部按 Grok 架构重组 | 保持历史 done | 原 T-ID 必须在新实现上重跑，并追加 evidence history |
| `adapter_rebind` | domain/application不变，transport/engine adapter替换 | 保持历史 done | adapter conformance 与原业务测试同时通过 |
| `split_applicability` | 一条旧证据混合了 Web/Desktop 等多个宿主 | 保持历史 done | 拆出各宿主新条目；旧证据只在已证明宿主有效 |
| `remediate` | 已发现具体缺陷，旧证据不足以满足v4不变量 | 历史 done保留 | v4 effective状态为blocked，修复并重跑后恢复 |
| `superseded` | 产品契约被明确替换 | 历史 done保留 | 填 `superseded_by`，新契约另建阻断条目 |

当前没有理由批量使用 `superseded`，也不允许用一次架构修订把既有业务成果全部清零。

### 6.2 九份台账的初步影响

| 台账 | V3 done | v4 处理 |
| --- | ---: | --- |
| API | 66 | API/auth/业务语义整体 `carry`；Electron engine只新增内部adapter/API，不重开既有Server/Tauri业务面 |
| Browser operations | 7 | 5条Control + closed union基本 `carry`；epoch fencing 先 `remediate`；真实Electron执行仍是原39项todo |
| Components | 13 | compiled、governance、no-data-function、draft/revision `carry`；Web iframe与Desktop renderer证据 `split_applicability` |
| Environment | 49 | provider/auth/server配置 `carry`；computer/supervisor URL/token等内部engine配置做 `adapter_rebind`审计 |
| Events | 33 | 现有audit/pipeline/component/MCP事件 `carry`；Grok scheduler/subagent/automation/engine事件 additive |
| Routes | 8 | 全部 `carry_extend`；Grok journey新增routes，不覆盖既有路径 |
| Tables | 58 | 全部历史table/migration `carry`；computer generation与Grok新能力用expand migration新增，不改旧条目 |
| Tests | 372 | 作为回归oracle保留；目标模块重构时自动转 `internal_refactor` 并在新实现上重跑 |
| UI | 87 | primitives/icons/layout/现有journey `carry_extend`；作为Grok UI迁移的基础，不重写为React |

精确的 `carry N / internal_refactor N / ...` 必须由 changed-symbol → T-ID 反向索引机械计算，本文件不估算 N。

### 6.3 G1 已完成部分：不动核心，只让新架构适配它

完全保留：

- 十 crate workspace与依赖方向；
- `ApplicationService` 唯一业务入口；
- Axum/Tauri 同一 Rust业务语义；
- PostgreSQL pool/schema/migration/read checksum；
- tracing/redaction/Prometheus；
- Tauri + Leptos/WASM 主 GUI。

Grok extension graph、coordinator、runner 与 Electron engine必须接到这些现有接口后面，不能反向要求G1换成Node风格gateway或第二数据库。

代码修改方式：

- 保留 `openbot-application::{service,app,ports}` public facade；
- 保留 `openbot-desktop::{transport,broker,session,window}`；
- 新增 engine/runner adapter，不改已有 use case的AuthContext与AppReply语义；
- 若内部文件移动，原 transport parity、PG与observability测试必须在新symbol上重跑。

另需消除现有文案矛盾：“G1 read checksum已闭合”与“read checksum/Drizzle深校验未闭合”必须拆成两个稳定 criterion；不能靠文字同时成立。

### 6.4 G2 已完成部分：作为 Grok 权限/账户能力的唯一底座

保留现有 Rust：

- identity/session/OIDC/SAML；
- Vault envelope/rotation；
- CEL与deny-first；
- durable decision/attempt/capability/outcome；
- hash-chain audit与tool approval。

Grok 的 `always/ask/never`、local permission、secret request、auto-review 与account UX按 `carry_extend` 处理：迁移其状态机和界面，但最终执行仍进入现有 Rust pipeline。

代码修改方式：

- 新增 `ExecutionRealm`、permission preference、direction epoch 等policy context；
- 复用现有 `ToolControlPlane`/approval/vault port；
- 不替换或绕过已经完成的G2模块；
- 原有外审/KMS/Windows等todo保持todo，不被Grok迁移伪装成完成。

### 6.5 G3 已完成部分：数据模型保持，Agent调度在上层重构

58个table、native 0016–0023、thread/message/run/run_event/lease/outbox/memory、channel/history/SSE/WS/queue/cancel均保留。

Grok scheduler、background、subagent、checkpoint、recovery翻译为 `internal_refactor` 或 `additive`：

- durable run/lease/event仍是权威；
- 新 scheduler 只能消费/更新既有port；
- 不引入SQLite、JSON transcript、blob mirror或内存queue作为第二真源；
- automations、background task、agent-to-agent消息需要表时，用0024+ expand migration和新T-ID；
- 任何修改 `run_runtime`/queue/cancel/realtime的批次，必须重跑其关联旧T-ID，但不改写历史status。

### 6.6 G4 已完成部分：稳定 facade 保留，内部大面积 Rust 化

已完成的三provider、retry、AG-UI decoder、RMCP/SafeDialer、catalog/OAuth/Drive、remember、durable approval、component tool/HITL继续有效。

建议在现有 facade 后新增：

```text
openbot-agent/src/host/{extension_graph,registry,composition}.rs
openbot-agent/src/runner/{scheduler,stream_attempt,turn,checkpoint,background}.rs
openbot-agent/src/{context,prompt,subagents,automations}.rs
openbot-agent/src/tools/**
```

处理规则：

- `BuiltInAgentRuntime`、`AgentToolGateway`、`ProviderRouter`、application ports保持兼容；
- 成熟 Grok runner pure logic 可 `T`；IO/authority走 `A`；
- provider/MCP/Drive现有 integration tests成为强制回归；
- subagent、automations、group/agent-to-agent、auto-review、richer context等全部 additive；
- prompt变化必须有`prompt_version`与golden，不静默覆盖旧行为。

### 6.7 G5/Batch51 已完成部分：六项保留，一项修复

保留：

- `ControlService::state/release/request_help/request_secret/take`；
- BrowserInput不增加独立IME composition/drag变体；
- actor/auth/computer/tab/generation/epoch绑定设计；
- secret独立zeroizing command。

需要 `remediate`：

- `T-BROP-0046` 的 `HumanLeaseEpoch::next` 当前使用 `saturating_add`。到`u64::MAX`后epoch不再推进，旧ticket可能继续匹配；测试只证明不回绕，未证明fail-closed。
- 修改为checked increment；溢出进入poisoned/restart-required状态，清lease并拒绝input/acting，直到Rust authority推进ComputerGeneration。

需要在Electron接线时新增而非重开：

- protocol/framing/CDP mapping；
- engine reject、ScreenHub、viewer ticket、frame/input broker；
- real process restart对旧ticket/capability/ref的失效证据。

### 6.8 G6/UI 已完成部分：作为 Grok GUI 迁移的组件库

全部保留：

- 27个primitives、icon mapping、token/i18n/design lint；
- Avatar、AgentPresence、layout、Sidebar、SettingsShell；
- 已完成routes与memory/accounts/gallery页面；
- compiled gallery renderer、Activity data、Approval/Choice；
- component/sandbox PG governance与Web runner已证明部分。

Grok React/TSX迁移不替换这些成果，而是：

- model/controller/store/projection翻成Rust；
- view重写为Leptos并复用现有primitives；
- CSS映射自有token；
- 新journey扩展Sidebar/Composer/Conversation/Computer/Automations；
- visual变化走新的golden，不能因布局更新取消a11y/i18n约束。

### 6.9 Sandboxed component：后端和Web保留，Desktop adapter拆分

保留：

- draft/save/publish/delete/revision/schema/agent grant/call-time authorize；
- compiled与sandboxed共用RefusedCard；
- Server Web的opaque iframe、`allow-scripts`、CSP、nonce、fragment payload与MessageChannel readiness中已经证明的部分。

具体修改：

1. 将当前 `SandboxedComponentFrame` 拆为宿主无关的source/session model；
2. `WebSandboxFrame` 继续复用现有iframe实现；
3. 新增 `DesktopSandboxCanvas`，通过Tauri typed command申请Electron render session与ScreenHub ticket；
4. `tauri_host.rs` 增加render/session/input typed surface；
5. `openbot-desktop` 新增 `component_renderer.rs` 与 `screen_server.rs`；
6. `openbot-computer` 新增 component engine runtime；
7. engine启动或授权失败仍使用同一个RefusedCard，正常Desktop路径不再因custom scheme恒拒绝。

台账处理：

- `T-CMP-0008` RefusedCard的核心行为保持；其“custom scheme必然拒绝”过渡证据改为“强制engine失败仍共用RefusedCard”；
- `T-CMP-0015` iframe属性只对Web宿主有效，标 `split_applicability`；
- `T-CMP-0016/0017` 的Web CSP/nonce证据保留；Desktop Electron CSP/session另建T-ID；
- `T-CMP-0018` 拆成Web MessageChannel与Desktop engine broker；
- `T-CMP-0021/0022` 继续作为Electron Desktop renderer todo。

### 6.10 Tauri 已完成部分：全部保留，只增加engine边界

保留：

- Tauri 2.11.5 pin与`tauri-host` feature；
- `tauri_host.rs` 本地bundle/CSP/window authority/ApplicationService adapter；
- `transport.rs`、`broker.rs`、`event.rs`、`session.rs`、`window.rs`、`preferences.rs`；
- Tauri dependency/supply-chain guards；
- Axum/Tauri同一Arc或同一业务结果的done evidence。

新增：

```text
crates/openbot-desktop/src/engine_sidecar.rs
crates/openbot-desktop/src/screen_server.rs
crates/openbot-desktop/src/component_renderer.rs

crates/openbot-computer/src/scope.rs
crates/openbot-computer/src/manager.rs
crates/openbot-computer/src/engine/{protocol,driver,supervisor,process}.rs
crates/openbot-computer/src/screen/{ingress,hub,ticket,coordinates}.rs
crates/openbot-computer/src/component/runtime.rs
```

这是一条adapter扩展，不是把Tauri换成Electron GUI。

### 6.11 Tables/env/tests 的定向修改

- `computer_snapshot`旧table mapping保持done；用expand migration新增durable generation/engine instance/release epoch，或新建独立computer instance表；
- `AGENT_COMPUTER_URL`、`COMPUTER_TOKEN`、`COMPUTER_SUPERVISOR_URL`、`SUPERVISOR_TOKEN`等配置逐项判断Server兼容面与Desktop inherited pipe；旧env disposition不静默删除；
- 372个done test成为v4回归oracle；changed-symbol反向命中的测试标 `revalidation=pending`；
- pure TS→Rust迁移新增differential fixture，不用旧test名称冒充新Grok能力覆盖；
- 任一internal_refactor在原T-ID与新delta test未绿前不得合并。

### 6.12 completed-impact overlay

不直接重写九份ledger的历史字段，先生成：

```text
parity-v4/completed-impact.yaml
```

每项：

```yaml
test_id: T-...
baseline_snapshot: V3-B51
baseline_status: done
baseline_evidence: "原 done_evidence"
v4:
  disposition: carry | carry_extend | internal_refactor | adapter_rebind | split_applicability | remediate | superseded
  source_lineage: []
  affected_symbols: []
  current_targets: []
  affected_test_ids: []
  revalidation: not_required | pending | passed | blocked
  revalidation_commit: null
  revalidation_evidence: null
  delta_ids: []
  superseded_by: null
```

报告固定为：

```text
V3-B51：693 done / 993 todo（历史不回写）
v4 对既有693项：
  carry N
  carry_extend N
  internal_refactor pending/passed N/N
  adapter_rebind pending/passed N/N
  split_applicability N
  remediate N
  superseded N
v4新增能力：done X / todo Y / total Z
```

N/X/Y/Z必须由symbol/T-ID索引复算，不在方案里手填。

---

## 7. 机械台账 v4

### 7.1 冻结 V3-B51

现有口径保持：

```text
v3 parity   = 693 done / 993 todo / 1686
v3 fixtures = 17 done / 22 todo / 39
```

它只回答旧 OpenBot v3 范围，不再代表 v4 全部工作量。

### 7.2 新增 Grok source census

先由 Rust `oxc_parser` 扫描固定 tree，生成而非人工猜测：

```text
inventory/grok/files.yaml
inventory/grok/exports.yaml
inventory/grok/routes-ui.yaml
inventory/grok/extensions.yaml
inventory/grok/tools.yaml
inventory/grok/protocols.yaml
inventory/grok/tests.yaml
```

每个责任项至少包含：

```yaml
id: GRB-...
source_file: ...
source_symbol: ...
maturity: production | partial | generated | artifact-only | experimental
class: T | A | S | C | R | P
target_crate: ...
target_symbol: ...
inputs: ...
outputs: ...
errors: ...
state_concurrency: ...
production_call_point: ...
fixture: ...
affected_gates: []
status: candidate | specified | fixture-frozen | ported | integrated | evidenced | accepted
```

只有 `accepted` 计 done；generated LOC、目录存在和 typecheck 均不计完成。

### 7.3 v4 ledger 分组

- `v3-retained`：既有 T-ID 仍适用；
- `v3-superseded`：内部实现被 Grok-derived Rust architecture替代，但历史证据保留；
- `grok-translation`：T/A 项；
- `grok-capability`：新产品 C 项；
- `electron-shim`：S 项；
- `rejected-implementation`：R 负向 guard；
- `partial-evidence`：P 项，不进入承诺范围前不得计 todo/done。

### 7.4 Tauri 与 Electron 相关台账处理

- 既有 Tauri GUI、transport、preference、window authority done 不重开；
- Tauri sandbox renderer 的 todo 目标改为 Electron component engine；
- Electron进程/framing/CDP/ScreenHub仍是 todo，并扩展为 engine lifecycle/conformance；
- Axum/Tauri同一 `ApplicationService` 的业务行为 done 保留；以后补 Electron engine 不是第三个业务 transport；
- 旧 Electron shim 目标保留行为，implementation target更新为 purpose-built minimal shim；
- Grok Electron main 的主 GUI能力映射为 Rust/Tauri delta，不把整目录列成一个大条目。

### 7.5 进度报告

在 census 完成前不猜 v4 总数。固定报告：

```text
V3-B51 baseline: 693 / 1686
Grok census: classified X / discovered Y
V4 accepted scope: done A / todo B / total C
Electron shim: done D / total E
Fixtures: V3 17/39 + V4 F/G
```

不得把约 49.6 万行直接加到分母，也不得继续用 58.9% 表示 v4 剩余工程量。

---

## 8. G0-G8 修订

| Gate | v4 修订 |
| --- | --- |
| G0 Evidence | 两输入原件、T-FIX-0026、OpenBot strict recount零skip、Grok tree/source census全分类、v4 source precedence与ledger v2 |
| G1 Core/PG | 保持已闭合声明；解决checksum“已闭合/深校验未闭合”文案冲突；新增Rust host不得破坏同一ApplicationService |
| G2 Auth/Vault/Policy | 大面积翻译Grok auth/account/secrets/permission UX，但authority仍在Rust；consumer session不得成为隐藏provider credential |
| G3 Thread/Realtime/Memory | 翻译session/transcript/backstop/blob/recovery思想，落到既有PG event/outbox；第二真源为0 |
| G4 Agent/Extensions | extension DAG、scheduler、runner、subagent、automation、MCP/hooks、provider routing进入Rust；完整recorded trace/budget/cancel |
| G5 Computer/Isolation | per-scope Electron engine、realm、box/local-exec、file/shell、egress、fault/compromise；无Tauri WebView不可信内容 |
| G6 GUI | Tauri/Leptos主GUI + Grok交互/feature大面积迁移；routes/components/golden/a11y/multi-window；Electron component canvas |
| G7 Screen/Handover | Electron frame ingress、ScreenHub、ticket、coordinate/input、HumanLease、secret、性能；无VNC/WebView |
| G8 Release | Tauri+Rust+Electron/Chromium+PG/helper同release epoch，三平台签名/更新/SBOM/reproducibility/soak/第二次外审 |

### 8.1 G5 子闸门

- `G5A ElectronEngine`：唯一 shim、sandbox/config/IPC/CDP/conformance；
- `G5B Scope`：user/Bot/thread/generation/profile/workspace交叉0；
- `G5C ExecutionRealm`：HostLocal/IsolatedComputer无隐式fallback；
- `G5D FileShell`：三平台OS sandbox、TOCTOU/process/resource/cancel；
- `G5E EngineCompromise`：malicious renderer/engine/frame/outcome不能扩大scope；
- `G5F ComponentRuntime`：临时process/profile、零egress/callback、预算与清理。

---

## 9. 实施工作流与顺序

不再给没有实证的日历周数；每阶段只用入口条件、产物与退出证据控制。

### P0：正式 v4 真源与 census 工具

产物：

- 后端真源 v4、GUI真源 v2、`CLAUDE.md` 同步；
- 本文件五项用户裁决写成 ADR；
- ledger schema v2、Gate criterion IDs；
- Grok AST/source census；
- V3-B51 baseline manifest；
- 旧提案 superseded 标记。

退出：所有 source responsibility 已有 class/maturity/target；v4 总数可机械复算。

### P1：Electron engine 最小闭环

产物：

- purpose-built shim + generated protocol；
- Rust spawn/auth/hello/ready/shutdown；
- `app.enableSandbox()` 与固定安全配置；
- 一个 Browser role 与一个 Component role 各完成 start/render/frame/stop；
- binary/ASAR/fuse/digest/release epoch guard。

退出：没有 listening debug port、没有 preload/Node in renderer、malformed/stale frame全拒绝、进程清理0 orphan。

### P2：Rust coordinator / extension runtime

产物：

- Grok coordinator lifecycle、generation、backoff、resync、supervisor翻译；
- extension DAG、rollback/teardown；
- scheduler lanes、foreground/background/subagent state；
- Desktop/Agent/Computer child ownership与restart。

退出：duplicate/wrong-direction/stale-generation、crash/reconnect、shutdown race、zombie retirement全有机械证据。

### P3：Agent runner 与 core extensions

产物：

- stream attempt/retry/stall/checkpoint；
- context/prompt/tools/summarization/redaction；
- auth/settings/inference/turn/session/transcript/MCP/permission/local-exec/attachments/secrets/memory/audit；
- provider/remote AG-UI/budget/cancel与既有Rust管线合并。

退出：无Node host/Agent；每个acting effect仍满足durable decision/attempt/capability-before-action。

### P4：Browser/Screen/HumanLease

产物：

- full closed BrowserOperation → Electron CDP；
- profile/tab/snapshot/ref/generation；
- frame ingress/ACK/ScreenHub/viewer ticket；
- input/coordinate/IME final text/drag/HumanLease；
- download/dialog/file chooser/popup/permission拒绝。

退出：G5A/B/E与G7协议/性能/跨scope矩阵。

### P5：Component + local/box/file/shell

产物：

- Electron Component role与Tauri canvas；
- Web iframe与Desktop engine两宿主同component contract；
- HostLocal/IsolatedComputer；
- file handle、shell helper、box/runsc、egress、resource/cancel；
- Grok permission stale/abandoned/retry UX Rust化。

退出：G5C/D/F、sandbox escape=0、component crash/DoS不影响GUI、cancel后5秒进程树0。

### P6：Grok GUI核心旅程迁移

顺序：

1. conversation/composer/cards/replies/search/outline；
2. roster/agent-info/groups/org-chart；
3. computer/terminal/permission；
4. settings/plugins/accounts；
5. access/onboarding/recovery/update/window shell。

退出：每个journey均有Rust model/controller + Leptos view + production ApplicationService + refresh/reconnect + keyboard/AX/golden。

### P7：高级产品能力

- automations/routines/history/editor；
- subagent/background work/cloud agents；
- cross-user sharing、content search、teach recording；
- rich viewers、voice、PDF/spreadsheet/Mermaid/KaTeX等逐项技术delta；
- notification/tray/wallpaper/webauthn proxy按产品价值与平台证据推进。

退出：每个能力不是孤立UI，而是数据/API/event/policy/audit/recovery/GUI/test全链闭合。

### P8：发行与最终验收

- Tauri GUI、Rust sidecars、Electron/Chromium、PostgreSQL、sandbox helper原子打包；
- macOS/Windows/Linux支持矩阵、签名、公证、安装/升级/回滚；
- SBOM、NOTICE、provenance、secret/OSV、reproducible build；
- 24h soak、RPO/RTO、production-scale migration drill；
- 第二次安全审计与完整DoD。

---

## 10. 防止“大面积翻译”退化成大面积空壳

每个迁移项生命周期固定为：

```text
candidate
→ specified
→ fixture-frozen
→ ported
→ integrated
→ evidenced
→ accepted
```

约束：

- 只有 `accepted` 计 done；
- 每批一个端到端 capability，不先铺几十个零调用 module；
- 每项必须有 source symbol、production call point、target symbol、fixture、错误/并发/取消语义；
- pure logic 走 TS↔Rust differential；authority/IO 走真实 integration；
- generated code按实际消费 wire 生成，不按文件/行数翻译；
- partial、mock、fallback、placeholder 先归 P/R；
- 新能力必须同时有 API/event/route/UI/test/fixture，不能只补UI；
- Tauri GUI 与 Electron engine 的职责交叉由反向 grep/AST gate 判红；
- Electron shim每多一个API或一行职责都要解释为何无法放入Rust。

---

## 11. 关联方影响

| 关联方 | 主要影响 | 交付要求 |
| --- | --- | --- |
| Desktop 用户 | Tauri GUI保持轻量；Electron只在Computer/component需要时启动 | 状态清晰、按需启动、无孤儿、原子更新、资源上限 |
| Server用户 | 同Electron engine进入runsc | scope/egress/profile隔离、readiness fail-closed |
| 组件作者 | HTML/CSS/JS契约保留；Desktop由Electron渲染 | `window.__args`、零callback/network、明确预算与错误 |
| 无障碍用户 | Tauri GUI保持AA；Desktop component frame仍有限制 | 可信文本fallback、跳过预览、键盘退出、具名说明 |
| 企业管理员 | 新增realm、engine、automation/subagent等控制面 | policy ceiling、审计、readiness、配置迁移 |
| 安全团队 | Tauri可信GUI与Electron不可信engine边界更清晰 | shim审计、renderer sandbox、IPC/CDP fuzz、fault matrix |
| SRE/发布 | 两个宿主和多个sidecar需原子epoch | digest、signing、orphan recovery、patch SLA、rollback |
| QA/SDET | Grok功能面扩大测试空间 | census→fixture→differential→integration→golden流水线 |
| 开发团队 | 可大面积复用成熟设计，但不能机械照搬空壳 | responsibility级映射、单能力竖切、十crate边界 |

---

## 12. 仍需后续明确、但不阻塞本次方向的事项

1. Electron继续固定 `43.3.0 / Chromium 150.0.7871.212`，还是在正式v4 PR做一次版本delta；默认建议先保留现有较新pin，不退回Grok的42.1.0。
2. Linux Desktop是否继续作为首版supported target；当前后端真源说支持，GUI golden/Cargo历史只覆盖macOS/Windows。Electron engine路线可以支持Linux，但必须补GUI、sandbox、packaging三套证据。
3. `P7` 高级产品能力的首批优先级；全部进入census，但在census前不猜总量或排期。
4. Desktop component的辅助可访问fallback采用结构化args、作者提供摘要还是可信自动投影；不得由模型自由文本冒充。

---

## 13. 下一步实施入口

下一批不应直接写 Browser CDP 或继续旧 UI todo，而应先完成 P0：

1. 把本文件裁决写入正式后端 v4 与 GUI v2；
2. 更新 `CLAUDE.md` 的 source precedence、全量Rust定义与 Tauri/Electron职责；
3. 扩展 ledger schema 与 `xtask`，生成 Grok source census；
4. 复算 v4 总范围并给出 retained/superseded/new/rejected/partial 五类清单；
5. 再以 Electron engine 最小 start→frame→stop 作为首个代码竖切。

在上述五项完成之前，`693/993/1686` 继续只作为 V3-B51 历史基线，不用于衡量 v4。
