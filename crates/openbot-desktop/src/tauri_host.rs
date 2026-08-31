//! Tauri 2.11.5 custom-protocol adapter for the shared Leptos bundle.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64 as HostAtomicU64, Ordering as HostAtomicOrdering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

use http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use http::{Method, Request, Response, StatusCode};
use openbot_contracts::agent::{
    AgentConnectionTestRequest, AgentMutationRequest, AgentProfileResponse, AgentProfilesResponse,
};
use openbot_contracts::auth::AuthContext;
use openbot_contracts::budget::RunCostBudgetPreference;
use openbot_contracts::command::{
    AppCommand, AppReply, BeginThreadRun, BeginThreadRunBody, CancelThreadRun,
    ChannelDetailResponse, CreateChannelRequest, MAX_THREAD_MESSAGE_BYTES, RouteChannelRequest,
    SubscriptionRequest, ThreadRunCancellationState,
};
use openbot_contracts::components::{
    ComponentAgentGrantRequest, ComponentCatalogueRequest, ComponentDecisionRequest,
    ComponentDraftRequest, ComponentFunctionCallRequest, ComponentFunctionGrantRequest,
    ComponentGovernanceMutation, ComponentHumanDecisionAnswer, ComponentPublicationRequest,
};
use openbot_contracts::desktop::{
    DESKTOP_STRUCTURED_CLOSE_COMMAND, DESKTOP_STRUCTURED_OPEN_COMMAND,
    DesktopStructuredSubscriptionCloseRequest, DesktopStructuredSubscriptionOpened,
};
use openbot_contracts::error::{AppError, SensitiveWriteReason};
use openbot_contracts::ids::{BotId, ChannelId, RunId, ThreadId};
use openbot_contracts::people::CurrentUserResponse;
use openbot_contracts::sandboxed::SaveSandboxedComponentRequest;
use openbot_contracts::tool::ToolApprovalDecision;
use openbot_contracts::ui::{UiLocale, UiPreferences, UiTheme, UpdateUiPreferences};
use serde::de::DeserializeOwned;
use serde_json::json;
use tauri::ipc::Channel;
use tauri::{Builder, State, Webview};

use crate::{
    CancellationToken, DesktopStructuredEventBridge, DesktopStructuredOpenError,
    DesktopStructuredPumpExit, DesktopStructuredSubscription, DesktopWindowLifecycle,
    InProcessTransport, OpenSessionError, WindowLabel, pump_tauri_structured_events,
    register_tauri_window_lifecycle,
};

const INDEX_MAX_BYTES: u64 = 1024 * 1024;
const ASSET_MAX_BYTES: u64 = 8 * 1024 * 1024;
const API_BODY_MAX_BYTES: usize = 4096;
const AGENT_BODY_MAX_BYTES: usize = 64 * 1024;
// Server's single request-body cap and the public message cap are both 1 MiB. Using the contracts
// constant keeps Desktop from accepting a larger renderer-materialized body than Axum.
const CHANNEL_THREAD_BODY_MAX_BYTES: usize = MAX_THREAD_MESSAGE_BYTES;
const COMPONENT_CATALOGUE_BODY_MAX_BYTES: usize = 256 * 1024;
const COMPONENT_DECISION_BODY_MAX_BYTES: usize = 256 * 1024;
const COMPONENT_GOVERNANCE_BODY_MAX_BYTES: usize = 68 * 1024;
const SANDBOXED_COMPONENT_BODY_MAX_BYTES: usize = 1024 * 1024;
const HTML_ROOT_MARKER: &str = "<html lang=\"en\">";
const CSP: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; \
                   connect-src 'self'; img-src 'self' data: blob:; font-src 'self'; \
                   object-src 'none'; base-uri 'none'; form-action 'self'; frame-src 'self'; frame-ancestors 'none'; \
                   worker-src 'none'; manifest-src 'none'; media-src 'none'";

/// Exact audited custom-command allowlist registered by [`register_tauri_protocol`].
pub const DESKTOP_TAURI_COMMANDS: [&str; 2] = [
    DESKTOP_STRUCTURED_OPEN_COMMAND,
    DESKTOP_STRUCTURED_CLOSE_COMMAND,
];

/// Host registration/open error without leaking filesystem content.
#[derive(Debug, thiserror::Error)]
pub enum TauriHostError {
    /// Bundle path/index failed validation.
    #[error("desktop_bundle_invalid")]
    InvalidBundle,
    /// Custom scheme is not one closed lowercase URI-scheme token.
    #[error("desktop_scheme_invalid")]
    InvalidScheme,
    /// A window label already owns an authority binding.
    #[error("desktop_window_already_bound")]
    WindowAlreadyBound,
    /// Window authority lock was poisoned.
    #[error("desktop_window_authority_unavailable")]
    AuthorityUnavailable,
    /// Fresh-session duration cannot be represented by the monotonic clock.
    #[error("desktop_freshness_invalid")]
    InvalidFreshness,
    /// No verified authority is bound to the requesting native window.
    #[error("desktop_window_unbound")]
    WindowUnbound,
    /// Typed application subscription or Desktop transport rejected the stream.
    #[error(transparent)]
    StructuredSubscription(#[from] DesktopStructuredOpenError),
    /// Host window-binding counter is exhausted; wrapping could attach work to a new window.
    #[error("desktop_window_binding_counter_exhausted")]
    WindowBindingCounterExhausted,
    /// Tauri framing was reached before the background application owner became ready.
    #[error("desktop_protocol_not_ready")]
    ProtocolNotReady,
    /// A second background owner attempted to replace the process-wide protocol.
    #[error("desktop_protocol_already_ready")]
    ProtocolAlreadyReady,
}

/// Host-verified authority for one webview label. The renderer cannot construct this type.
#[derive(Clone)]
struct WindowAuthority {
    auth: AuthContext,
    fresh_until: Option<Instant>,
    binding_id: u64,
    closed: CancellationToken,
}

impl WindowAuthority {
    fn is_fresh(&self) -> bool {
        self.fresh_until
            .is_some_and(|deadline| Instant::now() <= deadline)
    }
}

#[derive(Clone, Copy)]
enum ComponentGovernanceRoute<'a> {
    Functions {
        raw_name: &'a str,
    },
    Function {
        raw_name: &'a str,
        raw_function: &'a str,
    },
    Grants {
        raw_name: &'a str,
    },
    Grant {
        raw_name: &'a str,
        raw_agent_id: &'a str,
    },
    Publication {
        raw_name: &'a str,
    },
    Draft {
        raw_name: &'a str,
    },
}

#[derive(Clone, Copy)]
enum AgentRoute<'a> {
    Detail { raw_agent_id: &'a str },
    Duplicate { raw_agent_id: &'a str },
    Hide { raw_agent_id: &'a str },
    Unhide { raw_agent_id: &'a str },
    CallbackToken { raw_agent_id: &'a str },
}

#[derive(Clone, Copy)]
enum AgentReplyKind {
    Profile(StatusCode),
    CallbackToken,
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentBodyError {
    Malformed,
    TooLarge,
}

#[derive(Clone, Copy)]
enum ThreadRoute<'a> {
    Status {
        raw_thread_id: &'a str,
    },
    Conversation {
        raw_thread_id: &'a str,
    },
    Runs {
        raw_thread_id: &'a str,
    },
    Cancel {
        raw_thread_id: &'a str,
        raw_run_id: &'a str,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SensitiveBodyError {
    Malformed,
    TooLarge,
}

impl<'a> AgentRoute<'a> {
    fn raw_agent_id(self) -> &'a str {
        match self {
            Self::Detail { raw_agent_id }
            | Self::Duplicate { raw_agent_id }
            | Self::Hide { raw_agent_id }
            | Self::Unhide { raw_agent_id }
            | Self::CallbackToken { raw_agent_id } => raw_agent_id,
        }
    }
}

fn agent_route(path: &str) -> Option<AgentRoute<'_>> {
    let segments = path
        .strip_prefix("/api/agents/")?
        .split('/')
        .collect::<Vec<_>>();
    match segments.as_slice() {
        [raw_agent_id] => Some(AgentRoute::Detail { raw_agent_id }),
        [raw_agent_id, "duplicate"] => Some(AgentRoute::Duplicate { raw_agent_id }),
        [raw_agent_id, "hide"] => Some(AgentRoute::Hide { raw_agent_id }),
        [raw_agent_id, "unhide"] => Some(AgentRoute::Unhide { raw_agent_id }),
        [raw_agent_id, "callback-token"] => Some(AgentRoute::CallbackToken { raw_agent_id }),
        _ => None,
    }
}

fn raw_channel_id(path: &str) -> Option<&str> {
    let raw = path.strip_prefix("/api/channels/")?;
    (!raw.is_empty() && raw != "events" && !raw.contains('/')).then_some(raw)
}

impl<'a> ThreadRoute<'a> {
    fn raw_thread_id(self) -> &'a str {
        match self {
            Self::Status { raw_thread_id }
            | Self::Conversation { raw_thread_id }
            | Self::Runs { raw_thread_id }
            | Self::Cancel { raw_thread_id, .. } => raw_thread_id,
        }
    }
}

fn thread_route(path: &str) -> Option<ThreadRoute<'_>> {
    let segments = path
        .strip_prefix("/api/threads/")?
        .split('/')
        .collect::<Vec<_>>();
    match segments.as_slice() {
        [raw_thread_id] if *raw_thread_id != "mint" && !raw_thread_id.is_empty() => {
            Some(ThreadRoute::Status { raw_thread_id })
        }
        [raw_thread_id, "conversation"] if !raw_thread_id.is_empty() => {
            Some(ThreadRoute::Conversation { raw_thread_id })
        }
        [raw_thread_id, "runs"] if !raw_thread_id.is_empty() => {
            Some(ThreadRoute::Runs { raw_thread_id })
        }
        [raw_thread_id, "runs", raw_run_id, "cancel"]
            if !raw_thread_id.is_empty() && !raw_run_id.is_empty() =>
        {
            Some(ThreadRoute::Cancel {
                raw_thread_id,
                raw_run_id,
            })
        }
        _ => None,
    }
}

impl<'a> ComponentGovernanceRoute<'a> {
    fn component_name(self) -> &'a str {
        match self {
            Self::Functions { raw_name }
            | Self::Function { raw_name, .. }
            | Self::Grants { raw_name }
            | Self::Grant { raw_name, .. }
            | Self::Publication { raw_name }
            | Self::Draft { raw_name } => raw_name,
        }
    }
}

fn component_governance_route(path: &str) -> Option<ComponentGovernanceRoute<'_>> {
    let segments = path
        .strip_prefix("/api/components/")?
        .split('/')
        .collect::<Vec<_>>();
    match segments.as_slice() {
        [raw_name, "functions"] => Some(ComponentGovernanceRoute::Functions { raw_name }),
        [raw_name, "functions", raw_function] => Some(ComponentGovernanceRoute::Function {
            raw_name,
            raw_function,
        }),
        [raw_name, "grants"] => Some(ComponentGovernanceRoute::Grants { raw_name }),
        [raw_name, "grants", raw_agent_id] => Some(ComponentGovernanceRoute::Grant {
            raw_name,
            raw_agent_id,
        }),
        [raw_name, "publication"] => Some(ComponentGovernanceRoute::Publication { raw_name }),
        [raw_name, "draft"] => Some(ComponentGovernanceRoute::Draft { raw_name }),
        _ => None,
    }
}

/// Validated bundle, typed in-process application and window authority registry.
pub struct DesktopTauriProtocol {
    root: PathBuf,
    index: Arc<str>,
    transport: Arc<InProcessTransport>,
    structured_events: DesktopStructuredEventBridge,
    next_window_binding_id: HostAtomicU64,
    windows: RwLock<BTreeMap<String, WindowAuthority>>,
    os_locale: UiLocale,
}

/// Process-wide protocol hand-off used when `app_data_dir()` is available only inside setup.
pub(crate) struct DesktopTauriProtocolSlot {
    protocol: OnceLock<Arc<DesktopTauriProtocol>>,
}

impl DesktopTauriProtocolSlot {
    pub(crate) fn pending() -> Arc<Self> {
        Arc::new(Self {
            protocol: OnceLock::new(),
        })
    }

    pub(crate) fn ready(protocol: Arc<DesktopTauriProtocol>) -> Arc<Self> {
        let slot = Self::pending();
        assert!(
            slot.protocol.set(protocol).is_ok(),
            "fresh protocol slot must accept its initial value"
        );
        slot
    }

    #[cfg(feature = "desktop-local-runtime")]
    pub(crate) fn install(
        &self,
        protocol: Arc<DesktopTauriProtocol>,
    ) -> Result<(), TauriHostError> {
        self.protocol
            .set(protocol)
            .map_err(|_| TauriHostError::ProtocolAlreadyReady)
    }

    pub(crate) fn get(&self) -> Result<Arc<DesktopTauriProtocol>, TauriHostError> {
        self.protocol
            .get()
            .cloned()
            .ok_or(TauriHostError::ProtocolNotReady)
    }
}

impl DesktopTauriProtocol {
    /// Open a built Trunk bundle. The caller must inject an ApplicationService whose UI preference
    /// port is [`crate::DesktopUiPreferenceStore`] in Desktop Local mode.
    pub fn open(
        dist: impl AsRef<Path>,
        transport: Arc<InProcessTransport>,
    ) -> Result<Self, TauriHostError> {
        let root = fs::canonicalize(dist).map_err(|_| TauriHostError::InvalidBundle)?;
        if !fs::metadata(&root)
            .map_err(|_| TauriHostError::InvalidBundle)?
            .is_dir()
        {
            return Err(TauriHostError::InvalidBundle);
        }
        let index_path = root.join("index.html");
        let metadata = fs::metadata(&index_path).map_err(|_| TauriHostError::InvalidBundle)?;
        if !metadata.is_file() || metadata.len() > INDEX_MAX_BYTES {
            return Err(TauriHostError::InvalidBundle);
        }
        let index = fs::read_to_string(index_path).map_err(|_| TauriHostError::InvalidBundle)?;
        validate_index(&index)?;
        let structured_events = DesktopStructuredEventBridge::new(Arc::clone(&transport));
        Ok(Self {
            root,
            index: Arc::from(index),
            transport,
            structured_events,
            next_window_binding_id: HostAtomicU64::new(0),
            windows: RwLock::new(BTreeMap::new()),
            os_locale: detect_os_locale(),
        })
    }

    /// Bind one host-created webview label to verified local session authority.
    pub fn bind_window(
        &self,
        label: impl Into<String>,
        auth: AuthContext,
        fresh_for: Option<Duration>,
    ) -> Result<(), TauriHostError> {
        let label = label.into();
        let mut windows = self
            .windows
            .write()
            .map_err(|_| TauriHostError::AuthorityUnavailable)?;
        if windows.contains_key(&label) {
            return Err(TauriHostError::WindowAlreadyBound);
        }
        let binding_id = self
            .next_window_binding_id
            .fetch_update(
                HostAtomicOrdering::SeqCst,
                HostAtomicOrdering::SeqCst,
                |current| current.checked_add(1),
            )
            .map_err(|_| TauriHostError::WindowBindingCounterExhausted)?;
        let fresh_until = fresh_for
            .map(|duration| {
                Instant::now()
                    .checked_add(duration)
                    .ok_or(TauriHostError::InvalidFreshness)
            })
            .transpose()?;
        windows.insert(
            label,
            WindowAuthority {
                auth,
                fresh_until,
                binding_id,
                closed: CancellationToken::new(),
            },
        );
        Ok(())
    }

    /// Remove one closed window's authority immediately.
    pub fn unbind_window(&self, label: &str) -> Result<bool, TauriHostError> {
        let removed = self
            .windows
            .write()
            .map(|mut windows| windows.remove(label))
            .map_err(|_| TauriHostError::AuthorityUnavailable)?;
        if let Some(authority) = removed {
            authority.closed.cancel();
            self.structured_events
                .close_window(&WindowLabel::new(label.to_owned()));
            return Ok(true);
        }
        Ok(false)
    }

    /// Whether this exact host window label currently owns verified authority.
    pub fn is_window_bound(&self, label: &str) -> Result<bool, TauriHostError> {
        self.windows
            .read()
            .map(|windows| windows.contains_key(label))
            .map_err(|_| TauriHostError::AuthorityUnavailable)
    }

    /// Number of host window labels currently carrying verified authority.
    pub fn bound_window_count(&self) -> Result<usize, TauriHostError> {
        self.windows
            .read()
            .map(|windows| windows.len())
            .map_err(|_| TauriHostError::AuthorityUnavailable)
    }

    /// Open one structured stream using only the authority bound by the native host.
    ///
    /// The renderer may select a closed [`SubscriptionRequest`] and durable cursor, but it cannot
    /// provide an actor, tenant, auth generation, internal broker label, or subscription identity.
    pub async fn open_structured_subscription(
        &self,
        label: &str,
        request: SubscriptionRequest,
    ) -> Result<DesktopStructuredSubscription, TauriHostError> {
        let authority = self
            .authority(label)?
            .ok_or(TauriHostError::WindowUnbound)?;
        let binding_closed = authority.closed.clone();
        let opened = tokio::select! {
            result = self.structured_events.open(
                WindowLabel::new(label.to_owned()),
                &authority.auth,
                request,
            ) => result,
            () = binding_closed.cancelled() => {
                return Err(TauriHostError::WindowUnbound);
            }
        };
        let subscription = match opened {
            Ok(subscription) => subscription,
            Err(DesktopStructuredOpenError::WindowClosed) => {
                return Err(TauriHostError::WindowUnbound);
            }
            Err(error) => return Err(TauriHostError::from(error)),
        };
        match self.authority(label) {
            Ok(Some(current)) if current.binding_id == authority.binding_id => Ok(subscription),
            Ok(_) => {
                drop(subscription);
                Err(TauriHostError::WindowUnbound)
            }
            Err(error) => {
                drop(subscription);
                Err(error)
            }
        }
    }

