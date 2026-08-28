# Batch 50：Web Sandboxed Component Runtime

> 日期：2026-08-28。分支 `codex/2026-08-28-G6-web-sandbox-runtime`；
> base `40dac52`；WIP `8ccd88e`；implementation
> `c3e59a8663ba13d9644b3cad4e2599c64151bee0`；固定上游
> `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批把 Batch49 的 published 沙箱治理接入 provider、Agent gateway、durable conversation、Server Web
runner 与 Admin Playground。Web 路径具备 fail-closed 的 opaque-origin iframe 外壳；custom/Tauri scheme
明确拒绝在主 WebView 执行用户脚本。Desktop 独立 Chromium renderer、帧流/input broker、CPU/内存硬隔离
及其具名 a11y 豁免仍未实现，不能把本批写成 G6 或沙箱总项完成。

## 第一真源裁决

- dynamic tool name 只接受精确 `custom_` namespace。provider 每次 sampling 从权威 PostgreSQL 读取当前
  published、未 withheld、对当前 Agent 可见的定义；调用时再次复核 Agent scope、publication、withholding、
  当前 schema 与参数，旧 provider snapshot 不能绕过撤权；
- 沙箱 runtime port 只有 list/authorize，没有 data-function 调用。若 `custom_*` 治理行关联
  `component_functions`，adapter 以 `sandboxed_component_functions` corruption fail-closed；
- JSON Schema 在保存和运行时都编译，object 是结构约束；外部 `$ref` 明确拒绝，不能让模型定义或调用路径
  触发 renderer 外部读取。成功 confirmation 逐字保持上游
  `It is now on screen for the person.`；
- Server Web 只在 `http:`/`https:` host 创建 iframe，`sandbox` 恰为 `allow-scripts`，没有
  `allow-same-origin`、top navigation、popup、download、form 或 storage token。custom/Tauri scheme
  直接复用 `RefusedCard` 且 iframe 数为零；
- 不使用 `srcdoc`：它会继承父页面 CSP container。本批改为普通同源网络导航
  `/sandbox/runner?render=<random>#<base64url payload>`；query 里的随机 render id 只作缓存键，source、
  arguments 与 capability 全留在 fragment，不进入 HTTP request；
- runner 每次响应由 `ring::rand::SystemRandom` 生成 32-byte nonce，CSP 逐字为
  `default-src 'none'; connect-src 'none'; script-src 'nonce-<random>'; style-src 'unsafe-inline'; img-src data: blob:`。
  固定 bootstrap 是响应内唯一 nonce script；作者 HTML/CSS/JS 始终只作为 fragment 数据；
- bootstrap 用纯 ECMAScript 解 base64url 与严格 UTF-8，先验证 closed payload、尽力清 fragment，再写
  `window.__args`，最后才动态挂作者 script。bootstrap 先移除自身与 nonce；作者 script 固定前缀先清
  自身 nonce/attribute，再进入作者代码；
- host 初始挂 `about:blank`，随后导航 runner；第二次 load 才转移一次性 `MessagePort` capability，只接受
  string frame `openbot_sandbox_init:<cap>` / `ready:<cap>`。2 秒没有正确 ready 就销毁 broker 并显示共享
  `RefusedCard`；作者脚本不持有 port 或 host callback；
- production conversation 与 Playground 复用同一个 `SandboxedComponentFrame`。Playground 的 draft/sample
  仍只属管理员编辑面，invalid sample JSON 不创建 iframe；会话只消费 published source 与 durable
  successful tool result。

## 实施

- contracts 增加 dynamic sandbox name、exact confirmation 与 internal
  `AuthorizeSandboxedComponent` command；generic component decoder 接受 `kind=sandboxed`，但任何沙箱
  function row结构性拒绝；
- `SandboxedComponentAdministration` 增加 per-Agent dynamic definition 与 call-time authorize；PostgreSQL
  adapter 在同一权威治理面编译 schema、拒绝外部 `$ref`、记录拒绝 audit，并在 save 前先验证 schema；
- provider context 与 Server production assembly 注入同一 sandbox administration；Agent gateway 在 generic
  tool 路径前识别 exact `custom_` namespace，成功/拒绝均形成一个 durable tool reply；conversation
  projection 按 exact name 把成功结果交给 sandbox renderer，拒绝仍走共享卡；
