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
//! # 当前状态（G1 + people/tool + W-5 audit read boundary）
//!
//! Phase 0 本 crate 刻意为空；G1 起它承载一个垂直切片所需的全部四层：
//!
//! | 模块 | 内容 | 方案出处 |
//! | --- | --- | --- |
//! | [`service`] | [`ApplicationService`] trait 与 [`AppEventStream`] | §5.2 |
//! | [`ports`] | channel / people / audit typed ports | §5.2 / §6.2 / §8.6 / R40 / R56 |
//! | [`cursor`] | keyset 游标 [`ChannelCursor`] 的铸造与 fail-closed 解析 | §15.3 |
//! | [`use_cases`] | health/channel、people role/access 与 admin audit keyset 用例 | §6.2 / §6.5 / §8.6 / R56 |
//! | [`tool`] | metadata→scope→policy→approval→journal→capability→execute→outcome/audit | §8.1 / R41 |
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
//! # 401 留在认证 transport，403 已由 application 生产
//!
//! `AppError::Unauthenticated`（401）在本 crate 里没有生产者，而
//! `AppError::ForbiddenRole`（403）从 W-3a 起有 admin people、W-5 起有 admin audit 真实生产者：
//!
//! - **401**：`AuthContext` 无法由外部字节铸造（contracts 里它既不 `Serialize` 也不
//!   `Deserialize`），所以「拿到了一个 `AuthContext`」本身就是「已认证」的证据。未登录
//!   请求在 transport 的认证层就被挡下，根本走不到 [`ApplicationService::execute`]。
//!   在这里再写一次 401 检查，就是给一个类型系统已经排除的世界写分支。
//! - **403**：health 与 channel list 仍不要求角色；admin status/people/audit 在调用各自 port
//!   之前统一检查权威 `AuthContext` 的 `Role::Admin`。所以同一个 roleless actor
//!   对前两类成功、对后一类得到稳定 `forbidden_role`，不是 transport 自己复制一道门。
//!
//! `app::a_roleless_authenticated_actor_is_not_rejected` 把两侧一起钉住。W-3b 已把通用 tool
//! boundary 接到 Agent gateway/PostgreSQL journal；真实 browser/MCP 等 executor 仍属 G4。

// 本 crate 是 transport 与 domain 之间的唯一门，公开面即契约面：一个没有文档的公开条目
// 等于一个只有作者知道语义的契约。用 deny 而不是 warn —— warn 会被 `cargo test` 的输出
// 淹没，只有 clippy 的 `-D warnings` 拦得住，那是半道闸门。
#![deny(missing_docs)]

mod app;
pub mod cursor;
pub mod ports;
pub mod service;
pub mod tool;
pub mod use_cases;

#[cfg(test)]
mod fakes;

pub use app::OpenBotApplication;
pub use cursor::{ChannelCursor, channel_recency};
pub use ports::{
    AuditPageRequest, AuditReadError, AuditReader, ChannelReader, NoAuditReader,
    NoPeopleAdministration, PeopleAdministration, PeoplePageRequest, PeoplePortError, PortError,
};
pub use service::{
    APPLICATION_SPAN_FIELDS, AppEventStream, ApplicationService, EXECUTE_SPAN_NAME,
    SUBSCRIBE_SPAN_NAME, TRACE_ONLY_SPAN_FIELDS, command_kind, subscription_kind,
};
pub use tool::{
    AuthorizedToolCall, ExecutableToolCall, NoToolControlPlane, NoToolJournal, ResolvedToolScope,
    ToolApprovalRequest, ToolAuditDraftError, ToolControlPlane, ToolDecisionDraft,
    ToolExecutionReport, ToolJournal, ToolOutcomeDraft, ToolPolicyEvaluation, ToolPortError,
    ToolRefusalDraft, invoke_tool,
};
pub use use_cases::{
    DEFAULT_AUDIT_PAGE, DEFAULT_CHANNEL_PAGE, DEFAULT_PEOPLE_PAGE, MAX_AUDIT_PAGE, MAX_PEOPLE_PAGE,
    admin_status, change_person_access, change_person_role, current_user, health,
    list_audit_events, list_people, list_visible_channels,
};
