//! 端口（port）—— application **定义**，infra **实现**。
//!
//! 这是六边形架构里那根决定依赖方向的轴：依赖箭头是 `openbot-infra -> openbot-application`。
//! 本模块不 import 任何 I/O crate，也不出现任何 SQL —— 出现了就说明抽象漏了。

use async_trait::async_trait;
use openbot_contracts::audit::AuditPage;
use openbot_contracts::auth::Role;
use openbot_contracts::command::ChannelSummary;
use openbot_contracts::error::{AppError, IdentityConflictReason};
use openbot_contracts::ids::ActorId;
use openbot_contracts::people::{CurrentUser, PeoplePage, Person};
use time::OffsetDateTime;

use crate::cursor::ChannelCursor;

/// 端口失败。**恰两类**，因为 application 对它们只有两种正确反应。
///
/// 刻意全是 `&'static str` 字段：数据库返回的原值是不可信数据，把它塞进错误就是日志
/// 注入面，也会让错误文本变成事实上的用户文案（CLAUDE.md §4a 禁止）。需要细节的地方
/// 由实现侧自己 `tracing::error!` 记录，那是受控 trace。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PortError {
    /// 依赖此刻不能服务：连不上、池满、超时、被拒。
    #[error("port_unavailable dependency={dependency}")]
    Unavailable {
        /// 依赖的静态名，与 `AppError::DependencyUnavailable::dependency` 同域。
        dependency: &'static str,
    },

    /// 依赖回了东西，但那东西解不开：列类型不符、枚举取值不在域内、NOT NULL 却是 NULL。
    #[error("port_corrupt dependency={dependency} field={field}")]
    Corrupt {
        /// 依赖的静态名。
        dependency: &'static str,
        /// 解不开的**字段名**（不是值）。
        field: &'static str,
    },
}

impl PortError {
    /// 映射成 `AppError`（§15.3）。
    ///
    /// # 两类都落 `DependencyUnavailable`（503），这是一次有意的裁决
    ///
    /// `Unavailable` 落 503 没有争议。`Corrupt` 有两个候选，选择理由如下：
    ///
    /// - **不能落 400**。`MalformedPayload` 说的是「调用方送来的载荷坏了」，而这里坏的是
    ///   我们自己的存储。用 400 回一次服务端缺陷，等于让客户端去修一个它改不动的东西，
    ///   而且会污染「malformed payload 不产生 acting decision」这条判据的统计口径。
    /// - **不落 502 `VendorFailure`**，尽管 HTTP 语义上 502（上游回了无效响应）比 503
    ///   更贴。理由是稳定码是契约：§15.3 把 `vendor_failure` 绑定在「上游 vendor」上，
    ///   contracts 的类型文档也这么写。把自家 PostgreSQL 记成 vendor 失败，会让运维在
    ///   排查 provider 抖动时捞到一堆数据库故障 —— audit 侧更明显，`AuditKind::VendorFailure`
    ///   与 `AuditKind::DependencyFailure` 是两个不同的查询桶。**扭曲一个既有稳定码的含义，
    ///   比状态码差一格更贵。**
    ///
    /// 代价要说清楚：503 隐含「稍后重试可能好」，而损坏的行不会自愈。这条差异记在这里
    /// 供主控复核；真要区分，正确做法是给 `AppError` 加一个变体（那是改 contracts 的
    /// 独立决定，不能由本 crate 顺手做）。
    #[must_use]
    pub const fn into_app_error(self) -> AppError {
        match self {
            Self::Unavailable { dependency } | Self::Corrupt { dependency, .. } => {
                AppError::DependencyUnavailable { dependency }
            }
        }
    }
}

