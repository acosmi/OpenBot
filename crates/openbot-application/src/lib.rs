//! `openbot-application` —— **唯一业务入口**：所有 use case 的收口点。
//!
//! # 所有权边界（v3 §5.2 Hexagonal ownership / CLAUDE.md §4）
//!
//! 负责：
//!
//! - 暴露 typed service [`ApplicationService`]，两个方法：
//!   `execute(auth, command) -> Result<AppReply, AppError>` 与
//!   `subscribe(auth, request) -> Result<AppEventStream, AppError>`。
//! - 编排 use case：调 `openbot-domain` 做纯判定，调 `openbot-infra` / `openbot-agent` /
//!   `openbot-computer` 的 port 执行 effect，划事务边界。
//! - 工具执行管线（v3 §8.1）的顺序保证：validation → 权威 actor/target → effect 分类 →
//!   CEL + 内容策略 → 审批 → **事务写 decision + attempt** → 单次 capability → 执行 →
//!   outcome + commit_state。decision 写失败即不执行；执行了但 outcome 写不进去 →
//!   `ReconciliationRequired`，不自动重试。
//!
//! 明确**不**负责：
//!
//! - 认证本身、传输 framing、输入大小限制、错误到 HTTP/IPC 的映射 —— 那是 `openbot-server`
//!   与 `openbot-desktop` 这两个 transport 的活。
//! - 反过来：**任何 transport 都不得各自实现业务规则**（v3 §5.2）。Axum、Tauri、测试和迁移
//!   工具都只能穿过本 crate。
//! - 接受自由 method string、renderer 自报角色、renderer 自报 `principal=admin` 或任意数据库
//!   query —— 这四条在 v3 §5.2 是逐字禁止项。
//! - 用户可见文案与本地化（CLAUDE.md §4a：文案不进 domain / application）。
//!
//! # G1 状态（Rust Foundation，W5–10）
//!
//! Phase 0 本 crate 刻意为空；G1 起它承载一个垂直切片所需的全部四层：
//!
//! | 模块 | 内容 | 方案出处 |
//! | --- | --- | --- |
//! | [`service`] | [`ApplicationService`] trait 与 [`AppEventStream`] | §5.2 |
//! | [`ports`] | [`ChannelReader`] 端口与 [`PortError`]（依赖倒置的方向盘） | §5.2 |
//! | [`cursor`] | keyset 游标 [`ChannelCursor`] 的铸造与 fail-closed 解析 | §15.3 |
//! | [`use_cases`] | `list_visible_channels` 与 `health` 两个用例 | §6.5 / §28.1 R22 |
//!
//! 具体实现 [`OpenBotApplication`] 把上面四层接起来，是 transport 唯一需要构造的类型。
//!
//! # 端口在这里定义，不在 infra 定义
//!
//! 六边形架构的方向盘：`openbot-application` **定义** [`ChannelReader`]，`openbot-infra`
//! **实现**它。所以依赖箭头是 `infra -> application`，application 对数据库一无所知。
//! 反过来（application 依赖 infra 的具体类型）会让「换一个数据源」变成改业务代码，
//! 也会让本 crate 的测试必须有一个真库 —— 本 crate 的全部测试都用内存 fake，一行 SQL
//! 都不需要，这正是方向盘朝对了的可观察后果。
//!
//! # G1 里**没有**生产者的两个错误变体
//!
//! `AppError::Unauthenticated`（401）与 `AppError::ForbiddenRole`（403）在本 crate 里
//! 一次都没有被构造，这是刻意的，不是遗漏：
//!
//! - **401**：`AuthContext` 无法由外部字节铸造（contracts 里它既不 `Serialize` 也不
//!   `Deserialize`），所以「拿到了一个 `AuthContext`」本身就是「已认证」的证据。未登录
//!   请求在 transport 的认证层就被挡下，根本走不到 [`ApplicationService::execute`]。
//!   在这里再写一次 401 检查，就是给一个类型系统已经排除的世界写分支。
//! - **403**：G1 的两个用例都不要求任何角色 —— 上游 `channels/routes.ts::list` 只要
//!   session，可见性由 materialized membership 判定而不是由角色判定。凭空给 parity 路由
//!   加一道角色门，就是把「新增」写成「当前行为」（CLAUDE.md §4 / §28.1 R1）。
//!   G2 的 admin-only 路由落地时，403 才会有第一个真实生产者。
//!
//! 这两条由 `app` 模块的 `error_variants_without_producer_in_g1` 钉住，并配正向对照
//! （确实有生产者的变体必须能被本 crate 产出），免得它退化成一句注释。

// 本 crate 是 transport 与 domain 之间的唯一门，公开面即契约面：一个没有文档的公开条目
// 等于一个只有作者知道语义的契约。用 deny 而不是 warn —— warn 会被 `cargo test` 的输出
// 淹没，只有 clippy 的 `-D warnings` 拦得住，那是半道闸门。
#![deny(missing_docs)]

mod app;
pub mod cursor;
pub mod ports;
pub mod service;
pub mod use_cases;

#[cfg(test)]
mod fakes;

pub use app::OpenBotApplication;
pub use cursor::{ChannelCursor, channel_recency};
pub use ports::{ChannelReader, PortError};
pub use service::{
    APPLICATION_SPAN_FIELDS, AppEventStream, ApplicationService, EXECUTE_SPAN_NAME,
    SUBSCRIBE_SPAN_NAME, TRACE_ONLY_SPAN_FIELDS, command_kind, subscription_kind,
};
pub use use_cases::{DEFAULT_CHANNEL_PAGE, health, list_visible_channels};
