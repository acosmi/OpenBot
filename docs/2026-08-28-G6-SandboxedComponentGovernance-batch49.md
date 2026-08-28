# Batch 49：Sandboxed Component Governance

> 状态：WIP。日期：2026-08-28。分支
> `codex/2026-08-28-G6-sandboxed-component-governance`；base `316a723`；固定上游
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

## 待完成与证据

- [ ] contracts 与 application port/use cases；
- [ ] PostgreSQL 原子治理实现与真实 PostgreSQL 故障回滚测试；
- [ ] Axum/Tauri transport 与双路对拍；
- [ ] ledger、CLAUDE.md、recount 与定向质量闸门；
- [ ] 清理本批生成的编译/数据库临时产物。

本批不运行 `cargo xtask ci` 或 GitHub Actions，不触碰 `docs/assets/`，未经本轮明确授权不 push/建 PR。