/// 读取「当前 actor 可见的 channel」。
///
/// # 可见性判据只有一个：materialized membership（§6.5 条 5 / §28.1 R22）
///
/// 实现**必须**只 join `channel_memberships`，**绝不**再 join
/// `intelligence_channel_mappings`。两条独立理由，缺任一条这个 join 都还是错的：
///
/// 1. **它会静默撤销 §6.5 的修复**。Intelligence 已按 §4.1 退役，
///    `intelligence_channel_mappings` 按 §14.2 降级为只读 legacy provenance —— 不会再有
///    新的 mapping 行。于是 §6.5 刚给包 channel 补上的 membership，会被这个 join 原样
///    过滤回「对所有人不可达」，回到上游 issue #82 的状态。
/// 2. **上游两段判据不一致本身就是缺陷**：`channels/routes.ts::list` 的分页段只 join
///    membership，hydration 段额外 join mapping，于是 `nextCursor` 可以非空而本页为空 ——
///    客户端看到「还有下一页」，翻过去却什么都没有。
///
/// 推论，请在实现与复核时一起守：**分页与 hydration 必须共用同一个可见性判据**。
/// 这个 port 的存在形式本身就在兑现它 —— 它只有一次调用，一次判据，返回的行直接就是
/// 应答的行，application 侧不做任何二次过滤（由
/// `rows_are_never_post_filtered_after_the_visibility_query` 钉住）。这也是 G1 最容易被
/// 后人「顺手改回去」的地方：任何在这条链路上引入第二次过滤的改动都必须先推翻上面两条。
///
/// # `limit` 的语义：调用方已经把 `+1` 算好了
///
/// 传进来的 `limit` 就是**要读的行数**，实现照读即可，不要再自己加一。
/// application 侧按上游 `channels/routes.ts::list` 的写法多要一行来探测「还有没有下一页」，
/// 避免第二次 `count(*)` 查询；探测与截断都留在 application（见
/// [`crate::use_cases::list_visible_channels`]），port 不必知道这个技巧。
///
/// # 排序与游标
///
/// 返回的行必须已经按 `coalesce(last_message_at, created_at) DESC, id DESC` 排好序；
/// `cursor` 为 `Some` 时只返回严格小于该二元组的行：
/// `(coalesce(last_message_at, created_at), id) < (cursor.recency, cursor.id)`。
/// application **不会**重排 —— 它没有能力在时间戳相同的行之间复现数据库的定序。
///
/// # scope
///
/// `actor` 是权威身份，来自 `AuthContext`，**不是**调用方传来的过滤条件。
/// 租户维度由 membership 投影本身承载（一条 membership 行只属于一个租户的 channel）；
/// 多租户 Server 的显式 tenant 传参是 G2 的工作，见交付报告里的遗留项。
#[async_trait]
pub trait ChannelReader: Send + Sync {
    /// 列出 actor 通过 **materialized membership** 可见的 channel。
    ///
    /// # Errors
    ///
    /// 依赖不可用或返回了解不开的行时返回 [`PortError`]；**空结果不是错误**，
    /// 返回空 `Vec`（§15.3 末条）。
    async fn list_visible_channels(
        &self,
        actor: &ActorId,
        limit: u32,
        cursor: Option<ChannelCursor>,
    ) -> Result<Vec<ChannelSummary>, PortError>;
}

/// 管理员审计页的 typed 查询；自由 SQL/列名无法穿越此边界。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditPageRequest {
    /// keyset cursor。
    pub cursor: Option<String>,
    /// 零个、一个或多个 event type。
    pub event_types: Vec<String>,
    /// actor 过滤。
    pub actor_user_id: Option<ActorId>,
    /// target type 过滤。
    pub target_type: Option<String>,
    /// target id 过滤。
    pub target_id: Option<String>,
    /// created_at 开区间下界。
    pub from: Option<OffsetDateTime>,
    /// created_at 开区间上界。
    pub to: Option<OffsetDateTime>,
    /// application 已钳制到 1..=100。
    pub limit: u32,
}

/// 审计读端口错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AuditReadError {
    /// PostgreSQL 不可用。
    #[error("audit_reader_unavailable")]
    Unavailable,
    /// 持久化行违反 schema/domain 边界。
    #[error("audit_reader_corrupt field={field}")]
    Corrupt {
        /// 静态字段名，不带数据库原值。
        field: &'static str,
    },
    /// opaque cursor 无法验证。
    #[error("audit_cursor_invalid")]
    InvalidCursor,
}

impl AuditReadError {
    /// 映射为稳定 application 错误。
    #[must_use]
    pub const fn into_app_error(self) -> AppError {
        match self {
            Self::Unavailable | Self::Corrupt { .. } => AppError::DependencyUnavailable {
                dependency: "database",
            },
            Self::InvalidCursor => AppError::MalformedPayload { field: "cursor" },
        }
    }
}

/// 管理员 audit keyset 读端口。
#[async_trait]
pub trait AuditReader: Send + Sync {
    /// 读取一页已脱敏事件。
    async fn list_audit_events(
        &self,
        request: AuditPageRequest,
    ) -> Result<AuditPage, AuditReadError>;
}

/// 未注入 audit reader 时 fail-closed 503。
#[derive(Clone, Copy, Debug, Default)]
pub struct NoAuditReader;

#[async_trait]
impl AuditReader for NoAuditReader {
    async fn list_audit_events(
        &self,
        _request: AuditPageRequest,
    ) -> Result<AuditPage, AuditReadError> {
        Err(AuditReadError::Unavailable)
    }
}

/// people 页端口的 keyset 请求；字段均由 typed command 投影，不含自由 SQL。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeoplePageRequest {
    /// email/name 子串；`None` = 不搜索。
    pub search: Option<String>,
    /// opaque cursor。
    pub cursor: Option<String>,
    /// 已由 application 钳制到 1..=200。
    pub limit: u32,
}

/// people role/access 原子端口错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PeoplePortError {
    /// 数据库/审计依赖不可用。
    #[error("people_port_unavailable")]
    Unavailable,
    /// 权威数据损坏。
    #[error("people_port_corrupt field={field}")]
    Corrupt {
        /// 静态字段名。
        field: &'static str,
    },
    /// subject 不存在；对外统一 404 防枚举。
    #[error("people_not_found")]
    NotFound,
    /// domain floor/self/last-admin 拒绝。
    #[error("people_identity_conflict reason={reason}")]
    IdentityConflict {
        /// 稳定拒绝原因。
        reason: IdentityConflictReason,
    },
}

