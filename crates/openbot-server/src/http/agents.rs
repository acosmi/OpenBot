//! Remote Agent callback-token HTTP framing; authorization and mutation remain in application/infra.

use axum::Json;
use axum::extract::{Path, State};
use http::{HeaderMap, StatusCode};
use openbot_contracts::agent::CallbackTokenIssued;
use openbot_contracts::command::{AppCommand, AppReply};
use openbot_contracts::error::AppError;
use openbot_contracts::ids::BotId;

use crate::auth::SensitiveAuthenticated;
use crate::error::HttpError;
use crate::http::ServerState;

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

fn application_contract_error() -> HttpError {
    tracing::error!("callback token command 收到不匹配 reply");
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
        AgentCallbackTokenAdministration, AgentCallbackTokenError, ChannelCursor, ChannelReader,
        OpenBotApplication, PortError,
    };
    use openbot_contracts::agent::{CallbackTokenIssued, CallbackTokenRevoked};
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

    async fn send(router: axum::Router, request: Request<Body>) -> (StatusCode, Vec<u8>) {
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec();
        (status, body)
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
