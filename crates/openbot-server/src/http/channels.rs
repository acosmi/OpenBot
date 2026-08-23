//! `GET /api/channels` —— parity ledger `api-channels-list-get`。
//!
//! 台账原文把落点钉成 `openbot-server::http::channels::list (GET /api/channels)`，
//! `migration_rule: preserve`，`notes` 里列的错误码是「401 未登录 / 404 不可见 / 400
//! malformed」。本模块逐字兑现那一行。
//!
//! # 这个 handler 里没有一条业务判定
//!
//! v3 §5.2 逐字：transport 只做认证、framing、输入大小限制和错误映射。对照着看它做了什么：
//!
//! | 动作 | 归属 |
//! | --- | --- |
//! | 解析 `?limit=&cursor=` | framing |
//! | 拒绝畸形查询串 | framing（400） |
//! | 把身份换成 `AuthContext` | 认证（由 [`Authenticated`] 提取器完成） |
//! | 组装 `AppCommand::ListVisibleChannels` | framing |
//! | 序列化 `ChannelPage` | framing |
//! | 把 `AppError` 投影成状态码 + 稳定码 | 错误映射 |
//!
//! **可见性、分页、`limit` 钳制、游标解析全在 application**，这里一行都没有。三件具体的事
//! 由测试钉住：
//!
//! - `out_of_range_limit_is_clamped_by_the_application_not_the_transport` —— `?limit=999999`
//!   会**原样**变成 `AppCommand::ListVisibleChannels { limit: Some(999_999) }`。transport
//!   自己钳到 200 看起来无害，实际是它在替 application 决定分页上限；等哪天上限改了，
//!   就有两个真源。
//! - `valid_cursor_round_trips_through_the_transport_untouched` —— 游标是**不透明字符串**，
//!   transport 不解析、不校验、不重编码。一旦 transport 开始解析它，游标格式就变成公开
//!   契约，之后换排序键会成为破坏性变更（`openbot_contracts::command` 的字段文档逐字写着
//!   这条）。
//! - `tampered_cursor_is_four_hundred_with_a_stable_code` —— 坏游标由 application 判 400，
//!   transport 只负责把它渲染出去。
//!
//! # 响应体没有信封
//!
//! 顶层就是 `ChannelPage` 本身：`{"channels":[…],"nextCursor":…}`。它已经是 camelCase，
//! 与上游 `channelSummaryDto` 逐键对齐（v3 §15.1 把 `/api/channels` 的 input/output schema
//! 纳入 parity 面，所以字段名是契约不是风格）。再包一层 `{"data":…}` 会立刻破 parity。

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use openbot_contracts::command::{AppCommand, AppReply, ChannelPage};
use openbot_contracts::error::AppError;
use serde::Deserialize;

use crate::auth::Authenticated;
use crate::error::HttpError;
use crate::http::ServerState;

/// 查询串解析失败时报给调用方的静态字段名。
///
/// 不是 `"limit"` 也不是 `"cursor"`：`QueryRejection` 不告诉我们是哪个键坏了，而
/// `AppError::MalformedPayload::field` 是 `&'static str`（contracts 用类型堵死"把用户输入
/// 回显进错误"这条路）。猜一个具体键名比给一个诚实的粗粒度名字更糟 —— 它会让客户端去改
/// 一个根本没错的参数。
const QUERY_FIELD: &str = "query";

/// `GET /api/channels` 的查询串。
///
/// # `deny_unknown_fields` 是**相对上游的刻意收紧**
///
/// 上游用 zod 解析 query，未声明的键被静默丢掉。这里改成当场 400，理由与
/// `openbot_contracts::command::AppCommand` 上那条 `deny_unknown_fields` 逐字相同：
/// 静默忽略未知字段等于允许调用方以为自己传了个参数而实际没有 —— 那是一类特别难查的
/// 行为分歧。§5.2 那条「不得接受 renderer 自报 `principal=admin`」在查询串上同样成立：
/// `?principal=admin` 应当是 400，而不是被无声吞掉之后让人以为它"生效了但没用"。
///
/// **这是一次行为变更，不是 parity。** 记在这里供主控复核；如果它挡住了真实客户端，
/// 正确的回退是给那个客户端一条 ledger 条目，而不是改回静默忽略。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListChannelsQuery {
    /// 本页最多返回多少条。`None` = 由 application 取默认值；超过
    /// `openbot_contracts::command::MAX_CHANNEL_PAGE` 由 **application** 截断。
    ///
    /// 类型是 `Option<u32>`：负数与非数字在**反序列化阶段**就被拒成 400，压根到不了
    /// application（上游那句 `Math.max(…, 1)` 里防负数的那一半，在 Rust 侧由类型承担）。
    pub limit: Option<u32>,
    /// keyset 游标，**不透明字符串**，原样透传。
    pub cursor: Option<String>,
}

/// 列出当前 actor 可见的 channel。
///
/// # Errors
///
/// - 未认证 → 401（由 [`Authenticated`] 经 [`crate::auth::AuthResolver`] 产出）。
/// - 查询串畸形 → 400 `malformed_payload`。
/// - 游标解不开 → 400 `malformed_payload`（判定在 application，见模块文档）。
/// - 依赖不可用 → 503 `dependency_unavailable`。
///
/// **空结果不是错误**：200 + `{"channels":[],"nextCursor":null}`（§15.3 末条）。
pub async fn list(
    State(state): State<ServerState>,
    Authenticated(auth): Authenticated,
    query: Result<Query<ListChannelsQuery>, QueryRejection>,
) -> Result<Json<ChannelPage>, HttpError> {
    let Query(query) = query.map_err(|rejection| {
        // rejection 的文案带着 serde 的内部细节（"Failed to deserialize query string…"），
        // 那是**日志**内容，不是响应体内容。
        tracing::debug!(rejection = %rejection, "查询串解析失败");
        AppError::MalformedPayload { field: QUERY_FIELD }
    })?;

    let reply = state
        .application()
        .execute(
            auth,
            AppCommand::ListVisibleChannels {
                limit: query.limit,
                cursor: query.cursor,
            },
        )
        .await?;

    // 穷举 match 无通配：`AppReply` 新增变体会在这里编译失败，逼作者当场决定这条路由
    // 拿到它该怎么办，而不是让它落进一个静默的 `_ =>` 分支。
    match reply {
        AppReply::Channels(page) => Ok(Json(page)),
        // 走到这里说明 application 拿 `ListVisibleChannels` 回了个探活应答 —— 契约破了。
        // 不 `unreachable!()`：一条不该发生的路径该以可诊断的失败收场，而不是把整个
        // 进程打死；也不伪装成 200 空列表，那会把一次契约破损洗成"没有数据"。
        AppReply::Health(_) => {
            tracing::error!("ListVisibleChannels 收到 Health 应答 —— ApplicationService 契约破损");
            Err(AppError::DependencyUnavailable {
                dependency: "application",
            }
            .into())
        }
    }
}
