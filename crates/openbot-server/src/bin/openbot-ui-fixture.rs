//! Local-only deterministic GUI fixture host required by the GUI first-source golden workflow.

use std::error::Error;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures_core::Stream;
use openbot_application::cursor::ChannelCursor;
use openbot_application::{
    AgentDirectory, AgentReadScope, AppEventStream, ApplicationService, ChannelReadScope,
    ChannelReader, OpenBotApplication, PeopleAdministration, PeoplePageRequest, PeoplePortError,
    PortError, ThreadDirectory, ThreadDirectoryError, ToolApprovalAdministration,
    ToolApprovalAdministrationError, UiPreferenceAdministration, UiPreferenceAdministrationError,
};
use openbot_contracts::agent::{AgentProfile, AgentVisibility};
use openbot_contracts::auth::{AuthContext, AuthGeneration, Role};
use openbot_contracts::command::{AppEvent, ChannelActivityEvent, ChannelSummary};
use openbot_contracts::ids::{
    ActorId, BotId, ChannelId, DeploymentId, RunId, TenantId, ThreadId, ToolCallId,
};
use openbot_contracts::people::{CurrentUser, PeoplePage, Person};
use openbot_contracts::tool::{
    PendingToolApproval, PendingToolApprovals, ToolApprovalClass, ToolApprovalDecision,
    ToolApprovalEffect, ToolApprovalResolved,
};
use openbot_contracts::ui::{UiPreferences, UpdateUiPreferences};
use openbot_domain::identity::session::{SessionState, TrustedOrigins, evaluate_session};
use openbot_infra::auth::config::default_session_lifetime;
use openbot_server::{
    AuthResolver, ResolvedAuth, SensitiveWriteSecurity, ServerBuilder, StaticApp,
};
use time::{Duration, OffsetDateTime};

#[derive(Clone)]
struct FixtureChannels {
    rows: Arc<Vec<ChannelSummary>>,
}

impl FixtureChannels {
    fn new(now: OffsetDateTime) -> Self {
        let rows = (0..52)
            .map(|index| ChannelSummary {
                id: ChannelId::new(format!("channel-{index:02}")),
                name: if index == 0 {
                    "Finance Operations".to_owned()
                } else {
                    format!("Channel {index:02}")
                },
                agent_ids: vec![BotId::new(format!("bot-{}", index % 4))],
                last_message: Some(if index == 0 {
                    "Categorized three expenses.".to_owned()
                } else {
                    format!("Fixture activity {index:02}")
                }),
                last_message_at: Some(now - Duration::minutes(i64::from(index))),
                last_message_agent_id: Some(BotId::new(format!("bot-{}", index % 4))),
                created_at: now - Duration::days(2) - Duration::minutes(i64::from(index)),
                thread_id: (index == 0).then(|| ThreadId::new("fixture-thread-0")),
                active: index != 3,
            })
            .collect();
        Self {
            rows: Arc::new(rows),
        }
    }

    fn read(&self, limit: u32, cursor: Option<ChannelCursor>) -> Vec<ChannelSummary> {
        let mut rows = self.rows.as_ref().clone();
        if let Some(cursor) = cursor {
            rows.retain(|row| {
                let recency = row.last_message_at.unwrap_or(row.created_at);
                (recency, &row.id) < (cursor.recency, &cursor.id)
            });
        }
        rows.truncate(limit as usize);
        rows
    }
}

#[async_trait]
impl ChannelReader for FixtureChannels {
    async fn list_visible_channels(
        &self,
        _actor: &ActorId,
        limit: u32,
        cursor: Option<ChannelCursor>,
    ) -> Result<Vec<ChannelSummary>, PortError> {
        Ok(self.read(limit, cursor))
    }

    async fn list_visible_channels_scoped(
        &self,
        _scope: &ChannelReadScope,
        limit: u32,
        cursor: Option<ChannelCursor>,
    ) -> Result<Vec<ChannelSummary>, PortError> {
        Ok(self.read(limit, cursor))
    }