impl PeoplePortError {
    /// 映射 §15.3 AppError。
    #[must_use]
    pub const fn into_app_error(self) -> AppError {
        match self {
            Self::Unavailable | Self::Corrupt { .. } => AppError::DependencyUnavailable {
                dependency: "database",
            },
            Self::NotFound => AppError::NotVisible,
            Self::IdentityConflict { reason } => AppError::IdentityConflict { reason },
        }
    }
}

/// people/auth application 端口。
///
/// role/access 方法必须在实现侧同一事务读取 subject、其他有效 admin 与 auth generation，调用
/// domain 判定，写角色/deny/session/generation，并追加 audit；拆成多个端口调用会产生竞态。
#[async_trait]
pub trait PeopleAdministration: Send + Sync {
    /// 当前 actor 的公开 `/api/me` 投影。
    async fn current_user(&self, actor: &ActorId) -> Result<CurrentUser, PeoplePortError>;

    /// 管理员 people 页。
    async fn list_people(&self, request: PeoplePageRequest) -> Result<PeoplePage, PeoplePortError>;

    /// 原子角色变更。
    async fn change_role(
        &self,
        actor: &ActorId,
        subject: &ActorId,
        desired: Role,
    ) -> Result<Person, PeoplePortError>;

    /// 原子访问移除/恢复。
    async fn change_access(
        &self,
        actor: &ActorId,
        subject: &ActorId,
        revoked: bool,
    ) -> Result<Person, PeoplePortError>;
}

/// 未注入 people 适配器时的构造性 503，供现有 G1 宿主平滑升级。
#[derive(Clone, Copy, Debug, Default)]
pub struct NoPeopleAdministration;

#[async_trait]
impl PeopleAdministration for NoPeopleAdministration {
    async fn current_user(&self, _actor: &ActorId) -> Result<CurrentUser, PeoplePortError> {
        Err(PeoplePortError::Unavailable)
    }

    async fn list_people(
        &self,
        _request: PeoplePageRequest,
    ) -> Result<PeoplePage, PeoplePortError> {
        Err(PeoplePortError::Unavailable)
    }

    async fn change_role(
        &self,
        _actor: &ActorId,
        _subject: &ActorId,
        _desired: Role,
    ) -> Result<Person, PeoplePortError> {
        Err(PeoplePortError::Unavailable)
    }

    async fn change_access(
        &self,
        _actor: &ActorId,
        _subject: &ActorId,
        _revoked: bool,
    ) -> Result<Person, PeoplePortError> {
        Err(PeoplePortError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openbot_contracts::error::{AuditKind, ErrorCode};

    #[test]
    fn both_port_failures_map_to_dependency_unavailable_503() {
        let unavailable = PortError::Unavailable {
            dependency: "database",
        }
        .into_app_error();
        assert_eq!(unavailable.code(), ErrorCode::DEPENDENCY_UNAVAILABLE);
        assert_eq!(unavailable.http_status(), 503);
        assert_eq!(unavailable.audit_kind(), AuditKind::DependencyFailure);

        let corrupt = PortError::Corrupt {
            dependency: "database",
            field: "last_message_at",
        }
        .into_app_error();
        assert_eq!(corrupt.code(), ErrorCode::DEPENDENCY_UNAVAILABLE);
        assert_eq!(corrupt.http_status(), 503);
        assert_eq!(corrupt.audit_kind(), AuditKind::DependencyFailure);
    }

    /// 负向对照：损坏的行**不会**被映射成「调用方的锅」。
    ///
    /// 正向对照在同一条里 —— 400 这个码确实存在且能被别的路径产出（游标解析），
    /// 所以本断言不是在「没有 400 这个东西」的世界里成立的。
    #[test]
    fn corrupt_row_is_never_blamed_on_the_caller() {
        let corrupt = PortError::Corrupt {
            dependency: "database",
            field: "role",
        }
        .into_app_error();
        assert_ne!(corrupt.code(), ErrorCode::MALFORMED_PAYLOAD);
        assert_ne!(corrupt.http_status(), 400);

        let caller_fault = crate::cursor::ChannelCursor::decode("!!!").unwrap_err();
        assert_eq!(caller_fault.code(), ErrorCode::MALFORMED_PAYLOAD);
        assert_eq!(caller_fault.http_status(), 400);
    }

    /// 依赖名原样穿过映射，不被改写 —— 运维靠它区分是数据库还是别的依赖。
    #[test]
    fn dependency_name_survives_the_mapping() {
        let err = PortError::Unavailable {
            dependency: "browser_engine",
        }
        .into_app_error();
        assert_eq!(
            err,
            AppError::DependencyUnavailable {
                dependency: "browser_engine"
            }
        );
    }
}
