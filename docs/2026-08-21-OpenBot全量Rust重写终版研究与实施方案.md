# OpenBot 全量 Rust 重写：终版研究、前置审计与实施方案

> 日期：2026-08-21（America/Los_Angeles）
>
> 文档状态：终版实施基线 v2
>
> 目标：将 `CopilotKit/openbot` 的当前可观察产品能力完整重写为 Rust 实现
>
> 审计方式：两份第一真源只读；本文件为新结论，不反向改写第一真源
>
> 结论口径：源码、固定提交、一手规范或实测不能支持的说法，不进入实施事实

## 0. 最终裁决

### 0.1 可行性

**Go。** 按本文件给出的边界，OpenBot 可以完成全量 Rust 重写。

本项目对“全量 Rust”的最终定义固定为：

> GUI、业务、内置 Agent、策略、数据库访问、线程与记忆、实时事件、认证、凭据、审计、Supervisor 和全部高权限控制面由 Rust 实现；Chromium/Electron 仅作为受监管、可替换的浏览器执行引擎。

下列内容不违反该定义：

1. Leptos 编译成 WASM，并由 Tauri 的系统 WebView 渲染 HTML/CSS。
2. PostgreSQL、Chromium、Electron、操作系统 Keychain/KMS 作为外部引擎或系统设施存在。
3. 用户自己发布的 HTML/CSS/JavaScript 组件作为不可信数据，在零 Tauri 权限的独立沙箱中执行。
4. 用户接入的远程 AG-UI Agent 可以由任何语言实现；它属于外部不可信扩展，不属于第一方内置 Agent 或控制面。
5. 模型、MCP、Google Drive、OIDC/SAML IdP 是外部服务；所有调用、身份、凭据、授权和审计仍由 Rust 侧控制。

最终生产发行物中允许存在的第一方非 Rust 源码只有最小 Electron browser-engine shim。该 shim 不拥有产品身份、策略、审批、审计、模型/MCP/OIDC 凭据、任意文件或任意命令能力；其职责限定为 Chromium 生命周期、CDP、画面帧和封闭输入指令。除此之外，不保留 React、Hono、Bun、TypeScript Agent、TypeScript MCP runtime 或 JavaScript 业务控制面。

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

推荐资源基线固定为 **12 人、52 周**：1 名技术负责人，2 名数据/线程工程师，1 名认证/凭据工程师，2 名 Agent/协议工程师，2 名 browser/runtime/隔离工程师，2 名 Leptos/Tauri 工程师，1 名 SDET，1 名 SRE/发布工程师；第 20 周和第 40 周各安排一次不参与编码的独立安全审计。

52 周是本文件范围与人员假设下的计划基线，不是对未知代码的保证。任何新增数据库、额外 MCP 协议面、第二浏览器 driver、移动端、Firecracker、ACP 或新模型专用集成都不得挤入本次重写范围。

## 1. 第一真源与证据冻结

### 1.1 两份只读第一真源

| 文档 | SHA-256 | 本轮处理 |
| --- | --- | --- |
| `2026-08-21-OpenBot全量Rust重写与CrabCode复用审计结论.md` | `de9a0ed40522848d8cad4746beb87ac481036a1be48372e8caefac3c869cb95c` | 全文通读；未修改 |
| `2026-08-21-OpenBot全量Rust未完成能力深度研究与实现方案.md` | `5db37a2ca2471687e8d6e9c829c67cbc13484d1c1ee0d46b8c12182b7aaf49d5` | 全文通读；未修改 |

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
| Leptos | 稳定版 `0.8.19` |
| CrabCode browser kernel | Electron `43.3.0` / Chromium `150.0.7871.212` |
| Rust 工具链 | `1.94.1`，edition 2024 |
| PostgreSQL | 17 |

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
| `server/src` 静态 Hono route 注册 | 95；另有 Supervisor 5 条及 agent-computer 手写路由，不含 Better Auth/CopilotKit 动态注入 |
| `.test.ts/.test.tsx` 文件 | 105 |
| 版本控制测试文件中的 `test()` / `it()` 词法命中 | 1,007 |