- Server 新增无状态 `/sandbox/runner`，每响应随机 nonce、`no-store`、exact CSP 与单一 inline bootstrap；
  bootstrap 不依赖 first-party JS 文件，也不调用 `fetch`、`XMLHttpRequest`、`atob` 或 `TextDecoder`；
- UI 新增 production `SandboxedComponentFrame` 与 `/admin/playground`。管理页接 list/save/publish/delete、
  draft/source/schema/sample 编辑和同一 wrapper 预览；中英文本与样式进入既有 token/i18n 闸门。

## 证据

| 面 | 本轮亲自运行结果 |
| --- | --- |
| contracts / application / Agent / UI | **87 / 149 / 34 / 133**，均 0 失败 |
| Server / Desktop | 完整 **210 / 79**，均 0 失败 |
| transport | 既有穷举 parity **8/0/0**；sandbox Axum/Tauri 同一 Arc **1/0/0** |
| PostgreSQL 17.11 / SCRAM | Batch49 lifecycle **1/0/0**；新增 dynamic sandbox runtime **1/0/0** |
| Clippy / WASM / format | 9 crate all-target/all-feature Clippy `-D warnings`；contracts/UI wasm32；fmt/diff 全绿 |
| tools / i18n / design / CSS | tools 固定版本闸门全绿；**560** leaf；**89 Rust / 74 icons**；**292** class literals |
| production bundle | WASM gzip **1,400,622 B**；CSS **97,848 B**；fonts **740,216 B**；external/inline **1/0**；CSS 预算余 **456 B** |
| parity | API **66/103/169**；components **13/9/22**；UI **87/65/152**；总计 **686/1000/1686**；fixtures **17/22/39** |
| 机械闸门 | parity-check 0 violation；固定上游 strict recount **158/0/0** |

真库用例证明：provider schema 只有 publish 后刷新；合法参数授权成功，缺 required/类型错误拒绝；
withholding 在下一次 call-time authorize 立即生效；拒绝 audit 可复核；手工插入沙箱 function 行后所有动态
定义 fail-closed；外部 `$ref` 在 save 前拒绝。Batch49 的 publish/revision/audit 回滚用例同实例回归仍绿。

release fixture 的管理员页面实得 draft save、重复 publish、hard reload 后 revision 4；invalid sample JSON
为 `aria-invalid=true`、visible alert 1、iframe 0，恢复 object 后 iframe 1；中英标题、按钮和失败文案即时切换。
Playground 与 durable conversation 的 iframe 均实得 `sandbox="allow-scripts"`、`srcdoc=null`、同源 runner；
会话中 compiled refusal 与 sandbox startup failure 共两张共享 `component-refused`，作者 source 正文未进入
host DOM。hard reload 的 runner query id 不同。两页 duplicate id、nested interactive、外部资源、水平溢出、
最终 visible alert 与 console warn/error 均为 0。

## 不能提前标绿的边界

本轮可用 IAB 页面环境没有可交付的 `postMessage`、`addEventListener`、`MessageChannel` 能力，Chrome 又不可用。
因此真实 release 只能在 2 秒后按设计 fail-closed；这能证明拒绝路径，不能证明作者 JS 已执行。直接打开唯一
runner URL只验证了 payload 解析与 fragment 清除，不能替代 parent↔iframe channel 正向证据。viewport override
也没有改变 IAB 的 `innerWidth`，故不记多视口结果。

只关闭 `T-CMP-0008`、`T-CMP-0011`、`T-CMP-0015`、`T-CMP-0016`、`T-CMP-0017`。
`T-CMP-0009`（args 正向注入）、`0010`（作者 JS 执行）、`0012`（无网络实测）、`0013`（无 host
callback 实测）、`0014`（Web+Desktop 整体 opaque renderer）、`0018`（MessageChannel 正向握手）、
`0020`（sample args 经生产 wrapper 正向执行）、`0021`–`0022`（Desktop renderer/a11y）继续 todo。
`/admin/playground` 已有 production route/UI，但在正式 route journey、golden、AX 与可用浏览器正向沙箱证据
齐全前，不修改 route/UI ledger 为 done。

本批未运行 `cargo xtask ci` 或 GitHub Actions，未触碰 `docs/assets/`，未 push/建 PR。
