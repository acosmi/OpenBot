//! Local-only deterministic GUI fixture host required by the GUI first-source golden workflow.

use std::error::Error;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use openbot_application::cursor::ChannelCursor;
use openbot_application::{
    ApplicationService, ChannelReader, OpenBotApplication, PortError, ToolApprovalAdministration,
    ToolApprovalAdministrationError,
};
use openbot_contracts::auth::{AuthContext, AuthGeneration, Role};
use openbot_contracts::ids::{ActorId, BotId, DeploymentId, RunId, TenantId, ToolCallId};
use openbot_contracts::tool::{
    PendingToolApproval, PendingToolApprovals, ToolApprovalClass, ToolApprovalDecision,
    ToolApprovalEffect, ToolApprovalResolved,
};
use openbot_domain::identity::session::{SessionState, TrustedOrigins, evaluate_session};
use openbot_infra::auth::config::default_session_lifetime;
use openbot_server::auth::FixedAuthResolver;
use openbot_server::{ResolvedAuth, SensitiveWriteSecurity, ServerBuilder, StaticApp};
use time::{Duration, OffsetDateTime};

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

#[derive(Clone)]
struct FixtureApprovals {
    pending: Arc<Mutex<Option<PendingToolApproval>>>,
}

impl FixtureApprovals {
    fn new(now: OffsetDateTime) -> Self {
        Self {
            pending: Arc::new(Mutex::new(Some(PendingToolApproval {
                approval_id: "approval-fixture-1".to_owned(),
                call_id: ToolCallId::new("call-fixture-1"),
                run_id: RunId::new("run-fixture-1"),
                bot_id: BotId::new("bot-fixture-1"),
                tool_name: "mcp__workspace__overwrite_report".to_owned(),
                target_kind: "mcp_tool".to_owned(),
                target_id: "workspace/reports/q4.txt".to_owned(),
                effect: ToolApprovalEffect::Write,
                approval_class: ToolApprovalClass::EveryCall,
                arguments_summary: serde_json::json!({
                    "path": "/reports/q4.txt",
                    "mode": "overwrite",
                    "credential": "[redacted]"
                }),
                change_summary: Some(serde_json::json!({
                    "kind": "replace",
                    "linesRemoved": 12,
                    "linesAdded": 18
                })),
                requested_at: now,
                expires_at: now + Duration::hours(1),
            }))),
        }
    }
}

#[async_trait]
impl ToolApprovalAdministration for FixtureApprovals {
    async fn list_pending(
        &self,
        _auth: &AuthContext,
    ) -> Result<PendingToolApprovals, ToolApprovalAdministrationError> {
        Ok(PendingToolApprovals {
            approvals: self
                .pending
                .lock()
                .map_err(|_| ToolApprovalAdministrationError::Unavailable)?
                .iter()
                .cloned()
                .collect(),
        })
    }

    async fn decide(
        &self,
        _auth: &AuthContext,
        approval_id: &str,
        decision: ToolApprovalDecision,
    ) -> Result<ToolApprovalResolved, ToolApprovalAdministrationError> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| ToolApprovalAdministrationError::Unavailable)?;
        let Some(request) = pending.as_ref() else {
            return Err(ToolApprovalAdministrationError::NotVisible);
        };
        if request.approval_id != approval_id {
            return Err(ToolApprovalAdministrationError::NotVisible);
        }
        *pending = None;
        Ok(ToolApprovalResolved {
            approval_id: approval_id.to_owned(),
            decision,
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (dist, port) = arguments()?;
    let now = OffsetDateTime::now_utc();
    let generation = AuthGeneration::new(1);
    let context = AuthContext::for_test(
        DeploymentId::new("fixture-deployment"),
        TenantId::new("fixture-tenant"),
        ActorId::new("fixture-actor"),
        [Role::User],
        generation,
        false,
    );
    let lifetime = default_session_lifetime();
    let live = evaluate_session(
        lifetime,
        SessionState::rehydrate(now - Duration::minutes(1), now, generation),
        generation,
        now,
    )?;
    let resolver =
        FixedAuthResolver::granting_resolved(ResolvedAuth::from_live_session(context, live, None));
    let application: Arc<dyn ApplicationService> = Arc::new(
        OpenBotApplication::new(EmptyChannels)
            .with_tool_approvals(Arc::new(FixtureApprovals::new(now))),
    );
    let origin = format!("http://127.0.0.1:{port}");
    let router = ServerBuilder::new(application, Arc::new(resolver))
        .with_sensitive_write_security(SensitiveWriteSecurity::new(
            lifetime,
            TrustedOrigins::from_configured([origin.as_str()])?,
        ))
        .with_static_app(StaticApp::open(dist)?)
        .into_router();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("OPENBOT_UI_FIXTURE_URL={origin}/approvals");
    axum::serve(listener, router).await?;
    Ok(())
}

fn arguments() -> Result<(String, u16), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let mut dist = None;
    let mut port = 39015_u16;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--dist" => dist = arguments.next(),
            "--port" => {
                port = arguments.next().ok_or("--port requires a value")?.parse()?;
            }
            _ => return Err(format!("unknown argument `{argument}`").into()),
        }
    }
    let dist = dist.ok_or("--dist is required")?;
    Ok((dist, port))
}