    async fn get_visible_channel(
        &self,
        _scope: &ChannelReadScope,
        channel_id: &ChannelId,
    ) -> Result<Option<ChannelSummary>, PortError> {
        Ok(self.rows.iter().find(|row| &row.id == channel_id).cloned())
    }
}

#[derive(Clone)]
struct FixtureAgents {
    rows: Arc<Vec<AgentProfile>>,
}

impl FixtureAgents {
    fn new() -> Self {
        Self {
            rows: Arc::new(vec![
                AgentProfile {
                    id: BotId::new("fixture-owned-private"),
                    name: "Research Partner".to_owned(),
                    title: "Private research coworker".to_owned(),
                    role_description: "Finds primary sources and keeps citations attached to every conclusion.".to_owned(),
                    avatar_seed: "fixture-research".to_owned(),
                    visibility: AgentVisibility::Private,
                    endpoint: Some("https://research.example.test/ag-ui".to_owned()),
                    has_auth: true,
                    has_callback_token: true,
                    hidden: false,
                    system_owned: false,
                    can_manage: true,
                    mine: true,
                },
                AgentProfile {
                    id: BotId::new("fixture-owned-public"),
                    name: "Operations Guide".to_owned(),
                    title: "Operations coworker".to_owned(),
                    role_description: "Turns recurring operating work into clear, reviewable checklists.".to_owned(),
                    avatar_seed: "fixture-operations".to_owned(),
                    visibility: AgentVisibility::Public,
                    endpoint: None,
                    has_auth: false,
                    has_callback_token: false,
                    hidden: false,
                    system_owned: false,
                    can_manage: true,
                    mine: true,
                },
                AgentProfile {
                    id: BotId::new("fixture-system-public"),
                    name: "Knowledge Desk".to_owned(),
                    title: "System knowledge coworker".to_owned(),
                    role_description: "Answers from the deployment knowledge package without exposing its internals.".to_owned(),
                    avatar_seed: "fixture-knowledge".to_owned(),
                    visibility: AgentVisibility::Public,
                    endpoint: None,
                    has_auth: false,
                    has_callback_token: false,
                    hidden: false,
                    system_owned: true,
                    can_manage: false,
                    mine: false,
                },
                AgentProfile {
                    id: BotId::new("fixture-explore-public"),
                    name: "Risk Analyst".to_owned(),
                    title: "Shared risk coworker".to_owned(),
                    role_description: "Reviews operational changes and identifies controls that need evidence.".to_owned(),
                    avatar_seed: "fixture-risk".to_owned(),
                    visibility: AgentVisibility::Public,
                    endpoint: Some("https://risk.example.test/ag-ui".to_owned()),
                    has_auth: false,
                    has_callback_token: false,
                    hidden: false,
                    system_owned: false,
                    can_manage: false,
                    mine: false,
                },
            ]),
        }
    }
}

#[async_trait]
impl AgentDirectory for FixtureAgents {
    async fn list_visible_agents(
        &self,
        _scope: &AgentReadScope,
        hidden: bool,
    ) -> Result<Vec<AgentProfile>, PortError> {
        Ok(self
            .rows
            .iter()
            .filter(|profile| profile.hidden == hidden)
            .cloned()
            .collect())
    }

    async fn get_visible_agent(
        &self,
        _scope: &AgentReadScope,
        agent_id: &BotId,
    ) -> Result<Option<AgentProfile>, PortError> {
        Ok(self
            .rows
            .iter()
            .find(|profile| &profile.id == agent_id)
            .cloned())
    }
}

#[derive(Clone, Copy)]
struct FixturePeople;

