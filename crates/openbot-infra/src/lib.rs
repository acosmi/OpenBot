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
//! - **读环境变量**：连接参数一律由调用方以 [`db::pool::DatabaseConfig`] 显式传入。env 的三档
//!   裁决（v3 §15.4）属于启动 / transport 层，不在本 crate。
//!
//! # G1 状态（Rust Foundation，W5–10）
//!
//! 已落地的是数据库 schema 层，也就是 v3 §24 G1 判据「28 表/13 migration 映射」的执行面：
//!
//! - [`db::tables`] —— 上游 28 张表的类型化行结构，列名 / 列序 / 可空性 / 类型逐列对应上游
//!   第 13 条 migration（`0012`）跑完之后的终态。
//! - [`db::baseline`] —— fresh install 的 baseline DDL（v3 §14.1），把空库直接建成 0012 终态；
//!   不创建 `0010` / `0011` 已删除的 document / connector 表，对 extension 零操作。
//! - [`db::compat`] —— 迁移边界检查（v3 §14.1「Rust 不接收更早 schema」）。
//! - [`db::schema_facts`] —— schema 事实提取，与 `fixtures/db/schema-0012.json` 同构。
//! - [`db::pool`] —— `deadpool-postgres` 连接池。
//! - [`repo::channels::ChannelRepo`] —— 首个 vertical slice 的读侧：
//!   `openbot_application::ChannelReader` 的 PostgreSQL 实现。
//!
//! 尚未落地，也不在此假装存在：outbox、vault、safe dialer、provider adapters，以及
//! `ChannelReader` 之外的各个 port。
//! `parity/tables.yaml` 里 12 张 native 新表与 `db::migrations::*` 的 13 条 migration 台账落点
//! 同样仍是 `status: todo`。

pub mod auth;
pub mod db;
pub mod repo;
