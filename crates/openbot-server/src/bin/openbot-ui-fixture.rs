//! Local-only deterministic GUI fixture host required by the GUI first-source golden workflow.

use std::collections::{HashMap, HashSet};
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
    CancelThreadRunRequest, ChannelAdministration, ChannelAdministrationError,
    ChannelCreateRequest, ChannelReadScope, ChannelReader, ComponentAdministration,
    ComponentAdministrationError, CorrectMemoryRequest, McpConnectionAdministration,
    McpConnectionError, MemoryAdministration, MemoryAdministrationError, MemoryControlRequest,
    MemoryPageRequest, MutateMemoryRequest, OpenBotApplication, PeopleAdministration,
    PeoplePageRequest, PeoplePortError, PortError, RecallMemoriesRequest, RememberMemoryRequest,
    ThreadConversationRequest, ThreadDirectory, ThreadDirectoryError, ThreadEventSubscription,
    ThreadHistoryRequest, ToolApprovalAdministration, ToolApprovalAdministrationError,
    UiPreferenceAdministration, UiPreferenceAdministrationError, UpdateMemoryControlRequest,
};
use openbot_contracts::agent::{AgentProfile, AgentVisibility};
use openbot_contracts::auth::{AuthContext, AuthGeneration, Role};
use openbot_contracts::command::{
    AppEvent, ChannelActivityEvent, ChannelDetail, ChannelSummary, ThreadConversationSnapshot,
    ThreadForegroundRunState, ThreadHistory, ThreadHistoryMessage, ThreadHistoryRole,
    ThreadRunCancellation, ThreadRunCancellationState, ThreadRunEvent, ThreadRunEventKind,
    ThreadRunStarted,
};
use openbot_contracts::components::{
    CompiledComponentKind, CompiledComponentManifestEntry, ComponentCatalogueAdded,
    ComponentRecord, ComponentRecords,
};
use openbot_contracts::ids::{
    ActorId, BotId, ChannelId, DeploymentId, RunId, TenantId, ThreadId, ToolCallId,
};
use openbot_contracts::mcp::{
    McpConnection, McpConnectionDisconnected, McpConnections, McpOAuthAuthorization,
    McpOAuthClientRegistered, McpOAuthClientRegistration, McpOAuthReturnTo,
    McpVendorRevocationStatus,
};
use openbot_contracts::memory::{
    MemoryControl, MemoryKind, MemoryMutation, MemoryOrigin, MemoryPage, MemoryRecall,
    MemoryRecord, MemoryScope, MemorySensitivity, MemorySource, MemoryStatus,
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
const FIXTURE_GOOGLE_DRIVE_SERVER: &str = "google-drive";
const FIXTURE_GOOGLE_DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";

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

#[derive(Clone)]
struct FixtureConnections {
    actor: ActorId,
    redirect_uri: String,
    connected_at: OffsetDateTime,
    connection: Arc<Mutex<Option<McpConnection>>>,
}

impl FixtureConnections {
    fn new(actor: ActorId, port: u16, connected_at: OffsetDateTime) -> Self {
        Self {
            actor,
            redirect_uri: format!("http://127.0.0.1:{port}/api/plugins/oauth/callback"),
            connected_at,
            connection: Arc::new(Mutex::new(None)),
        }
    }

    fn ensure_actor(&self, auth: &AuthContext) -> Result<(), McpConnectionError> {
        if auth.actor() == &self.actor {
            Ok(())
        } else {
            Err(McpConnectionError::NotVisible)
        }
    }

    fn ensure_server(server_id: &str) -> Result<(), McpConnectionError> {
        if server_id == FIXTURE_GOOGLE_DRIVE_SERVER {
            Ok(())
        } else {
            Err(McpConnectionError::NotVisible)
        }
    }
}

#[async_trait]
impl McpConnectionAdministration for FixtureConnections {
    async fn list_connections(
        &self,
        auth: &AuthContext,
    ) -> Result<McpConnections, McpConnectionError> {
        self.ensure_actor(auth)?;
        Ok(McpConnections {
            available_server_ids: vec![FIXTURE_GOOGLE_DRIVE_SERVER.to_owned()],
            connections: self
                .connection
                .lock()
                .map_err(|_| McpConnectionError::Unavailable)?
                .iter()
                .cloned()
                .collect(),
            redirect_uri: Some(self.redirect_uri.clone()),
        })
    }

    async fn begin_oauth(
        &self,
        auth: &AuthContext,
        server_id: &str,
        return_to: McpOAuthReturnTo,
    ) -> Result<McpOAuthAuthorization, McpConnectionError> {
        self.ensure_actor(auth)?;
        Self::ensure_server(server_id)?;
        if return_to != McpOAuthReturnTo::Settings {
            return Err(McpConnectionError::InvalidInput { field: "return_to" });
        }
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        *self
            .connection
            .lock()
            .map_err(|_| McpConnectionError::Unavailable)? = Some(McpConnection {
            server_id: server_id.to_owned(),
            scope: FIXTURE_GOOGLE_DRIVE_SCOPE.to_owned(),
            connected_at: self.connected_at,
        });
        Ok(McpOAuthAuthorization {
            authorization_url: "/settings/connected-accounts?connected=google-drive".to_owned(),
        })
    }

    async fn disconnect(
        &self,
        auth: &AuthContext,
        server_id: &str,
    ) -> Result<McpConnectionDisconnected, McpConnectionError> {
        self.ensure_actor(auth)?;
        Self::ensure_server(server_id)?;
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        let removed = self
            .connection
            .lock()
            .map_err(|_| McpConnectionError::Unavailable)?
            .take();
        if removed.is_none() {
            return Err(McpConnectionError::NotVisible);
        }
        Ok(McpConnectionDisconnected {
            server_id: server_id.to_owned(),
            vendor_revocation: McpVendorRevocationStatus::Pending,
        })
    }

    async fn register_oauth_client(
        &self,
        _auth: &AuthContext,
        _server_id: &str,
        _registration: &McpOAuthClientRegistration,
    ) -> Result<McpOAuthClientRegistered, McpConnectionError> {
        Err(McpConnectionError::Unavailable)
    }
}

#[derive(Clone)]
struct FixtureComponents {
    rows: Arc<Mutex<Vec<ComponentRecord>>>,
    now: OffsetDateTime,
}

impl FixtureComponents {
    fn new(now: OffsetDateTime) -> Self {
        Self {
            rows: Arc::new(Mutex::new(vec![
                ComponentRecord {
                    name: "showLegacyWidget".to_owned(),
                    title: "Legacy widget".to_owned(),
                    kind: CompiledComponentKind::Card,
                    draft_description: "A published row whose renderer left this build.".to_owned(),
                    published_description: Some(
                        "A published row whose renderer left this build.".to_owned(),
                    ),
                    published: true,
                    published_at: Some(now - Duration::days(2)),
                    updated_by: Some("fixture-admin".to_owned()),
                    updated_at: now - Duration::days(2),
                    has_unpublished_changes: false,
                    withheld_from: Vec::new(),
                    functions: Vec::new(),
                },
                ComponentRecord {
                    name: "showFutureChart".to_owned(),
                    title: "Future chart".to_owned(),
                    kind: CompiledComponentKind::Chart,
                    draft_description: "An unpublished fixture row.".to_owned(),
                    published_description: None,
                    published: false,
                    published_at: None,
                    updated_by: Some("fixture-admin".to_owned()),
                    updated_at: now - Duration::days(1),
                    has_unpublished_changes: true,
                    withheld_from: vec!["fixture-owned-private".to_owned()],
                    functions: vec!["readFixture".to_owned()],
                },
            ])),
            now,
        }
    }
}

#[async_trait]
impl ComponentAdministration for FixtureComponents {
    async fn list_components(
        &self,
        _auth: &AuthContext,
    ) -> Result<ComponentRecords, ComponentAdministrationError> {
        let mut components = self
            .rows
            .lock()
            .map_err(|_| ComponentAdministrationError::Unavailable)?
            .clone();
        components.sort_by(|left, right| {
            (left.kind.as_str(), &left.title, &left.name).cmp(&(
                right.kind.as_str(),
                &right.title,
                &right.name,
            ))
        });
        Ok(ComponentRecords { components })
    }

    async fn sync_catalogue(
        &self,
        _auth: &AuthContext,
        entries: &[CompiledComponentManifestEntry],
    ) -> Result<ComponentCatalogueAdded, ComponentAdministrationError> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| ComponentAdministrationError::Unavailable)?;
        let mut added = Vec::new();
        for entry in entries {
            if rows.iter().any(|row| row.name == entry.name) {
                continue;
            }
            rows.push(ComponentRecord {
                name: entry.name.clone(),
                title: entry.title.clone(),
                kind: entry.kind,
                draft_description: entry.description.clone(),
                published_description: Some(entry.description.clone()),
                published: true,
                published_at: Some(self.now),
                updated_by: Some("the build".to_owned()),
                updated_at: self.now,
                has_unpublished_changes: false,
                withheld_from: Vec::new(),
                functions: Vec::new(),
            });
            added.push(entry.name.clone());
        }
        Ok(ComponentCatalogueAdded { added })
    }
}