#[async_trait]
impl PeopleAdministration for FixturePeople {
    async fn current_user(&self, actor: &ActorId) -> Result<CurrentUser, PeoplePortError> {
        Ok(CurrentUser {
            id: actor.clone(),
            email: "fixture@example.test".to_owned(),
            name: Some("Fixture User".to_owned()),
            image: None,
            role: Role::User,
        })
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

#[derive(Clone, Copy)]
struct FixtureThreads;

#[async_trait]
impl ThreadDirectory for FixtureThreads {
    async fn mint_thread_id(
        &self,
        _deployment: &DeploymentId,
    ) -> Result<ThreadId, ThreadDirectoryError> {
        Err(ThreadDirectoryError::Unavailable)
    }

    async fn thread_known(
        &self,
        _deployment: &DeploymentId,
        _tenant: &TenantId,
        _actor: &ActorId,
        _thread: &ThreadId,
    ) -> Result<bool, ThreadDirectoryError> {
        Err(ThreadDirectoryError::Unavailable)
    }

    async fn subscribe_channel_activity(
        &self,
        _request: openbot_application::ChannelActivitySubscription,
    ) -> Result<AppEventStream, ThreadDirectoryError> {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(750)).await;
            _ = sender
                .send(AppEvent::ChannelActivity(ChannelActivityEvent {
                    channel_id: ChannelId::new("channel-00"),
                    last_message: Some("Categorized three expenses.".to_owned()),
                    last_message_at: Some(OffsetDateTime::now_utc()),
                    last_message_agent_id: Some(BotId::new("bot-0")),
                }))
                .await;
        });
        Ok(Box::pin(FixtureEventStream { receiver }))
    }
}

struct FixtureEventStream {
    receiver: tokio::sync::mpsc::Receiver<AppEvent>,
}

impl Stream for FixtureEventStream {
    type Item = AppEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}

struct FixtureAuthResolver {
    resolved: ResolvedAuth,
    revoked: AtomicBool,
}

#[async_trait]
impl AuthResolver for FixtureAuthResolver {
    async fn resolve(
        &self,
        _parts: &axum::http::request::Parts,
    ) -> Result<AuthContext, openbot_contracts::error::AppError> {
        if self.revoked.load(Ordering::SeqCst) {
            Err(openbot_contracts::error::AppError::Unauthenticated)
        } else {
            Ok(self.resolved.context().clone())
        }
    }

    async fn resolve_with_assurance(
        &self,
        _parts: &axum::http::request::Parts,
    ) -> Result<ResolvedAuth, openbot_contracts::error::AppError> {
        if self.revoked.load(Ordering::SeqCst) {
            Err(openbot_contracts::error::AppError::Unauthenticated)
        } else {
            Ok(self.resolved.clone())
        }
    }

    async fn revoke_session(
        &self,
        resolved: &ResolvedAuth,
    ) -> Result<(), openbot_contracts::error::AppError> {
        if !resolved.has_revocable_session() {
            return Err(openbot_contracts::error::AppError::RequestConflict {
                resource: "session",
            });
        }
        self.revoked.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Clone)]
struct FixtureApprovals {
    pending: Arc<Mutex<Option<PendingToolApproval>>>,
}

#[derive(Default)]
struct FixturePreferences {
    stored: Mutex<UiPreferences>,
}

#[async_trait]
impl UiPreferenceAdministration for FixturePreferences {
    async fn get(
        &self,
        _auth: &AuthContext,
    ) -> Result<UiPreferences, UiPreferenceAdministrationError> {
        self.stored
            .lock()
            .map(|stored| *stored)
            .map_err(|_| UiPreferenceAdministrationError::Unavailable)
    }

    async fn update(
        &self,
        _auth: &AuthContext,
        update: UpdateUiPreferences,
    ) -> Result<UiPreferences, UiPreferenceAdministrationError> {
        let mut stored = self
            .stored
            .lock()
            .map_err(|_| UiPreferenceAdministrationError::Unavailable)?;
        stored.theme = update.theme.or(stored.theme);
        stored.locale = update.locale.or(stored.locale);
        Ok(*stored)
    }
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
    let resolver = FixtureAuthResolver {
        resolved: ResolvedAuth::from_live_session(
            context,
            live,
            Some("fixture-session".to_owned()),
        ),
        revoked: AtomicBool::new(false),
    };
    let application: Arc<dyn ApplicationService> = Arc::new(
        OpenBotApplication::new(FixtureChannels::new(now))
            .with_agent_directory(Arc::new(FixtureAgents::new()))
            .with_people(FixturePeople)
            .with_threads(FixtureThreads)
            .with_tool_approvals(Arc::new(FixtureApprovals::new(now)))
            .with_ui_preferences(Arc::new(FixturePreferences::default())),
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
