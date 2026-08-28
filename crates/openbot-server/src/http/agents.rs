//! Remote Agent callback-token HTTP framing; authorization and mutation remain in application/infra.

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use http::header::CACHE_CONTROL;
use http::{HeaderMap, HeaderValue, StatusCode};
use openbot_contracts::agent::{AgentProfileResponse, AgentProfilesResponse, CallbackTokenIssued};
use openbot_contracts::command::{AppCommand, AppReply};
use openbot_contracts::error::AppError;
use openbot_contracts::ids::BotId;
use serde::Deserialize;

use crate::auth::{Authenticated, SensitiveAuthenticated};
use crate::error::HttpError;
use crate::http::ServerState;

/// Closed Agent roster query.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentListQuery {
    /// True selects this actor's hidden roster instead of the default roster.
    pub hidden: Option<bool>,
}

/// `GET /api/agents`.
pub async fn list_get(
    State(state): State<ServerState>,
    Authenticated(auth): Authenticated,
    query: Result<Query<AgentListQuery>, QueryRejection>,
) -> Result<(HeaderMap, Json<AgentProfilesResponse>), HttpError> {
    let Query(query) = query.map_err(|error| {
        tracing::debug!(error = %error, "agent list query rejected");
        AppError::MalformedPayload { field: "query" }
    })?;
    match state
        .application()
        .execute(
            auth,
            AppCommand::ListVisibleAgents {
                hidden: query.hidden.unwrap_or(false),
            },
        )
        .await?
    {
        AppReply::Agents(agents) => {
            Ok((no_store_headers(), Json(AgentProfilesResponse { agents })))
        }
        _ => Err(application_contract_error()),
    }
}

/// `GET /api/agents/{agent_id}`.
pub async fn get(
    State(state): State<ServerState>,
    Authenticated(auth): Authenticated,
    Path(agent_id): Path<String>,
) -> Result<(HeaderMap, Json<AgentProfileResponse>), HttpError> {
    match state
        .application()
        .execute(
            auth,
            AppCommand::GetVisibleAgent {
                agent_id: BotId::new(agent_id),
            },
        )
        .await?
    {
        AppReply::Agent(agent) => Ok((no_store_headers(), Json(AgentProfileResponse { agent }))),
        _ => Err(application_contract_error()),
    }
}