这些数字用于完整性核算，不等于质量证明；1,007 是词法命中，不冒充 AST 解析后的精确 test 数。本轮依赖安装在审计环境中长时间未完成后被终止，因此本文件不宣称上游测试当前全部通过。Phase 0 必须在干净、可联网的受控 CI 中运行、生成 AST 级测试 inventory 并归档原始结果。

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
| `allowed_groups` 已存储但没有身份组写入/授权使用 | 只有配置 IdP group claim mapping 后才允许包使用 `allowed_groups`；否则启动拒绝该包，不再保留无效安全字段 |
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
7. settings、主题、connected accounts、component gallery，以及 native memory 的查看/纠正/删除/禁用控制。
8. admin people、identity providers、credentials、computers、boundaries、audit。
9. admin plugins、connector、单 tool grant 页面、OAuth client 配置。
10. compiled components、sandboxed component playground、draft/publish/unpublish、HITL decision。

31 个现有 route 文件全部映射到一个 Leptos route 或 layout。路由可以合并代码，不能删除用户可观察能力。

### 3.2 Coworker、Channel、Routing 与 Tenant Package

必须保持：

- public/private、owner/admin 访问规则；无权访问统一返回 404，避免资源枚举；
- per-user hidden roster；
- coworker soft delete 后旧 channel 可读、不可再次运行；
- channel membership 覆盖所有读写、realtime、screen 和 control 路径；
- 显式 `@` 选择优先，未标注消息进入 routing，router 失败使用默认 coworker；
- routing audit 记录候选、选择和原因，不记录原始用户消息；
- tool/connector holdings 参与 routing 候选描述，但 discovery 不产生权限；
- tenant package 的 brand、agents、channels、model、knowledge 五类 YAML 继续做 schema 与引用检查；
- `knowledge.sources` 保留为兼容输入并产生“不执行本地同步”的明确状态，不建立 customer document index。

### 3.3 Generative UI 与 Components

正式实现同时覆盖两条路径：

1. **Compiled gallery**：现有 React gallery 全部重写为 Leptos component；保留参数 schema、published 状态、per-Bot withholding、data-function grant 和 tool-call-time 再授权。
2. **Sandboxed component**：保留用户 authored HTML/CSS/JavaScript、draft/publish/revision/sample arguments；它属于不可信用户数据，不属于第一方 GUI 控制面。

Sandboxed component 固定运行位置：Server Web 在浏览器 opaque-origin sandbox iframe；Desktop 不在主 Tauri WebView 创建用户脚本 iframe，而是在独立、零 Tauri capability 的 component Chromium renderer 中运行，通过 typed MessageChannel/broker 返回渲染帧和交互事件。

固定运行规则：

- 独立 opaque-origin iframe/Chromium renderer，不在主 Tauri WebView 执行；
- `sandbox="allow-scripts"`，不得使用 `allow-same-origin`、top navigation、popup、download 或 storage；
- CSP 固定为 `default-src 'none'; connect-src 'none'; script-src 'nonce-<per-render-random>'; style-src 'unsafe-inline'; img-src data: blob:`；Rust wrapper 只给本次封装后的用户脚本设置 nonce，默认无网络；
- 不加载 Node、preload、Electron API 或 Tauri API；
- 使用一次性 MessageChannel capability 与 Rust typed broker 通信；
- data function 每次调用重新检查 component revision、Bot grant、actor ACL、policy 和 audit；
- 组件崩溃、超时或 schema 错误只终止该 iframe，不影响 transcript 或主 GUI。

### 3.4 外部扩展兼容面

- 任意 remote AG-UI endpoint；
- write-only endpoint authorization header；
- standing role 去重注入；
- per-agent callback token hash；
- 10 分钟、绑定 actor/bot/run/tool 的签名 run assertion；
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
8. memory 只保存三类：用户明确要求保留的 preference、带来源的事实、Bot 的 operational checkpoint；每条记录包含 scope、source message/thread、sensitivity、created_by、supersedes、expires_at。
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

所有 ID 是 string newtype，不擅自限定为 UUID；创建端可以使用 UUIDv7/ULID，兼容端必须接受上游既有字符串。

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

`AuthContext` 只能由 Rust 根据 session、连接 peer、数据库 ACL 和资源映射构造。模型、renderer、MCP server、remote Agent 或 browser engine 传来的同名字段一律视为普通不可信输入。

## 6. Auth、People、Session 与 Vault

### 6.1 Desktop 与 Server 身份模型

