//! 认证边界 —— [`AuthResolver`] port 与它在 Axum 侧的提取器 [`Authenticated`]。
//!
//! # 为什么 G1 的认证是一个 port，而不是一份实现
//!
//! `openbot_contracts::auth::AuthContext` **刻意既不 `Serialize` 也不 `Deserialize`**
//! （§5.3）：只要它能被反序列化，任何 transport 都可以拿 renderer / 模型 / MCP server /
//! remote Agent 送来的字节直接铸造一个身份。生产构造入口因此只有一个 ——
//! `AuthContextBuilder::from_verified_session`，而它的名字本身就是一句断言：调用点必须
//! 能指出 session、连接 peer、数据库 ACL 三者各自的来源。
//!
//! 真正的 OIDC / SAML / session 是 **G2**（§24 G2「OIDC/SAML/session/role/group/revoke
//! 全矩阵」），不在 G1 范围内。于是本模块定义**边界**而不是实现：transport 知道"要有人
//! 把请求的认证材料换成权威身份"，但不知道也不关心那件事怎么做。
//!
//! # 本 crate 不提供任何"默认放行"的实现
//!
//! 这是刻意的，不是"还没写"。一个默认可用的 `AuthResolver` 会在生产里变成后门：它一旦
//! 存在，接线层忘记注入真实实现就不会编译失败，只会静默地把每个请求当成合法用户。
//! 所以 [`ServerBuilder::new`](crate::http::ServerBuilder::new) 强制传入一个
//! `Arc<dyn AuthResolver>` —— 拿不出实现的宿主根本组装不出 router。
//!
//! [`FixedAuthResolver`] 是唯一的例外，且它被 `#[cfg(any(test, feature = "testkit"))]`
//! 挡在默认 feature 图之外（`testkit` 默认关）。它也不提供 `Default`：身份必须由调用方
//! 显式交出来，写不出 `FixedAuthResolver::default()` 这种"从哪来的身份？"的代码。
//!
//! # G2 落地时，生产实现接在哪一层
//!
//! **不在本 crate**。链路是：
//!
//! ```text
//! openbot-server（本 crate）      定义 AuthResolver port
//!         ↑ 实现
//! 接线层 / Server 二进制           把 session store + IdP + DB ACL 组装成一个实现，
//!                                 内部调用 AuthContextBuilder::from_verified_session
//!         ↓ 注入
//! ServerBuilder::new(app, auth)   router 拿到 Arc<dyn AuthResolver>
//! ```
//!
//! 依赖方向与 `openbot-application` 的 `ChannelReader` 完全一致：**上层定义 port，
//! 外围实现它**。所以 G2 加认证不需要改本模块一行 —— 那正是这条边界存在的意义。
//! 具体实现落 `openbot-infra`（session / ACL 查询）还是一个新的认证 module，是 G2 的
//! 立项决定，本 crate 不预设。

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use http::request::Parts;
use openbot_contracts::auth::AuthContext;
use openbot_contracts::error::AppError;
use tracing::Span;

use crate::error::HttpError;
use crate::http::ServerState;
use crate::telemetry::ACTOR_ID_FIELD;

/// 把一次请求的认证材料解析成权威身份。
///
/// 实现必须只依据**服务端可验证**的东西：session cookie 对应的 session 行、连接 peer、
/// 数据库 ACL。请求头里自称的角色、`principal`、租户一律是普通不可信输入（§5.3）。
///
/// 入参是 `&Parts` 而不是整个 `Request`：认证只看头部与扩展，看不到 body。这条限制是
/// 构造性的 —— 一个拿不到 body 的实现不可能"从请求体里读身份"。
#[async_trait]
pub trait AuthResolver: Send + Sync {
    /// 从请求的认证材料解析出权威身份。
    ///
    /// # Errors
    ///
    /// 无凭据 / 凭据无效 / session 已失效 → [`AppError::Unauthenticated`]（401）。
    /// 依赖（session store、目录服务）不可用 → [`AppError::DependencyUnavailable`]（503）；
    /// **不得**在依赖不可用时放行，也不得把它伪装成 401 —— 前者是后门，后者会让运维
    /// 在一堆"用户登录失败"里找不到真正的故障。
    async fn resolve(&self, parts: &Parts) -> Result<AuthContext, AppError>;
}

/// 已认证请求的提取器。
///
/// 把认证做成**提取器**而不是 handler 里的一行调用，是为了让"这条路由要不要认证"
/// 出现在 handler 的签名里：`list(_, Authenticated(auth), _)` 一眼可见，
/// 而漏写一行 `let auth = resolve(...)` 不会有任何东西提醒你。
///
/// 它同时是 §16.4「只取需要的 ID 字段」的落点：解析成功后只把 `actor_id` 记进当前 span，
/// **绝不**把 `AuthContext` 整体（角色集合、auth generation）交给 tracing。
pub struct Authenticated(pub AuthContext);

impl FromRequestParts<ServerState> for Authenticated {
    type Rejection = HttpError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        let auth = state.auth_resolver().resolve(parts).await?;
        // 只记 ID，不记上下文本体。`AuthContext` 没有 `Serialize`，但它**有** `Debug`，
        // 而 `Debug` 会把角色集合与 auth generation 一起打出来 —— 那是那道防线上的缺口。
        // 这里用 `Display` 只投影一个 ID 字段，把缺口堵上。
        Span::current().record(ACTOR_ID_FIELD, tracing::field::display(auth.actor()));
        Ok(Self(auth))
    }
}

/// 固定身份 / 固定拒绝的 [`AuthResolver`]，**只在测试与 `testkit` feature 下存在**。
///
/// 它没有 `Default`，也没有 `new()`：两个构造器 [`Self::granting`] 与 [`Self::rejecting`]
/// 都要求调用方明确说出"放行成谁"或"以什么理由拒绝"。
#[cfg(any(test, feature = "testkit"))]
pub struct FixedAuthResolver {
    outcome: Result<AuthContext, AppError>,
}

#[cfg(any(test, feature = "testkit"))]
impl FixedAuthResolver {
    /// 恒定放行成给定身份。
    #[must_use]
    pub const fn granting(auth: AuthContext) -> Self {
        Self { outcome: Ok(auth) }
    }

    /// 恒定拒绝，理由由调用方给出。
    #[must_use]
    pub const fn rejecting(error: AppError) -> Self {
        Self {
            outcome: Err(error),
        }
    }
}

#[cfg(any(test, feature = "testkit"))]
#[async_trait]
impl AuthResolver for FixedAuthResolver {
    async fn resolve(&self, _parts: &Parts) -> Result<AuthContext, AppError> {
        self.outcome.clone()
    }
}