#[derive(Clone)]
struct FixtureMemory {
    tenant: TenantId,
    actor: ActorId,
    writes_enabled: Arc<AtomicBool>,
    rows: Arc<Mutex<Vec<MemoryRecord>>>,
    next: Arc<AtomicU64>,
}

impl FixtureMemory {
    fn new(tenant: TenantId, actor: ActorId, now: OffsetDateTime) -> Self {
        let rows = (0..52)
            .map(|index| {
                let status = match index {
                    3 => MemoryStatus::Superseded,
                    4 => MemoryStatus::Forbidden,
                    5 => MemoryStatus::Deleted,
                    _ => MemoryStatus::Active,
                };
                let memory_kind = if index == 1 {
                    MemoryKind::Fact
                } else {
                    MemoryKind::Preference
                };
                let scope = match index {
                    1 => MemoryScope::Thread {
                        thread_id: ThreadId::new(FIXTURE_EXISTING_THREAD),
                    },
                    2 => MemoryScope::Bot {
                        bot_id: BotId::new("fixture-owned-private"),
                    },
                    _ => MemoryScope::User,
                };
                let content = match index {
                    0 => Some("Prefers concise answers with primary-source citations.".to_owned()),
                    1 => Some("The finance review is scheduled for Friday.".to_owned()),
                    2 => Some("Use the private research coworker for source audits.".to_owned()),
                    3 => Some("Superseded fixture preference.".to_owned()),
                    4 | 5 => None,
                    _ => Some(format!("Fixture memory entry {index:02}")),
                };
                MemoryRecord {
                    memory_id: format!("memory-{index:02}"),
                    owner_user_id: actor.as_str().to_owned(),
                    scope,
                    memory_kind,
                    content,
                    tags: match index {
                        0 => vec!["writing".to_owned(), "citations".to_owned()],
                        1 => vec!["finance".to_owned()],
                        2 => vec!["research".to_owned()],
                        _ => vec!["fixture".to_owned()],
                    },
                    sensitivity: if index == 2 {
                        MemorySensitivity::Sensitive
                    } else {
                        MemorySensitivity::Normal
                    },
                    source: (index == 1).then(|| MemorySource {
                        thread_id: ThreadId::new(FIXTURE_EXISTING_THREAD),
                        message_id: "fixture-user-message".to_owned(),
                    }),
                    origin: match index {
                        1 | 2 => MemoryOrigin::RememberTool,
                        6 => MemoryOrigin::VerifiedImport,
                        _ => MemoryOrigin::UserAction,
                    },
                    created_by: actor.as_str().to_owned(),
                    supersedes_id: None,
                    status,
                    expires_at: None,
                    created_at: now - Duration::minutes(i64::from(index)),
                    updated_at: now - Duration::minutes(i64::from(index)),
                }
            })
            .collect();
        Self {
            tenant,
            actor,
            writes_enabled: Arc::new(AtomicBool::new(true)),
            rows: Arc::new(Mutex::new(rows)),
            next: Arc::new(AtomicU64::new(1)),
        }
    }