| 模式 | 身份真源 | 固定行为 |
| --- | --- | --- |
| Desktop Local | 当前 OS 用户 + 本地 app instance | 单用户 admin；不启动 Web SSO；本机 capability 不可远程使用 |
| Desktop Remote / Server | Rust session + OIDC/SAML identity | 多用户、role、membership、revocation、fresh authorization |

无 IdP 时，Server 只有显式 `OPENBOT_SINGLE_USER=true` 才启动；该模式只允许 loopback 或管理员明确配置的受控网络绑定。`NODE_ENV` 不改变此规则。

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

OIDC discovery/JWKS 与任何 IdP metadata fetch 使用和 remote Agent/MCP 相同的 safe dialer、redirect/IP 校验、大小/时间上限；SAML metadata 默认由管理员粘贴/上传并离线验证，不允许一个未验证 URL驱动 server 内网请求。

pre-auth surface 只公开环境配置的 provider ID 和“存在企业 SSO”布尔值，不列出企业 domain/provider；email routing 的成功/失败使用统一响应并按 IP/email hash 限速，避免组织枚举和 callback flood。

Rust 选型：OIDC 使用 `openidconnect` 4.x；SAML 使用固定版本 `samael`/xmlsec 组合并接受独立 XML signature wrapping/replay 外审。SAML 外审未通过时不得发布 GA，不能关闭功能冒充对等完成，也不能用 Node/Java sidecar 绕过 Rust 边界。

### 6.3 Session

- server cookie：`HttpOnly`、`Secure`、`SameSite=Lax`，host-only，短 idle + 绝对期限；
- session token 数据库只保存 keyed hash，不保存可直接使用的明文 token；
- 敏感 admin 写操作要求 fresh session，并校验 CSRF/origin；
- refresh/reauth 不沿用旧 auth generation；
- WebSocket handshake 绑定 fresh session，之后每次高权限 server request 再检查 generation；
- 从 Better Auth 切换到 Rust Auth 时，旧 Better Auth session 全部失效，用户统一重新登录一次；不反向工程其 cookie。

### 6.4 Vault

| 环境 | Master key | Record key |
| --- | --- | --- |
| Desktop | Keychain / Windows Credential Manager / Secret Service | 每记录随机 DEK，由 master key 包装 |
| Server | KMS/HSM 或受控 secret manager 中的 tenant KEK | 每记录随机 DEK，由 tenant KEK 包装 |

record AEAD 的 AAD 固定绑定 `tenant_id + secret_id + kind + owner + consumer + key_version`。secret 数据模型同时记录 resource、scope、expiry、credential generation 和 revocation state。

迁移期必须兼容读取当前 AES-GCM v1 envelope（12 字节 IV、无 AAD）。迁移顺序固定为：读 v1 → 解密 → 事务写 v2 → 校验回读 → 标记旧 envelope retired。不能在同一 release 同时更换 Auth、KEK 和 credential schema。

以下值永不进入 Leptos state、Agent prompt、AG-UI、browser event、普通日志、trace、metric、crash dump 或 screen URL：model key、MCP/OAuth refresh token、OIDC/SAML secret、computer bootstrap secret、run signing key、updater key。

### 6.5 Group access 的修正

当前 `users.groups` 与 `channels.allowed_groups` 不是生效的控制。Rust 版作确定修正：

1. 每个动态 IdP 可配置一个明确的 group claim path 和规范化规则。
2. Tenant package 出现非空 `allowed_groups`，但相关 IdP 没有 group mapping 时，package 校验失败并指出原因。
3. 登录时将 verified group claims 写入 membership projection；每次 session refresh 重算。
4. group 只负责 provision channel membership，所有运行时 channel route 仍检查 materialized membership。
5. IdP 撤组后递增 auth generation 并撤销相应 membership；不等待下次应用重启。

## 7. Rust Agent、Provider 与 AG-UI

### 7.1 产品中存在两类 Agent

| 类型 | 实现责任 | 语言约束 |
| --- | --- | --- |
| built-in Agent | `openbot-agent` 内的 Rust loop | 第一方生产逻辑必须 Rust |
| remote AG-UI Agent | 外部 endpoint | 任意语言；按不可信服务处理 |

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