    /// Close one host-minted subscription only within the host-observed actual window.
    pub fn close_structured_subscription(
        &self,
        label: &str,
        subscription_id: u64,
    ) -> Result<bool, TauriHostError> {
        self.authority(label)?
            .ok_or(TauriHostError::WindowUnbound)?;
        Ok(self
            .structured_events
            .close_subscription(&WindowLabel::new(label.to_owned()), subscription_id))
    }

    /// Open and drain one structured stream into a real Tauri IPC [`Channel`].
    ///
    /// A future native binary command wrapper only needs to pass its host-observed window label,
    /// typed request, and Channel here; all ACL, sequence, gap, and terminal behavior stays below
    /// that wrapper.
    pub async fn pump_structured_events(
        &self,
        label: &str,
        request: SubscriptionRequest,
        channel: Channel<String>,
    ) -> Result<DesktopStructuredPumpExit, TauriHostError> {
        let subscription = self.open_structured_subscription(label, request).await?;
        Ok(pump_tauri_structured_events(subscription, channel).await)
    }

    /// Handle one custom-protocol request. Public for deterministic host-adapter tests.
    pub async fn handle(&self, label: &str, mut request: Request<Vec<u8>>) -> Response<Vec<u8>> {
        let authority = match self.authority(label) {
            Ok(Some(authority)) => authority,
            Ok(None) => return error_response(AppError::Unauthenticated),
            Err(_) => return dependency_response(),
        };
        let path = request.uri().path().to_owned();
        if path == "/api/me" {
            return self.current_user(request, authority).await;
        }
        if path == "/api/me/preferences" {
            return self.preferences(request, authority).await;
        }
        if path == "/api/me/run-cost-budget" {
            return self.run_cost_budget(request, authority).await;
        }
        if path == "/api/channels" {
            return self.channels(request, authority).await;
        }
        if let Some(raw_channel_id) = raw_channel_id(&path) {
            return self
                .channel_detail(request, authority, raw_channel_id)
                .await;
        }
        if path == "/api/route" {
            return self.route_channel(request, authority).await;
        }
        if path == "/api/threads/mint" {
            return self.thread_mint(request, authority).await;
        }
        if let Some(route) = thread_route(&path) {
            return self.thread_unary(request, authority, route).await;
        }
        if path == "/api/agents/test-connection" {
            return self.agent_test_connection(request, authority).await;
        }
        if path == "/api/agents" {
            return self.agents(request, authority).await;
        }
        if let Some(route) = agent_route(&path) {
            return self.agent_detail(request, authority, route).await;
        }
        if path == "/api/tool-approvals" {
            return self.approvals(request, authority).await;
        }
        if path == "/api/components" {
            return self.components(request, authority).await;
        }
        if path == "/api/components/catalogue" {
            return self.component_catalogue(request, authority).await;
        }
        if path == "/api/components/functions" {
            return self.component_functions(request, authority).await;
        }
        if path == "/api/components/human-decisions" {
            return self.component_human_decisions(request, authority).await;
        }
        if path == "/api/sandboxed/published" {
            return self
                .published_sandboxed_components(request, authority)
                .await;
        }
        if path == "/api/sandboxed" {
            return self.sandboxed_components(request, authority).await;
        }
        if let Some(raw_name) = path
            .strip_prefix("/api/sandboxed/")
            .and_then(|rest| rest.strip_suffix("/publish"))
        {
            return self
                .publish_sandboxed_component(request, authority, raw_name)
                .await;
        }
        if let Some(raw_name) = path.strip_prefix("/api/sandboxed/") {
            return self
                .delete_sandboxed_component(request, authority, raw_name)
                .await;
        }
        if let Some(raw_decision_id) = path
            .strip_prefix("/api/components/human-decisions/")
            .and_then(|rest| rest.strip_suffix("/answer"))
        {
            return self
                .component_human_decision_answer(request, authority, raw_decision_id)
                .await;
        }
        if let Some(raw_agent_id) = path.strip_prefix("/api/components/for-agent/") {
            return self
                .components_for_agent(request, authority, raw_agent_id)
                .await;
        }
        if let Some(route) = component_governance_route(&path) {
            return self.component_governance(request, authority, route).await;
        }
        if let Some(raw_name) = path
            .strip_prefix("/api/components/")
            .and_then(|rest| rest.strip_suffix("/decision"))
        {
            return self.component_decision(request, authority, raw_name).await;
        }
        if let Some(raw_name) = path
            .strip_prefix("/api/components/")
            .and_then(|rest| rest.strip_suffix("/call"))
        {
            return self.component_call(request, authority, raw_name).await;
        }
        if let Some(raw_id) = path.strip_prefix("/api/tool-approvals/") {
            return self.approval_decision(request, authority, raw_id).await;
        }
        if path == "/api" || path.starts_with("/api/") {
            request.body_mut().fill(0);
            return empty_response(StatusCode::NOT_FOUND);
        }
        if request.method() != Method::GET {
            request.body_mut().fill(0);
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        if path == "/" || path == "/index.html" || is_spa_path(&path) {
            let preferences = match self
                .transport
                .execute(authority.auth, AppCommand::GetUiPreferences)
                .await
            {
                Ok(AppReply::UiPreferences(preferences)) => preferences,
                Ok(_) => return dependency_response(),
                Err(error) => return error_response(error),
            };
            return index_response(&self.index, preferences, self.os_locale);
        }
        self.asset(&path)
    }

    fn authority(&self, label: &str) -> Result<Option<WindowAuthority>, TauriHostError> {
        self.windows
            .read()
            .map(|windows| windows.get(label).cloned())
            .map_err(|_| TauriHostError::AuthorityUnavailable)
    }

    async fn current_user(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::GET || !request.body().is_empty() {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        match self
            .transport
            .execute(authority.auth, AppCommand::GetCurrentUser)
            .await
        {
            Ok(AppReply::CurrentUser(user)) => json_response(&CurrentUserResponse { user }),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn preferences(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        let command = match *request.method() {
            Method::GET if request.body().is_empty() => AppCommand::GetUiPreferences,
            Method::PUT if request.body().len() <= API_BODY_MAX_BYTES => {
                let update = match serde_json::from_slice::<UpdateUiPreferences>(request.body()) {
                    Ok(update) if !update.is_empty() => update,
                    _ => return error_response(AppError::MalformedPayload { field: "body" }),
                };
                AppCommand::UpdateUiPreferences(update)
            }
            Method::PUT => return payload_too_large(),
            _ => return empty_response(StatusCode::METHOD_NOT_ALLOWED),
        };
        match self.transport.execute(authority.auth, command).await {
            Ok(AppReply::UiPreferences(preferences)) => json_response(&preferences),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn run_cost_budget(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        let command = match *request.method() {
            Method::GET if request.body().is_empty() => AppCommand::GetRunCostBudget,
            Method::PUT if request.body().len() <= API_BODY_MAX_BYTES => {
                let preference =
                    match serde_json::from_slice::<RunCostBudgetPreference>(request.body()) {
                        Ok(preference) => preference,
                        Err(_) => {
                            return error_response(AppError::MalformedPayload { field: "body" });
                        }
                    };
                AppCommand::ReplaceRunCostBudget(preference)
            }
            Method::PUT => return payload_too_large(),
            _ => return empty_response(StatusCode::METHOD_NOT_ALLOWED),
        };
        match self.transport.execute(authority.auth, command).await {
            Ok(AppReply::RunCostBudget(preference)) => json_response(&preference),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn channels(
        &self,
        mut request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        match *request.method() {
            Method::GET if request.body().is_empty() => {
                let (limit, cursor) = match channel_list_query(request.uri().query()) {
                    Some(query) => query,
                    None => {
                        return error_response(AppError::MalformedPayload { field: "query" });
                    }
                };
                match self
                    .transport
                    .execute(
                        authority.auth,
                        AppCommand::ListVisibleChannels { limit, cursor },
                    )
                    .await
                {
                    Ok(AppReply::Channels(page)) => json_response(&page),
                    Ok(_) => dependency_response(),
                    Err(error) => error_response(error),
                }
            }
            Method::POST => {
                let body = match parse_sensitive_body::<CreateChannelRequest>(
                    &mut request,
                    CHANNEL_THREAD_BODY_MAX_BYTES,
                ) {
                    Ok(body) => body,
                    Err(error) => return sensitive_body_error_response(error),
                };
                match self
                    .transport
                    .execute(
                        authority.auth,
                        AppCommand::CreateChannel {
                            agent_ids: body.agent_ids,
                        },
                    )
                    .await
                {
                    Ok(AppReply::Channel(channel)) => json_response_with_status(
                        &ChannelDetailResponse { channel },
                        StatusCode::CREATED,
                    ),
                    Ok(_) => dependency_response(),
                    Err(error) => error_response(error),
                }
            }
            _ => {
                request.body_mut().fill(0);
                empty_response(StatusCode::METHOD_NOT_ALLOWED)
            }
        }
    }

    async fn channel_detail(
        &self,
        mut request: Request<Vec<u8>>,
        authority: WindowAuthority,
        raw_channel_id: &str,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::GET || !request.body().is_empty() {
            request.body_mut().fill(0);
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        let channel_id = match percent_decode_segment(raw_channel_id) {
            Some(channel_id) => ChannelId::new(channel_id),
            None => {
                return error_response(AppError::MalformedPayload {
                    field: "channel_id",
                });
            }
        };
        match self
            .transport
            .execute(authority.auth, AppCommand::GetVisibleChannel { channel_id })
            .await
        {
            Ok(AppReply::Channel(channel)) => json_response(&ChannelDetailResponse { channel }),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn route_channel(
        &self,
        mut request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::POST {
            request.body_mut().fill(0);
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        let body = match parse_sensitive_body::<RouteChannelRequest>(
            &mut request,
            CHANNEL_THREAD_BODY_MAX_BYTES,
        ) {
            Ok(body) => body,
            Err(error) => return sensitive_body_error_response(error),
        };
        match self
            .transport
            .execute(
                authority.auth,
                AppCommand::RouteChannelMessage {
                    text: body.text,
                    agent_id: body.agent_id,
                },
            )
            .await
        {
            Ok(AppReply::ChannelRouting(decision)) => json_response(&decision),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn thread_mint(
        &self,
        mut request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::POST || !request.body().is_empty() {
            request.body_mut().fill(0);
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        match self
            .transport
            .execute(authority.auth, AppCommand::MintThreadId)
            .await
        {
            Ok(AppReply::ThreadMinted(minted)) => json_response(&minted),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn thread_unary(
        &self,
        mut request: Request<Vec<u8>>,
        authority: WindowAuthority,
        route: ThreadRoute<'_>,
    ) -> Response<Vec<u8>> {
        let supported = matches!(
            (route, request.method()),
            (ThreadRoute::Status { .. }, &Method::GET)
                | (ThreadRoute::Conversation { .. }, &Method::GET)
                | (ThreadRoute::Runs { .. }, &Method::POST)
                | (ThreadRoute::Cancel { .. }, &Method::POST)
        );
        if !supported {
            request.body_mut().fill(0);
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        let thread_id = match percent_decode_segment(route.raw_thread_id()) {
            Some(thread_id) => ThreadId::new(thread_id),
            None => {
                request.body_mut().fill(0);
                return error_response(AppError::MalformedPayload { field: "thread_id" });
            }
        };
        match (route, request.method()) {
            (ThreadRoute::Status { .. }, &Method::GET) if request.body().is_empty() => {
                self.thread_command(authority.auth, AppCommand::GetThreadStatus { thread_id })
                    .await
            }
            (ThreadRoute::Conversation { .. }, &Method::GET) if request.body().is_empty() => {
                self.thread_command(
                    authority.auth,
                    AppCommand::GetThreadConversation { thread_id },
                )
                .await
            }
            (ThreadRoute::Runs { .. }, &Method::POST) => {
                let body = match parse_sensitive_body::<BeginThreadRunBody>(
                    &mut request,
                    CHANNEL_THREAD_BODY_MAX_BYTES,
                ) {
                    Ok(body) => body,
                    Err(error) => return sensitive_body_error_response(error),
                };
                self.thread_command(
                    authority.auth,
                    AppCommand::BeginThreadRun(BeginThreadRun {
                        thread_id,
                        run_id: body.run_id,
                        bot_id: body.bot_id,
                        anchor: body.anchor,
                        message: body.message,
                    }),
                )
                .await
            }
            (ThreadRoute::Cancel { raw_run_id, .. }, &Method::POST)
                if request.body().is_empty() =>
            {
                let run_id = match percent_decode_segment(raw_run_id) {
                    Some(run_id) => RunId::new(run_id),
                    None => {
                        return error_response(AppError::MalformedPayload { field: "run_id" });
                    }
                };
                self.thread_command(
                    authority.auth,
                    AppCommand::CancelThreadRun(CancelThreadRun { thread_id, run_id }),
                )
                .await
            }
            _ => {
                request.body_mut().fill(0);
                empty_response(StatusCode::METHOD_NOT_ALLOWED)
            }
        }
    }

    async fn thread_command(&self, auth: AuthContext, command: AppCommand) -> Response<Vec<u8>> {
        match self.transport.execute(auth, command).await {
            Ok(AppReply::ThreadStatus(status)) => json_response(&status),
            Ok(AppReply::ThreadConversation(snapshot)) => json_response(&snapshot),
            Ok(AppReply::ThreadRunStarted(started)) => {
                let status = if started.replayed {
                    StatusCode::OK
                } else {
                    StatusCode::CREATED
                };
                json_response_with_status(&started, status)
            }
            Ok(AppReply::ThreadRunCancellation(cancelled)) => {
                let status = if cancelled.state == ThreadRunCancellationState::AlreadyTerminal {
                    StatusCode::OK
                } else {
                    StatusCode::ACCEPTED
                };
                json_response_with_status(&cancelled, status)
            }
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn agents(
        &self,
        mut request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        match *request.method() {
            Method::GET if request.body().is_empty() => {
                let hidden = match agent_hidden_query(request.uri().query()) {
                    Some(hidden) => hidden,
                    None => {
                        return error_response(AppError::MalformedPayload { field: "query" });
                    }
                };
                match self
                    .transport
                    .execute(authority.auth, AppCommand::ListVisibleAgents { hidden })
                    .await
                {
                    Ok(AppReply::Agents(agents)) => {
                        json_response(&AgentProfilesResponse { agents })
                    }
                    Ok(_) => dependency_response(),
                    Err(error) => error_response(error),
                }
            }
            Method::POST => {
                if !authority.is_fresh() {
                    request.body_mut().fill(0);
                    return error_response(AppError::SensitiveWriteRefused {
                        reason: SensitiveWriteReason::SessionNotFresh,
                    });
                }
                let form = match parse_agent_body::<AgentMutationRequest>(&mut request) {
                    Ok(form) => form,
                    Err(error) => return agent_body_error_response(error),
                };
                match self
                    .transport
                    .execute(authority.auth, AppCommand::CreateAgent(form))
                    .await
                {
                    Ok(AppReply::Agent(agent)) => json_response_with_status(
                        &AgentProfileResponse { agent },
                        StatusCode::CREATED,
                    ),
                    Ok(_) => dependency_response(),
                    Err(error) => error_response(error),
                }
            }
            _ => {
                request.body_mut().fill(0);
                empty_response(StatusCode::METHOD_NOT_ALLOWED)
            }
        }
    }

    async fn agent_test_connection(
        &self,
        mut request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::POST {
            request.body_mut().fill(0);
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        if !authority.is_fresh() {
            request.body_mut().fill(0);
            return error_response(AppError::SensitiveWriteRefused {
                reason: SensitiveWriteReason::SessionNotFresh,
            });
        }
        let probe = match parse_agent_body::<AgentConnectionTestRequest>(&mut request) {
            Ok(probe) => probe,
            Err(error) => return agent_body_error_response(error),
        };
        match self
            .transport
            .execute(authority.auth, AppCommand::TestAgentConnection(probe))
            .await
        {
            Ok(AppReply::AgentConnectionVerdict(verdict)) => json_response(&verdict),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn agent_detail(
        &self,
        mut request: Request<Vec<u8>>,
        authority: WindowAuthority,
        route: AgentRoute<'_>,
    ) -> Response<Vec<u8>> {
        let supported = matches!(
            (route, request.method()),
            (AgentRoute::Detail { .. }, &Method::GET)
                | (AgentRoute::Detail { .. }, &Method::PATCH)
                | (AgentRoute::Detail { .. }, &Method::DELETE)
                | (AgentRoute::Duplicate { .. }, &Method::POST)
                | (AgentRoute::Hide { .. }, &Method::POST)
                | (AgentRoute::Unhide { .. }, &Method::POST)
                | (AgentRoute::CallbackToken { .. }, &Method::POST)
                | (AgentRoute::CallbackToken { .. }, &Method::DELETE)
        );
        if !supported {
            request.body_mut().fill(0);
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        let write = request.method() != Method::GET;
        if write && !authority.is_fresh() {
            request.body_mut().fill(0);
            return error_response(AppError::SensitiveWriteRefused {
                reason: SensitiveWriteReason::SessionNotFresh,
            });
        }
        let agent_id = match percent_decode_segment(route.raw_agent_id()) {
            Some(agent_id) => BotId::new(agent_id),
            None => {
                request.body_mut().fill(0);
                return error_response(AppError::MalformedPayload { field: "agent_id" });
            }
        };
        let (command, reply_kind) = match (route, request.method()) {
            (AgentRoute::Detail { .. }, &Method::GET) if request.body().is_empty() => (
                AppCommand::GetVisibleAgent { agent_id },
                AgentReplyKind::Profile(StatusCode::OK),
            ),
            (AgentRoute::Detail { .. }, &Method::PATCH) => {
                let form = match parse_agent_body::<AgentMutationRequest>(&mut request) {
                    Ok(form) => form,
                    Err(error) => return agent_body_error_response(error),
                };
                (
                    AppCommand::UpdateAgent {
                        agent_id,
                        request: form,
                    },
                    AgentReplyKind::Profile(StatusCode::OK),
                )
            }
            (AgentRoute::Detail { .. }, &Method::DELETE) if request.body().is_empty() => {
                (AppCommand::DeleteAgent { agent_id }, AgentReplyKind::Empty)
            }
            (AgentRoute::Duplicate { .. }, &Method::POST) if request.body().is_empty() => (
                AppCommand::DuplicateAgent { agent_id },
                AgentReplyKind::Profile(StatusCode::CREATED),
            ),
            (AgentRoute::Hide { .. }, &Method::POST) if request.body().is_empty() => (
                AppCommand::SetAgentHidden {
                    agent_id,
                    hidden: true,
                },
                AgentReplyKind::Empty,
            ),
            (AgentRoute::Unhide { .. }, &Method::POST) if request.body().is_empty() => (
                AppCommand::SetAgentHidden {
                    agent_id,
                    hidden: false,
                },
                AgentReplyKind::Empty,
            ),
            (AgentRoute::CallbackToken { .. }, &Method::POST) if request.body().is_empty() => (
                AppCommand::IssueAgentCallbackToken { agent_id },
                AgentReplyKind::CallbackToken,
            ),
            (AgentRoute::CallbackToken { .. }, &Method::DELETE) if request.body().is_empty() => (
                AppCommand::RevokeAgentCallbackToken { agent_id },
                AgentReplyKind::Empty,
            ),
            _ => {
                request.body_mut().fill(0);
                return empty_response(StatusCode::METHOD_NOT_ALLOWED);
            }
        };
        match (
            self.transport.execute(authority.auth, command).await,
            reply_kind,
        ) {
            (Ok(AppReply::Agent(agent)), AgentReplyKind::Profile(status)) => {
                json_response_with_status(&AgentProfileResponse { agent }, status)
            }
            (Ok(AppReply::AgentCallbackToken(token)), AgentReplyKind::CallbackToken) => {
                json_response_with_status(&token, StatusCode::CREATED)
            }
            (Ok(AppReply::AgentCallbackTokenRevoked(_)), AgentReplyKind::Empty) => {
                empty_response(StatusCode::NO_CONTENT)
            }
            (Ok(AppReply::AgentLifecycle(_)), AgentReplyKind::Empty) => {
                empty_response(StatusCode::NO_CONTENT)
            }
            (Ok(_), _) => dependency_response(),
            (Err(error), _) => error_response(error),
        }
    }

    async fn approvals(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::GET || !request.body().is_empty() {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        match self
            .transport
            .execute(authority.auth, AppCommand::ListPendingToolApprovals)
            .await
        {
            Ok(AppReply::PendingToolApprovals(approvals)) => json_response(&approvals),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn components(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::GET || !request.body().is_empty() {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        match self
            .transport
            .execute(authority.auth, AppCommand::ListComponents)
            .await
        {
            Ok(AppReply::Components(components)) => json_response(&components),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn component_catalogue(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::PUT {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        if request.body().len() > COMPONENT_CATALOGUE_BODY_MAX_BYTES {
            return payload_too_large();
        }
        let catalogue = match serde_json::from_slice::<ComponentCatalogueRequest>(request.body()) {
            Ok(catalogue) => catalogue,
            Err(_) => return error_response(AppError::MalformedPayload { field: "body" }),
        };
        match self
            .transport
            .execute(
                authority.auth,
                AppCommand::SyncComponentCatalogue(catalogue),
            )
            .await
        {
            Ok(AppReply::ComponentCatalogueAdded(added)) => json_response(&added),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn sandboxed_components(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        let command = match *request.method() {
            Method::GET if request.body().is_empty() => AppCommand::ListSandboxedComponents,
            Method::POST if !authority.is_fresh() => {
                return error_response(AppError::SensitiveWriteRefused {
                    reason: SensitiveWriteReason::SessionNotFresh,
                });
            }
            Method::POST if request.body().len() > SANDBOXED_COMPONENT_BODY_MAX_BYTES => {
                return payload_too_large();
            }
            Method::POST => {
                let draft =
                    match serde_json::from_slice::<SaveSandboxedComponentRequest>(request.body()) {
                        Ok(draft) => draft,
                        Err(_) => {
                            return error_response(AppError::MalformedPayload { field: "body" });
                        }
                    };
                AppCommand::SaveSandboxedComponent(draft)
            }
            _ => return empty_response(StatusCode::METHOD_NOT_ALLOWED),
        };
        match self.transport.execute(authority.auth, command).await {
            Ok(AppReply::SandboxedComponents(components)) => json_response(&components),
            Ok(AppReply::SandboxedComponent(component)) => json_response(&component),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn published_sandboxed_components(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::GET || !request.body().is_empty() {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        match self
            .transport
            .execute(authority.auth, AppCommand::ListPublishedSandboxedComponents)
            .await
        {
            Ok(AppReply::PublishedSandboxedComponents(components)) => json_response(&components),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn publish_sandboxed_component(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
        raw_name: &str,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::POST || !request.body().is_empty() {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        if !authority.is_fresh() {
            return error_response(AppError::SensitiveWriteRefused {
                reason: SensitiveWriteReason::SessionNotFresh,
            });
        }
        let component_name = match percent_decode_segment(raw_name) {
            Some(name) => name,
            None => {
                return error_response(AppError::MalformedPayload {
                    field: "component_name",
                });
            }
        };
        match self
            .transport
            .execute(
                authority.auth,
                AppCommand::PublishSandboxedComponent { component_name },
            )
            .await
        {
            Ok(AppReply::SandboxedComponent(component)) => json_response(&component),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn delete_sandboxed_component(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
        raw_name: &str,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::DELETE || !request.body().is_empty() {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        if !authority.is_fresh() {
            return error_response(AppError::SensitiveWriteRefused {
                reason: SensitiveWriteReason::SessionNotFresh,
            });
        }
        let component_name = match percent_decode_segment(raw_name) {
            Some(name) => name,
            None => {
                return error_response(AppError::MalformedPayload {
                    field: "component_name",
                });
            }
        };
        match self
            .transport
            .execute(
                authority.auth,
                AppCommand::DeleteSandboxedComponent { component_name },
            )
            .await
        {
            Ok(AppReply::SandboxedComponentDeleted(deleted)) => json_response(&deleted),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn components_for_agent(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
        raw_agent_id: &str,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::GET || !request.body().is_empty() {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        let agent_id = match percent_decode_segment(raw_agent_id) {
            Some(agent_id) => BotId::new(agent_id),
            None => return error_response(AppError::MalformedPayload { field: "agent_id" }),
        };
        match self
            .transport
            .execute(
                authority.auth,
                AppCommand::ListComponentsForAgent { agent_id },
            )
            .await
        {
            Ok(AppReply::GrantedComponents(components)) => json_response(&components),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn component_decision(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
        raw_name: &str,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::POST {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        if request.body().len() > COMPONENT_DECISION_BODY_MAX_BYTES {
            return payload_too_large();
        }
        let component_name = match percent_decode_segment(raw_name) {
            Some(name) => name,
            None => {
                return error_response(AppError::MalformedPayload {
                    field: "component_name",
                });
            }
        };
        let decision = match serde_json::from_slice::<ComponentDecisionRequest>(request.body()) {
            Ok(decision) => decision,
            Err(_) => return error_response(AppError::MalformedPayload { field: "body" }),
        };
        match self
            .transport
            .execute(
                authority.auth,
                AppCommand::DecideComponent {
                    component_name,
                    request: decision,
                },
            )
            .await
        {
            Ok(AppReply::ComponentDecision(decision)) => json_response(&decision),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn component_functions(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::GET || !request.body().is_empty() {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        match self
            .transport
            .execute(authority.auth, AppCommand::ListComponentDataFunctions)
            .await
        {
            Ok(AppReply::ComponentDataFunctions(functions)) => json_response(&functions),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn component_governance(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
        route: ComponentGovernanceRoute<'_>,
    ) -> Response<Vec<u8>> {
        if !authority.is_fresh() {
            return error_response(AppError::SensitiveWriteRefused {
                reason: SensitiveWriteReason::SessionNotFresh,
            });
        }
        if request.body().len() > COMPONENT_GOVERNANCE_BODY_MAX_BYTES {
            return payload_too_large();
        }
        let component_name = match percent_decode_segment(route.component_name()) {
            Some(component_name) => component_name,
            None => {
                return error_response(AppError::MalformedPayload {
                    field: "component_name",
                });
            }
        };
        let mutation = match route {
            ComponentGovernanceRoute::Functions { .. } if request.method() == Method::POST => {
                let request =
                    match serde_json::from_slice::<ComponentFunctionGrantRequest>(request.body()) {
                        Ok(request) => request,
                        Err(_) => {
                            return error_response(AppError::MalformedPayload { field: "body" });
                        }
                    };
                ComponentGovernanceMutation::SetFunctionGrant {
                    component_name,
                    function: request.function,
                    granted: true,
                }
            }
            ComponentGovernanceRoute::Function { raw_function, .. }
                if request.method() == Method::DELETE && request.body().is_empty() =>
            {
                let function = match percent_decode_segment(raw_function) {
                    Some(function) => function,
                    None => {
                        return error_response(AppError::MalformedPayload { field: "function" });
                    }
                };
                ComponentGovernanceMutation::SetFunctionGrant {
                    component_name,
                    function,
                    granted: false,
                }
            }
            ComponentGovernanceRoute::Grants { .. } if request.method() == Method::POST => {
                let request =
                    match serde_json::from_slice::<ComponentAgentGrantRequest>(request.body()) {
                        Ok(request) => request,
                        Err(_) => {
                            return error_response(AppError::MalformedPayload { field: "body" });
                        }
                    };
                ComponentGovernanceMutation::SetAgentGrant {
                    component_name,
                    agent_id: request.agent_id,
                    granted: true,
                }
            }
            ComponentGovernanceRoute::Grant { raw_agent_id, .. }
                if request.method() == Method::DELETE && request.body().is_empty() =>
            {
                let agent_id = match percent_decode_segment(raw_agent_id) {
                    Some(agent_id) => BotId::new(agent_id),
                    None => {
                        return error_response(AppError::MalformedPayload { field: "agent_id" });
                    }
                };
                ComponentGovernanceMutation::SetAgentGrant {
                    component_name,
                    agent_id,
                    granted: false,
                }
            }
            ComponentGovernanceRoute::Publication { .. } if request.method() == Method::POST => {
                let request =
                    match serde_json::from_slice::<ComponentPublicationRequest>(request.body()) {
                        Ok(request) => request,
                        Err(_) => {
                            return error_response(AppError::MalformedPayload { field: "body" });
                        }
                    };
                ComponentGovernanceMutation::SetPublication {
                    component_name,
                    published: request.published,
                }
            }
            ComponentGovernanceRoute::Draft { .. } if request.method() == Method::PUT => {
                let request = match serde_json::from_slice::<ComponentDraftRequest>(request.body())
                {
                    Ok(request) => request,
                    Err(_) => {
                        return error_response(AppError::MalformedPayload { field: "body" });
                    }
                };
                ComponentGovernanceMutation::SaveDraft {
                    component_name,
                    description: request.description,
                }
            }
            _ => return empty_response(StatusCode::METHOD_NOT_ALLOWED),
        };
        match self
            .transport
            .execute(
                authority.auth,
                AppCommand::UpdateComponentGovernance(mutation),
            )
            .await
        {
            Ok(AppReply::ComponentGovernanceUpdated(receipt)) => json_response(&receipt),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn component_human_decisions(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::GET || !request.body().is_empty() {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        match self
            .transport
            .execute(
                authority.auth,
                AppCommand::ListPendingComponentHumanDecisions,
            )
            .await
        {
            Ok(AppReply::PendingComponentHumanDecisions(decisions)) => json_response(&decisions),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn component_human_decision_answer(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
        raw_decision_id: &str,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::POST {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        if request.body().len() > COMPONENT_DECISION_BODY_MAX_BYTES {
            return payload_too_large();
        }
        let decision_id = match percent_decode_segment(raw_decision_id) {
            Some(decision_id) => decision_id,
            None => {
                return error_response(AppError::MalformedPayload {
                    field: "decision_id",
                });
            }
        };
        let answer = match serde_json::from_slice::<ComponentHumanDecisionAnswer>(request.body()) {
            Ok(answer) => answer,
            Err(_) => return error_response(AppError::MalformedPayload { field: "body" }),
        };
        match self
            .transport
            .execute(
                authority.auth,
                AppCommand::ResolveComponentHumanDecision {
                    decision_id,
                    answer,
                },
            )
            .await
        {
            Ok(AppReply::ComponentHumanDecisionResolved(resolved)) => json_response(&resolved),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn component_call(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
        raw_name: &str,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::POST {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        if request.body().len() > COMPONENT_DECISION_BODY_MAX_BYTES {
            return payload_too_large();
        }
        let component_name = match percent_decode_segment(raw_name) {
            Some(name) => name,
            None => {
                return error_response(AppError::MalformedPayload {
                    field: "component_name",
                });
            }
        };
        let call = match serde_json::from_slice::<ComponentFunctionCallRequest>(request.body()) {
            Ok(call) => call,
            Err(_) => return error_response(AppError::MalformedPayload { field: "body" }),
        };
        match self
            .transport
            .execute(
                authority.auth,
                AppCommand::CallComponentFunction {
                    component_name,
                    request: call,
                },
            )
            .await
        {
            Ok(AppReply::ComponentFunctionCall(result)) => {
                let status = if result.error.is_some() {
                    StatusCode::BAD_GATEWAY
                } else {
                    StatusCode::OK
                };
                json_response_with_status(&result, status)
            }
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    async fn approval_decision(
        &self,
        request: Request<Vec<u8>>,
        authority: WindowAuthority,
        raw_id: &str,
    ) -> Response<Vec<u8>> {
        if request.method() != Method::POST {
            return empty_response(StatusCode::METHOD_NOT_ALLOWED);
        }
        if !authority.is_fresh() {
            return error_response(AppError::SensitiveWriteRefused {
                reason: SensitiveWriteReason::SessionNotFresh,
            });
        }
        if request.body().len() > API_BODY_MAX_BYTES {
            return payload_too_large();
        }
        let approval_id = match percent_decode_segment(raw_id) {
            Some(id) if !id.is_empty() && id.len() <= 128 && !id.as_bytes().contains(&0) => id,
            _ => {
                return error_response(AppError::MalformedPayload {
                    field: "approvalId",
                });
            }
        };
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct DecisionBody {
            decision: ToolApprovalDecision,
        }
        let body = match serde_json::from_slice::<DecisionBody>(request.body()) {
            Ok(body) => body,
            Err(_) => return error_response(AppError::MalformedPayload { field: "body" }),
        };
        match self
            .transport
            .execute(
                authority.auth,
                AppCommand::DecideToolApproval {
                    approval_id,
                    decision: body.decision,
                },
            )
            .await
        {
            Ok(AppReply::ToolApprovalResolved(resolved)) => json_response(&resolved),
            Ok(_) => dependency_response(),
            Err(error) => error_response(error),
        }
    }

    fn asset(&self, path: &str) -> Response<Vec<u8>> {
        let Some(relative) = closed_asset_path(path) else {
            return empty_response(StatusCode::NOT_FOUND);
        };
        let candidate = self.root.join(relative);
        let Ok(canonical) = fs::canonicalize(candidate) else {
            return empty_response(StatusCode::NOT_FOUND);
        };
        if !canonical.starts_with(&self.root) {
            return empty_response(StatusCode::NOT_FOUND);
        }
        let Ok(metadata) = fs::metadata(&canonical) else {
            return empty_response(StatusCode::NOT_FOUND);
        };
        if !metadata.is_file() || metadata.len() > ASSET_MAX_BYTES {
            return empty_response(StatusCode::NOT_FOUND);
        }
        let Ok(body) = fs::read(&canonical) else {
            return empty_response(StatusCode::NOT_FOUND);
        };
        response(StatusCode::OK, content_type(&canonical), body, false)
    }
}

#[tauri::command]
async fn openbot_structured_events_open(
    webview: Webview,
    protocol: State<'_, Arc<DesktopTauriProtocolSlot>>,
    request: SubscriptionRequest,
    channel: Channel<String>,
) -> Result<DesktopStructuredSubscriptionOpened, String> {
    let label = webview.label().to_owned();
    let protocol = protocol
        .get()
        .map_err(|error| tauri_host_error_code(&error).to_owned())?;
    let subscription = protocol
        .open_structured_subscription(&label, request)
        .await
        .map_err(|error| tauri_host_error_code(&error).to_owned())?;
    let opened = DesktopStructuredSubscriptionOpened {
        subscription_id: subscription.subscription_id(),
    };
    tauri::async_runtime::spawn(async move {
        let _ = pump_tauri_structured_events(subscription, channel).await;
    });
    Ok(opened)
}

#[tauri::command]
fn openbot_structured_events_close(
    webview: Webview,
    protocol: State<'_, Arc<DesktopTauriProtocolSlot>>,
    request: DesktopStructuredSubscriptionCloseRequest,
) -> Result<bool, String> {
    protocol
        .get()
        .map_err(|error| tauri_host_error_code(&error).to_owned())?
        .close_structured_subscription(webview.label(), request.subscription_id)
        .map_err(|error| tauri_host_error_code(&error).to_owned())
}

fn tauri_host_error_code(error: &TauriHostError) -> &'static str {
    match error {
        TauriHostError::InvalidBundle => "desktop_bundle_invalid",
        TauriHostError::InvalidScheme => "desktop_scheme_invalid",
        TauriHostError::WindowAlreadyBound => "desktop_window_already_bound",
        TauriHostError::AuthorityUnavailable => "desktop_window_authority_unavailable",
        TauriHostError::InvalidFreshness => "desktop_freshness_invalid",
        TauriHostError::WindowUnbound => "desktop_window_unbound",
        TauriHostError::WindowBindingCounterExhausted => "desktop_window_binding_counter_exhausted",
        TauriHostError::ProtocolNotReady => "desktop_protocol_not_ready",
        TauriHostError::ProtocolAlreadyReady => "desktop_protocol_already_ready",
        TauriHostError::StructuredSubscription(DesktopStructuredOpenError::CounterExhausted) => {
            "structured_subscription_counter_exhausted"
        }
        TauriHostError::StructuredSubscription(
            DesktopStructuredOpenError::WindowBudgetExhausted,
        ) => "structured_subscription_window_budget_exhausted",
        TauriHostError::StructuredSubscription(DesktopStructuredOpenError::WindowClosed) => {
            "desktop_window_unbound"
        }
        TauriHostError::StructuredSubscription(DesktopStructuredOpenError::Session(
            OpenSessionError::Application(error),
        )) => error.code().as_str(),
        TauriHostError::StructuredSubscription(DesktopStructuredOpenError::Session(
            OpenSessionError::WindowAlreadyOpen(_),
        )) => "desktop_subscription_conflict",
        TauriHostError::StructuredSubscription(DesktopStructuredOpenError::Session(
            OpenSessionError::ShuttingDown,
        )) => "desktop_shutting_down",
    }
}

/// Register the exact caller-selected custom scheme and audited structured open/close commands.
pub fn register_tauri_protocol(
    builder: Builder<tauri::Wry>,
    scheme: &str,
    protocol: Arc<DesktopTauriProtocol>,
) -> Result<Builder<tauri::Wry>, TauriHostError> {
    if !valid_scheme(scheme) {
        return Err(TauriHostError::InvalidScheme);
    }
    let scheme = scheme.to_owned();
    let lifecycle = Arc::new(
        DesktopWindowLifecycle::new(&scheme, Arc::clone(&protocol))
            .map_err(|_| TauriHostError::InvalidScheme)?,
    );
    let slot = DesktopTauriProtocolSlot::ready(protocol);
    let builder = register_tauri_window_lifecycle(builder, lifecycle);
    register_tauri_protocol_slot(builder, &scheme, slot)
}

pub(crate) fn register_tauri_protocol_slot(
    builder: Builder<tauri::Wry>,
    scheme: &str,
    slot: Arc<DesktopTauriProtocolSlot>,
) -> Result<Builder<tauri::Wry>, TauriHostError> {
    if !valid_scheme(scheme) {
        return Err(TauriHostError::InvalidScheme);
    }
    let scheme = scheme.to_owned();
    let request_slot = Arc::clone(&slot);
    let builder = builder
        .manage(slot)
        .invoke_handler(tauri::generate_handler![
            openbot_structured_events_open,
            openbot_structured_events_close
        ]);
    Ok(builder.register_asynchronous_uri_scheme_protocol(
        scheme,
        move |context, request, responder| {
            let Ok(protocol) = request_slot.get() else {
                responder.respond(empty_response(StatusCode::SERVICE_UNAVAILABLE));
                return;
            };
            let label = context.webview_label().to_owned();
            tauri::async_runtime::spawn(async move {
                responder.respond(protocol.handle(&label, request).await);
            });
        },
    ))
}

/// Detect the first-release Desktop OS fallback without accepting arbitrary locales.
#[must_use]
pub fn detect_os_locale() -> UiLocale {
    sys_locale::get_locale().map_or(UiLocale::En, |locale| {
        let normalized = locale.to_ascii_lowercase();
        if normalized == "zh" || normalized.starts_with("zh-") || normalized.starts_with("zh_") {
            UiLocale::ZhCn
        } else {
            UiLocale::En
        }
    })
}

fn validate_index(index: &str) -> Result<(), TauriHostError> {
    if index.matches(HTML_ROOT_MARKER).count() != 1 || index.to_ascii_lowercase().contains("<base")
    {
        return Err(TauriHostError::InvalidBundle);
    }
    let lower = index.to_ascii_lowercase();
    let scripts = lower.matches("<script").count();
    if scripts != 1
        || lower.matches(" src=").count() != 1
        || !(lower.contains(" src=\"/openbot-bootstrap.mjs\"")
            || lower.contains(" src=\"./openbot-bootstrap.mjs\""))
    {
        return Err(TauriHostError::InvalidBundle);
    }
    let start = lower.find("<script").ok_or(TauriHostError::InvalidBundle)?;
    let opening_end = lower[start..]
        .find('>')
        .map(|offset| start + offset)
        .ok_or(TauriHostError::InvalidBundle)?;
    let close = lower[opening_end + 1..]
        .find("</script>")
        .map(|offset| opening_end + 1 + offset)
        .ok_or(TauriHostError::InvalidBundle)?;
    if !lower[opening_end + 1..close].trim().is_empty() {
        return Err(TauriHostError::InvalidBundle);
    }
    Ok(())
}

fn index_response(index: &str, stored: UiPreferences, os_locale: UiLocale) -> Response<Vec<u8>> {
    let theme = stored.theme.unwrap_or(UiTheme::System);
    let locale = stored.locale.unwrap_or(os_locale);
    let replacement = match theme {
        UiTheme::System => format!("<html lang=\"{}\">", locale.as_str()),
        UiTheme::Light => format!("<html lang=\"{}\" class=\"light\">", locale.as_str()),
        UiTheme::Dark => format!("<html lang=\"{}\" class=\"dark\">", locale.as_str()),
    };
    let body = index
        .replacen(HTML_ROOT_MARKER, &replacement, 1)
        .into_bytes();
    let mut response = response(StatusCode::OK, "text/html; charset=utf-8", body, true);
    response
        .headers_mut()
        .insert(CONTENT_SECURITY_POLICY, CSP.parse().expect("static CSP"));
    response
}

fn json_response<T: serde::Serialize>(value: &T) -> Response<Vec<u8>> {
    json_response_with_status(value, StatusCode::OK)
}

fn json_response_with_status<T: serde::Serialize>(
    value: &T,
    status: StatusCode,
) -> Response<Vec<u8>> {
    match serde_json::to_vec(value) {
        Ok(body) => response(status, "application/json", body, true),
        Err(_) => dependency_response(),
    }
}

fn parse_agent_body<T: serde::de::DeserializeOwned>(
    request: &mut Request<Vec<u8>>,
) -> Result<T, AgentBodyError> {
    if request.body().len() > AGENT_BODY_MAX_BYTES {
        request.body_mut().fill(0);
        return Err(AgentBodyError::TooLarge);
    }
    let parsed = serde_json::from_slice::<T>(request.body());
    request.body_mut().fill(0);
    parsed.map_err(|_| AgentBodyError::Malformed)
}

fn agent_body_error_response(error: AgentBodyError) -> Response<Vec<u8>> {
    match error {
        AgentBodyError::Malformed => error_response(AppError::MalformedPayload { field: "body" }),
        AgentBodyError::TooLarge => payload_too_large(),
    }
}

fn parse_sensitive_body<T: DeserializeOwned>(
    request: &mut Request<Vec<u8>>,
    maximum: usize,
) -> Result<T, SensitiveBodyError> {
    if request.body().len() > maximum {
        request.body_mut().fill(0);
        return Err(SensitiveBodyError::TooLarge);
    }
    let parsed = serde_json::from_slice::<T>(request.body());
    request.body_mut().fill(0);
    parsed.map_err(|_| SensitiveBodyError::Malformed)
}

fn sensitive_body_error_response(error: SensitiveBodyError) -> Response<Vec<u8>> {
    match error {
        SensitiveBodyError::Malformed => {
            error_response(AppError::MalformedPayload { field: "body" })
        }
        SensitiveBodyError::TooLarge => payload_too_large(),
    }
}

fn error_response(error: AppError) -> Response<Vec<u8>> {
    let body = serde_json::to_vec(&json!({"code": error.code().as_str()}))
        .unwrap_or_else(|_| b"{\"code\":\"dependency_unavailable\"}".to_vec());
    response(
        StatusCode::from_u16(error.http_status()).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
        "application/json",
        body,
        true,
    )
}

fn dependency_response() -> Response<Vec<u8>> {
    error_response(AppError::DependencyUnavailable {
        dependency: "application",
    })
}

fn payload_too_large() -> Response<Vec<u8>> {
    response(
        StatusCode::PAYLOAD_TOO_LARGE,
        "application/json",
        b"{\"code\":\"malformed_payload\"}".to_vec(),
        true,
    )
}

fn empty_response(status: StatusCode) -> Response<Vec<u8>> {
    response(status, "application/octet-stream", Vec::new(), true)
}

fn response(
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
    no_store: bool,
) -> Response<Vec<u8>> {
    let mut response = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(body)
        .expect("closed desktop response");
    if no_store {
        response
            .headers_mut()
            .insert(CACHE_CONTROL, "no-store".parse().expect("static no-store"));
    }
    response
}

fn closed_asset_path(path: &str) -> Option<&str> {
    let relative = path.strip_prefix('/')?;
    if relative.is_empty()
        || relative.contains('%')
        || relative.contains('\\')
        || relative
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return None;
    }
    matches!(
        Path::new(relative)
            .extension()
            .and_then(|value| value.to_str()),
        Some("css" | "js" | "mjs" | "wasm" | "woff2" | "txt")
    )
    .then_some(relative)
}

fn is_spa_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("/.well-known/")
        && !path.starts_with("/fonts/")
        && !path
            .rsplit('/')
            .next()
            .is_some_and(|segment| segment.contains('.'))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|value| value.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub(crate) fn valid_scheme(scheme: &str) -> bool {
    let mut bytes = scheme.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && scheme.len() <= 32
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"+-.".contains(&byte)
        })
        && !matches!(scheme, "http" | "https" | "tauri" | "asset")
}

fn agent_hidden_query(query: Option<&str>) -> Option<bool> {
    match query {
        None | Some("") | Some("hidden=false") => Some(false),
        Some("hidden=true") => Some(true),
        Some(_) => None,
    }
}

fn channel_list_query(query: Option<&str>) -> Option<(Option<u32>, Option<String>)> {
    let Some(query) = query else {
        return Some((None, None));
    };
    if query.is_empty() {
        return Some((None, None));
    }
    if query.len() > API_BODY_MAX_BYTES {
        return None;
    }
    let mut limit = None;
    let mut cursor = None;
    for pair in query.split('&') {
        let (raw_key, raw_value) = pair.split_once('=')?;
        let key = percent_decode_query_component(raw_key)?;
        let value = percent_decode_query_component(raw_value)?;
        match key.as_str() {
            "limit" if limit.is_none() => limit = Some(value.parse().ok()?),
            "cursor" if cursor.is_none() => cursor = Some(value),
            _ => return None,
        }
    }
    Some((limit, cursor))
}

fn percent_decode_segment(raw: &str) -> Option<String> {
    if raw.contains('/') {
        return None;
    }
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = *bytes.get(index + 1)?;
            let low = *bytes.get(index + 2)?;
            decoded.push(hex(high)? << 4 | hex(low)?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn percent_decode_query_component(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = *bytes.get(index + 1)?;
                let low = *bytes.get(index + 2)?;
                decoded.push(hex(high)? << 4 | hex(low)?);
                index += 3;
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

const fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DesktopStructuredEventFrame;
    use async_trait::async_trait;
    use openbot_application::cursor::ChannelCursor;
    use openbot_application::{
        AgentAdministration, AgentAdministrationError, AgentAdministrationScope,
        AgentCallbackTokenAdministration, AgentCallbackTokenError, AgentDirectory,
        AgentReachability, AgentReadScope, BeginThreadRunRequest, CancelThreadRunRequest,
        ChannelAdministration, ChannelAdministrationError, ChannelCreateRequest, ChannelReadScope,
        ChannelReader, ChannelRoutingBackend, ChannelRoutingBackendError, ComponentAdministration,
        ComponentAdministrationError, ComponentFunctionArguments, ComponentFunctionCallPlan,
        ComponentRuntimeScope, OpenBotApplication, PeopleAdministration, PeoplePageRequest,
        PeoplePortError, PortError, RoutingAuditRecord, RunCostBudgetAdministration,
        RunCostBudgetAdministrationError, RunCostCap, SandboxedComponentAdministration,
        SandboxedComponentAdministrationError, SandboxedComponentDraft, ThreadConversationRequest,
        ThreadDirectory, ThreadDirectoryError, ToolApprovalAdministration,
        ToolApprovalAdministrationError, UiPreferenceAdministration,
        UiPreferenceAdministrationError,
    };
    use openbot_contracts::agent::{
        AgentConnectionVerdict, AgentLifecycleReceipt, AgentLifecycleState, AgentProfile,
        AgentVisibility, CallbackTokenIssued, CallbackTokenRevoked,
    };
    use openbot_contracts::auth::{AuthGeneration, Role};
    use openbot_contracts::budget::{RunCostBudgetPreference, RunCostCapInput};
    use openbot_contracts::command::{
        ChannelDetail, ChannelPage, ChannelRoutingDecision, ChannelSummary,
        ThreadConversationSnapshot, ThreadForegroundRunState, ThreadMinted, ThreadRunAnchor,
        ThreadRunCancellation, ThreadRunStarted, ThreadStatus,
    };
    use openbot_contracts::components::{
        BOT_ACTIVITY_FUNCTION_NAME, BotActivityReport, CompiledComponentManifestEntry,
        ComponentApprovalAnswer, ComponentApprovalDecision, ComponentCatalogueAdded,
        ComponentDataFunctions, ComponentDecision, ComponentDecisionRequest, ComponentFunctionCall,
        ComponentFunctionData, ComponentGovernanceMutation, ComponentGovernanceReceipt,
        ComponentHumanDecisionAnswer, ComponentHumanDecisionResolved, ComponentRecord,
        ComponentRecords, GrantedCompiledComponent, GrantedCompiledComponents,
        PendingComponentHumanDecisions, SHOW_QUOTE_COMPONENT_NAME, compiled_component_manifest,
    };
    use openbot_contracts::ids::{ActorId, BotId, DeploymentId, TenantId};
    use openbot_contracts::people::{CurrentUser, PeoplePage, Person};
    use openbot_contracts::sandboxed::{
        PublishedSandboxedComponent, PublishedSandboxedComponents, SandboxedComponentDeleted,
        SandboxedComponentRecord, SandboxedComponentResponse, SandboxedComponents,
        SaveSandboxedComponentRequest,
    };
    use openbot_contracts::tool::{PendingToolApprovals, ToolApprovalResolved};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tauri::ipc::InvokeResponseBody;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct NeverAppStream;

    impl futures_core::Stream for NeverAppStream {
        type Item = openbot_contracts::command::AppEvent;

        fn poll_next(
            self: core::pin::Pin<&mut Self>,
            _context: &mut core::task::Context<'_>,
        ) -> core::task::Poll<Option<Self::Item>> {
            core::task::Poll::Pending
        }
    }

    struct BlockingSubscriptionService {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl openbot_application::ApplicationService for BlockingSubscriptionService {
        async fn execute(
            &self,
            _auth: AuthContext,
            _command: AppCommand,
        ) -> Result<AppReply, AppError> {
            Ok(AppReply::Health(openbot_contracts::command::HealthReport {
                ok: true,
            }))
        }

        async fn subscribe(
            &self,
            _auth: AuthContext,
            _request: SubscriptionRequest,
        ) -> Result<openbot_application::AppEventStream, AppError> {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(Box::pin(NeverAppStream))
        }
    }

    #[derive(Clone)]
    struct FakeChannelRuntime {
        inner: Arc<FakeChannelRuntimeInner>,
    }

    struct FakeChannelRuntimeInner {
        channels: Mutex<Vec<ChannelDetail>>,
        threads: Mutex<Vec<ThreadId>>,
        runs: Mutex<BTreeMap<(ThreadId, RunId), FakePersistedRun>>,
        next_channel: AtomicU64,
        next_thread: AtomicU64,
    }

    #[derive(Clone)]
    struct FakePersistedRun {
        command: BeginThreadRun,
        receipt: ThreadRunStarted,
        cancelled: bool,
    }

    impl FakeChannelRuntime {
        fn new() -> Self {
            let thread_id = ThreadId::new("550e8400-e29b-81d4-a716-446655440001");
            Self {
                inner: Arc::new(FakeChannelRuntimeInner {
                    channels: Mutex::new(vec![ChannelDetail {
                        id: ChannelId::new("channel-one"),
                        name: "Agent One".to_owned(),
                        agent_ids: vec![BotId::new("agent-one")],
                        thread_id: Some(thread_id.clone()),
                        active: true,
                    }]),
                    threads: Mutex::new(vec![thread_id]),
                    runs: Mutex::new(BTreeMap::new()),
                    next_channel: AtomicU64::new(1),
                    next_thread: AtomicU64::new(2),
                }),
            }
        }

        fn authorize_scope(
            deployment: &DeploymentId,
            tenant: &TenantId,
            actor: &ActorId,
        ) -> Result<(), ThreadDirectoryError> {
            if deployment.as_str() != "dep"
                || tenant.as_str() != "tenant"
                || actor.as_str() != "actor"
            {
                return Err(ThreadDirectoryError::NotVisible);
            }
            Ok(())
        }

        fn thread_id(sequence: u64) -> ThreadId {
            ThreadId::new(format!("550e8400-e29b-81d4-a716-{sequence:012x}"))
        }

        fn summary(detail: &ChannelDetail) -> ChannelSummary {
            ChannelSummary {
                id: detail.id.clone(),
                name: detail.name.clone(),
                agent_ids: detail.agent_ids.clone(),
                last_message: None,
                last_message_at: None,
                last_message_agent_id: None,
                created_at: time::OffsetDateTime::UNIX_EPOCH,
                thread_id: detail.thread_id.clone(),
                active: detail.active,
            }
        }
    }

    #[async_trait]
    impl ChannelReader for FakeChannelRuntime {
        async fn list_visible_channels(
            &self,
            actor: &ActorId,
            limit: u32,
            cursor: Option<ChannelCursor>,
        ) -> Result<Vec<ChannelSummary>, PortError> {
            if actor.as_str() != "actor" {
                return Err(PortError::Unavailable {
                    dependency: "fixture_channels",
                });
            }
            if cursor.is_some() {
                return Ok(Vec::new());
            }
            Ok(self
                .inner
                .channels
                .lock()
                .unwrap()
                .iter()
                .take(limit as usize)
                .map(Self::summary)
                .collect())
        }

        async fn get_visible_channel(
            &self,
            scope: &ChannelReadScope,
            channel_id: &ChannelId,
        ) -> Result<Option<ChannelSummary>, PortError> {
            if scope.deployment.as_str() != "dep"
                || scope.tenant.as_str() != "tenant"
                || scope.actor.as_str() != "actor"
            {
                return Err(PortError::Unavailable {
                    dependency: "fixture_channels",
                });
            }
            Ok(self
                .inner
                .channels
                .lock()
                .unwrap()
                .iter()
                .find(|channel| &channel.id == channel_id)
                .map(Self::summary))
        }
    }

    #[async_trait]
    impl ChannelAdministration for FakeChannelRuntime {
        async fn create_channel(
            &self,
            request: ChannelCreateRequest,
        ) -> Result<ChannelDetail, ChannelAdministrationError> {
            if request.scope.deployment.as_str() != "dep"
                || request.scope.tenant.as_str() != "tenant"
                || request.scope.actor.as_str() != "actor"
            {
                return Err(ChannelAdministrationError::NotVisible);
            }
            let sequence = self.inner.next_channel.fetch_add(1, Ordering::SeqCst);
            let thread_id = Self::thread_id(self.inner.next_thread.fetch_add(1, Ordering::SeqCst));
            let channel = ChannelDetail {
                id: ChannelId::new(format!("desktop-channel-{sequence}")),
                name: request
                    .agent_ids
                    .iter()
                    .map(BotId::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                agent_ids: request.agent_ids,
                thread_id: Some(thread_id.clone()),
                active: true,
            };
            self.inner.threads.lock().unwrap().push(thread_id);
            self.inner.channels.lock().unwrap().push(channel.clone());
            Ok(channel)
        }
    }

    #[async_trait]
    impl ThreadDirectory for FakeChannelRuntime {
        async fn mint_thread_id(
            &self,
            deployment: &DeploymentId,
        ) -> Result<ThreadId, ThreadDirectoryError> {
            if deployment.as_str() != "dep" {
                return Err(ThreadDirectoryError::NotVisible);
            }
            let thread_id = Self::thread_id(self.inner.next_thread.fetch_add(1, Ordering::SeqCst));
            self.inner.threads.lock().unwrap().push(thread_id.clone());
            Ok(thread_id)
        }

        async fn thread_known(
            &self,
            deployment: &DeploymentId,
            tenant: &TenantId,
            actor: &ActorId,
            thread: &ThreadId,
        ) -> Result<bool, ThreadDirectoryError> {
            Self::authorize_scope(deployment, tenant, actor)?;
            Ok(self.inner.threads.lock().unwrap().contains(thread))
        }

        async fn begin_thread_run(
            &self,
            request: BeginThreadRunRequest,
        ) -> Result<ThreadRunStarted, ThreadDirectoryError> {
            Self::authorize_scope(&request.deployment, &request.tenant, &request.actor)?;
            if !self
                .inner
                .threads
                .lock()
                .unwrap()
                .contains(&request.command.thread_id)
            {
                return Err(ThreadDirectoryError::NotVisible);
            }
            if let ThreadRunAnchor::Channel { channel_id } = &request.command.anchor {
                let channel_matches = self.inner.channels.lock().unwrap().iter().any(|channel| {
                    &channel.id == channel_id
                        && channel.thread_id.as_ref() == Some(&request.command.thread_id)
                        && channel.agent_ids.contains(&request.command.bot_id)
                });
                if !channel_matches {
                    return Err(ThreadDirectoryError::NotVisible);
                }
            }
            let key = (
                request.command.thread_id.clone(),
                request.command.run_id.clone(),
            );
            let mut runs = self.inner.runs.lock().unwrap();
            if let Some(existing) = runs.get(&key) {
                if existing.command != request.command {
                    return Err(ThreadDirectoryError::RequestConflict);
                }
                let mut replayed = existing.receipt.clone();
                replayed.replayed = true;
                return Ok(replayed);
            }
            let receipt = ThreadRunStarted {
                thread_id: request.command.thread_id.clone(),
                run_id: request.command.run_id.clone(),
                message_sequence: 1,
                event_sequence: 1,
                replayed: false,
            };
            runs.insert(
                key,
                FakePersistedRun {
                    command: request.command,
                    receipt: receipt.clone(),
                    cancelled: false,
                },
            );
            Ok(receipt)
        }

        async fn cancel_thread_run(
            &self,
            request: CancelThreadRunRequest,
        ) -> Result<ThreadRunCancellation, ThreadDirectoryError> {
            Self::authorize_scope(&request.deployment, &request.tenant, &request.actor)?;
            let key = (
                request.command.thread_id.clone(),
                request.command.run_id.clone(),
            );
            let mut runs = self.inner.runs.lock().unwrap();
            let run = runs.get_mut(&key).ok_or(ThreadDirectoryError::NotVisible)?;
            let state = if run.cancelled {
                ThreadRunCancellationState::AlreadyRequested
            } else {
                run.cancelled = true;
                ThreadRunCancellationState::Requested
            };
            Ok(ThreadRunCancellation {
                thread_id: request.command.thread_id,
                run_id: request.command.run_id,
                state,
            })
        }

        async fn thread_conversation(
            &self,
            request: ThreadConversationRequest,
        ) -> Result<ThreadConversationSnapshot, ThreadDirectoryError> {
            Self::authorize_scope(&request.deployment, &request.tenant, &request.actor)?;
            let runs = self.inner.runs.lock().unwrap();
            let run = runs
                .values()
                .find(|run| run.command.thread_id == request.thread);
            Ok(run.map_or_else(ThreadConversationSnapshot::default, |run| {
                ThreadConversationSnapshot {
                    messages: Vec::new(),
                    active_run_id: Some(run.command.run_id.clone()),
                    active_run_state: Some(if run.cancelled {
                        ThreadForegroundRunState::Cancelling
                    } else {
                        ThreadForegroundRunState::Running
                    }),
                    active_run_cancellable: !run.cancelled,
                    active_run_text: String::new(),
                    last_event_sequence: Some(run.receipt.event_sequence),
                }
            }))
        }
    }

    struct FakeRouting;

    #[async_trait]
    impl ChannelRoutingBackend for FakeRouting {
        async fn complete(&self, _prompt: &str) -> Result<String, ChannelRoutingBackendError> {
            Ok(r#"{"agentId":"agent-one","reason":"fixture","confidence":1.0}"#.to_owned())
        }

        async fn reachable_systems(
            &self,
            agents: &[BotId],
        ) -> Result<Vec<AgentReachability>, ChannelRoutingBackendError> {
            Ok(agents
                .iter()
                .cloned()
                .map(|agent_id| AgentReachability {
                    agent_id,
                    systems: Vec::new(),
                })
                .collect())
        }

        async fn record_routing(
            &self,
            _record: RoutingAuditRecord,
        ) -> Result<(), ChannelRoutingBackendError> {
            Ok(())
        }
    }

    struct FakeAgents {
        rows: Mutex<Vec<AgentProfile>>,
        next: AtomicU64,
        callback_sequence: AtomicU64,
    }

    impl FakeAgents {
        fn new() -> Self {
            Self {
                rows: Mutex::new(vec![AgentProfile {
                    id: BotId::new("agent-one"),
                    name: "Agent One".to_owned(),
                    title: "First Agent".to_owned(),
                    role_description: "Existing standing role".to_owned(),
                    avatar_seed: "agent-one".to_owned(),
                    visibility: AgentVisibility::Public,
                    endpoint: Some("https://agent.example/ag-ui".to_owned()),
                    has_auth: false,
                    has_callback_token: false,
                    hidden: false,
                    system_owned: false,
                    can_manage: true,
                    mine: true,
                }]),
                next: AtomicU64::new(1),
                callback_sequence: AtomicU64::new(1),
            }
        }

        fn authorize(scope: &AgentAdministrationScope) -> Result<(), AgentAdministrationError> {
            if scope.tenant != TenantId::new("tenant")
                || scope.actor != ActorId::new("actor")
                || scope.auth_generation != AuthGeneration::new(1)
            {
                return Err(AgentAdministrationError::Forbidden);
            }
            Ok(())
        }
    }

    #[async_trait]
    impl AgentDirectory for FakeAgents {
        async fn list_visible_agents(
            &self,
            scope: &AgentReadScope,
            hidden: bool,
        ) -> Result<Vec<AgentProfile>, PortError> {
            if scope.tenant != TenantId::new("tenant") || scope.actor != ActorId::new("actor") {
                return Err(PortError::Unavailable {
                    dependency: "fixture_agents",
                });
            }
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|profile| profile.hidden == hidden)
                .cloned()
                .collect())
        }

        async fn get_visible_agent(
            &self,
            scope: &AgentReadScope,
            agent_id: &BotId,
        ) -> Result<Option<AgentProfile>, PortError> {
            if scope.tenant != TenantId::new("tenant") || scope.actor != ActorId::new("actor") {
                return Err(PortError::Unavailable {
                    dependency: "fixture_agents",
                });
            }
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|profile| &profile.id == agent_id)
                .cloned())
        }
    }

    #[async_trait]
    impl AgentAdministration for FakeAgents {
        async fn create_agent(
            &self,
            scope: &AgentAdministrationScope,
            request: AgentMutationRequest,
        ) -> Result<AgentProfile, AgentAdministrationError> {
            Self::authorize(scope)?;
            let sequence = self.next.fetch_add(1, Ordering::SeqCst);
            let profile = AgentProfile {
                id: BotId::new(format!("desktop-agent-{sequence}")),
                name: request.name,
                title: request.title,
                role_description: request.role_description,
                avatar_seed: format!("desktop-agent-{sequence}"),
                visibility: request.visibility,
                endpoint: request.endpoint,
                has_auth: request.auth.is_some(),
                has_callback_token: false,
                hidden: false,
                system_owned: false,
                can_manage: true,
                mine: true,
            };
            self.rows.lock().unwrap().push(profile.clone());
            Ok(profile)
        }

        async fn update_agent(
            &self,
            scope: &AgentAdministrationScope,
            agent_id: &BotId,
            request: AgentMutationRequest,
        ) -> Result<AgentProfile, AgentAdministrationError> {
            Self::authorize(scope)?;
            let mut rows = self.rows.lock().unwrap();
            let profile = rows
                .iter_mut()
                .find(|profile| &profile.id == agent_id)
                .ok_or(AgentAdministrationError::NotVisible)?;
            profile.name = request.name;
            profile.title = request.title;
            profile.role_description = request.role_description;
            profile.visibility = request.visibility;
            profile.endpoint = request.endpoint;
            if request.auth.is_some() {
                profile.has_auth = true;
            }
            if profile.endpoint.is_none() {
                profile.has_auth = false;
                profile.has_callback_token = false;
            }
            Ok(profile.clone())
        }

        async fn duplicate_agent(
            &self,
            scope: &AgentAdministrationScope,
            agent_id: &BotId,
        ) -> Result<AgentProfile, AgentAdministrationError> {
            Self::authorize(scope)?;
            let mut rows = self.rows.lock().unwrap();
            let source = rows
                .iter()
                .find(|profile| &profile.id == agent_id)
                .cloned()
                .ok_or(AgentAdministrationError::NotVisible)?;
            let sequence = self.next.fetch_add(1, Ordering::SeqCst);
            let copy = AgentProfile {
                id: BotId::new(format!("desktop-agent-{sequence}")),
                name: source.name,
                title: source.title,
                role_description: source.role_description,
                avatar_seed: source.avatar_seed,
                visibility: AgentVisibility::Private,
                endpoint: None,
                has_auth: false,
                has_callback_token: false,
                hidden: false,
                system_owned: false,
                can_manage: true,
                mine: true,
            };
            rows.push(copy.clone());
            Ok(copy)
        }

        async fn set_agent_hidden(
            &self,
            scope: &AgentAdministrationScope,
            agent_id: &BotId,
            hidden: bool,
        ) -> Result<AgentLifecycleReceipt, AgentAdministrationError> {
            Self::authorize(scope)?;
            let mut rows = self.rows.lock().unwrap();
            let profile = rows
                .iter_mut()
                .find(|profile| &profile.id == agent_id)
                .ok_or(AgentAdministrationError::NotVisible)?;
            profile.hidden = hidden;
            Ok(AgentLifecycleReceipt {
                agent_id: agent_id.clone(),
                state: if hidden {
                    AgentLifecycleState::Hidden
                } else {
                    AgentLifecycleState::Visible
                },
            })
        }

        async fn delete_agent(
            &self,
            scope: &AgentAdministrationScope,
            agent_id: &BotId,
        ) -> Result<AgentLifecycleReceipt, AgentAdministrationError> {
            Self::authorize(scope)?;
            let mut rows = self.rows.lock().unwrap();
            let index = rows
                .iter()
                .position(|profile| &profile.id == agent_id)
                .ok_or(AgentAdministrationError::NotVisible)?;
            rows.remove(index);
            Ok(AgentLifecycleReceipt {
                agent_id: agent_id.clone(),
                state: AgentLifecycleState::Deleted,
            })
        }

        async fn test_agent_connection(
            &self,
            scope: &AgentAdministrationScope,
            _request: AgentConnectionTestRequest,
        ) -> Result<AgentConnectionVerdict, AgentAdministrationError> {
            Self::authorize(scope)?;
            Ok(AgentConnectionVerdict::working(vec![
                "RUN_STARTED".to_owned(),
            ]))
        }
    }

    struct FakeCallbackTokens(Arc<FakeAgents>);

    #[async_trait]
    impl AgentCallbackTokenAdministration for FakeCallbackTokens {
        async fn issue(
            &self,
            auth: &AuthContext,
            agent: &BotId,
        ) -> Result<CallbackTokenIssued, AgentCallbackTokenError> {
            if auth.tenant() != &TenantId::new("tenant")
                || auth.actor() != &ActorId::new("actor")
                || auth.auth_generation() != AuthGeneration::new(1)
            {
                return Err(AgentCallbackTokenError::NotVisible);
            }
            let mut rows = self.0.rows.lock().unwrap();
            let profile = rows
                .iter_mut()
                .find(|profile| &profile.id == agent && profile.endpoint.is_some())
                .ok_or(AgentCallbackTokenError::NotVisible)?;
            profile.has_callback_token = true;
            let sequence = self.0.callback_sequence.fetch_add(1, Ordering::SeqCst);
            CallbackTokenIssued::new(format!("obot_agt_DESKTOP_CALLBACK_{sequence:032}"))
                .map_err(|_| AgentCallbackTokenError::Corrupt { field: "token" })
        }

        async fn revoke(
            &self,
            auth: &AuthContext,
            agent: &BotId,
        ) -> Result<CallbackTokenRevoked, AgentCallbackTokenError> {
            if auth.tenant() != &TenantId::new("tenant")
                || auth.actor() != &ActorId::new("actor")
                || auth.auth_generation() != AuthGeneration::new(1)
            {
                return Err(AgentCallbackTokenError::NotVisible);
            }
            let mut rows = self.0.rows.lock().unwrap();
            let profile = rows
                .iter_mut()
                .find(|profile| &profile.id == agent && profile.endpoint.is_some())
                .ok_or(AgentCallbackTokenError::NotVisible)?;
            profile.has_callback_token = false;
            Ok(CallbackTokenRevoked)
        }
    }

    struct FakePeople;

    #[async_trait]
    impl PeopleAdministration for FakePeople {
        async fn current_user(&self, actor: &ActorId) -> Result<CurrentUser, PeoplePortError> {
            Ok(CurrentUser {
                id: actor.clone(),
                email: "desktop@example.test".to_owned(),
                name: Some("Desktop User".to_owned()),
                image: None,
                role: Role::Admin,
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

    struct FakePreferences(Mutex<UiPreferences>);

    #[async_trait]
    impl UiPreferenceAdministration for FakePreferences {
        async fn get(
            &self,
            _auth: &AuthContext,
        ) -> Result<UiPreferences, UiPreferenceAdministrationError> {
            Ok(*self.0.lock().unwrap())
        }

        async fn update(
            &self,
            _auth: &AuthContext,
            update: UpdateUiPreferences,
        ) -> Result<UiPreferences, UiPreferenceAdministrationError> {
            let mut stored = self.0.lock().unwrap();
            stored.theme = update.theme.or(stored.theme);
            stored.locale = update.locale.or(stored.locale);
            Ok(*stored)
        }
    }

    struct FakeRunCostBudgets(Mutex<Option<RunCostCap>>);

    #[async_trait]
    impl RunCostBudgetAdministration for FakeRunCostBudgets {
        async fn get(
            &self,
            _auth: &AuthContext,
        ) -> Result<Option<RunCostCap>, RunCostBudgetAdministrationError> {
            Ok(self.0.lock().unwrap().clone())
        }

        async fn replace(
            &self,
            _auth: &AuthContext,
            cap: Option<RunCostCap>,
        ) -> Result<Option<RunCostCap>, RunCostBudgetAdministrationError> {
            *self.0.lock().unwrap() = cap.clone();
            Ok(cap)
        }
    }

    struct FakeApprovals;

    struct FakeComponents;

    struct FakeSandboxed;

    #[async_trait]
    impl SandboxedComponentAdministration for FakeSandboxed {
        async fn list_sandboxed_components(
            &self,
            auth: &AuthContext,
        ) -> Result<SandboxedComponents, SandboxedComponentAdministrationError> {
            Ok(SandboxedComponents {
                components: vec![sandboxed_draft_record(auth.actor().as_str())],
            })
        }

        async fn list_published_sandboxed_components(
            &self,
            _auth: &AuthContext,
        ) -> Result<PublishedSandboxedComponents, SandboxedComponentAdministrationError> {
            Ok(PublishedSandboxedComponents {
                components: vec![PublishedSandboxedComponent {
                    name: "custom_delivery_eta".to_owned(),
                    html: "<p>ETA</p>".to_owned(),
                    css: "p{}".to_owned(),
                    js_functions: "function draw(){}".to_owned(),
                    argument_schema: BTreeMap::new(),
                }],
            })
        }

        async fn save_sandboxed_component(
            &self,
            auth: &AuthContext,
            draft: &SandboxedComponentDraft,
        ) -> Result<SandboxedComponentRecord, SandboxedComponentAdministrationError> {
            Ok(SandboxedComponentRecord {
                name: draft.name.clone(),
                title: draft.title.clone(),
                draft_description: draft.description.clone(),
                draft_html: draft.html.clone(),
                draft_css: draft.css.clone(),
                draft_js_functions: draft.js_functions.clone(),
                draft_argument_schema: draft.argument_schema.clone(),
                published_html: None,
                published_css: None,
                published_js_functions: None,
                published_argument_schema: None,
                sample_arguments: draft.sample_arguments.clone(),
                revision: 0,
                published: false,
                published_at: None,
                authored_by: Some(auth.actor().as_str().to_owned()),
                has_unpublished_changes: false,
            })
        }

        async fn publish_sandboxed_component(
            &self,
            auth: &AuthContext,
            _component_name: &str,
        ) -> Result<SandboxedComponentRecord, SandboxedComponentAdministrationError> {
            let mut record = sandboxed_draft_record(auth.actor().as_str());
            record.published_html = Some(record.draft_html.clone());
            record.published_css = Some(record.draft_css.clone());
            record.published_js_functions = Some(record.draft_js_functions.clone());
            record.published_argument_schema = Some(record.draft_argument_schema.clone());
            record.revision = 1;
            record.published = true;
            record.published_at = Some(time::OffsetDateTime::UNIX_EPOCH);
            Ok(record)
        }

        async fn delete_sandboxed_component(
            &self,
            _auth: &AuthContext,
            _component_name: &str,
        ) -> Result<(), SandboxedComponentAdministrationError> {
            Ok(())
        }
    }

    fn sandboxed_draft_record(actor: &str) -> SandboxedComponentRecord {
        SandboxedComponentRecord {
            name: "custom_delivery_eta".to_owned(),
            title: "Delivery ETA".to_owned(),
            draft_description: "Delivery estimate".to_owned(),
            draft_html: "<p>ETA</p>".to_owned(),
            draft_css: "p{}".to_owned(),
            draft_js_functions: "function draw(){}".to_owned(),
            draft_argument_schema: BTreeMap::new(),
            published_html: None,
            published_css: None,
            published_js_functions: None,
            published_argument_schema: None,
            sample_arguments: BTreeMap::new(),
            revision: 0,
            published: false,
            published_at: None,
            authored_by: Some(actor.to_owned()),
            has_unpublished_changes: false,
        }
    }

    #[async_trait]
    impl ComponentAdministration for FakeComponents {
        async fn list_components(
            &self,
            _auth: &AuthContext,
        ) -> Result<ComponentRecords, ComponentAdministrationError> {
            Ok(ComponentRecords::default())
        }

        async fn sync_catalogue(
            &self,
            _auth: &AuthContext,
            entries: &[CompiledComponentManifestEntry],
        ) -> Result<ComponentCatalogueAdded, ComponentAdministrationError> {
            Ok(ComponentCatalogueAdded {
                added: entries.iter().map(|entry| entry.name.clone()).collect(),
            })
        }

        async fn update_component_governance(
            &self,
            auth: &AuthContext,
            mutation: &ComponentGovernanceMutation,
        ) -> Result<ComponentRecord, ComponentAdministrationError> {
            let mut component = ComponentRecord {
                name: mutation.component_name().to_owned(),
                title: "Quotation".to_owned(),
                kind: openbot_contracts::components::CompiledComponentKind::Card,
                draft_description: "quote".to_owned(),
                published_description: Some("quote".to_owned()),
                published: true,
                published_at: Some(time::OffsetDateTime::UNIX_EPOCH),
                updated_by: Some(auth.actor().as_str().to_owned()),
                updated_at: time::OffsetDateTime::UNIX_EPOCH,
                has_unpublished_changes: false,
                withheld_from: Vec::new(),
                functions: Vec::new(),
            };
            match mutation {
                ComponentGovernanceMutation::SetAgentGrant {
                    agent_id, granted, ..
                } if !granted => component.withheld_from.push(agent_id.as_str().to_owned()),
                ComponentGovernanceMutation::SetFunctionGrant {
                    function, granted, ..
                } if *granted => component.functions.push(function.clone()),
                ComponentGovernanceMutation::SetPublication { published, .. } => {
                    component.published = *published;
                }
                ComponentGovernanceMutation::SaveDraft { description, .. } => {
                    component.draft_description = description.clone();
                    component.has_unpublished_changes = true;
                }
                _ => {}
            }
            Ok(component)
        }

        async fn list_components_for_agent(
            &self,
            _scope: &ComponentRuntimeScope,
            _renderer_names: &[String],
        ) -> Result<GrantedCompiledComponents, ComponentAdministrationError> {
            Ok(GrantedCompiledComponents {
                components: vec![GrantedCompiledComponent {
                    name: SHOW_QUOTE_COMPONENT_NAME.to_owned(),
                    description: "published quote".to_owned(),
                }],
            })
        }

        async fn decide_component(
            &self,
            _scope: &ComponentRuntimeScope,
            _component_name: &str,
            _build_has_renderer: bool,
            _functions: &[String],
        ) -> Result<ComponentDecision, ComponentAdministrationError> {
            Ok(ComponentDecision::allowed())
        }

        async fn call_component_function(
            &self,
            _scope: &ComponentRuntimeScope,
            _component_name: &str,
            _build_has_renderer: bool,
            plan: &ComponentFunctionCallPlan,
        ) -> Result<ComponentFunctionCall, ComponentAdministrationError> {
            let days = match plan.arguments {
                Some(ComponentFunctionArguments::BotActivity { days }) => days,
                _ => return Err(ComponentAdministrationError::Corrupt { field: "arguments" }),
            };
            Ok(ComponentFunctionCall::succeeded(
                ComponentFunctionData::BotActivity(BotActivityReport {
                    days,
                    rows: Vec::new(),
                }),
            ))
        }

        async fn list_component_human_decisions(
            &self,
            _auth: &AuthContext,
        ) -> Result<PendingComponentHumanDecisions, ComponentAdministrationError> {
            Ok(PendingComponentHumanDecisions::default())
        }

        async fn resolve_component_human_decision(
            &self,
            _auth: &AuthContext,
            decision_id: &str,
            answer: &ComponentHumanDecisionAnswer,
        ) -> Result<ComponentHumanDecisionResolved, ComponentAdministrationError> {
            Ok(ComponentHumanDecisionResolved {
                decision_id: decision_id.to_owned(),
                answer: answer.clone(),
                replayed: false,
            })
        }
    }

    #[async_trait]
    impl ToolApprovalAdministration for FakeApprovals {
        async fn list_pending(
            &self,
            _auth: &AuthContext,
        ) -> Result<PendingToolApprovals, ToolApprovalAdministrationError> {
            Ok(PendingToolApprovals {
                approvals: Vec::new(),
            })
        }

        async fn decide(
            &self,
            _auth: &AuthContext,
            approval_id: &str,
            decision: ToolApprovalDecision,
        ) -> Result<ToolApprovalResolved, ToolApprovalAdministrationError> {
            Ok(ToolApprovalResolved {
                approval_id: approval_id.to_owned(),
                decision,
            })
        }
    }

    fn auth() -> AuthContext {
        AuthContext::for_test(
            DeploymentId::new("dep"),
            TenantId::new("tenant"),
            ActorId::new("actor"),
            [Role::User],
            AuthGeneration::new(1),
            true,
        )
    }

    fn admin_auth() -> AuthContext {
        AuthContext::for_test(
            DeploymentId::new("dep"),
            TenantId::new("tenant"),
            ActorId::new("admin"),
            [Role::Admin],
            AuthGeneration::new(1),
            true,
        )
    }

    fn protocol_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "openbot-tauri-protocol-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("index.html"),
            "<!doctype html><html lang=\"en\"><head><script type=\"module\" src=\"/openbot-bootstrap.mjs\"></script></head><body></body></html>",
        )
        .unwrap();
        fs::write(root.join("openbot-bootstrap.mjs"), "export {};").unwrap();
        root
    }

    fn protocol() -> (Arc<DesktopTauriProtocol>, PathBuf) {
        let root = protocol_root();
        let preferences = Arc::new(FakePreferences(Mutex::new(UiPreferences {
            theme: Some(UiTheme::Dark),
            locale: Some(UiLocale::ZhCn),
        })));
        let agents = Arc::new(FakeAgents::new());
        let channels = FakeChannelRuntime::new();
        let application = Arc::new(
            OpenBotApplication::new(channels.clone())
                .with_channel_administration(Arc::new(channels.clone()))
                .with_channel_routing(Arc::new(FakeRouting))
                .with_threads(channels)
                .with_people(FakePeople)
                .with_agent_callback_tokens(FakeCallbackTokens(agents.clone()))
                .with_agent_directory(agents.clone())
                .with_agent_administration(agents)
                .with_ui_preferences(preferences)
                .with_run_cost_budgets(Arc::new(FakeRunCostBudgets(Mutex::new(None))))
                .with_component_administration(Arc::new(FakeComponents))
                .with_sandboxed_component_administration(Arc::new(FakeSandboxed))
                .with_tool_approvals(Arc::new(FakeApprovals)),
        );
        let transport = Arc::new(InProcessTransport::new(application));
        let protocol = Arc::new(DesktopTauriProtocol::open(&root, transport).unwrap());
        (protocol, root)
    }

    #[test]
    fn window_binding_identity_exhaustion_fails_without_authority() {
        let (protocol, root) = protocol();
        protocol
            .next_window_binding_id
            .store(u64::MAX, Ordering::SeqCst);
        assert!(matches!(
            protocol.bind_window("main", auth(), None),
            Err(TauriHostError::WindowBindingCounterExhausted)
        ));
        assert!(protocol.authority("main").unwrap().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registration_installs_the_closed_wry_protocol_and_command_handler() {
        let (protocol, root) = protocol();
        let builder = Builder::<tauri::Wry>::default();
        assert!(register_tauri_protocol(builder, "openbot", protocol).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "desktop-local-runtime")]
    #[test]
    fn deferred_protocol_slot_is_not_ready_then_install_once() {
        let (protocol, root) = protocol();
        let slot = DesktopTauriProtocolSlot::pending();
        assert!(matches!(slot.get(), Err(TauriHostError::ProtocolNotReady)));
        slot.install(Arc::clone(&protocol)).unwrap();
        assert!(Arc::ptr_eq(&slot.get().unwrap(), &protocol));
        assert!(matches!(
            slot.install(protocol),
            Err(TauriHostError::ProtocolAlreadyReady)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn command_audit_list_and_generated_handler_are_exactly_joined() {
        let source = include_str!("tauri_host.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        assert_eq!(production.matches("#[tauri::command]").count(), 2);
        let handler = production
            .split("tauri::generate_handler![")
            .nth(1)
            .and_then(|tail| tail.split("])").next())
            .expect("generated handler must remain explicit");
        for command in DESKTOP_TAURI_COMMANDS {
            assert!(
                production.contains(&format!("fn {command}(")),
                "missing command function {command}"
            );
            assert!(handler.contains(command), "handler omitted {command}");
        }
        assert_eq!(
            handler
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .count(),
            DESKTOP_TAURI_COMMANDS.len()
        );
    }

    #[test]
    fn scheme_asset_and_percent_decoders_are_closed() {
        assert_eq!(
            DESKTOP_TAURI_COMMANDS,
            [
                "openbot_structured_events_open",
                "openbot_structured_events_close"
            ]
        );
        assert!(valid_scheme("app-ui"));
        for invalid in ["", "OpenBot", "http", "tauri", "has space", "a/b"] {
            assert!(!valid_scheme(invalid), "{invalid}");
        }
        assert_eq!(closed_asset_path("/app-1.css"), Some("app-1.css"));
        for invalid in ["/../secret", "/a%2Fb.js", "/missing", "/a//b.js"] {
            assert_eq!(closed_asset_path(invalid), None, "{invalid}");
        }
        assert_eq!(
            percent_decode_segment("approval%2Fone"),
            Some("approval/one".to_owned())
        );
        assert_eq!(percent_decode_segment("bad%2"), None);
        assert_eq!(percent_decode_segment("raw/slash"), None);
        assert_eq!(agent_hidden_query(None), Some(false));
        assert_eq!(agent_hidden_query(Some("")), Some(false));
        assert_eq!(agent_hidden_query(Some("hidden=false")), Some(false));
        assert_eq!(agent_hidden_query(Some("hidden=true")), Some(true));
        assert_eq!(agent_hidden_query(Some("hidden=1")), None);
        assert!(matches!(
            agent_route("/api/agents/agent%2Done/duplicate"),
            Some(AgentRoute::Duplicate { .. })
        ));
        assert!(agent_route("/api/agents/agent/one").is_none());
        assert_eq!(
            raw_channel_id("/api/channels/channel%2Done"),
            Some("channel%2Done")
        );
        assert!(raw_channel_id("/api/channels/events").is_none());
        assert!(raw_channel_id("/api/channels/channel/one").is_none());
        assert!(matches!(
            thread_route("/api/threads/thread%2Done/runs/run%2Done/cancel"),
            Some(ThreadRoute::Cancel { .. })
        ));
        assert!(thread_route("/api/threads/mint").is_none());
        assert!(thread_route("/api/threads/thread/one/runs").is_none());
        assert_eq!(channel_list_query(None), Some((None, None)));
        assert_eq!(
            channel_list_query(Some("cursor=opaque%2B%2F%3D&limit=50")),
            Some((Some(50), Some("opaque+/=".to_owned())))
        );
        for invalid in [
            "limit=-1",
            "limit=1&limit=2",
            "cursor=a&cursor=b",
            "unknown=value",
            "limit",
            "limit=%GG",
        ] {
            assert_eq!(channel_list_query(Some(invalid)), None, "{invalid}");
        }
    }

    #[test]
    fn structured_command_errors_are_stable_and_hide_internal_labels() {
        let duplicate = TauriHostError::StructuredSubscription(
            DesktopStructuredOpenError::Session(OpenSessionError::WindowAlreadyOpen(
                crate::WindowAlreadyOpen(WindowLabel::new("private-internal-label")),
            )),
        );
        assert_eq!(
            tauri_host_error_code(&duplicate),
            "desktop_subscription_conflict"
        );
        assert!(!tauri_host_error_code(&duplicate).contains("private"));
        assert_eq!(
            tauri_host_error_code(&TauriHostError::StructuredSubscription(
                DesktopStructuredOpenError::WindowBudgetExhausted,
            )),
            "structured_subscription_window_budget_exhausted"
        );
        assert_eq!(
            tauri_host_error_code(&TauriHostError::StructuredSubscription(
                DesktopStructuredOpenError::WindowClosed,
            )),
            "desktop_window_unbound"
        );
        let application =
            TauriHostError::StructuredSubscription(DesktopStructuredOpenError::Session(
                OpenSessionError::Application(AppError::NotVisible),
            ));
        assert_eq!(tauri_host_error_code(&application), "not_visible");
    }

    #[test]
    fn agent_body_parser_zeroes_success_malformed_and_oversized_buffers() {
        let mut valid = Request::builder()
            .body(
                br#"{"endpoint":"https://agent.example/ag-ui","auth":{"header":"Authorization","value":"Bearer DESKTOP_SECRET"}}"#
                    .to_vec(),
            )
            .unwrap();
        let parsed = parse_agent_body::<AgentConnectionTestRequest>(&mut valid).unwrap();
        assert!(parsed.auth.is_some());
        assert!(valid.body().iter().all(|byte| *byte == 0));

        let mut malformed = Request::builder()
            .body(b"{DESKTOP_SECRET".to_vec())
            .unwrap();
        assert!(matches!(
            parse_agent_body::<AgentConnectionTestRequest>(&mut malformed),
            Err(AgentBodyError::Malformed)
        ));
        assert!(malformed.body().iter().all(|byte| *byte == 0));

        let mut oversized = Request::builder()
            .body(vec![b'x'; AGENT_BODY_MAX_BYTES + 1])
            .unwrap();
        assert!(matches!(
            parse_agent_body::<AgentConnectionTestRequest>(&mut oversized),
            Err(AgentBodyError::TooLarge)
        ));
        assert!(oversized.body().iter().all(|byte| *byte == 0));
    }

    #[test]
    fn sensitive_body_parser_zeroes_user_text_on_every_exit() {
        let mut valid = Request::builder()
            .body(br#"{"text":"DESKTOP_MESSAGE_SECRET","agentId":"agent-one"}"#.to_vec())
            .unwrap();
        let parsed =
            parse_sensitive_body::<RouteChannelRequest>(&mut valid, CHANNEL_THREAD_BODY_MAX_BYTES)
                .unwrap();
        assert_eq!(parsed.text, "DESKTOP_MESSAGE_SECRET");
        assert!(valid.body().iter().all(|byte| *byte == 0));

        let mut malformed = Request::builder()
            .body(b"{DESKTOP_MESSAGE_SECRET".to_vec())
            .unwrap();
        assert!(matches!(
            parse_sensitive_body::<RouteChannelRequest>(
                &mut malformed,
                CHANNEL_THREAD_BODY_MAX_BYTES,
            ),
            Err(SensitiveBodyError::Malformed)
        ));
        assert!(malformed.body().iter().all(|byte| *byte == 0));

        let mut oversized = Request::builder()
            .body(vec![b'x'; CHANNEL_THREAD_BODY_MAX_BYTES + 1])
            .unwrap();
        assert!(matches!(
            parse_sensitive_body::<RouteChannelRequest>(
                &mut oversized,
                CHANNEL_THREAD_BODY_MAX_BYTES,
            ),
            Err(SensitiveBodyError::TooLarge)
        ));
        assert!(oversized.body().iter().all(|byte| *byte == 0));
    }

    #[tokio::test]
    async fn an_inflight_old_binding_cannot_attach_to_a_recreated_window_label() {
        let root = protocol_root();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let service = Arc::new(BlockingSubscriptionService {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        });
        let transport = Arc::new(InProcessTransport::new(service));
        let protocol = Arc::new(DesktopTauriProtocol::open(&root, transport).unwrap());
        protocol.bind_window("main", auth(), None).unwrap();

        let opening = Arc::clone(&protocol);
        let task = tokio::spawn(async move {
            opening
                .open_structured_subscription("main", SubscriptionRequest::Health)
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .expect("old binding must enter application subscribe");
        assert!(protocol.unbind_window("main").unwrap());
        protocol.bind_window("main", admin_auth(), None).unwrap();
        release.notify_one();

        let outcome = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("stale open must finish")
            .unwrap();
        assert!(matches!(outcome, Err(TauriHostError::WindowUnbound)));
        assert_eq!(protocol.transport.broker().window_count(), 0);
        assert_eq!(protocol.structured_events.active_subscription_count(), 0);
        assert!(protocol.unbind_window("main").unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn exact_close_is_scoped_to_the_host_observed_window() {
        let (protocol, root) = protocol();
        protocol.bind_window("main", auth(), None).unwrap();
        protocol.bind_window("auxiliary", auth(), None).unwrap();
        let mut main = protocol
            .open_structured_subscription("main", SubscriptionRequest::Health)
            .await
            .unwrap();
        let auxiliary = protocol
            .open_structured_subscription("auxiliary", SubscriptionRequest::Health)
            .await
            .unwrap();
        assert_eq!(protocol.transport.broker().window_count(), 2);

        assert!(
            !protocol
                .close_structured_subscription("main", auxiliary.subscription_id())
                .unwrap()
        );
        assert!(
            protocol
                .close_structured_subscription("main", main.subscription_id())
                .unwrap()
        );
        assert!(
            !protocol
                .close_structured_subscription("main", main.subscription_id())
                .unwrap()
        );
        assert!(matches!(
            protocol.close_structured_subscription("unbound", auxiliary.subscription_id()),
            Err(TauriHostError::WindowUnbound)
        ));
        assert_eq!(protocol.transport.broker().window_count(), 1);

        let terminal = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let frame = main.next_frame().await.unwrap().unwrap();
                if frame.terminal_reason().is_some() {
                    break frame;
                }
            }
        })
        .await
        .expect("exact close must wake the selected subscription");
        assert_eq!(
            terminal.terminal_reason(),
            Some(crate::DesktopStructuredTerminalReason::SubscriptionClosed)
        );
        assert_eq!(protocol.structured_events.active_subscription_count(), 1);

        drop(auxiliary);
        assert_eq!(protocol.transport.broker().window_count(), 0);
        assert_eq!(protocol.structured_events.active_subscription_count(), 0);
        assert!(protocol.unbind_window("main").unwrap());
        assert!(protocol.unbind_window("auxiliary").unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn structured_channel_uses_bound_authority_and_unbind_closes_the_route() {
        let (protocol, root) = protocol();
        assert!(matches!(
            protocol
                .open_structured_subscription("main", SubscriptionRequest::Health)
                .await,
            Err(TauriHostError::WindowUnbound)
        ));
        assert_eq!(protocol.transport.broker().window_count(), 0);

        protocol.bind_window("main", auth(), None).unwrap();
        let frames = Arc::new(Mutex::new(Vec::<DesktopStructuredEventFrame>::new()));
        let sink = Arc::clone(&frames);
        let channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                panic!("structured frame must use Tauri's JSON IPC lane");
            };
            let frame_json = serde_json::from_str::<String>(&json)?;
            sink.lock()
                .unwrap()
                .push(serde_json::from_str(&frame_json)?);
            Ok(())
        });
        let running = Arc::clone(&protocol);
        let pump = tokio::spawn(async move {
            running
                .pump_structured_events("main", SubscriptionRequest::Health, channel)
                .await
        });

        tokio::time::timeout(Duration::from_secs(2), async {
            while protocol.transport.broker().window_count() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("host-owned structured route must open");
        assert_eq!(protocol.structured_events.active_subscription_count(), 1);
        assert!(protocol.unbind_window("main").unwrap());
        assert_eq!(protocol.transport.broker().window_count(), 0);
        assert_eq!(protocol.structured_events.active_subscription_count(), 0);

        let exit = tokio::time::timeout(Duration::from_secs(2), pump)
            .await
            .expect("window close must wake a pending structured pump")
            .unwrap()
            .unwrap();
        assert_eq!(
            exit,
            DesktopStructuredPumpExit::Terminal(
                crate::DesktopStructuredTerminalReason::SubscriptionClosed
            )
        );
        assert!(matches!(
            frames.lock().unwrap().last(),
            Some(DesktopStructuredEventFrame::Terminal {
                reason: crate::DesktopStructuredTerminalReason::SubscriptionClosed,
                ..
            })
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn channel_and_thread_unary_routes_share_typed_application_without_fake_streaming() {
        const SECRET: &str = "DESKTOP_CHANNEL_MESSAGE_SECRET";
        let (protocol, root) = protocol();
        // Server uses OriginAuthenticated, not the fresh-session extractor, for these writes.
        // The custom scheme is intrinsically same-origin, so an ordinary bound window must work.
        protocol.bind_window("channel", auth(), None).unwrap();

        let list = protocol
            .handle(
                "channel",
                Request::builder()
                    .uri("/api/channels?limit=50")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(list.status(), StatusCode::OK);
        assert_eq!(list.headers()[CACHE_CONTROL], "no-store");
        let page = serde_json::from_slice::<ChannelPage>(list.body()).unwrap();
        assert_eq!(page.channels.len(), 1);
        assert_eq!(page.channels[0].id.as_str(), "channel-one");

        for query in ["unknown=value", "limit=-1", "limit=1&limit=2"] {
            let rejected = protocol
                .handle(
                    "channel",
                    Request::builder()
                        .uri(format!("/api/channels?{query}"))
                        .body(Vec::new())
                        .unwrap(),
                )
                .await;
            assert_eq!(rejected.status(), StatusCode::BAD_REQUEST, "{query}");
        }

        let created = protocol
            .handle(
                "channel",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/channels")
                    .body(br#"{"agentIds":["agent-one"]}"#.to_vec())
                    .unwrap(),
            )
            .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = serde_json::from_slice::<ChannelDetailResponse>(created.body())
            .unwrap()
            .channel;
        assert_eq!(created.id.as_str(), "desktop-channel-1");
        assert_eq!(created.agent_ids.as_slice(), [BotId::new("agent-one")]);
        let thread_id = created.thread_id.clone().unwrap();

        let detail = protocol
            .handle(
                "channel",
                Request::builder()
                    .uri("/api/channels/desktop-channel-1")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(detail.status(), StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<ChannelDetailResponse>(detail.body())
                .unwrap()
                .channel,
            created
        );

        let routing = protocol
            .handle(
                "channel",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/route")
                    .body(format!(r#"{{"text":"{SECRET}","agentId":"agent-one"}}"#).into_bytes())
                    .unwrap(),
            )
            .await;
        assert_eq!(routing.status(), StatusCode::OK);
        assert!(!String::from_utf8_lossy(routing.body()).contains(SECRET));
        let routing = serde_json::from_slice::<ChannelRoutingDecision>(routing.body()).unwrap();
        assert_eq!(routing.agent_id.as_str(), "agent-one");
        assert!(routing.via_mention);

        let minted = protocol
            .handle(
                "channel",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/threads/mint")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(minted.status(), StatusCode::OK);
        let minted = serde_json::from_slice::<ThreadMinted>(minted.body()).unwrap();
        let status = protocol
            .handle(
                "channel",
                Request::builder()
                    .uri(format!("/api/threads/{}", minted.thread_id.as_str()))
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(status.status(), StatusCode::OK);
        assert!(
            serde_json::from_slice::<ThreadStatus>(status.body())
                .unwrap()
                .known
        );

        let run_id = RunId::new("desktop-run-1");
        let begin_body = serde_json::to_vec(&BeginThreadRunBody {
            run_id: run_id.clone(),
            bot_id: BotId::new("agent-one"),
            anchor: ThreadRunAnchor::Channel {
                channel_id: created.id.clone(),
            },
            message: SECRET.to_owned(),
        })
        .unwrap();
        let begin_path = format!("/api/threads/{}/runs", thread_id.as_str());
        let started = protocol
            .handle(
                "channel",
                Request::builder()
                    .method(Method::POST)
                    .uri(&begin_path)
                    .body(begin_body.clone())
                    .unwrap(),
            )
            .await;
        assert_eq!(started.status(), StatusCode::CREATED);
        assert!(!String::from_utf8_lossy(started.body()).contains(SECRET));
        let started = serde_json::from_slice::<ThreadRunStarted>(started.body()).unwrap();
        assert_eq!(started.thread_id, thread_id);
        assert_eq!(started.run_id, run_id);
        assert!(!started.replayed);

        let replayed = protocol
            .handle(
                "channel",
                Request::builder()
                    .method(Method::POST)
                    .uri(&begin_path)
                    .body(begin_body)
                    .unwrap(),
            )
            .await;
        assert_eq!(replayed.status(), StatusCode::OK);
        assert!(
            serde_json::from_slice::<ThreadRunStarted>(replayed.body())
                .unwrap()
                .replayed
        );

        let conversation_path = format!("/api/threads/{}/conversation", started.thread_id.as_str());
        let conversation = protocol
            .handle(
                "channel",
                Request::builder()
                    .uri(&conversation_path)
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(conversation.status(), StatusCode::OK);
        let conversation =
            serde_json::from_slice::<ThreadConversationSnapshot>(conversation.body()).unwrap();
        assert_eq!(conversation.active_run_id.as_ref(), Some(&run_id));
        assert_eq!(
            conversation.active_run_state,
            Some(ThreadForegroundRunState::Running)
        );
        assert!(conversation.active_run_cancellable);

        let cancel_path = format!(
            "/api/threads/{}/runs/{}/cancel",
            started.thread_id.as_str(),
            run_id.as_str()
        );
        let cancelled = protocol
            .handle(
                "channel",
                Request::builder()
                    .method(Method::POST)
                    .uri(&cancel_path)
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(cancelled.status(), StatusCode::ACCEPTED);
        assert_eq!(
            serde_json::from_slice::<ThreadRunCancellation>(cancelled.body())
                .unwrap()
                .state,
            ThreadRunCancellationState::Requested
        );
        let cancelled_again = protocol
            .handle(
                "channel",
                Request::builder()
                    .method(Method::POST)
                    .uri(&cancel_path)
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(cancelled_again.status(), StatusCode::ACCEPTED);
        assert_eq!(
            serde_json::from_slice::<ThreadRunCancellation>(cancelled_again.body())
                .unwrap()
                .state,
            ThreadRunCancellationState::AlreadyRequested
        );

        let cancelling = protocol
            .handle(
                "channel",
                Request::builder()
                    .uri(&conversation_path)
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        let cancelling =
            serde_json::from_slice::<ThreadConversationSnapshot>(cancelling.body()).unwrap();
        assert_eq!(
            cancelling.active_run_state,
            Some(ThreadForegroundRunState::Cancelling)
        );
        assert!(!cancelling.active_run_cancellable);

        let malformed = protocol
            .handle(
                "channel",
                Request::builder()
                    .method(Method::POST)
                    .uri(&begin_path)
                    .body(format!("{{malformed:{SECRET}").into_bytes())
                    .unwrap(),
            )
            .await;
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        assert!(!String::from_utf8_lossy(malformed.body()).contains(SECRET));

        let oversized = protocol
            .handle(
                "channel",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/route")
                    .body(vec![b'x'; CHANNEL_THREAD_BODY_MAX_BYTES + 1])
                    .unwrap(),
            )
            .await;
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let ignored_mint_body = protocol
            .handle(
                "channel",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/threads/mint")
                    .body(SECRET.as_bytes().to_vec())
                    .unwrap(),
            )
            .await;
        assert_eq!(ignored_mint_body.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert!(!String::from_utf8_lossy(ignored_mint_body.body()).contains(SECRET));

        let thread_events = protocol
            .handle(
                "channel",
                Request::builder()
                    .uri(format!(
                        "/api/threads/{}/events",
                        started.thread_id.as_str()
                    ))
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(thread_events.status(), StatusCode::NOT_FOUND);
        let channel_events = protocol
            .handle(
                "channel",
                Request::builder()
                    .uri("/api/channels/events")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(channel_events.status(), StatusCode::NOT_FOUND);

        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn agent_routes_share_typed_application_freshness_and_secret_free_framing() {
        const SECRET: &str = "DESKTOP_AGENT_SECRET_CANARY";
        let (protocol, root) = protocol();
        protocol.bind_window("agents", auth(), None).unwrap();

        let list = protocol
            .handle(
                "agents",
                Request::builder()
                    .uri("/api/agents")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(list.status(), StatusCode::OK);
        assert_eq!(list.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(
            serde_json::from_slice::<AgentProfilesResponse>(list.body())
                .unwrap()
                .agents
                .len(),
            1
        );

        let detail = protocol
            .handle(
                "agents",
                Request::builder()
                    .uri("/api/agents/agent-one")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(detail.status(), StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<AgentProfileResponse>(detail.body())
                .unwrap()
                .agent
                .id
                .as_str(),
            "agent-one"
        );

        let stale = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/agents")
                    .body(format!("{{malformed:{SECRET}").into_bytes())
                    .unwrap(),
            )
            .await;
        assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);
        assert!(!String::from_utf8_lossy(stale.body()).contains(SECRET));

        let stale_callback = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/agents/agent-one/callback-token")
                    .body(SECRET.as_bytes().to_vec())
                    .unwrap(),
            )
            .await;
        assert_eq!(stale_callback.status(), StatusCode::UNAUTHORIZED);
        assert!(!String::from_utf8_lossy(stale_callback.body()).contains(SECRET));

        assert!(protocol.unbind_window("agents").unwrap());
        protocol
            .bind_window("agents", auth(), Some(Duration::from_secs(60)))
            .unwrap();

        let issued = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/agents/agent-one/callback-token")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(issued.status(), StatusCode::CREATED);
        assert_eq!(issued.headers()[CACHE_CONTROL], "no-store");
        let first_token = serde_json::from_slice::<CallbackTokenIssued>(issued.body())
            .unwrap()
            .expose()
            .to_owned();
        assert!(first_token.starts_with("obot_agt_"));
        let issued_again = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/agents/agent-one/callback-token")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(issued_again.status(), StatusCode::CREATED);
        let second_token = serde_json::from_slice::<CallbackTokenIssued>(issued_again.body())
            .unwrap()
            .expose()
            .to_owned();
        assert_ne!(first_token, second_token);
        assert!(!String::from_utf8_lossy(issued_again.body()).contains(&first_token));
        let callback_profile = protocol
            .handle(
                "agents",
                Request::builder()
                    .uri("/api/agents/agent-one")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert!(
            serde_json::from_slice::<AgentProfileResponse>(callback_profile.body())
                .unwrap()
                .agent
                .has_callback_token
        );
        let callback_body_rejected = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/agents/agent-one/callback-token")
                    .body(SECRET.as_bytes().to_vec())
                    .unwrap(),
            )
            .await;
        assert_eq!(
            callback_body_rejected.status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert!(!String::from_utf8_lossy(callback_body_rejected.body()).contains(SECRET));
        let revoked = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/agents/agent-one/callback-token")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
        assert!(revoked.body().is_empty());
        let revoked_profile = protocol
            .handle(
                "agents",
                Request::builder()
                    .uri("/api/agents/agent-one")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert!(
            !serde_json::from_slice::<AgentProfileResponse>(revoked_profile.body())
                .unwrap()
                .agent
                .has_callback_token
        );

        let bad_query = protocol
            .handle(
                "agents",
                Request::builder()
                    .uri("/api/agents?hidden=yes")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(bad_query.status(), StatusCode::BAD_REQUEST);

        let forged = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/agents")
                    .body(br#"{"name":"Forged","title":"Title","roleDescription":"Role","visibility":"private","ownerUserId":"admin"}"#.to_vec())
                    .unwrap(),
            )
            .await;
        assert_eq!(forged.status(), StatusCode::BAD_REQUEST);

        let oversized = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/agents")
                    .body(vec![b'x'; AGENT_BODY_MAX_BYTES + 1])
                    .unwrap(),
            )
            .await;
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let create_body = format!(
            "{{\"name\":\"Remote\",\"title\":\"Remote title\",\"roleDescription\":\"Remote role\",\"visibility\":\"public\",\"endpoint\":\"https://agent.example/ag-ui\",\"auth\":{{\"header\":\"Authorization\",\"value\":\"Bearer {SECRET}\"}}}}"
        );
        let created = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/agents")
                    .body(create_body.into_bytes())
                    .unwrap(),
            )
            .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        assert_eq!(created.headers()[CACHE_CONTROL], "no-store");
        assert!(!String::from_utf8_lossy(created.body()).contains(SECRET));
        let created = serde_json::from_slice::<AgentProfileResponse>(created.body())
            .unwrap()
            .agent;
        assert_eq!(created.id.as_str(), "desktop-agent-1");
        assert!(created.has_auth);

        let probe = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/agents/test-connection")
                    .body(
                        format!(
                            "{{\"endpoint\":\"https://agent.example/ag-ui\",\"auth\":{{\"header\":\"Authorization\",\"value\":\"Bearer {SECRET}\"}}}}"
                        )
                        .into_bytes(),
                    )
                    .unwrap(),
            )
            .await;
        assert_eq!(probe.status(), StatusCode::OK);
        assert!(!String::from_utf8_lossy(probe.body()).contains(SECRET));
        assert_eq!(
            serde_json::from_slice::<AgentConnectionVerdict>(probe.body()).unwrap(),
            AgentConnectionVerdict::working(vec!["RUN_STARTED".to_owned()])
        );

        let updated = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::PATCH)
                    .uri("/api/agents/desktop-agent-1")
                    .body(br#"{"name":"Remote updated","title":"Updated title","roleDescription":"Updated role","visibility":"public","endpoint":"https://agent.example/ag-ui-v2"}"#.to_vec())
                    .unwrap(),
            )
            .await;
        assert_eq!(updated.status(), StatusCode::OK);
        let updated = serde_json::from_slice::<AgentProfileResponse>(updated.body())
            .unwrap()
            .agent;
        assert!(updated.has_auth);
        assert_eq!(
            updated.endpoint.as_deref(),
            Some("https://agent.example/ag-ui-v2")
        );

        let duplicated = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/agents/desktop-agent-1/duplicate")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(duplicated.status(), StatusCode::CREATED);
        let duplicate = serde_json::from_slice::<AgentProfileResponse>(duplicated.body())
            .unwrap()
            .agent;
        assert_eq!(duplicate.visibility, AgentVisibility::Private);
        assert!(!duplicate.has_auth);
        assert!(duplicate.endpoint.is_none());

        let hidden = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/agents/desktop-agent-1/hide")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(hidden.status(), StatusCode::NO_CONTENT);
        assert!(hidden.body().is_empty());

        let default_roster = protocol
            .handle(
                "agents",
                Request::builder()
                    .uri("/api/agents?hidden=false")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert!(
            serde_json::from_slice::<AgentProfilesResponse>(default_roster.body())
                .unwrap()
                .agents
                .iter()
                .all(|agent| agent.id.as_str() != "desktop-agent-1")
        );
        let hidden_roster = protocol
            .handle(
                "agents",
                Request::builder()
                    .uri("/api/agents?hidden=true")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(
            serde_json::from_slice::<AgentProfilesResponse>(hidden_roster.body())
                .unwrap()
                .agents[0]
                .id
                .as_str(),
            "desktop-agent-1"
        );

        let unhidden = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/agents/desktop-agent-1/unhide")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(unhidden.status(), StatusCode::NO_CONTENT);

        let deleted = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/agents/desktop-agent-1")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
        let missing = protocol
            .handle(
                "agents",
                Request::builder()
                    .uri("/api/agents/desktop-agent-1")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        let wrong_method = protocol
            .handle(
                "agents",
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/agents")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn bound_window_gets_rewritten_bundle_and_typed_preferences_and_approval() {
        let (protocol, root) = protocol();
        let unbound = protocol
            .handle(
                "main",
                Request::builder().uri("/").body(Vec::new()).unwrap(),
            )
            .await;
        assert_eq!(unbound.status(), StatusCode::UNAUTHORIZED);

        protocol.bind_window("main", auth(), None).unwrap();
        let index = protocol
            .handle(
                "main",
                Request::builder()
                    .uri("/approvals")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(index.status(), StatusCode::OK);
        assert_eq!(index.headers()[CONTENT_SECURITY_POLICY], CSP);
        assert!(
            String::from_utf8(index.body().clone())
                .unwrap()
                .contains("lang=\"zh-CN\" class=\"dark\"")
        );

        let current_user = protocol
            .handle(
                "main",
                Request::builder().uri("/api/me").body(Vec::new()).unwrap(),
            )
            .await;
        assert_eq!(current_user.status(), StatusCode::OK);
        assert_eq!(current_user.headers()[CACHE_CONTROL], "no-store");
        let current_user =
            serde_json::from_slice::<CurrentUserResponse>(current_user.body()).unwrap();
        assert_eq!(current_user.user.id.as_str(), "actor");
        assert_eq!(current_user.user.email, "desktop@example.test");
        assert_eq!(current_user.user.role, Role::Admin);

        let update = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/me/preferences")
                    .body(br#"{"theme":"light"}"#.to_vec())
                    .unwrap(),
            )
            .await;
        assert_eq!(update.status(), StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<UiPreferences>(update.body()).unwrap(),
            UiPreferences {
                theme: Some(UiTheme::Light),
                locale: Some(UiLocale::ZhCn),
            }
        );

        let budget = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/me/run-cost-budget")
                    .body(br#"{"cap":{"currency":"USD","maxCostMicroUnits":"250000"}}"#.to_vec())
                    .unwrap(),
            )
            .await;
        assert_eq!(budget.status(), StatusCode::OK);
        assert_eq!(budget.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(
            serde_json::from_slice::<RunCostBudgetPreference>(budget.body()).unwrap(),
            RunCostBudgetPreference {
                cap: Some(RunCostCapInput {
                    currency: "USD".to_owned(),
                    max_cost_micro_units: "250000".to_owned(),
                }),
            }
        );
        let smuggled = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/me/run-cost-budget")
                    .body(
                        br#"{"cap":{"currency":"USD","maxCostMicroUnits":"1","actor":"admin"}}"#
                            .to_vec(),
                    )
                    .unwrap(),
            )
            .await;
        assert_eq!(smuggled.status(), StatusCode::BAD_REQUEST);

        let approvals = protocol
            .handle(
                "main",
                Request::builder()
                    .uri("/api/tool-approvals")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(approvals.status(), StatusCode::OK);
        assert_eq!(approvals.body(), br#"{"approvals":[]}"#);

        let components = protocol
            .handle(
                "main",
                Request::builder()
                    .uri("/api/components")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(components.status(), StatusCode::OK);
        assert_eq!(components.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(components.body(), br#"{"components":[]}"#);

        let catalogue = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/components/catalogue")
                    .body(
                        serde_json::to_vec(&ComponentCatalogueRequest {
                            components: compiled_component_manifest(),
                        })
                        .unwrap(),
                    )
                    .unwrap(),
            )
            .await;
        assert_eq!(catalogue.status(), StatusCode::OK);
        assert_eq!(catalogue.headers()[CACHE_CONTROL], "no-store");
        let expected = compiled_component_manifest()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(
            serde_json::from_slice::<ComponentCatalogueAdded>(catalogue.body())
                .unwrap()
                .added,
            expected
        );

        let runtime_grants = protocol
            .handle(
                "main",
                Request::builder()
                    .uri("/api/components/for-agent/agent%2Done")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(runtime_grants.status(), StatusCode::OK);
        assert_eq!(runtime_grants.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(
            serde_json::from_slice::<GrantedCompiledComponents>(runtime_grants.body())
                .unwrap()
                .components[0]
                .name,
            SHOW_QUOTE_COMPONENT_NAME
        );

        let component_decision = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/components/showQuote/decision")
                    .body(
                        serde_json::to_vec(&ComponentDecisionRequest {
                            agent_id: BotId::new("agent-one"),
                            functions: Vec::new(),
                        })
                        .unwrap(),
                    )
                    .unwrap(),
            )
            .await;
        assert_eq!(component_decision.status(), StatusCode::OK);
        assert_eq!(component_decision.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(
            serde_json::from_slice::<ComponentDecision>(component_decision.body()).unwrap(),
            ComponentDecision::allowed()
        );

        let component_functions = protocol
            .handle(
                "main",
                Request::builder()
                    .uri("/api/components/functions")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(component_functions.status(), StatusCode::OK);
        assert_eq!(component_functions.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(
            serde_json::from_slice::<ComponentDataFunctions>(component_functions.body())
                .unwrap()
                .functions[0]
                .name,
            BOT_ACTIVITY_FUNCTION_NAME
        );

        let component_call = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/components/showActivityReport/call")
                    .body(
                        serde_json::to_vec(&ComponentFunctionCallRequest {
                            agent_id: BotId::new("agent-one"),
                            function: BOT_ACTIVITY_FUNCTION_NAME.to_owned(),
                            args: serde_json::json!({"days": 14}),
                        })
                        .unwrap(),
                    )
                    .unwrap(),
            )
            .await;
        assert_eq!(component_call.status(), StatusCode::OK);
        assert_eq!(component_call.headers()[CACHE_CONTROL], "no-store");
        assert!(matches!(
            serde_json::from_slice::<ComponentFunctionCall>(component_call.body())
                .unwrap()
                .data,
            Some(ComponentFunctionData::BotActivity(BotActivityReport {
                days: 14,
                ..
            }))
        ));

        let human_decisions = protocol
            .handle(
                "main",
                Request::builder()
                    .uri("/api/components/human-decisions")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(human_decisions.status(), StatusCode::OK);
        assert_eq!(human_decisions.headers()[CACHE_CONTROL], "no-store");
        assert!(
            serde_json::from_slice::<PendingComponentHumanDecisions>(human_decisions.body())
                .unwrap()
                .decisions
                .is_empty()
        );

        let human_answer = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/components/human-decisions/decision%2D1/answer")
                    .body(
                        serde_json::to_vec(&ComponentHumanDecisionAnswer::Approval(
                            ComponentApprovalAnswer {
                                decision: ComponentApprovalDecision::Approved,
                                note: None,
                            },
                        ))
                        .unwrap(),
                    )
                    .unwrap(),
            )
            .await;
        assert_eq!(human_answer.status(), StatusCode::OK);
        assert_eq!(human_answer.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(
            serde_json::from_slice::<ComponentHumanDecisionResolved>(human_answer.body())
                .unwrap()
                .decision_id,
            "decision-1"
        );

        let stale = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/tool-approvals/approval-1")
                    .body(br#"{"decision":"grant"}"#.to_vec())
                    .unwrap(),
            )
            .await;
        assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);

        assert!(protocol.unbind_window("main").unwrap());
        protocol
            .bind_window("main", auth(), Some(Duration::from_secs(60)))
            .unwrap();
        let granted = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/tool-approvals/approval%2D1")
                    .body(br#"{"decision":"grant"}"#.to_vec())
                    .unwrap(),
            )
            .await;
        assert_eq!(granted.status(), StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<ToolApprovalResolved>(granted.body()).unwrap(),
            ToolApprovalResolved {
                approval_id: "approval-1".to_owned(),
                decision: ToolApprovalDecision::Grant,
            }
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn component_governance_routes_share_typed_application_and_require_fresh_admin() {
        let (protocol, root) = protocol();
        protocol.bind_window("admin", admin_auth(), None).unwrap();
        let stale = protocol
            .handle(
                "admin",
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/components/showQuote/draft")
                    .body(br#"{"description":"edited"}"#.to_vec())
                    .unwrap(),
            )
            .await;
        assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);
        assert!(protocol.unbind_window("admin").unwrap());
        protocol
            .bind_window("admin", admin_auth(), Some(Duration::from_secs(60)))
            .unwrap();

        let requests = [
            (
                Method::POST,
                "/api/components/showQuote/functions",
                br#"{"function":"botActivity"}"#.to_vec(),
            ),
            (
                Method::DELETE,
                "/api/components/showQuote/functions/botActivity",
                Vec::new(),
            ),
            (
                Method::POST,
                "/api/components/showQuote/grants",
                br#"{"agentId":"agent-one"}"#.to_vec(),
            ),
            (
                Method::DELETE,
                "/api/components/showQuote/grants/agent-one",
                Vec::new(),
            ),
            (
                Method::POST,
                "/api/components/showQuote/publication",
                br#"{"published":false}"#.to_vec(),
            ),
            (
                Method::PUT,
                "/api/components/showQuote/draft",
                br#"{"description":"edited quote"}"#.to_vec(),
            ),
        ];
        for (method, uri, body) in requests {
            let response = protocol
                .handle(
                    "admin",
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(body)
                        .unwrap(),
                )
                .await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
            assert_eq!(
                serde_json::from_slice::<ComponentGovernanceReceipt>(response.body())
                    .unwrap()
                    .component
                    .name,
                SHOW_QUOTE_COMPONENT_NAME
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn sandboxed_routes_share_typed_application_and_require_fresh_admin_for_writes() {
        let (protocol, root) = protocol();
        protocol.bind_window("main", auth(), None).unwrap();
        let published = protocol
            .handle(
                "main",
                Request::builder()
                    .uri("/api/sandboxed/published")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(published.status(), StatusCode::OK);
        let published =
            serde_json::from_slice::<PublishedSandboxedComponents>(published.body()).unwrap();
        assert_eq!(published.components[0].name, "custom_delivery_eta");

        let drafts = protocol
            .handle(
                "main",
                Request::builder()
                    .uri("/api/sandboxed")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(drafts.status(), StatusCode::FORBIDDEN);

        assert!(protocol.unbind_window("main").unwrap());
        protocol.bind_window("main", admin_auth(), None).unwrap();
        let stale = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/sandboxed")
                    .body(b"{".to_vec())
                    .unwrap(),
            )
            .await;
        assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);

        assert!(protocol.unbind_window("main").unwrap());
        protocol
            .bind_window("main", admin_auth(), Some(Duration::from_secs(60)))
            .unwrap();
        let draft = SaveSandboxedComponentRequest {
            slug: "delivery_eta".to_owned(),
            title: "Delivery ETA".to_owned(),
            description: "Delivery estimate".to_owned(),
            html: "<p>ETA</p>".to_owned(),
            css: "p{}".to_owned(),
            js_functions: "function draw(){}".to_owned(),
            argument_schema: BTreeMap::new(),
            sample_arguments: BTreeMap::new(),
        };
        let saved = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/sandboxed")
                    .body(serde_json::to_vec(&draft).unwrap())
                    .unwrap(),
            )
            .await;
        assert_eq!(saved.status(), StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<SandboxedComponentResponse>(saved.body())
                .unwrap()
                .component
                .name,
            "custom_delivery_eta"
        );

        let published = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/sandboxed/custom_delivery_eta/publish")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(published.status(), StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<SandboxedComponentResponse>(published.body())
                .unwrap()
                .component
                .revision,
            1
        );

        let deleted = protocol
            .handle(
                "main",
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/sandboxed/custom_delivery_eta")
                    .body(Vec::new())
                    .unwrap(),
            )
            .await;
        assert_eq!(deleted.status(), StatusCode::OK);
        assert!(
            serde_json::from_slice::<SandboxedComponentDeleted>(deleted.body())
                .unwrap()
                .ok
        );
        fs::remove_dir_all(root).unwrap();
    }
}
