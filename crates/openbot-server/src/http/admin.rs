//! `/api/me` 与首批 admin people HTTP framing；业务规则只在 application。

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use http::HeaderMap;
use openbot_contracts::auth::Role;
use openbot_contracts::command::{AppCommand, AppReply};
use openbot_contracts::error::AppError;
use openbot_contracts::ids::ActorId;
use openbot_contracts::people::{AdminStatus, CurrentUser, PeoplePage, Person};
use openbot_domain::text::is_ecmascript_whitespace;
use serde::{Deserialize, Serialize};

use crate::auth::{Authenticated, SensitiveAuthenticated};
use crate::error::HttpError;
use crate::http::ServerState;

/// `/api/me` 上游信封。
#[derive(Debug, Serialize)]
pub struct MeResponse {
    /// 当前用户。
    pub user: CurrentUser,
}

/// role/access 上游信封。
#[derive(Debug, Serialize)]
pub struct PersonResponse {
    /// 变化后的 person。
    pub person: Person,
}

/// people query；limit 先保留原串，按 JS `Number.parseInt(value, 10)` 的十进制前缀语义解析。
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeopleQuery {
    /// email/name 子串。
    pub search: Option<String>,
    /// opaque cursor。
    pub cursor: Option<String>,
    /// 原始页长。
    pub limit: Option<String>,
}

/// role 请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleBody {
    /// 目标角色。
    pub role: Role,
}

/// access 请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessBody {
    /// `true` 移除，`false` 恢复。
    pub revoked: bool,
}

/// `GET /api/me`。
pub async fn me(
    State(state): State<ServerState>,
    Authenticated(auth): Authenticated,
) -> Result<Json<MeResponse>, HttpError> {
    match state
        .application()
        .execute(auth, AppCommand::GetCurrentUser)
        .await?
    {
        AppReply::CurrentUser(user) => Ok(Json(MeResponse { user })),
        _ => Err(application_contract_error()),
    }
}

/// `GET /api/admin/status`。
pub async fn status(
    State(state): State<ServerState>,
    Authenticated(auth): Authenticated,
) -> Result<Json<AdminStatus>, HttpError> {
    match state
        .application()
        .execute(auth, AppCommand::AdminStatus)
        .await?
    {
        AppReply::AdminStatus(status) => Ok(Json(status)),
        _ => Err(application_contract_error()),
    }
}

/// `GET /api/admin/people`。
pub async fn people_list(
    State(state): State<ServerState>,
    Authenticated(auth): Authenticated,
    query: Result<Query<PeopleQuery>, QueryRejection>,
) -> Result<Json<PeoplePage>, HttpError> {
    let Query(query) = query.map_err(|rejection| {
        tracing::debug!(rejection = %rejection, "people query 解析失败");
        AppError::MalformedPayload { field: "query" }
    })?;
    let limit = query.limit.as_deref().and_then(parse_decimal_prefix);
    match state
        .application()
        .execute(
            auth,
            AppCommand::ListPeople {
                search: query.search,
                cursor: query.cursor,
                limit,
            },
        )
        .await?
    {
        AppReply::People(page) => Ok(Json(page)),
        _ => Err(application_contract_error()),
    }
}

/// `POST /api/admin/people/{user_id}/role`。
pub async fn people_role(
    State(state): State<ServerState>,
    SensitiveAuthenticated(resolved): SensitiveAuthenticated,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    body: Result<Json<RoleBody>, JsonRejection>,
) -> Result<Json<PersonResponse>, HttpError> {
    state
        .authorize_sensitive_write(&resolved, request_origin(&headers))
        .await?;
    let Json(body) = body.map_err(|rejection| {
        tracing::debug!(rejection = %rejection, "people role body 解析失败");
        AppError::MalformedPayload { field: "body" }
    })?;
    match state
        .application()
        .execute(
            resolved.into_context(),
            AppCommand::ChangePersonRole {
                user_id: ActorId::new(user_id),
                role: body.role,
            },
        )
        .await?
    {
        AppReply::Person(person) => Ok(Json(PersonResponse { person })),
        _ => Err(application_contract_error()),
    }
}