每个 thread 一个 foreground actor，串行处理 prompt、steer、cancel、tool result、MCP/computer lifecycle 和 timeout。默认 tool step cap 保持当前行为：8；默认 run absolute deadline 30 分钟；任何后台任务必须是独立 durable run，不共享 foreground mutable future。

### 7.3 Provider adapters

首版固定三类 provider：

1. `openai-compatible`：OpenAI Chat Completions/Responses，以及明确声明兼容的网关/xAI endpoint；
2. `anthropic`；
3. `google-generative-ai`。

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

### 7.4 Retry、Cancel、Budget 与 Commit

- `CancellationToken` 按 run → provider/tool/computer/process tree 传播；
- UI 先显示 `Cancelling`，收到子任务终止事实后才显示 `Cancelled`；
- 429、明确可重试 5xx 和连接前失败可指数退避；认证、schema、policy 错误不重试；
- 非幂等请求已发送但未确认时，`commit_state=Unknown`，进入 reconciliation；
- tool 只有显式 `parallel_safe=true` 且资源锁不冲突才并行；结果按原 call 顺序回注；
- budget 同时限制 absolute deadline、idle deadline、provider token、tool steps、并发 tool、computer runtime 和用户配置的费用上限；
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

使用固定版 `cel-rust`，但不能凭语法相似宣称替代 `cel-js`。Phase 0 从现有默认、测试和生产脱敏 policy 构建 corpus；Rust 对每条 expression、context、结果和错误语义做 golden 对照。

规则固定：deny 先于 allow；missing/empty/broken policy fail-closed；`dry-run` 只改变执行拦截，不跳过 decision/audit；policy version 进入 approval 和 capability，版本变化后旧批准失效。

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

### 8.6 Audit

- Server：业务 DB role 对 audit 只有 INSERT/SELECT，无 UPDATE/DELETE/TRUNCATE；migration/retention 使用分离角色；
- audit 分区按 retention policy 关闭，不允许业务代码删除单行来“清理”；
- 对 audit event 建 hash chain，并将周期 checkpoint 签名写入不可变对象存储；
- retention 删除旧分区前先写并外存该分区的首尾 hash、event count 和 signed closure checkpoint，保留链边界；
- Desktop：同样 append-only，但只承诺可追溯，不宣称抵抗设备所有者/root 篡改；
- payload 使用字段 allowlist，不保存原始 header/body、prompt、tool full result、screen frame、文件内容、secret 或可验证 secret hash；
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

单 server 固定上限：1,000 tools、每 tool description 4 KiB、input schema 256 KiB、单 call model-visible text 20,000 Unicode scalar values；超限 listing/call 显式失败或可见截断，不静默把任意 vendor payload塞入模型上下文。

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

每个 thread/channel 有独立 workspace/download/artifact root。相同 principal 的浏览器 profile 可以跨 thread 保留登录，但 profile 同一时刻只能被一个 ComputerInstance 持锁；切换 workspace 前必须结束前一 lease。

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

## 11. Browser Engine 与 OpenBot/CrabCode 复用

### 11.1 单一 engine

Desktop 与 Server 使用同一个最小 Electron/Chromium engine 和同一 conformance suite；Server 将其置于 runsc container。首版不维护 Playwright engine 与 CrabCode engine 两套生产实现。当前 OpenBot 的 Playwright 代码是行为 oracle 与 fixture 来源。

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

它不接收 `actor_id`、role、policy、intent 或 `policy_decision_id`；这些留在 Rust。`BrowserOperation` 是封闭 enum：navigate、snapshot、read、click、type、key、scroll、screenshot、screencast、input、download/artifact。禁止自由 CDP method、自由 HTTP passthrough、自由本机路径、任意 shell 或任意环境变量。

### 11.3 Browser 安全配置

- remote page：`nodeIntegration=false`、`contextIsolation=true`、`sandbox=true`、`webSecurity=true`；
- remote page 无 preload、无 Electron/Tauri API；
- permission request/check handler 默认拒绝 camera、microphone、screen capture、geolocation、USB、HID、serial、Bluetooth、notification 和 clipboard；
- popup/new-window 默认拒绝；外部打开只接受 Rust 重新验证的 URL；
- 不开放 remote debugging port，只通过 `webContents.debugger` 使用 CDP；
- 启用 ASAR integrity，关闭未用 Electron fuses，禁止 `ELECTRON_RUN_AS_NODE`；
- download 进入 quarantine，校验名称、MIME、大小，不自动打开；
- file upload 只接受 Rust 铸造、scope 绑定的 artifact handle；
- Electron/Chromium critical/high 修复在 72 小时内升级；无法及时升级时关闭受影响能力或停止发行；
- browser sidecar 不自更新，必须与 Rust/Tauri 原子版本、原子签名。

