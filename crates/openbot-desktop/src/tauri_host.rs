//! Tauri 2.11.5 custom-protocol adapter for the shared Leptos bundle.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE};
use http::{Method, Request, Response, StatusCode};
use openbot_contracts::auth::AuthContext;
use openbot_contracts::command::{AppCommand, AppReply};
use openbot_contracts::components::{
    ComponentCatalogueRequest, ComponentDecisionRequest, ComponentFunctionCallRequest,
    ComponentHumanDecisionAnswer,
};
use openbot_contracts::error::{AppError, SensitiveWriteReason};
use openbot_contracts::ids::BotId;
use openbot_contracts::tool::ToolApprovalDecision;
use openbot_contracts::ui::{UiLocale, UiPreferences, UiTheme, UpdateUiPreferences};
use serde_json::json;
use tauri::{Builder, Runtime};

use crate::InProcessTransport;

const INDEX_MAX_BYTES: u64 = 1024 * 1024;
const ASSET_MAX_BYTES: u64 = 8 * 1024 * 1024;
const API_BODY_MAX_BYTES: usize = 4096;
const COMPONENT_CATALOGUE_BODY_MAX_BYTES: usize = 256 * 1024;
const COMPONENT_DECISION_BODY_MAX_BYTES: usize = 256 * 1024;
const HTML_ROOT_MARKER: &str = "<html lang=\"en\">";
const CSP: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; \
                   connect-src 'self'; img-src 'self' data: blob:; font-src 'self'; \
                   object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; \
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
        if path == "/api/me/preferences" {
            return self.preferences(request, authority).await;
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
        ChannelReader, ComponentAdministration, ComponentAdministrationError,
        ComponentFunctionArguments, ComponentFunctionCallPlan, ComponentRuntimeScope,
        OpenBotApplication, PortError, ToolApprovalAdministration, ToolApprovalAdministrationError,
        UiPreferenceAdministration, UiPreferenceAdministrationError,
    };
    use openbot_contracts::auth::{AuthGeneration, Role};
    use openbot_contracts::command::ChannelSummary;
    use openbot_contracts::components::{
        BOT_ACTIVITY_FUNCTION_NAME, BotActivityReport, CompiledComponentManifestEntry,
        ComponentApprovalAnswer, ComponentApprovalDecision, ComponentCatalogueAdded,
        ComponentDataFunctions, ComponentDecision, ComponentDecisionRequest, ComponentFunctionCall,
        ComponentFunctionData, ComponentHumanDecisionAnswer, ComponentHumanDecisionResolved,
        ComponentRecords, GrantedCompiledComponent, GrantedCompiledComponents,
        PendingComponentHumanDecisions, SHOW_QUOTE_COMPONENT_NAME, compiled_component_manifest,
    };
    use openbot_contracts::ids::{ActorId, BotId, DeploymentId, TenantId};
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
        let application = Arc::new(
            OpenBotApplication::new(EmptyChannels)
                .with_ui_preferences(preferences)
                .with_component_administration(Arc::new(FakeComponents))
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
}
