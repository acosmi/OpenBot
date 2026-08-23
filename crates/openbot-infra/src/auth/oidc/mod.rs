//! OIDC 协议实现（v3 §6.2）。
//!
//! # 这个模块最重要的一条性质：它发不出请求
//!
//! 协议实现在这里，**传输实现不在这里**。出网是一个注入的端口
//! （[`transport::MetadataTransport`]），由项目自己的 safe dialer 兑现；本模块里没有
//! socket、没有 DNS、没有 HTTP 客户端，一行都没有。
//!
//! 为什么把这条放在最前面：v3 §6.2 末段要求 OIDC discovery / JWKS 与任何 IdP metadata
//! fetch「使用和 remote Agent/MCP **相同的** safe dialer、redirect/IP 校验、大小/时间上限」。
//! 「相同的」是这条约束的全部重量 —— §10.5 把 SSRF 面收口在**一个** dialer 上，前提是
//! 只有一条出网路径。协议库自带的第二条路径不会因为「我们记得不用它」而消失。
//!
//! 所以这里的构造是把纪律换成结构：**绕过 safe dialer 的代码在这个模块里写不出来**，
//! 因为没有可以写的地方。根 `Cargo.toml` 把 `openidconnect` 声明成
//! `default-features = false`（关掉它自带的 `reqwest` + `rustls-tls`）是同一条裁决的另一半。
//! 端口为什么是自己的窄 trait 而不是 `oauth2::AsyncHttpClient`，见 [`transport`] 的模块文档。
//!
//! # 模块地图
//!
//! | 模块 | 负责 | §6.2 对应 |
//! | --- | --- | --- |
//! | [`transport`] | 注入的出网端口 + 响应闸门（状态码 / 大小 / `Content-Type`） | 末段 |
//! | [`provider`] | 三家 provider 的类型化差异、issuer 闸门、注册表 | 条 1 / 2 / 9 |
//! | [`email`] | routing 用的窄 email 类型 | 条 2 |
//! | [`routing`] | email domain routing + **统一响应** | 条 2 / 末段 |
//! | [`ratelimit`] | 限速**判定**（纯函数，不含存储） | 末段 |
//! | [`preauth`] | pre-auth 面投影（只有 provider ID + 一个布尔值） | 末段 |
//! | [`redirect`] | 精确 redirect URI（登记期规范形 + 使用期逐字节） | 条 3 |
//! | [`attempt`] | 一次性的 `state` / `nonce` / PKCE S256 | 条 3 |
//! | [`authorize`] | 组装 Authorization Code 请求 | 条 3 |
//! | [`discovery`] | discovery 文档取回与 issuer 校验 | 条 3 |
//! | [`jwks`] | JWKS 缓存、轮转与**重拉限速** | 条 3 |
//! | [`claims`] | ID token 校验 + Entra 地址链 + 租户策略 | 条 3 / 5 |
//! | [`error`] | 稳定 code | v3 §15.3 |
//!
//! # 时间与随机数都不是本模块自己取的
//!
//! 每一条过期 / 冷却 / 限速判定都收一个调用方传入的 `now: OffsetDateTime`。**唯一的例外**
//! 是 ID token 的 `exp` / `iat`：那由 `openidconnect` 判，而它的时钟注入点要求一个
//! `chrono::DateTime<Utc>`，`chrono` 不在本 crate 的依赖面上。这条不对称写在 [`claims`]
//! 的模块文档里，没有藏。
//!
//! # 明确**不做**（本轮没做，也不假装做了）
//!
//! 1. **SAML**（§6.2 条 4）。`samael 0.0.22` 在本机构建失败：它经 `openssl-sys 0.9.117`
//!    要求一份 OpenSSL 安装，MSVC 目标上探测不到，另需 libxml2 + xmlsec。SAML 的**校验
//!    规则**由 domain 层实现，XML 签名验证的落地是一次独立立项（§6.2 本身也写明「SAML
//!    外审未通过时不得发布 GA」）。
//! 2. **真实的 HTTP safe dialer**。它牵涉 TLS 栈选型（`rustls` 需要 `aws-lc-rs` 或 `ring`，
//!    两者都带 C / 汇编构建脚本），按 §16.3 是一次需要独立 delta audit 的供应链决定。
//! 3. **token endpoint 交换**（`code` + `code_verifier` 换 token）。它需要 POST，而本模块的
//!    端口刻意只有 GET，见 [`authorize`] 模块文档末段。
//! 4. **凭据落库**。`sso_providers.oidc_config` 的加解密属于 vault 模块。本模块的
//!    [`OidcProviderConfig`] 里**没有** client secret 字段，secret 只以单次调用的参数形式
//!    出现在 [`claims::build_verifier`] 上。
//! 5. **`openbot_domain::identity` 的身份判定**。规范化 email、auth generation、role 与
//!    revocation 都不在这里；[`email`] 只做 routing 需要的那一半域名解析，
//!    [`claims::VerifiedIdentity::email`] 交出的还不是规范化 email。集成方向是**本模块
//!    调用领域层**，不是反过来。

pub mod attempt;
pub mod authorize;
pub mod claims;
pub mod discovery;
pub mod email;
pub mod error;
pub mod jwks;
pub mod preauth;
pub mod provider;
pub mod ratelimit;
pub mod redirect;
pub mod routing;
pub mod transport;

pub use attempt::{LoginAttempt, LoginAttemptStore, S256Pkce};
pub use authorize::authorization_url;
pub use claims::{DirectoryClaims, VerifiedIdentity, verify_id_token};
pub use discovery::{FetchBudget, discover};
pub use email::EmailDomain;
pub use error::OidcError;
pub use jwks::{JwksCache, JwksRefreshPolicy};
pub use preauth::PreAuthSurface;
pub use provider::{
    EntraTenantPolicy, IssuerTrust, OidcProviderConfig, ProviderId, ProviderKind, ProviderOrigin,
    ProviderRegistry,
};
pub use ratelimit::{RateLimitCounter, RateLimitDecision, RateLimitPolicy};
pub use redirect::CanonicalRedirectUri;
pub use routing::{EmailRoutingOutcome, UniformRoutingResponse, route_email};
pub use transport::{MetadataRequest, MetadataResponse, MetadataTransport, TransportUnavailable};
