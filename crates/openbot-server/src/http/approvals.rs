//! Human tool-approval HTTP framing; all binding/decision rules stay behind typed ApplicationService.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderMap, HeaderValue};
use openbot_contracts::command::{AppCommand, AppReply};
use openbot_contracts::error::AppError;
use openbot_contracts::tool::{PendingToolApprovals, ToolApprovalDecision, ToolApprovalResolved};
use serde::Deserialize;

use crate::auth::{Authenticated, SensitiveAuthenticated};
use crate::error::HttpError;
use crate::http::ServerState;

/// `GET /api/tool-approvals`; current actor only, no-store.
pub async fn pending_get(
    State(state): State<ServerState>,
    Authenticated(auth): Authenticated,
) -> Result<(HeaderMap, Json<PendingToolApprovals>), HttpError> {
    let approvals = match state
        .application()
        .execute(auth, AppCommand::ListPendingToolApprovals)
        .await?
    {
        AppReply::PendingToolApprovals(approvals) => approvals,
        _ => return Err(application_contract_error()),
    };
    Ok((no_store(), Json(approvals)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
/// Closed decision body; actor/binding/expiry cannot be supplied by the renderer.
pub struct ApprovalDecisionBody {
    decision: ToolApprovalDecision,
}

/// `POST /api/tool-approvals/{approval_id}`; fresh same-origin proof before body parse.
pub async fn decision_post(
    State(state): State<ServerState>,
    SensitiveAuthenticated(resolved): SensitiveAuthenticated,
    headers: HeaderMap,
    Path(approval_id): Path<String>,
    body: Result<Json<ApprovalDecisionBody>, JsonRejection>,
) -> Result<(HeaderMap, Json<ToolApprovalResolved>), HttpError> {
    state
        .authorize_fresh_origin_write(&resolved, request_origin(&headers))
        .await?;
    let Json(body) = body.map_err(|rejection| {
        tracing::debug!(rejection = %rejection, "tool approval decision body 解析失败");
        AppError::MalformedPayload { field: "body" }
    })?;
    let receipt = match state
        .application()
        .execute(
            resolved.into_context(),
            AppCommand::DecideToolApproval {
                approval_id,
                decision: body.decision,
            },
        )
        .await?
    {
        AppReply::ToolApprovalResolved(receipt) => receipt,
        _ => return Err(application_contract_error()),
    };
    Ok((no_store(), Json(receipt)))
}

fn request_origin(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(http::header::ORIGIN)
        .map(|value| value.to_str().unwrap_or(""))
}

fn no_store() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers
}

fn application_contract_error() -> HttpError {
    AppError::DependencyUnavailable {
        dependency: "application",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::response::Response;
    use http::{Method, Request, StatusCode};
    use openbot_application::cursor::ChannelCursor;
    use openbot_application::{
        ApplicationService, ChannelReader, OpenBotApplication, PortError,
        ToolApprovalAdministration, ToolApprovalAdministrationError,
    };
    use openbot_contracts::auth::{AuthContext, AuthGeneration, Role};
    use openbot_contracts::ids::{ActorId, BotId, DeploymentId, RunId, TenantId, ToolCallId};
    use openbot_contracts::tool::{PendingToolApproval, ToolApprovalClass, ToolApprovalEffect};
    use openbot_domain::identity::session::{SessionState, TrustedOrigins, evaluate_session};
    use openbot_infra::auth::config::default_session_lifetime;
    use std::sync::{Arc, Mutex};
    use time::{Duration, OffsetDateTime};
    use tower::ServiceExt as _;

    use crate::auth::{FixedAuthResolver, ResolvedAuth, SensitiveWriteSecurity};
    use crate::http::ServerBuilder;

    struct EmptyChannels;

    #[async_trait]
    impl ChannelReader for EmptyChannels {
        async fn list_visible_channels(
            &self,
            _actor: &ActorId,
            _limit: u32,
            _cursor: Option<ChannelCursor>,
        ) -> Result<Vec<openbot_contracts::command::ChannelSummary>, PortError> {
            Ok(Vec::new())
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Call {
        List(ActorId),
        Decide(ActorId, String, ToolApprovalDecision),
    }

    #[derive(Clone, Default)]
    struct FakeApprovals {
        calls: Arc<Mutex<Vec<Call>>>,
    }

    #[async_trait]
    impl ToolApprovalAdministration for FakeApprovals {
        async fn list_pending(
            &self,
            auth: &AuthContext,
        ) -> Result<PendingToolApprovals, ToolApprovalAdministrationError> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::List(auth.actor().clone()));
            Ok(PendingToolApprovals {
                approvals: vec![PendingToolApproval {
                    approval_id: "approval-1".to_owned(),
                    call_id: ToolCallId::new("call-1"),
                    run_id: RunId::new("run-1"),
                    bot_id: BotId::new("bot-1"),
                    tool_name: "mcp__notes__delete".to_owned(),
                    target_kind: "mcp_tool".to_owned(),
                    target_id: "notes/delete".to_owned(),
                    effect: ToolApprovalEffect::Write,
                    approval_class: ToolApprovalClass::EveryCall,
                    arguments_summary: serde_json::json!({"id":"note-1"}),
                    change_summary: None,
                    requested_at: OffsetDateTime::UNIX_EPOCH,
                    expires_at: OffsetDateTime::UNIX_EPOCH + Duration::minutes(5),
                }],
            })
        }

        async fn decide(
            &self,
            auth: &AuthContext,
            approval_id: &str,
            decision: ToolApprovalDecision,
        ) -> Result<ToolApprovalResolved, ToolApprovalAdministrationError> {
            self.calls.lock().unwrap().push(Call::Decide(
                auth.actor().clone(),
                approval_id.to_owned(),
                decision,
            ));
            Ok(ToolApprovalResolved {
                approval_id: approval_id.to_owned(),
                decision,
            })
        }
    }

    fn router(approvals: FakeApprovals) -> Router {
        let generation = AuthGeneration::new(1);
        let context = AuthContext::for_test(
            DeploymentId::new("dep"),
            TenantId::new("tenant"),
            ActorId::new("actor"),
            [Role::User],
            generation,
            false,
        );
        let now = OffsetDateTime::now_utc();
        let lifetime = default_session_lifetime();
        let live = evaluate_session(
            lifetime,
            SessionState::rehydrate(now - Duration::minutes(1), now, generation),
            generation,
            now,
        )
        .unwrap();
        let resolver = FixedAuthResolver::granting_resolved(ResolvedAuth::from_live_session(
            context, live, None,
        ));
        let application: Arc<dyn ApplicationService> = Arc::new(
            OpenBotApplication::new(EmptyChannels).with_tool_approvals(Arc::new(approvals)),
        );
        crate::router(
            ServerBuilder::new(application, Arc::new(resolver))
                .with_sensitive_write_security(SensitiveWriteSecurity::new(
                    lifetime,
                    TrustedOrigins::from_configured(["https://app.example.test"]).unwrap(),
                ))
                .build(),
        )
    }

    async fn send(
        router: Router,
        method: Method,
        uri: &str,
        origin: Option<&str>,
        body: &'static str,
    ) -> Response {
        let mut request = Request::builder().method(method).uri(uri);
        if !body.is_empty() {
            request = request.header(http::header::CONTENT_TYPE, "application/json");
        }
        if let Some(origin) = origin {
            request = request.header(http::header::ORIGIN, origin);
        }
        router
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn pending_and_decision_routes_are_no_store_typed_and_guard_before_parse() {
        let approvals = FakeApprovals::default();
        let pending = send(
            router(approvals.clone()),
            Method::GET,
            "/api/tool-approvals",
            None,
            "",
        )
        .await;
        assert_eq!(pending.status(), StatusCode::OK);
        assert_eq!(pending.headers()[CACHE_CONTROL], "no-store");
        let body = to_bytes(pending.into_body(), 4096).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["approvals"][0]["argumentsSummary"]
                ["id"],
            "note-1"
        );

        let before_parse = send(
            router(approvals.clone()),
            Method::POST,
            "/api/tool-approvals/approval-1",
            None,
            "{",
        )
        .await;
        assert_eq!(before_parse.status(), StatusCode::FORBIDDEN);
        assert_eq!(approvals.calls.lock().unwrap().len(), 1);

        let extra = send(
            router(approvals.clone()),
            Method::POST,
            "/api/tool-approvals/approval-1",
            Some("https://app.example.test"),
            r#"{"decision":"grant","actor":"admin"}"#,
        )
        .await;
        assert_eq!(extra.status(), StatusCode::BAD_REQUEST);
        assert_eq!(approvals.calls.lock().unwrap().len(), 1);

        let granted = send(
            router(approvals.clone()),
            Method::POST,
            "/api/tool-approvals/approval-1",
            Some("https://app.example.test"),
            r#"{"decision":"grant"}"#,
        )
        .await;
        assert_eq!(granted.status(), StatusCode::OK);
        assert_eq!(granted.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(
            approvals.calls.lock().unwrap().as_slice(),
            [
                Call::List(ActorId::new("actor")),
                Call::Decide(
                    ActorId::new("actor"),
                    "approval-1".to_owned(),
                    ToolApprovalDecision::Grant
                )
            ]
        );
    }
}
