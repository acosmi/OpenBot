//! `openbot-infra` —— 适配器层：PostgreSQL、vault、HTTP safe dialer、provider adapters。
//!
//! # 所有权边界（v3 §5.1 / §6.4 / §10.5 / §14）
//!
//! 负责：
//!
//! - PostgreSQL 访问：连接池、SQL、migration 执行、outbox。**PostgreSQL 17 是唯一数据库语义**
//!   （v3 §14.1，不需要 pgvector）；Rust/PostgreSQL 是 thread、message、run、memory、realtime
//!   cursor、run lock 的唯一真源（v3 §4.1）。
//! - Schema 兼容期只允许 **expand**（v3 §14.3 / CLAUDE.md §4）：新表、nullable column、backfill、
//!   index、非破坏性 constraint；禁止 drop / rename / 类型收紧 / 主键改写；无 downgrade migration。
//! - vault：凭据加解密与 `CredentialPrincipalId` 绑定（v3 §6.4）。
//! - HTTP safe dialer：出网前的 DNS / IP 校验与 egress 限制（v3 §10.5），SSRF 面在这里收口。
//! - provider adapters：模型厂商 HTTP/流式协议的翻译层。
//!
//! 明确**不**负责：
//!
//! - 业务规则与 use case 编排 —— 在 `openbot-application`；本 crate 只提供 port 的实现。
//! - 领域不变量判定 —— 在 `openbot-domain`。
//! - Agent loop、AG-UI 事件语义、tool runtime、MCP 会话 —— 在 `openbot-agent`。
//! - 接受来自 transport 的任意 SQL query（v3 §5.2 逐字禁止）。
//! - 任何 phone-home 遥测端点（v3 §16.4 零 phone-home）：OTel exporter 只在管理员显式配置
//!   collector 地址时才建连。
//!
//! # Phase 0 状态
//!
//! 刻意为空。28 表 schema 与 13 条 migration 的落地是 G1 的产物（v3 §14.2 parity ledger）。
