# Batch 49：Sandboxed Component Governance

> 日期：2026-08-28。分支 `codex/2026-08-28-G6-sandboxed-component-governance`；
> base `316a723`；WIP `93000d3`；implementation
> `9e46e128c572ee76c0585da1075e3195b0fdcbdf`；固定上游
> `CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。

本批只闭合沙箱组件的后端治理生命周期：管理员草稿、已登录者 published 投影、原子发布、
revision 单调递增、样例参数持久化，以及 Axum/Tauri 同一 ApplicationService 的 typed transport。
用户脚本执行、opaque-origin iframe、CSP/nonce、MessageChannel 与 Desktop 独立 renderer 不在没有
真实隔离实现前提前标绿。

## 第一真源裁决

- `sandboxed_components` 的 draft/published 双列是发布边界；任何读取模型/renderer 的路径只允许
  published 列，不允许 draft fallback；
- save、publish、delete 各自在一个 SERIALIZABLE PostgreSQL 事务里同时修改沙箱源、共享
  `components` 治理行与 audit。上游 TypeScript 的分步 delete 不是需要保留的故障窗口；
- `custom_` 名字空间与 `components.kind = sandboxed` 双重约束，禁止覆盖 compiled 组件；
- title、源码、JSON schema 与 sample arguments 在 contracts/application 边界先做闭合、有限验证，
  数据库只接受已规范化 DTO；样例参数只用于管理员预览，不进入 published provider 投影；
- admin 草稿面与写面由 application 再校验角色；transport 的敏感写仍要求 trusted Origin 与 fresh
  session，且在读取 body 前完成；
- published 输出没有 data-function 字段，沙箱源表也不连接 `component_functions`，保持“无 data
  function”是结构性事实。

## 实施

- contracts 新增 exact camelCase draft/record/published/envelope；`argumentSchema` 与
  `sampleArguments` 用 map 类型把 object 约束写进类型，published DTO 装不下 draft、sample、作者；
- application 新增独立 `SandboxedComponentAdministration`，它没有 data-function/callback 方法。
  exact 上游 slug grammar 为 2–40 字节小写字母/数字/中间下划线，服务端统一加 `custom_`；
  in-process 与 HTTP 共用 1 MiB 总边界，JSON 深度与 PostgreSQL 不接受的 NUL 先拒绝；
- PostgreSQL adapter 的 save/publish/delete 都走 SERIALIZABLE；source、共享 `components` 治理行、
  allowlisted hash-chain audit 同 commit。publish 锁双行、取一次 DB clock、复制描述及四类源并 checked
  revision+1；delete 以治理行 `kind=sandboxed` 判所有权，允许清掉历史治理 orphan，却不能触碰 compiled；
- published 查询用 FULL JOIN 同时发现任一方向的 source/governance orphan、published bit 漂移或空
  published 列，统一 fail-closed，绝不 fallback 到 draft；
- Axum 新的 parts-only `SensitiveOriginAuthenticated` 在 JSON extractor 前完成 fresh admin + trusted
  Origin；Tauri custom protocol 复用窗口 authority。两者的五个接口穿同一个
  `Arc<dyn ApplicationService>`，逐 JSON 语义对拍；
- `hasUnpublishedChanges` 没有擅自“修正”为比较描述/schema：按固定上游只比较 HTML/CSS/JS 三列。
  `authoredBy` 改存权威 actor id 而非 email，避免把可变 PII 当授权/审计身份。

## 证据

| 面 | 本轮亲自运行结果 |
| --- | --- |
| contracts / application / domain | **86 / 148 / 369**，均 0 失败 |
| Server / Desktop | 完整 **209 / 79**，均 0 失败 |
| transport | 既有穷举 parity **8/0/0**；新增同一 Arc 的 sandbox 五接口 Axum/Tauri 对拍 **1/0/0** |
| PostgreSQL 17.11 / SCRAM | lifecycle + compiled collision/orphan + 三类 audit 故障回滚 **1/0/0** |
| Clippy / WASM / format | 9 crate all-target/all-feature Clippy `-D warnings`；contracts/UI wasm32；fmt/diff 全绿 |
| parity | API **66/103/169**；components **8/14/22**；总计 **681/1005/1686**；fixtures **17/22/39** |
| 机械闸门 | parity-check 0 violation；固定上游 strict recount **158/0/0** |

真库实得 revision `0→1→2`，published audit payload revision 恰 `[1,2]`；只改描述/schema/sample
时 `hasUnpublishedChanges=false`，只改 HTML 后为 true，且两类草稿在 publish 前都不越界。
手工制造 shared governance `published=false` 与 `published_css=NULL` 后，published 投影分别以
`component_governance` / `published_css` corruption fail-closed，恢复权威列后才重新可读。
强制 `component.draft_saved` / `component.published` / `component.unpublished` audit INSERT 失败时，
对应 source、共享治理、revision 与 delete 全部回滚。`showQuote` 删除和 `custom_collision` 接管均被拒，
历史 `kind=sandboxed` 治理 orphan 可由同一 delete 原子清理。

关闭 `T-API-0049`–`0053`、`T-API-0103` 与 `T-CMP-0019`。`T-CMP-0020` 只完成后端
sample 持久化，playground 与会话复用同一隔离 wrapper 尚未实施，继续 todo；`T-CMP-0008`、
`0009`–`0018`、`0021`–`0022` 同样继续 todo。没有 production UI/CSS 变化，故本批不冒充
browser/golden/bundle 证据。

本批未运行 `cargo xtask ci` 或 GitHub Actions，未触碰 `docs/assets/`，未 push/建 PR。