### 11.4 CrabCode 复用清单

| 资产 | 正确用途 | 禁止做法 |
| --- | --- | --- |
| `acosmi-supervisor` / daemon launcher / heartbeat | 进程 registry、PID identity、watchdog、shutdown、socket lock | 整 crate 无审计复制 |
| permission / shell parser / sandbox / exec | 平台 sandbox、command plan、process tree、fidelity | 把 CrabCode 单用户路径语义当 OpenBot ACL |
| `acosmi-cmd-browser` | Rust browser request adapter、explicit target、timeouts | 暴露自由 method/path |
| `components/browser-shell` | Electron/Chromium lifecycle、tab、snapshot、input、download、framing | 复制完整 desktop host、Design/账号/Office 能力 |
| app-server protocol/transport | framing、origin、auth、thread/turn fixture | 复制 200+ method 的产品专属 dispatcher |
| 历史 Tauri host | path/artifact/log/menu/deep-link/single-instance/cleanup 模式 | 回滚整个已删除提交 |
| TS Agent/MCP | 行为 fixture、错误语义 | 进入最终生产控制面 |

所有行目前都是“权属清理后可用”，不是自动授权。每个复制文件须有 `SOURCE_PROVENANCE`：权利人、原路径、上游路径/commit、原/目标 hash、许可证、修改声明、书面授权编号。

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

CDP `startScreencast` 当前只选择 JPEG（生产）与 PNG（诊断），不把 WebP 写成已支持格式。ACK 在帧成功进入 size-1 latest buffer 后发送；慢消费者只能丢旧帧，不能形成无界队列。

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

坐标转换使用 frame metadata、DPI、zoom、scroll、canvas letterbox。输入 union 包含 mouse、wheel、key、insert text、IME composition 和 drag；不提供自由 CDP。

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

- 目标：1280×800、10 fps passive/15 fps driving、JPEG quality 65；
- loopback capture-to-paint p95 ≤ 200 ms，p99 ≤ 400 ms；
- 每 viewer 最多 1 个待发 frame，ScreenHub 每 tab 最多 2 个 frame buffer；
- 最后 viewer 断开后 2 秒内停止 screencast；
- `Page.startScreencast` capability 不存在时，降级为 `captureScreenshot` 2 fps，并在 UI 明示“低频预览”；不称实时；
- Electron offscreen/beginFrameSubscription 不作为第二生产路径，只作为实验 fixture。

必须覆盖 DPI/zoom/scroll、resize、navigation、tab switch/close、frame corruption/order、慢消费者、ticket replay、engine restart、多 viewer、IME、drag、human lease race 和跨 scope frame 注入。

## 13. Tauri/Leptos 与 in-process transport

### 13.1 GUI

- Leptos CSR/WASM；不维护 React 第二 GUI；
- Server 由 Axum 提供相同静态 bundle；Desktop 由 Tauri custom protocol 提供；
- 主 WebView 只加载打包本地内容，拒绝 remote navigation；
- strict CSP，无 `eval`、无远程 script、无宽泛 `connect-src`；
- deep link、file association、clipboard 和 external URL 都当不可信输入；
- Tauri capability 按 window label 单独配置，禁止 `windows:["*"]`、宽泛 filesystem 和 remote API access；
- production 禁用 devtools，所有 command 枚举注册并生成审计清单。

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

R1 不要求 pgvector，也不重建 customer document index。升级前先要求旧 OpenBot 把数据库迁到当前第 13 条 migration；Rust 不接收更早 schema。Fresh install 使用当前最终 schema 的 Rust baseline，不创建已删除的 document/vector 表；现有数据库中的 `vector` extension 原样保留，不能因未使用而在兼容 migration 中删除。

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
- stale snapshot/generation 409；lease conflict 409；
- unknown commit 202/409 对应 reconciliation，不伪装 500 或 success；
- 空、新 thread history 200 + empty list。

错误给用户的文本可本地化，但 stable code、HTTP status 和 audit event 类型不能随文案变化。