/// `POST /api/admin/people/{user_id}/access`。
pub async fn people_access(
    State(state): State<ServerState>,
    SensitiveAuthenticated(resolved): SensitiveAuthenticated,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    body: Result<Json<AccessBody>, JsonRejection>,
) -> Result<Json<PersonResponse>, HttpError> {
    state
        .authorize_sensitive_write(&resolved, request_origin(&headers))
        .await?;
    let Json(body) = body.map_err(|rejection| {
        tracing::debug!(rejection = %rejection, "people access body 解析失败");
        AppError::MalformedPayload { field: "body" }
    })?;
    match state
        .application()
        .execute(
            resolved.into_context(),
            AppCommand::ChangePersonAccess {
                user_id: ActorId::new(user_id),
                revoked: body.revoked,
            },
        )
        .await?
    {
        AppReply::Person(person) => Ok(Json(PersonResponse { person })),
        _ => Err(application_contract_error()),
    }
}

fn request_origin(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(http::header::ORIGIN)
        .map(|value| value.to_str().unwrap_or(""))
}

fn application_contract_error() -> HttpError {
    tracing::error!("application command 收到不匹配 reply");
    AppError::DependencyUnavailable {
        dependency: "application",
    }
    .into()
}

/// 对齐 `Number.parseInt(raw, 10)`：前导空白/正负号 + 十进制前缀；没有数字即 `None`。
fn parse_decimal_prefix(raw: &str) -> Option<i64> {
    let raw = raw.trim_start_matches(is_ecmascript_whitespace);
    let (negative, digits) = match raw.as_bytes().first() {
        Some(b'-') => (true, &raw[1..]),
        Some(b'+') => (false, &raw[1..]),
        _ => (false, raw),
    };
    let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let magnitude = digits.parse::<u128>().unwrap_or(u128::MAX);
    if negative {
        Some(if magnitude > (i64::MAX as u128) {
            i64::MIN
        } else {
            -(magnitude as i64)
        })
    } else {
        Some(i64::try_from(magnitude).unwrap_or(i64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use http::{Request, StatusCode};
    use openbot_application::{
        ChannelCursor, ChannelReader, OpenBotApplication, PeopleAdministration, PeoplePageRequest,
        PeoplePortError, PortError,
    };
    use openbot_contracts::auth::{AuthContext, Role};
    use openbot_contracts::command::ChannelSummary;
    use openbot_contracts::ids::{DeploymentId, TenantId};
    use openbot_domain::identity::session::TrustedOrigins;
    use openbot_infra::auth::config::default_session_lifetime;
    use time::OffsetDateTime;
    use tower::ServiceExt as _;

    use crate::SINGLE_USER_ACTOR_ID;
    use crate::auth::{FixedAuthResolver, SensitiveWriteSecurity, SingleUserAuthResolver};
    use crate::http::{ServerBuilder, router};

    use super::*;

    #[derive(Clone, Default)]
    struct EmptyChannels;

    #[async_trait]
    impl ChannelReader for EmptyChannels {
        async fn list_visible_channels(
            &self,
            _actor: &ActorId,
            _limit: u32,
            _cursor: Option<ChannelCursor>,
        ) -> Result<Vec<ChannelSummary>, PortError> {
            Ok(Vec::new())
        }
    }

    #[derive(Clone)]
    struct FakePeople {
        people: Arc<Mutex<Vec<Person>>>,
        pages: Arc<Mutex<Vec<PeoplePageRequest>>>,
    }

    impl FakePeople {
        fn new() -> Self {
            Self {
                people: Arc::new(Mutex::new(vec![
                    person(SINGLE_USER_ACTOR_ID, Role::Admin),
                    person("target", Role::User),
                    person("plain-user", Role::User),
                ])),
                pages: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl PeopleAdministration for FakePeople {
        async fn current_user(&self, actor: &ActorId) -> Result<CurrentUser, PeoplePortError> {
            let person = self
                .people
                .lock()
                .unwrap()
                .iter()
                .find(|person| &person.id == actor)
                .cloned()
                .ok_or(PeoplePortError::NotFound)?;
            Ok(CurrentUser {
                id: person.id,
                email: person.email,
                name: person.name,
                image: person.image,
                role: person.role,
            })
        }

        async fn list_people(
            &self,
            request: PeoplePageRequest,
        ) -> Result<PeoplePage, PeoplePortError> {
            self.pages.lock().unwrap().push(request);
            Ok(PeoplePage {
                people: self.people.lock().unwrap().clone(),
                next_cursor: None,
            })
        }

        async fn change_role(
            &self,
            _actor: &ActorId,
            subject: &ActorId,
            desired: Role,
        ) -> Result<Person, PeoplePortError> {
            let mut people = self.people.lock().unwrap();
            let person = people
                .iter_mut()
                .find(|person| &person.id == subject)
                .ok_or(PeoplePortError::NotFound)?;
            person.role = desired;
            Ok(person.clone())
        }

        async fn change_access(
            &self,
            _actor: &ActorId,
            subject: &ActorId,
            revoked: bool,
        ) -> Result<Person, PeoplePortError> {
            let mut people = self.people.lock().unwrap();
            let person = people
                .iter_mut()
                .find(|person| &person.id == subject)
                .ok_or(PeoplePortError::NotFound)?;
            person.revoked = revoked;
            Ok(person.clone())
        }
    }

    fn person(id: &str, role: Role) -> Person {
        Person {
            id: ActorId::new(id),
            email: format!("{id}@example.test"),
            name: Some(id.to_owned()),
            image: None,
            role,
            providers: vec!["test".to_owned()],
            last_signed_in_at: Some(OffsetDateTime::UNIX_EPOCH),
            revoked: false,
            configured_admin: false,
        }
    }

    fn application(people: FakePeople) -> Arc<dyn openbot_application::ApplicationService> {
        Arc::new(OpenBotApplication::new(EmptyChannels).with_people(people))
    }

    fn security() -> SensitiveWriteSecurity {
        SensitiveWriteSecurity::new(
            default_session_lifetime(),
            TrustedOrigins::from_configured(["https://app.example.test"]).unwrap(),
        )
    }

    fn single_user_router(people: FakePeople) -> Router {
        let auth = SingleUserAuthResolver::new(
            DeploymentId::new("dep-1"),
            TenantId::new("tenant-1"),
            ActorId::new(SINGLE_USER_ACTOR_ID),
            default_session_lifetime(),
        );
        router(
            ServerBuilder::new(application(people), Arc::new(auth))
                .with_sensitive_write_security(security())
                .build(),
        )
    }

    fn fixed_router(people: FakePeople, auth: AuthContext) -> Router {
        router(
            ServerBuilder::new(
                application(people),
                Arc::new(FixedAuthResolver::granting(auth)),
            )
            .with_sensitive_write_security(security())
            .build(),
        )
    }

    fn fixed_auth(actor: &str, role: Role) -> AuthContext {
        AuthContext::for_test(
            DeploymentId::new("dep-1"),
            TenantId::new("tenant-1"),
            ActorId::new(actor),
            [role],
            openbot_contracts::auth::AuthGeneration::new(1),
            false,
        )
    }

    async fn send(router: Router, request: Request<Body>) -> (StatusCode, String) {
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    fn get(path: &str) -> Request<Body> {
        Request::builder().uri(path).body(Body::empty()).unwrap()
    }

    fn post(path: &str, origin: Option<&str>, body: &str) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(path)
            .header(http::header::CONTENT_TYPE, "application/json");
        if let Some(origin) = origin {
            builder = builder.header(http::header::ORIGIN, origin);
        }
        builder.body(Body::from(body.to_owned())).unwrap()
    }

    #[test]
    fn decimal_prefix_matches_javascript_parse_int_base_ten_edges() {
        assert_eq!(parse_decimal_prefix("50"), Some(50));
        assert_eq!(parse_decimal_prefix("-7tail"), Some(-7));
        assert_eq!(parse_decimal_prefix("+7"), Some(7));
        assert_eq!(parse_decimal_prefix("1e3"), Some(1));
        assert_eq!(parse_decimal_prefix("0x10"), Some(0));
        assert_eq!(parse_decimal_prefix("abc"), None);
        assert_eq!(parse_decimal_prefix(""), None);
        assert_eq!(parse_decimal_prefix(&"9".repeat(100)), Some(i64::MAX));
        assert_eq!(parse_decimal_prefix("\u{FEFF}\u{3000}+7tail"), Some(7));
        assert_eq!(
            parse_decimal_prefix("\u{0085}7"),
            None,
            "Rust White_Space 的 U+0085 不能冒充 ECMAScript WhiteSpace",
        );
    }

    #[tokio::test]
    async fn me_status_and_people_list_match_upstream_wire_and_application_clamps_limit() {
        let people = FakePeople::new();
        let router = single_user_router(people.clone());
        let (status, body) = send(router.clone(), get("/api/me")).await;
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["user"]["id"], SINGLE_USER_ACTOR_ID);
        assert_eq!(json["user"]["role"], "admin");

        let (status, body) = send(router.clone(), get("/api/admin/status")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"status":"ok"}"#);

        let (status, body) = send(
            router,
            get("/api/admin/people?search=target&cursor=bad&limit=-7tail"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(json["people"].is_array());
        assert!(json.get("nextCursor").is_some());
        assert_eq!(
            people.pages.lock().unwrap().as_slice(),
            [PeoplePageRequest {
                search: Some("target".to_owned()),
                cursor: Some("bad".to_owned()),
                limit: 1,
            }],
        );
    }

    #[tokio::test]
    async fn admin_status_is_forbidden_for_a_plain_authenticated_user() {
        let router = fixed_router(FakePeople::new(), fixed_auth("plain-user", Role::User));
        let (status, body) = send(router, get("/api/admin/status")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap()["code"],
            "forbidden_role",
        );
    }

    #[tokio::test]
    async fn sensitive_people_write_requires_origin_and_live_fresh_session() {
        let people = FakePeople::new();
        let single = single_user_router(people.clone());
        let path = "/api/admin/people/target/role";

        let (status, body) = send(single.clone(), post(path, None, r#"{"role":"admin"}"#)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap()["code"],
            "identity_sensitive_write_origin_missing",
        );

        let (status, body) = send(
            single.clone(),
            post(
                path,
                Some("https://evil.example.test"),
                r#"{"role":"admin"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap()["code"],
            "identity_sensitive_write_origin_untrusted",
        );

        let no_assurance = fixed_router(
            people.clone(),
            fixed_auth(SINGLE_USER_ACTOR_ID, Role::Admin),
        );
        let (status, body) = send(
            no_assurance,
            post(
                path,
                Some("https://app.example.test"),
                r#"{"role":"admin"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap()["code"],
            "identity_sensitive_write_session_not_fresh",
        );

        let (status, body) = send(
            single,
            post(
                path,
                Some("https://app.example.test"),
                r#"{"role":"admin"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap()["person"]["role"],
            "admin",
        );
    }

    #[tokio::test]
    async fn access_write_and_malformed_bodies_keep_exact_http_shapes() {
        let people = FakePeople::new();
        let router = single_user_router(people);
        let path = "/api/admin/people/target/access";
        let trusted = Some("https://app.example.test");

        let (status, body) = send(router.clone(), post(path, trusted, r#"{"revoked":true}"#)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap()["person"]["revoked"],
            true,
        );

        for malformed in [
            r#"{"revoked":"yes"}"#,
            r#"{"revoked":true,"actor":"admin"}"#,
        ] {
            let (status, body) = send(router.clone(), post(path, trusted, malformed)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&body).unwrap()["code"],
                "malformed_payload",
            );
        }
    }
}