/// `POST /api/agents/{agent_id}/callback-token`; cleartext appears in this response once.
pub async fn callback_token_post(
    State(state): State<ServerState>,
    SensitiveAuthenticated(resolved): SensitiveAuthenticated,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> Result<(StatusCode, Json<CallbackTokenIssued>), HttpError> {
    state
        .authorize_fresh_origin_write(&resolved, request_origin(&headers))
        .await?;
    match state
        .application()
        .execute(
            resolved.into_context(),
            AppCommand::IssueAgentCallbackToken {
                agent_id: BotId::new(agent_id),
            },
        )
        .await?
    {
        AppReply::AgentCallbackToken(token) => Ok((StatusCode::CREATED, Json(token))),
        _ => Err(application_contract_error()),
    }
}

/// `DELETE /api/agents/{agent_id}/callback-token`.
pub async fn callback_token_delete(
    State(state): State<ServerState>,
    SensitiveAuthenticated(resolved): SensitiveAuthenticated,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> Result<StatusCode, HttpError> {
    state
        .authorize_fresh_origin_write(&resolved, request_origin(&headers))
        .await?;
    match state
        .application()
        .execute(
            resolved.into_context(),
            AppCommand::RevokeAgentCallbackToken {
                agent_id: BotId::new(agent_id),
            },
        )
        .await?
    {
        AppReply::AgentCallbackTokenRevoked(_) => Ok(StatusCode::NO_CONTENT),
        _ => Err(application_contract_error()),
    }
}

fn request_origin(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(http::header::ORIGIN)
        .map(|value| value.to_str().unwrap_or(""))
}

fn no_store_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers
}

fn application_contract_error() -> HttpError {
    tracing::error!("agent command 收到不匹配 reply");
    AppError::DependencyUnavailable {
        dependency: "application",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use http::{Method, Request};
    use openbot_application::{
        AgentCallbackTokenAdministration, AgentCallbackTokenError, AgentDirectory, AgentReadScope,
        ChannelCursor, ChannelReader, OpenBotApplication, PortError,
    };
    use openbot_contracts::agent::{
        AgentProfile, AgentVisibility, CallbackTokenIssued, CallbackTokenRevoked,
    };
    use openbot_contracts::auth::{AuthContext, AuthGeneration, Role};
    use openbot_contracts::command::ChannelSummary;
    use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};
    use openbot_domain::identity::session::{
        SessionLifetimePolicy, SessionState, TrustedOrigins, evaluate_session,
    };
    use time::{Duration, OffsetDateTime};
    use tower::ServiceExt as _;

    use super::*;
    use crate::auth::{FixedAuthResolver, ResolvedAuth, SensitiveWriteSecurity};
    use crate::http::{ServerBuilder, router};

    type AgentDirectoryCall = (AgentReadScope, Option<BotId>, bool);

    #[derive(Clone, Copy)]
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
    struct FakeAgentDirectory {
        profiles: Arc<Vec<AgentProfile>>,
        calls: Arc<Mutex<Vec<AgentDirectoryCall>>>,
        unavailable: bool,
    }

    impl FakeAgentDirectory {
        fn new() -> Self {
            Self {
                profiles: Arc::new(vec![AgentProfile {
                    id: BotId::new("agent-1"),
                    name: "Agent One".to_owned(),
                    title: "Operations".to_owned(),
                    role_description: "Handle operations".to_owned(),
                    avatar_seed: "seed".to_owned(),
                    visibility: AgentVisibility::Public,
                    endpoint: None,
                    has_auth: false,
                    has_callback_token: false,
                    hidden: false,
                    system_owned: true,
                    can_manage: false,
                    mine: false,
                }]),
                calls: Arc::new(Mutex::new(Vec::new())),
                unavailable: false,
            }
        }

        fn unavailable() -> Self {
            Self {
                unavailable: true,
                ..Self::new()
            }
        }
    }

    #[async_trait]
    impl AgentDirectory for FakeAgentDirectory {
        async fn list_visible_agents(
            &self,
            scope: &AgentReadScope,
            hidden: bool,
        ) -> Result<Vec<AgentProfile>, PortError> {
            self.calls
                .lock()
                .unwrap()
                .push((scope.clone(), None, hidden));
            if self.unavailable {
                return Err(PortError::Unavailable {
                    dependency: "database",
                });
            }
            Ok(self.profiles.as_ref().clone())
        }

        async fn get_visible_agent(
            &self,
            scope: &AgentReadScope,
            agent_id: &BotId,
        ) -> Result<Option<AgentProfile>, PortError> {
            self.calls
                .lock()
                .unwrap()
                .push((scope.clone(), Some(agent_id.clone()), false));
            if self.unavailable {
                return Err(PortError::Unavailable {
                    dependency: "database",
                });
            }
            Ok(self
                .profiles
                .iter()
                .find(|profile| &profile.id == agent_id)
                .cloned())
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Call {
        Issue(ActorId, BotId),
        Revoke(ActorId, BotId),
    }

    #[derive(Clone, Default)]
    struct FakeTokens {
        calls: Arc<Mutex<Vec<Call>>>,
    }

    #[async_trait]
    impl AgentCallbackTokenAdministration for FakeTokens {
        async fn issue(
            &self,
            auth: &AuthContext,
            agent: &BotId,
        ) -> Result<CallbackTokenIssued, AgentCallbackTokenError> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Issue(auth.actor().clone(), agent.clone()));
            CallbackTokenIssued::new("obot_agt_one-time-response".to_owned())
                .map_err(|_| AgentCallbackTokenError::Corrupt { field: "token" })
        }

        async fn revoke(
            &self,
            auth: &AuthContext,
            agent: &BotId,
        ) -> Result<CallbackTokenRevoked, AgentCallbackTokenError> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Revoke(auth.actor().clone(), agent.clone()));
            Ok(CallbackTokenRevoked)
        }
    }

    fn lifetime() -> SessionLifetimePolicy {
        SessionLifetimePolicy::new(Duration::hours(8), Duration::days(7), Duration::minutes(15))
            .unwrap()
    }

    fn app(tokens: FakeTokens, session_age: Duration) -> axum::Router {
        let generation = AuthGeneration::new(1);
        let context = AuthContext::for_test(
            DeploymentId::new("dep"),
            TenantId::new("tenant"),
            ActorId::new("owner"),
            [Role::User],
            generation,
            false,
        );
        let now = OffsetDateTime::now_utc();
        let live = evaluate_session(
            lifetime(),
            SessionState::rehydrate(now - session_age, now, generation),
            generation,
            now,
        )
        .unwrap();
        let resolver = FixedAuthResolver::granting_resolved(ResolvedAuth::from_live_session(
            context, live, None,
        ));
        router(
            ServerBuilder::new(
                Arc::new(OpenBotApplication::new(EmptyChannels).with_agent_callback_tokens(tokens)),
                Arc::new(resolver),
            )
            .with_sensitive_write_security(SensitiveWriteSecurity::new(
                lifetime(),
                TrustedOrigins::from_configured(["https://app.example.test"]).unwrap(),
            ))
            .build(),
        )
    }

    fn request(method: Method, origin: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri("/api/agents/remote-1/callback-token");
        if let Some(origin) = origin {
            builder = builder.header(http::header::ORIGIN, origin);
        }
        builder.body(Body::empty()).unwrap()
    }

    async fn send_full(
        router: axum::Router,
        request: Request<Body>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec();
        (status, headers, body)
    }

    async fn send(router: axum::Router, request: Request<Body>) -> (StatusCode, Vec<u8>) {
        let (status, _, body) = send_full(router, request).await;
        (status, body)
    }

    fn read_app(agents: FakeAgentDirectory, resolver: FixedAuthResolver) -> axum::Router {
        router(
            ServerBuilder::new(
                Arc::new(
                    OpenBotApplication::new(EmptyChannels).with_agent_directory(Arc::new(agents)),
                ),
                Arc::new(resolver),
            )
            .build(),
        )
    }

    fn read_auth() -> AuthContext {
        AuthContext::for_test(
            DeploymentId::new("dep"),
            TenantId::new("tenant"),
            ActorId::new("reader"),
            [Role::User],
            AuthGeneration::new(1),
            false,
        )
    }

    #[tokio::test]
    async fn agent_list_and_detail_use_authenticated_scope_and_exact_envelopes() {
        let agents = FakeAgentDirectory::new();
        let visible = agents.clone();
        let (status, headers, body) = send_full(
            read_app(agents, FixedAuthResolver::granting(read_auth())),
            Request::builder()
                .method(Method::GET)
                .uri("/api/agents?hidden=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[CACHE_CONTROL], "no-store");
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["agents"][0]["id"], "agent-1");
        assert_eq!(value["agents"][0]["systemOwned"], true);
        assert!(value["agents"][0].get("ownerUserId").is_none());
        assert_eq!(
            visible.calls.lock().unwrap().as_slice(),
            [(
                AgentReadScope {
                    tenant: TenantId::new("tenant"),
                    actor: ActorId::new("reader"),
                    admin: false,
                },
                None,
                true,
            )]
        );

        let (status, headers, body) = send_full(
            read_app(
                FakeAgentDirectory::new(),
                FixedAuthResolver::granting(read_auth()),
            ),
            Request::builder()
                .method(Method::GET)
                .uri("/api/agents/agent-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[CACHE_CONTROL], "no-store");
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["agent"]["name"], "Agent One");
    }

    #[tokio::test]
    async fn agent_reads_authenticate_first_reject_unknown_query_and_collapse_missing_to_404() {
        let agents = FakeAgentDirectory::new();
        let untouched = agents.clone();
        let (status, _) = send(
            read_app(
                agents,
                FixedAuthResolver::rejecting(AppError::Unauthenticated),
            ),
            Request::builder()
                .method(Method::GET)
                .uri("/api/agents?principal=admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(untouched.calls.lock().unwrap().is_empty());

        let agents = FakeAgentDirectory::new();
        let untouched = agents.clone();
        let (status, _) = send(
            read_app(agents, FixedAuthResolver::granting(read_auth())),
            Request::builder()
                .method(Method::GET)
                .uri("/api/agents?principal=admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(untouched.calls.lock().unwrap().is_empty());

        let (status, body) = send(
            read_app(
                FakeAgentDirectory::new(),
                FixedAuthResolver::granting(read_auth()),
            ),
            Request::builder()
                .method(Method::GET)
                .uri("/api/agents/missing")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"code":"not_visible"})
        );

        let (status, _) = send(
            read_app(
                FakeAgentDirectory::new(),
                FixedAuthResolver::granting(read_auth()),
            ),
            Request::builder()
                .method(Method::GET)
                .uri("/api/agents/%0A")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _) = send(
            read_app(
                FakeAgentDirectory::unavailable(),
                FixedAuthResolver::granting(read_auth()),
            ),
            Request::builder()
                .method(Method::GET)
                .uri("/api/agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn callback_token_write_requires_origin_and_freshness_before_application() {
        let tokens = FakeTokens::default();
        let (status, _) = send(
            app(tokens.clone(), Duration::minutes(1)),
            request(Method::POST, None),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = send(
            app(tokens.clone(), Duration::minutes(1)),
            request(Method::POST, Some("https://evil.example.test")),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = send(
            app(tokens.clone(), Duration::minutes(16)),
            request(Method::POST, Some("https://app.example.test")),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(tokens.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn callback_token_cleartext_is_once_and_delete_has_no_body() {
        let tokens = FakeTokens::default();
        let router = app(tokens.clone(), Duration::minutes(1));
        let (status, body) = send(
            router.clone(),
            request(Method::POST, Some("https://app.example.test")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["token"], "obot_agt_one-time-response");

        let (status, body) = send(
            router,
            request(Method::DELETE, Some("https://app.example.test")),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_empty());
        assert_eq!(
            tokens.calls.lock().unwrap().as_slice(),
            [
                Call::Issue(ActorId::new("owner"), BotId::new("remote-1")),
                Call::Revoke(ActorId::new("owner"), BotId::new("remote-1")),
            ]
        );
    }
}
