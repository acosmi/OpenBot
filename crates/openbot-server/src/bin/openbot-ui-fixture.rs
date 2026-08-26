//! Local-only deterministic GUI fixture host required by the GUI first-source golden workflow.

use std::collections::HashMap;
use std::error::Error;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures_core::Stream;
use openbot_application::cursor::ChannelCursor;
use openbot_application::{
    AgentDirectory, AgentReadScope, AppEventStream, ApplicationService, BeginThreadRunRequest,
    ChannelAdministration, ChannelAdministrationError, ChannelCreateRequest, ChannelReadScope,
    ChannelReader, OpenBotApplication, PeopleAdministration, PeoplePageRequest, PeoplePortError,
    PortError, ThreadConversationRequest, ThreadDirectory, ThreadDirectoryError,
    ThreadEventSubscription, ThreadHistoryRequest, ToolApprovalAdministration,
    ToolApprovalAdministrationError, UiPreferenceAdministration, UiPreferenceAdministrationError,
};
use openbot_contracts::agent::{AgentProfile, AgentVisibility};
use openbot_contracts::auth::{AuthContext, AuthGeneration, Role};
use openbot_contracts::command::{
    AppEvent, ChannelActivityEvent, ChannelDetail, ChannelSummary, ThreadConversationSnapshot,
    ThreadHistory, ThreadHistoryMessage, ThreadHistoryRole, ThreadRunEvent, ThreadRunEventKind,
    ThreadRunStarted,
};
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

const FIXTURE_EXISTING_THREAD: &str = "550e8400-e29b-81d4-a716-446655440000";

#[derive(Clone)]
struct FixtureChannels {
    rows: Arc<Mutex<Vec<ChannelSummary>>>,
    next: Arc<AtomicU64>,
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
                thread_id: (index == 0).then(|| ThreadId::new(FIXTURE_EXISTING_THREAD)),
                active: index != 3,
            })
            .collect();
        Self {
            rows: Arc::new(Mutex::new(rows)),
            next: Arc::new(AtomicU64::new(1)),
        }
    }

    fn read(&self, limit: u32, cursor: Option<ChannelCursor>) -> Vec<ChannelSummary> {
        let mut rows = self.rows.lock().expect("fixture channel lock").clone();
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
        Ok(self
            .rows
            .lock()
            .expect("fixture channel lock")
            .iter()
            .find(|row| &row.id == channel_id)
            .cloned())
    }
}

