//! `openbot-contracts` —— 协议层：native/wasm-safe 的 ID、DTO、event、error、schema。
//!
//! # 所有权边界（v3 §5.1 / §5.2 / §5.3）
//!
//! 负责：
//!
//! - 核心 ID 的 string newtype 定义（v3 §5.3 十五个：`DeploymentId` / `TenantId` / `ActorId` /
//!   `BotId` / `ChannelId` / `ThreadId` / `RunId` / `ToolCallId` / `CredentialPrincipalId` /
//!   `ComputerId` / `ComputerGeneration` / `TabId` / `DocumentGeneration` / `PolicyDecisionId` /
//!   `AuditEventId`）。ID 一律不擅自限定为 UUID；创建端可以用 UUIDv7/ULID，兼容端必须接受
//!   上游既有字符串。
//! - 跨边界 DTO、event 族、错误码与 schema。错误以 **code** 穿越边界（CLAUDE.md §4a），
//!   状态码与 audit 类型固定（v3 §15.3），文案不在这里。
//! - 唯一一个既编 native 又编 wasm 的 crate：`openbot-ui` 只依赖它（设计系统文档 §13）。
//!
//! 明确**不**负责：
//!
//! - 领域不变量与状态机 —— 在 `openbot-domain`。
//! - use case 编排 —— 在 `openbot-application`。
//! - 任何 I/O：数据库、HTTP、文件、进程 —— 在 `openbot-infra` / `openbot-computer`。
//! - 构造 `AuthContext`。`AuthContext` 只能由 Rust 从 session、连接 peer、数据库 ACL 和资源
//!   映射构造（v3 §5.3）；模型、renderer、MCP server、remote Agent、browser engine 传来的
//!   同名字段一律是普通不可信输入。
//! - 用户可见文案与本地化（CLAUDE.md §4a：文案不进 domain / application）。
//!
//! # Phase 0 状态
//!
//! 本 crate 在 Phase 0（Evidence Freeze）刻意为空：G0 只冻结证据与骨架，类型定义是 G1
//! （Rust Core 与 PostgreSQL）的产物。禁止在这里提前塞占位类型 —— 没有 parity ledger 条目
//! 背书的类型无法判定 parity / 新增 / 替代。