## 16. 部署、打包、更新与可观测性

### 16.1 Server 发行物

发布：

- `openbot-server` OCI image；
- `openbot-supervisor` OCI image；
- 固定 digest 的 `openbot-computer` image；
- PostgreSQL migration binary；
- Docker Compose production/dev profiles；
- SBOM、provenance、NOTICE、config schema 和 runbook。

all-in-one image 只允许 `OPENBOT_SINGLE_USER=true` 的 local/dev profile；multi-user production 未配置独立 Supervisor + runsc 时 readiness 失败，不能静默退回共享 browser。

### 16.2 Desktop 发行物

- macOS arm64/x64 signed + notarized；
- Windows 11 x64 Authenticode installer；
- Linux x64 AppImage/deb 为 supported desktop；
- Electron/Chromium、PostgreSQL、helper 与 Rust/Tauri 作为一个 release epoch 原子交付；
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
secret scan
license/NOTICE/provenance verification
CycloneDX/SPDX SBOM
reproducibility check
artifact signature/provenance verification
```

`Cargo.lock` 和 engine lockfile 提交；git dependency 必须固定 commit。build.rs、proc macro、FFI 和 `unsafe` crate 单列审计；核心 crate `unsafe_code = "deny"`，确需 unsafe 的窄 crate 有 owner、测试和安全说明。

### 16.4 Observability

Rust 全链使用 `tracing` + OpenTelemetry；Server 暴露 Prometheus metrics；Desktop 默认只保留 7 天 redacted local ring buffer，不自动外传。

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

必须同时假设：恶意网页、prompt injection、恶意 remote Agent、恶意 MCP server、被攻陷 browser engine、被攻陷 Tauri renderer、普通用户越权、管理员误配、同主机其他进程、供应链包、数据库故障、网络中断和 provider 返回畸形流。

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
| Memory | domain/agent | explicit preference/fact/operational checkpoint、provenance、delete | scope/recall/supersede/delete fixtures；无跨用户 recall |
| Built-in Agent | agent | 3 provider families、stream/tool loop、8-step、cancel/budget/recovery | recorded stream、partial JSON、429/断流/unknown commit |
| Remote AG-UI | agent/server | endpoint safety、standing role、callback token/run assertion、stall | 官方 schema golden；SSRF/token/expiry/断流负向 |
| Tool/Policy/Audit | domain/application | schema/effect/CEL/approval/decision-attempt-outcome | corpus 对等；audit-before-action 违规 0 |
| MCP | agent | RMCP 3.1.4 Streamable HTTP tools/OAuth/per-call | 官方相关 client conformance 100%；恶意 server suite |
| Google Drive | agent | REST adapter、per-user OAuth、disconnect、read-only result | live sandbox tenant contract；撤权下一调用生效 |
| Skills/grants | application | ownership、grant、stale suspension、catalog generation | 授予/撤销/消失/重现全状态测试 |
| Components | ui/application | compiled Leptos、sandboxed HTML/CSS/JS、publish/withhold/data function/HITL | render/schema/visual/a11y；sandbox escape 0 |
| ComputerManager | computer | security scope、generation、driver、lease、quota、reconcile | 多用户同 Bot + 多 Bot 隔离；crash/reset/upgrade |
| Supervisor | computer | server runsc containers；desktop process tree | socket 不在 API；digest/namespace/resource/cleanup |
| Browser actions | computer | navigate/read/snapshot/click/type/key/scroll/download/artifact | OpenBot + CrabCode fixture；旧 ref 100% 拒绝 |
| Screen/input | computer/server/desktop | CDP、binary hub、ticket、coordinates、IME、human lease | latency/fps/backpressure/replay/race/跨 scope 注入 |
| File/shell | computer | canonical handle、symlink/hardlink、env、timeout/cancel | path corpus；cancel 5 秒内进程树归零 |
| Leptos GUI | ui | 31 route 对应旅程、settings/admin/sign-in、web/desktop | route ledger 100%；visual/a11y/E2E |
| Tauri | desktop | capability、typed in-process、multi-window、update/sidecar | XSS 模拟；queue saturation；签名安装/升级/回滚 |
| Server/deployment | server/testkit | OCI/Compose/migration/health/readiness/multi-replica | clean checkout；8-Bot soak；backup/restore |
| Observability | 全部 | OTel/metrics/log/redaction/support bundle | run→decision→tool→computer 全链可追踪；无 secret |
| Release/legal | testkit/CI | SBOM/provenance/NOTICE/signature/brand separation | 未知 license/未登记复制/unsigned binary 构建失败 |

## 19. 52 周实施顺序

### 19.1 团队

| 角色 | 人数 | 唯一主责 |
| --- | ---: | --- |
| 技术负责人 | 1 | architecture decisions、contracts、delta audit、闸门 |
| Rust data/thread 工程师 | 2 | PostgreSQL、native thread/realtime/memory、migration |
| Rust auth/vault 工程师 | 1 | OIDC/SAML/session/group/vault/policy/audit |
| Agent/protocol 工程师 | 2 | provider、Agent loop、AG-UI、MCP、Drive |
| Browser/runtime/security 工程师 | 2 | CrabCode 抽取、engine、Supervisor、isolation、screen/file/shell |
| Leptos/Tauri 工程师 | 2 | 31 routes、components、in-process、desktop packaging |
| SDET | 1 | parity ledger、golden、E2E、fault/perf/security regression |
| SRE/release | 1 | OTel、CI、OCI、signing、backup、migration、rollout |

### 19.2 Calendar

| 周 | 阶段 | 交付物 | 退出闸门 |
| --- | --- | --- | --- |
| W1–4 | Evidence Freeze | 全部 commit、SBOM/NOTICE/provenance；API/page/table/env/event/test ledger；旧系统 trace | 未归类能力/route/table/test 为 0；上游测试原始结果归档 |
| W5–10 | Rust Foundation | 10 crate、contracts、ApplicationService、Axum/Tauri adapter、Postgres read、OTel | 三平台编译；HTTP/in-process 同 use case 结果一致；DB read checksum 100% |
| W11–20 | Data/Auth/Governance | repository、native thread base、Auth/Vault、CEL、audit、tenant package | 28 表映射；身份矩阵；v1 decrypt；audit-before-action 0 违规；第 20 周外审 |
| W11–24 | Computer 并行线 | provenance-approved CrabCode substrate、scope、Supervisor、engine、file/shell | 同 Bot 不同用户 + 多 Bot 负向隔离；crash/reset/update；orphan 0 |
| W11–28 | GUI 并行线 | Leptos shell、31 routes、compiled/sandboxed components | route journey 100%；web/Tauri parity；sandbox/a11y/visual gate |
| W21–32 | Agent/Protocol | native realtime/memory、3 providers、built-in Agent、remote AG-UI、MCP/Drive | trace/conformance；callback/stall/cancel/recovery；无 Intelligence 运行 |
| W25–34 | Screen/Full Chain | binary screen、ticket、human/secret、tool/audit/computer/UI E2E | screen SLO；跨 scope frame 0；全主路径 E2E |
| W33–40 | Parity Closure | 全部 AST 级 test inventory mapping、perf/soak/fault/security、signed packages | ledger coverage 100%；0 P0/P1；第 40 周外审通过 |
| W41–45 | Migration Rehearsal | Postgres + Intelligence export/import；3 次 production-scale drill | 三次 checksum 0 差异；RPO/RTO 达标；cutover runbook 签字 |
| W46–52 | Pilot/GA | 7 天 internal、3 天 5%、3 天 25%、3 天 50%、7 天 100%、buffer | 全 SLO 连续 7 天；旧系统只读归档；GA evidence bundle |

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
```

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
2. 映射 deployment/user/bot/thread ID；
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
6. 所有环境变量标记 preserve/rename/remove，并提供启动错误或 migration 文档；未知变量不静默忽略。

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
- DPI/zoom/scroll/letterbox/resize/tab close、ticket replay、多 viewer、IME/drag；
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