#[async_trait]
impl ChannelAdministration for FixtureChannels {
    async fn create_channel(
        &self,
        request: ChannelCreateRequest,
    ) -> Result<ChannelDetail, ChannelAdministrationError> {
        let sequence = self.next.fetch_add(1, Ordering::SeqCst);
        let id = ChannelId::new(format!("channel-created-{sequence}"));
        let thread_id = ThreadId::new(uuid::Uuid::now_v7().to_string());
        let name = request
            .agent_ids
            .iter()
            .map(|id| id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let created_at = OffsetDateTime::now_utc();
        self.rows
            .lock()
            .map_err(|_| ChannelAdministrationError::Unavailable)?
            .insert(
                0,
                ChannelSummary {
                    id: id.clone(),
                    name: name.clone(),
                    agent_ids: request.agent_ids.clone(),
                    last_message: None,
                    last_message_at: None,
                    last_message_agent_id: None,
                    created_at,
                    thread_id: Some(thread_id.clone()),
                    active: true,
                },
            );
        Ok(ChannelDetail {
            id,
            name,
            agent_ids: request.agent_ids,
            thread_id: Some(thread_id),
            active: true,
        })
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
                AgentProfile {
                    id: BotId::new("fixture-hidden-private"),
                    name: "Hidden Counsel".to_owned(),
                    title: "Hidden private coworker".to_owned(),
                    role_description: "A directly addressable coworker hidden from the default roster.".to_owned(),
                    avatar_seed: "fixture-hidden".to_owned(),
                    visibility: AgentVisibility::Private,
                    endpoint: None,
                    has_auth: false,
                    has_callback_token: false,
                    hidden: true,
                    system_owned: false,
                    can_manage: true,
                    mine: true,
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

#[derive(Clone)]
struct FixtureThreads {
    inner: Arc<FixtureThreadsInner>,
    channels: FixtureChannels,
}

struct FixtureThreadsInner {
    snapshots: Mutex<HashMap<String, ThreadConversationSnapshot>>,
    events: Mutex<HashMap<String, Vec<ThreadRunEvent>>>,
    subscribers: Mutex<HashMap<String, Vec<tokio::sync::mpsc::Sender<AppEvent>>>>,
    receipts: Mutex<HashMap<String, ThreadRunStarted>>,
}

impl FixtureThreads {
    fn new(channels: FixtureChannels) -> Self {
        let mut snapshots = HashMap::new();
        snapshots.insert(
            FIXTURE_EXISTING_THREAD.to_owned(),
            ThreadConversationSnapshot {
                messages: vec![
                    ThreadHistoryMessage {
                        id: "fixture-user-message".to_owned(),
                        role: ThreadHistoryRole::User,
                        content: "Categorize these expenses.".to_owned(),
                        tool_call_id: None,
                        tool_calls: None,
                    },
                    ThreadHistoryMessage {
                        id: "fixture-assistant-message".to_owned(),
                        role: ThreadHistoryRole::Assistant,
                        content: "Categorized three expenses.".to_owned(),
                        tool_call_id: None,
                        tool_calls: None,
                    },
                ],
                active_run_id: None,
                active_run_text: String::new(),
                last_event_sequence: None,
            },
        );
        Self {
            inner: Arc::new(FixtureThreadsInner {
                snapshots: Mutex::new(snapshots),
                events: Mutex::new(HashMap::new()),
                subscribers: Mutex::new(HashMap::new()),
                receipts: Mutex::new(HashMap::new()),
            }),
            channels,
        }
    }

    fn publish(&self, thread: &ThreadId, event: ThreadRunEvent) {
        self.inner
            .events
            .lock()
            .expect("fixture events lock")
            .entry(thread.as_str().to_owned())
            .or_default()
            .push(event.clone());
        if let Some(subscribers) = self
            .inner
            .subscribers
            .lock()
            .expect("fixture subscribers lock")
            .get_mut(thread.as_str())
        {
            subscribers.retain(|sender| {
                sender
                    .try_send(AppEvent::ThreadRunEvent(event.clone()))
                    .is_ok()
            });
        }
    }
}

#[async_trait]
impl ThreadDirectory for FixtureThreads {
    async fn mint_thread_id(
        &self,
        _deployment: &DeploymentId,
    ) -> Result<ThreadId, ThreadDirectoryError> {
        Ok(ThreadId::new(uuid::Uuid::now_v7().to_string()))
    }

    async fn thread_known(
        &self,
        _deployment: &DeploymentId,
        _tenant: &TenantId,
        _actor: &ActorId,
        thread: &ThreadId,
    ) -> Result<bool, ThreadDirectoryError> {
        Ok(self
            .inner
            .snapshots
            .lock()
            .map_err(|_| ThreadDirectoryError::Unavailable)?
            .contains_key(thread.as_str()))
    }

    async fn begin_thread_run(
        &self,
        request: BeginThreadRunRequest,
    ) -> Result<ThreadRunStarted, ThreadDirectoryError> {
        if let Some(receipt) = self
            .inner
            .receipts
            .lock()
            .map_err(|_| ThreadDirectoryError::Unavailable)?
            .get(request.command.run_id.as_str())
            .cloned()
        {
            return Ok(ThreadRunStarted {
                replayed: true,
                ..receipt
            });
        }
        let thread = request.command.thread_id.clone();
        let run = request.command.run_id.clone();
        if let openbot_contracts::command::ThreadRunAnchor::Channel { channel_id } =
            &request.command.anchor
            && let Ok(mut channels) = self.channels.rows.lock()
            && let Some(channel) = channels
                .iter_mut()
                .find(|channel| channel.id == *channel_id)
        {
            channel.thread_id = Some(thread.clone());
        }
        let (message_sequence, event_sequence) = {
            let mut snapshots = self
                .inner
                .snapshots
                .lock()
                .map_err(|_| ThreadDirectoryError::Unavailable)?;
            let snapshot = snapshots.entry(thread.as_str().to_owned()).or_default();
            let message_sequence = u64::try_from(snapshot.messages.len()).map_err(|_| {
                ThreadDirectoryError::Corrupt {
                    field: "fixture_sequence",
                }
            })?;
            let event_sequence = snapshot
                .last_event_sequence
                .map_or(0, |value| value.saturating_add(1));
            snapshot.messages.push(ThreadHistoryMessage {
                id: format!("{}:user", run.as_str()),
                role: ThreadHistoryRole::User,
                content: request.command.message.clone(),
                tool_call_id: None,
                tool_calls: None,
            });
            snapshot.active_run_id = Some(run.clone());
            snapshot.active_run_text.clear();
            snapshot.last_event_sequence = Some(event_sequence);
            (message_sequence, event_sequence)
        };
        let started = ThreadRunStarted {
            thread_id: thread.clone(),
            run_id: run.clone(),
            message_sequence,
            event_sequence,
            replayed: false,
        };
        self.inner
            .receipts
            .lock()
            .map_err(|_| ThreadDirectoryError::Unavailable)?
            .insert(run.as_str().to_owned(), started.clone());
        self.publish(
            &thread,
            ThreadRunEvent {
                thread_id: thread.clone(),
                run_id: run.clone(),
                event_sequence,
                event_type: ThreadRunEventKind::Started,
                payload: serde_json::json!({"runId":run,"messageId":format!("{}:user",run.as_str()),"botId":request.command.bot_id}),
                terminal: false,
                created_at: OffsetDateTime::now_utc(),
            },
        );
        let runtime = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
            let chunk_sequence = event_sequence.saturating_add(1);
            if let Ok(mut snapshots) = runtime.inner.snapshots.lock()
                && let Some(snapshot) = snapshots.get_mut(thread.as_str())
            {
                snapshot.active_run_text = "Fixture reply".to_owned();
                snapshot.last_event_sequence = Some(chunk_sequence);
            }
            runtime.publish(
                &thread,
                ThreadRunEvent {
                    thread_id: thread.clone(),
                    run_id: run.clone(),
                    event_sequence: chunk_sequence,
                    event_type: ThreadRunEventKind::SemanticChunk,
                    payload: serde_json::json!({"channel":"text","delta":"Fixture reply"}),
                    terminal: false,
                    created_at: OffsetDateTime::now_utc(),
                },
            );
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            let terminal_sequence = chunk_sequence.saturating_add(1);
            if let Ok(mut snapshots) = runtime.inner.snapshots.lock()
                && let Some(snapshot) = snapshots.get_mut(thread.as_str())
            {
                snapshot.messages.push(ThreadHistoryMessage {
                    id: format!("{}:assistant", run.as_str()),
                    role: ThreadHistoryRole::Assistant,
                    content: "Fixture reply".to_owned(),
                    tool_call_id: None,
                    tool_calls: None,
                });
                snapshot.active_run_id = None;
                snapshot.active_run_text.clear();
                snapshot.last_event_sequence = Some(terminal_sequence);
            }
            runtime.publish(
                &thread,
                ThreadRunEvent {
                    thread_id: thread.clone(),
                    run_id: run,
                    event_sequence: terminal_sequence,
                    event_type: ThreadRunEventKind::Completed,
                    payload: serde_json::json!({"status":"completed"}),
                    terminal: true,
                    created_at: OffsetDateTime::now_utc(),
                },
            );
        });
        Ok(started)
    }

    async fn thread_history(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<ThreadHistory, ThreadDirectoryError> {
        Ok(ThreadHistory {
            messages: self
                .inner
                .snapshots
                .lock()
                .map_err(|_| ThreadDirectoryError::Unavailable)?
                .get(request.thread.as_str())
                .map(|snapshot| snapshot.messages.clone())
                .unwrap_or_default(),
        })
    }

    async fn thread_conversation(
        &self,
        request: ThreadConversationRequest,
    ) -> Result<ThreadConversationSnapshot, ThreadDirectoryError> {
        Ok(self
            .inner
            .snapshots
            .lock()
            .map_err(|_| ThreadDirectoryError::Unavailable)?
            .get(request.thread.as_str())
            .cloned()
            .unwrap_or_default())
    }

    async fn subscribe_thread_events(
        &self,
        request: ThreadEventSubscription,
    ) -> Result<AppEventStream, ThreadDirectoryError> {
        let (sender, receiver) = tokio::sync::mpsc::channel(64);
        let cursor = request.after_event_sequence;
        for event in self
            .inner
            .events
            .lock()
            .map_err(|_| ThreadDirectoryError::Unavailable)?
            .get(request.thread.as_str())
            .into_iter()
            .flatten()
            .filter(|event| cursor.is_none_or(|cursor| event.event_sequence > cursor))
        {
            sender
                .try_send(AppEvent::ThreadRunEvent(event.clone()))
                .map_err(|_| ThreadDirectoryError::Unavailable)?;
        }
        self.inner
            .subscribers
            .lock()
            .map_err(|_| ThreadDirectoryError::Unavailable)?
            .entry(request.thread.as_str().to_owned())
            .or_default()
            .push(sender);
        Ok(Box::pin(FixtureEventStream { receiver }))
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
    let channels = FixtureChannels::new(now);
    let threads = FixtureThreads::new(channels.clone());
    let application: Arc<dyn ApplicationService> = Arc::new(
        OpenBotApplication::new(channels.clone())
            .with_channel_administration(Arc::new(channels))
            .with_agent_directory(Arc::new(FixtureAgents::new()))
            .with_people(FixturePeople)
            .with_threads(threads)
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
