//! Tauri 2.11.5 custom-protocol adapter for the shared Leptos bundle.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use http::{Method, Request, Response, StatusCode};
use openbot_contracts::agent::{
    AgentConnectionTestRequest, AgentMutationRequest, AgentProfileResponse, AgentProfilesResponse,
};
use openbot_contracts::auth::AuthContext;
use openbot_contracts::command::{AppCommand, AppReply};
use openbot_contracts::components::{
    ComponentAgentGrantRequest, ComponentCatalogueRequest, ComponentDecisionRequest,
    ComponentDraftRequest, ComponentFunctionCallRequest, ComponentFunctionGrantRequest,
    ComponentGovernanceMutation, ComponentHumanDecisionAnswer, ComponentPublicationRequest,
};
use openbot_contracts::error::{AppError, SensitiveWriteReason};
use openbot_contracts::ids::BotId;
use openbot_contracts::people::CurrentUserResponse;
use openbot_contracts::sandboxed::SaveSandboxedComponentRequest;
use openbot_contracts::tool::ToolApprovalDecision;
use openbot_contracts::ui::{UiLocale, UiPreferences, UiTheme, UpdateUiPreferences};
use serde_json::json;
use tauri::{Builder, Runtime};

use crate::InProcessTransport;

const INDEX_MAX_BYTES: u64 = 1024 * 1024;
const ASSET_MAX_BYTES: u64 = 8 * 1024 * 1024;
const API_BODY_MAX_BYTES: usize = 4096;
const AGENT_BODY_MAX_BYTES: usize = 64 * 1024;
const COMPONENT_CATALOGUE_BODY_MAX_BYTES: usize = 256 * 1024;
const COMPONENT_DECISION_BODY_MAX_BYTES: usize = 256 * 1024;
const COMPONENT_GOVERNANCE_BODY_MAX_BYTES: usize = 68 * 1024;
const SANDBOXED_COMPONENT_BODY_MAX_BYTES: usize = 1024 * 1024;
const HTML_ROOT_MARKER: &str = "<html lang=\"en\">";
const CSP: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; \
                   connect-src 'self'; img-src 'self' data: blob:; font-src 'self'; \
                   object-src 'none'; base-uri 'none'; form-action 'self'; frame-src 'self'; frame-ancestors 'none'; \
                   worker-src 'none'; manifest-src 'none'; media-src 'none'";

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
}

/// Host-verified authority for one webview label. The renderer cannot construct this type.
#[derive(Clone)]
struct WindowAuthority {
    auth: AuthContext,
    fresh_until: Option<Instant>,
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
    windows: RwLock<BTreeMap<String, WindowAuthority>>,
    os_locale: UiLocale,
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
        Ok(Self {
            root,
            index: Arc::from(index),
            transport,
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
        let fresh_until = fresh_for
            .map(|duration| {
                Instant::now()
                    .checked_add(duration)
                    .ok_or(TauriHostError::InvalidFreshness)
            })
            .transpose()?;
        windows.insert(label, WindowAuthority { auth, fresh_until });
        Ok(())
    }

    /// Remove one closed window's authority immediately.
    pub fn unbind_window(&self, label: &str) -> Result<bool, TauriHostError> {
        self.windows
            .write()
            .map(|mut windows| windows.remove(label).is_some())
            .map_err(|_| TauriHostError::AuthorityUnavailable)
    }

    /// Handle one custom-protocol request. Public for deterministic host-adapter tests.
    pub async fn handle(&self, label: &str, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
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
            return empty_response(StatusCode::NOT_FOUND);
        }
        if request.method() != Method::GET {
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

/// Register one exact caller-selected custom scheme on a Tauri builder.
pub fn register_tauri_protocol<R: Runtime>(
    builder: Builder<R>,
    scheme: &str,
    protocol: Arc<DesktopTauriProtocol>,
) -> Result<Builder<R>, TauriHostError> {
    if !valid_scheme(scheme) {
        return Err(TauriHostError::InvalidScheme);
    }
    let scheme = scheme.to_owned();
    Ok(builder.register_asynchronous_uri_scheme_protocol(
        scheme,
        move |context, request, responder| {
            let protocol = Arc::clone(&protocol);
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

fn valid_scheme(scheme: &str) -> bool {
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
    use async_trait::async_trait;
    use openbot_application::cursor::ChannelCursor;
    use openbot_application::{
        AgentAdministration, AgentAdministrationError, AgentAdministrationScope,
        AgentCallbackTokenAdministration, AgentCallbackTokenError, AgentDirectory, AgentReadScope,
        ChannelReader, ComponentAdministration, ComponentAdministrationError,
        ComponentFunctionArguments, ComponentFunctionCallPlan, ComponentRuntimeScope,
        OpenBotApplication, PeopleAdministration, PeoplePageRequest, PeoplePortError, PortError,
        SandboxedComponentAdministration, SandboxedComponentAdministrationError,
        SandboxedComponentDraft, ToolApprovalAdministration, ToolApprovalAdministrationError,
        UiPreferenceAdministration, UiPreferenceAdministrationError,
    };
    use openbot_contracts::agent::{
        AgentConnectionVerdict, AgentLifecycleReceipt, AgentLifecycleState, AgentProfile,
        AgentVisibility, CallbackTokenIssued, CallbackTokenRevoked,
    };
    use openbot_contracts::auth::{AuthGeneration, Role};
    use openbot_contracts::command::ChannelSummary;
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

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

    fn protocol() -> (Arc<DesktopTauriProtocol>, PathBuf) {
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
        let preferences = Arc::new(FakePreferences(Mutex::new(UiPreferences {
            theme: Some(UiTheme::Dark),
            locale: Some(UiLocale::ZhCn),
        })));
        let agents = Arc::new(FakeAgents::new());
        let application = Arc::new(
            OpenBotApplication::new(EmptyChannels)
                .with_people(FakePeople)
                .with_agent_callback_tokens(FakeCallbackTokens(agents.clone()))
                .with_agent_directory(agents.clone())
                .with_agent_administration(agents)
                .with_ui_preferences(preferences)
                .with_component_administration(Arc::new(FakeComponents))
                .with_sandboxed_component_administration(Arc::new(FakeSandboxed))
                .with_tool_approvals(Arc::new(FakeApprovals)),
        );
        let transport = Arc::new(InProcessTransport::new(application));
        let protocol = Arc::new(DesktopTauriProtocol::open(&root, transport).unwrap());
        (protocol, root)
    }

    #[test]
    fn scheme_asset_and_percent_decoders_are_closed() {
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