    fn ensure_scope(
        &self,
        tenant: &TenantId,
        actor: &ActorId,
    ) -> Result<(), MemoryAdministrationError> {
        if tenant == &self.tenant && actor == &self.actor {
            Ok(())
        } else {
            Err(MemoryAdministrationError::NotVisible)
        }
    }

    fn ensure_writes_enabled(&self) -> Result<(), MemoryAdministrationError> {
        if self.writes_enabled.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(MemoryAdministrationError::WritesDisabled)
        }
    }

    fn next_id(&self) -> String {
        let sequence = self.next.fetch_add(1, Ordering::SeqCst);
        format!("memory-fixture-{sequence}")
    }
}

#[async_trait]
impl MemoryAdministration for FixtureMemory {
    async fn memory_control(
        &self,
        request: MemoryControlRequest,
    ) -> Result<MemoryControl, MemoryAdministrationError> {
        self.ensure_scope(&request.tenant, &request.actor)?;
        Ok(MemoryControl {
            writes_enabled: self.writes_enabled.load(Ordering::SeqCst),
        })
    }

    async fn update_memory_control(
        &self,
        request: UpdateMemoryControlRequest,
    ) -> Result<MemoryControl, MemoryAdministrationError> {
        self.ensure_scope(&request.tenant, &request.actor)?;
        self.writes_enabled
            .store(request.update.writes_enabled, Ordering::SeqCst);
        Ok(MemoryControl {
            writes_enabled: request.update.writes_enabled,
        })
    }