## 23. 许可证、来源与品牌

### 23.1 代码许可

1. OpenBot 固定源码为 MIT。逐语言翻译仍按衍生实现治理，发行包保留 `Copyright (c) 2026 CopilotKit` 和 MIT 文本。
2. OpenAI Codex 是 Apache-2.0；复制/改造文件保留 SPDX、copyright、来源 commit、显著修改声明和适用 NOTICE。
3. Grok Build 第一方代码为 Apache-2.0，但部分工具来自 Codex/OpenCode；必须回溯原始来源和第三方声明，不能只记 xAI。
4. AG-UI 为 MIT；RMCP/规范在许可证迁移过程中，按固定 commit 的实际文件 license 处理，不能给整个仓库套一个猜测。
5. Electron 自身 MIT，Chromium、Node、FFmpeg 等随其发行包的第三方 notices 必须原样交付。
6. Steel Browser/CDP 参考代码分别按其 Apache/BSD 等固定来源处理。
7. CrabCode 根 `THIRD_PARTY_NOTICES.md` 明示其为 closed-source proprietary；workspace `license = MIT` 只是一条 metadata，不能覆盖根声明或单文件来源。

所有复用生成机器可读 provenance：source repo、commit、original path、destination、license、copyright、modified flag、source/target hash、authorization。