    async fn remember(
        &self,
        request: RememberMemoryRequest,
    ) -> Result<MemoryRecord, MemoryAdministrationError> {
        self.ensure_scope(&request.tenant, &request.actor)?;
        self.ensure_writes_enabled()?;
        let now = OffsetDateTime::now_utc();
        let mut tags = request.input.tags;
        tags.sort();
        tags.dedup();
        let record = MemoryRecord {
            memory_id: self.next_id(),
            owner_user_id: request.actor.as_str().to_owned(),
            scope: request.input.scope,
            memory_kind: request.input.memory_kind,
            content: Some(request.input.content),
            tags,
            sensitivity: request.input.sensitivity,
            source: request.input.source,
            origin: MemoryOrigin::UserAction,
            created_by: request.actor.as_str().to_owned(),
            supersedes_id: None,
            status: MemoryStatus::Active,
            expires_at: request.input.expires_at,
            created_at: now,
            updated_at: now,
        };
        self.rows
            .lock()
            .map_err(|_| MemoryAdministrationError::Unavailable)?
            .insert(0, record.clone());
        Ok(record)
    }

    async fn list_memories(
        &self,
        request: MemoryPageRequest,
    ) -> Result<MemoryPage, MemoryAdministrationError> {
        self.ensure_scope(&request.tenant, &request.actor)?;
        let rows = self
            .rows
            .lock()
            .map_err(|_| MemoryAdministrationError::Unavailable)?;
        let start = if let Some(cursor) = request.cursor.as_deref() {
            rows.iter()
                .position(|record| record.memory_id == cursor)
                .map(|position| position.saturating_add(1))
                .ok_or(MemoryAdministrationError::InvalidInput { field: "cursor" })?
        } else {
            0
        };
        let limit = request.limit as usize;
        let memories = rows
            .iter()
            .skip(start)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        let next_cursor = (rows.len() > start.saturating_add(memories.len()))
            .then(|| memories.last().map(|record| record.memory_id.clone()))
            .flatten();
        Ok(MemoryPage {
            memories,
            next_cursor,
        })
    }

    async fn correct(
        &self,
        request: CorrectMemoryRequest,
    ) -> Result<MemoryRecord, MemoryAdministrationError> {
        self.ensure_scope(&request.tenant, &request.actor)?;
        self.ensure_writes_enabled()?;
        let now = OffsetDateTime::now_utc();
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| MemoryAdministrationError::Unavailable)?;
        let old = rows
            .iter_mut()
            .find(|record| record.memory_id == request.memory_id)
            .ok_or(MemoryAdministrationError::NotVisible)?;
        if old.status != MemoryStatus::Active {
            return Err(MemoryAdministrationError::Conflict);
        }
        old.status = MemoryStatus::Superseded;
        old.updated_at = now;
        let old = old.clone();
        let mut tags = request.correction.tags;
        tags.sort();
        tags.dedup();
        let replacement = MemoryRecord {
            memory_id: self.next_id(),
            owner_user_id: request.actor.as_str().to_owned(),
            scope: old.scope,
            memory_kind: old.memory_kind,
            content: Some(request.correction.content),
            tags,
            sensitivity: request.correction.sensitivity,
            source: old.source,
            origin: MemoryOrigin::UserAction,
            created_by: request.actor.as_str().to_owned(),
            supersedes_id: Some(old.memory_id),
            status: MemoryStatus::Active,
            expires_at: request.correction.expires_at,
            created_at: now,
            updated_at: now,
        };
        rows.insert(0, replacement.clone());
        Ok(replacement)
    }

    async fn mutate(
        &self,
        request: MutateMemoryRequest,
    ) -> Result<MemoryRecord, MemoryAdministrationError> {
        self.ensure_scope(&request.tenant, &request.actor)?;
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| MemoryAdministrationError::Unavailable)?;
        let record = rows
            .iter_mut()
            .find(|record| record.memory_id == request.memory_id)
            .ok_or(MemoryAdministrationError::NotVisible)?;
        record.status = match request.mutation {
            MemoryMutation::Forbid if record.status == MemoryStatus::Deleted => {
                MemoryStatus::Deleted
            }
            MemoryMutation::Forbid => MemoryStatus::Forbidden,
            MemoryMutation::Delete => MemoryStatus::Deleted,
        };
        record.content = None;
        record.updated_at = OffsetDateTime::now_utc();
        Ok(record.clone())
    }

    async fn recall(
        &self,
        request: RecallMemoriesRequest,
    ) -> Result<MemoryRecall, MemoryAdministrationError> {
        self.ensure_scope(&request.tenant, &request.actor)?;
        let query = request.input.query.to_lowercase();
        let now = OffsetDateTime::now_utc();
        let limit = request.input.limit.unwrap_or(50).clamp(1, 100) as usize;
        let memories = self
            .rows
            .lock()
            .map_err(|_| MemoryAdministrationError::Unavailable)?
            .iter()
            .filter(|record| record.status == MemoryStatus::Active)
            .filter(|record| record.expires_at.is_none_or(|expiry| expiry > now))
            .filter(|record| {
                record
                    .content
                    .as_deref()
                    .is_some_and(|content| content.to_lowercase().contains(&query))
            })
            .filter(|record| {
                request
                    .input
                    .tags
                    .iter()
                    .all(|tag| record.tags.contains(tag))
            })
            .filter(|record| match &record.scope {
                MemoryScope::User => true,
                MemoryScope::Bot { bot_id } => request.input.bot_id.as_ref() == Some(bot_id),
                MemoryScope::Thread { thread_id } => {
                    request.input.thread_id.as_ref() == Some(thread_id)
                }
            })
            .take(limit)
            .cloned()
            .collect();
        Ok(MemoryRecall { memories })
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
    cancelled_runs: Mutex<HashSet<String>>,
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
                active_run_state: None,
                active_run_cancellable: false,
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
                cancelled_runs: Mutex::new(HashSet::new()),
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
            snapshot.active_run_state = Some(ThreadForegroundRunState::Running);
            snapshot.active_run_cancellable = true;
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
            if runtime
                .inner
                .cancelled_runs
                .lock()
                .is_ok_and(|runs| runs.contains(run.as_str()))
            {
                return;
            }
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
            if runtime
                .inner
                .cancelled_runs
                .lock()
                .is_ok_and(|runs| runs.contains(run.as_str()))
            {
                return;
            }
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
                snapshot.active_run_state = None;
                snapshot.active_run_cancellable = false;
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

    async fn cancel_thread_run(
        &self,
        request: CancelThreadRunRequest,
    ) -> Result<ThreadRunCancellation, ThreadDirectoryError> {
        let run_id = request.command.run_id.clone();
        let thread_id = request.command.thread_id.clone();
        {
            let mut snapshots = self
                .inner
                .snapshots
                .lock()
                .map_err(|_| ThreadDirectoryError::Unavailable)?;
            let Some(snapshot) = snapshots.get_mut(thread_id.as_str()) else {
                return Err(ThreadDirectoryError::NotVisible);
            };
            if snapshot.active_run_id.as_ref() != Some(&run_id) {
                return Ok(ThreadRunCancellation {
                    thread_id,
                    run_id,
                    state: ThreadRunCancellationState::AlreadyTerminal,
                });
            }
            if matches!(
                snapshot.active_run_state,
                Some(ThreadForegroundRunState::Cancelling)
            ) {
                return Ok(ThreadRunCancellation {
                    thread_id,
                    run_id,
                    state: ThreadRunCancellationState::AlreadyRequested,
                });
            }
            snapshot.active_run_state = Some(ThreadForegroundRunState::Cancelling);
            snapshot.active_run_cancellable = false;
        }
        self.inner
            .cancelled_runs
            .lock()
            .map_err(|_| ThreadDirectoryError::Unavailable)?
            .insert(run_id.as_str().to_owned());
        let runtime = self.clone();
        let terminal_thread = thread_id.clone();
        let terminal_run = run_id.clone();
        tokio::spawn(async move {
            // Production only emits Cancelled after the cancellation token has stopped the child.
            // Keep that observable ordering in the browser fixture instead of collapsing both
            // states into one fake repository call.
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            let terminal_sequence = {
                let Ok(mut snapshots) = runtime.inner.snapshots.lock() else {
                    return;
                };
                let Some(snapshot) = snapshots.get_mut(terminal_thread.as_str()) else {
                    return;
                };
                if snapshot.active_run_id.as_ref() != Some(&terminal_run)
                    || !matches!(
                        snapshot.active_run_state,
                        Some(ThreadForegroundRunState::Cancelling)
                    )
                {
                    return;
                }
                let terminal_sequence = snapshot
                    .last_event_sequence
                    .map_or(0, |value| value.saturating_add(1));
                if !snapshot.active_run_text.is_empty() {
                    snapshot.messages.push(ThreadHistoryMessage {
                        id: format!("{}:assistant", terminal_run.as_str()),
                        role: ThreadHistoryRole::Assistant,
                        content: snapshot.active_run_text.clone(),
                        tool_call_id: None,
                        tool_calls: None,
                    });
                }
                snapshot.active_run_id = None;
                snapshot.active_run_state = None;
                snapshot.active_run_text.clear();
                snapshot.last_event_sequence = Some(terminal_sequence);
                terminal_sequence
            };
            runtime.publish(
                &terminal_thread,
                ThreadRunEvent {
                    thread_id: terminal_thread.clone(),
                    run_id: terminal_run.clone(),
                    event_sequence: terminal_sequence,
                    event_type: ThreadRunEventKind::Cancelled,
                    payload: serde_json::json!({"status":"cancelled"}),
                    terminal: true,
                    created_at: OffsetDateTime::now_utc(),
                },
            );
        });
        Ok(ThreadRunCancellation {
            thread_id,
            run_id,
            state: ThreadRunCancellationState::Requested,
        })
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
            // production `subscribe_channel_activity` 用 LISTEN/NOTIFY 长连保持该流打开。fixture
            // 若发完就 drop sender，流立刻结束、socket 关闭，AppSidebar 会以 reset 后的 backoff
            // 无限重连并对每次重连全量重取 channel 列表 —— 侧栏持续闪 "加载中"，那是 harness
            // 造出来的假象，不是被验收的 GUI 行为。持住 sender，让流像生产一样保持打开。
            std::future::pending::<()>().await;
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
        // Keep the deterministic browser fixture slow enough to exercise the serialized/coalesced
        // client queue and its visible pending state across browser-control round trips instead of
        // letting loopback finish before the next assertion can observe it.
        tokio::time::sleep(std::time::Duration::from_millis(1_000)).await;
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
    let tenant = TenantId::new("fixture-tenant");
    let actor = ActorId::new("fixture-actor");
    let context = AuthContext::for_test(
        DeploymentId::new("fixture-deployment"),
        tenant.clone(),
        actor.clone(),
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
    let memory = FixtureMemory::new(tenant, actor.clone(), now);
    let connections = FixtureConnections::new(actor, port, now);
    let components = FixtureComponents::new(now);
    let application: Arc<dyn ApplicationService> = Arc::new(
        OpenBotApplication::new(channels.clone())
            .with_channel_administration(Arc::new(channels))
            .with_agent_directory(Arc::new(FixtureAgents::new()))
            .with_component_administration(Arc::new(components))
            .with_people(FixturePeople)
            .with_threads(threads)
            .with_memory(memory)
            .with_mcp_connections(Arc::new(connections))
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