### 23.2 新项目发行许可

本次实施默认是内部、闭源、all-rights-reserved 的第一方新代码；MIT/Apache 等第三方代码按各自条款分区随包。该默认值避免在权利人尚未书面决定时擅自把 CrabCode 专有资产开放。若未来开源，必须另立书面发布决议并重新做 whole-tree license audit，不在本次重写中自动发生。

### 23.3 服务许可不等于源码许可

复用 Codex/Grok Build 开源代码不授予 OpenAI/xAI 模型、消费者订阅或 OAuth 使用权。模型只使用官方 developer/API credential；不得复制 Codex/Grok/CrabCode 的消费者账号桥或私有 token。

CopilotKit Intelligence、OpenAI API、xAI API、Google Drive 和 IdP 都有独立服务条款、数据处理与费用边界；Rust 代码许可证不能替代服务合同。

CopilotKit 当前服务条款把 managed services 与 open-source components 分开，并限制使用服务构建相似/竞争服务及逆向服务源码。因此 native thread/memory/realtime 只能依据 OpenBot MIT 源码、开放协议、自有需求与黑盒可观察用户契约做 clean-room 实现；不得把 managed Intelligence 私有响应、反编译结果或未授权内部资料当源码。旧数据导出必须使用客户账户依法可用的 export/API，并在迁移前取得合同/法务确认。

### 23.4 品牌

MIT/Apache 不授予商标权。对外产品名称、bundle ID、domain、deep-link scheme、图标不得包含或仿冒 OpenBot、CopilotKit、Codex、OpenAI、Grok、xAI。内部仓库可以使用 `openbot-rs` 作为迁移代号；外部发行前必须使用完成商标清查的新品牌。法律 notices 和准确兼容性说明可以引用来源，并明确“无从属、认证或背书关系”。

## 24. Go/No-Go 闸门

### G0：Evidence 与权属

- 固定 source/provenance/SBOM/NOTICE；
- API/page/table/env/event/test parity ledger 未分类项=0；
- CrabCode 每个拟复制文件有授权或明确转 clean-room；
- 上游基线测试原始结果归档。

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
- engine compromise fixture 无法扩大 scope。

### G6：GUI/Components/Tauri

- 31 route journey 100%；
- compiled gallery全部 Leptos；sandbox escape=0；
- multi-window ACL、Tauri XSS、queue saturation/shutdown；
- web/desktop visual/a11y parity。

### G7：Screen/Handover

- 目标 fps/latency/backpressure；
- ticket/replay/origin/generation；
- coordinates/IME/drag；
- human lease 时 Agent acting 100% 拒绝；secret canary 0 泄漏。

### G8：Migration/Release

- 三次 production-scale backup/import/restore；
- RPO/RTO；
- signed OCI/installer/atomic sidecar update；
- Phase 0 AST 级 test inventory mapping 100%；
- 第二次外部安全审计无 P0/P1；
- 供应链、NOTICE、brand、runbook 全通过。

任何闸门失败都只能修复后重跑，不能以“后续补齐”进入下一发布阶段。

## 25. Definition of Done

只有同时满足以下条件，才能称“OpenBot 已完成全量 Rust 重写”：

1. 固定 OpenBot commit 的正式页面、API、数据、协议、治理、部署和用户旅程全部在 Rust-owned 路径有可重复证据。
2. 第一方 React/Hono/Bun/TypeScript Agent/MCP/Auth/DB 生产链清零。
3. 非 Rust 例外仅为最小 browser-engine shim、外部引擎/服务和受隔离的用户脚本数据。
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
