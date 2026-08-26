//! Same-origin browser transport for typed GUI APIs.
//!
//! This module only performs HTTP framing. Approval binding, actor resolution and the durable
//! decision remain behind `ApplicationService` on the Server.

use openbot_contracts::agent::AgentProfile;
#[cfg(target_arch = "wasm32")]
use openbot_contracts::agent::{AgentProfileResponse, AgentProfilesResponse};
#[cfg(target_arch = "wasm32")]
use openbot_contracts::command::{
    BeginThreadRunBody, CreateChannelRequest, RouteChannelRequest, ThreadRunAnchor,
};
use openbot_contracts::command::{
    ChannelDetail, ChannelPage, ChannelRoutingDecision, ThreadRunStarted,
};
use openbot_contracts::ids::{BotId, ChannelId, RunId, ThreadId};
use openbot_contracts::people::CurrentUser;
#[cfg(target_arch = "wasm32")]
use openbot_contracts::people::CurrentUserResponse;
use openbot_contracts::tool::{PendingToolApprovals, ToolApprovalDecision, ToolApprovalResolved};
use openbot_contracts::ui::{SessionStatus, UiPreferences, UpdateUiPreferences};

/// Stable, payload-free failure categories suitable for localized presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// The browser could not complete the request.
    Network,
    /// The current session is absent or expired.
    Unauthorized,
    /// The actor is not allowed to access the approval.
    Forbidden,
    /// The authenticated actor cannot see the requested resource.
    NotFound,
    /// The approval was concurrently resolved or invalidated.
    Conflict,
    /// The Server response did not match the closed contract.
    InvalidResponse,
    /// The Server returned another unsuccessful status.
    Server,
    /// The browser-only API was called by a non-WASM target.
    Unavailable,
}

/// Roster page size; the application still owns the authoritative 1..=200 clamp.
pub const CHANNEL_PAGE_SIZE: u32 = 50;

/// Create one private channel for a single URL-selected recipient.
pub async fn create_channel(agent_id: &BotId) -> Result<ChannelDetail, ApiError> {
    validate_agent_id(agent_id.as_str())?;
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let request = Request::post("/api/channels")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .json(&CreateChannelRequest {
                agent_ids: vec![agent_id.clone()],
            })
            .map_err(|_| ApiError::InvalidResponse)?;
        let response = request.send().await.map_err(|_| ApiError::Network)?;
        if response.status() != 201 {
            return Err(status_error(response.status()));
        }
        let channel = response
            .json::<openbot_contracts::command::ChannelDetailResponse>()
            .await
            .map(|response| response.channel)
            .map_err(|_| ApiError::InvalidResponse)?;
        if channel.agent_ids.as_slice() != [agent_id.clone()]
            || channel.thread_id.is_none()
            || !channel.active
        {
            return Err(ApiError::InvalidResponse);
        }
        Ok(channel)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = agent_id;
        Err(ApiError::Unavailable)
    }
}

/// Validate or infer one first-message recipient through the native routing API.
pub async fn route_channel_message(
    text: &str,
    agent_id: Option<&BotId>,
) -> Result<ChannelRoutingDecision, ApiError> {
    if let Some(agent_id) = agent_id {
        validate_agent_id(agent_id.as_str())?;
    }
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let request = Request::post("/api/route")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .json(&RouteChannelRequest {
                text: text.to_owned(),
                agent_id: agent_id.cloned(),
            })
            .map_err(|_| ApiError::InvalidResponse)?;
        let response = request.send().await.map_err(|_| ApiError::Network)?;
        if response.status() != 200 {
            return Err(status_error(response.status()));
        }
        let decision = response
            .json::<ChannelRoutingDecision>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        if let Some(explicit) = agent_id
            && (decision.agent_id != *explicit || !decision.via_mention)
        {
            return Err(ApiError::InvalidResponse);
        }
        Ok(decision)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (text, agent_id);
        Err(ApiError::Unavailable)
    }
}

/// Begin the first durable native run for a channel-created thread.
pub async fn begin_channel_run(
    thread_id: &ThreadId,
    channel_id: &ChannelId,
    agent_id: &BotId,
    run_id: &RunId,
    message: &str,
) -> Result<ThreadRunStarted, ApiError> {
    validate_channel_id(channel_id.as_str())?;
    validate_agent_id(agent_id.as_str())?;
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = thread_run_path(thread_id.as_str())?;
        let request = Request::post(&path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .json(&BeginThreadRunBody {
                run_id: run_id.clone(),
                bot_id: agent_id.clone(),
                anchor: ThreadRunAnchor::Channel {
                    channel_id: channel_id.clone(),
                },
                message: message.to_owned(),
            })
            .map_err(|_| ApiError::InvalidResponse)?;
        let response = request.send().await.map_err(|_| ApiError::Network)?;
        let status = response.status();
        if !matches!(status, 200 | 201) {
            return Err(status_error(status));
        }
        let started = response
            .json::<ThreadRunStarted>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        if started.thread_id != *thread_id
            || started.run_id != *run_id
            || started.replayed != (status == 200)
        {
            return Err(ApiError::InvalidResponse);
        }
        Ok(started)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (thread_id, channel_id, agent_id, run_id, message);
        Err(ApiError::Unavailable)
    }
}

/// Mint one browser-CSPRNG-backed durable idempotency key.
#[must_use]
pub fn mint_run_id() -> RunId {
    RunId::new(uuid::Uuid::now_v7().to_string())
}

/// Load the current actor's authoritative visible or per-user-hidden coworker roster.
pub async fn list_agents(hidden: bool) -> Result<Vec<AgentProfile>, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = if hidden {
            "/api/agents?hidden=true"
        } else {
            "/api/agents"
        };
        let response = Request::get(path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        response
            .json::<AgentProfilesResponse>()
            .await
            .map(|response| response.agents)
            .map_err(|_| ApiError::InvalidResponse)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = hidden;
        Err(ApiError::Unavailable)
    }
}

/// Load one current-actor-visible coworker profile.
pub async fn load_agent(agent_id: &str) -> Result<AgentProfile, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = agent_detail_path(agent_id)?;
        let response = Request::get(&path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        response
            .json::<AgentProfileResponse>()
            .await
            .map(|response| response.agent)
            .map_err(|_| ApiError::InvalidResponse)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = agent_id;
        Err(ApiError::Unavailable)
    }
}

/// Load one authoritative channel roster page.
pub async fn list_channels(cursor: Option<&str>) -> Result<ChannelPage, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = channel_list_path(cursor)?;
        let response = Request::get(&path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        let page = response
            .json::<ChannelPage>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        if page.channels.len() > CHANNEL_PAGE_SIZE as usize {
            return Err(ApiError::InvalidResponse);
        }
        Ok(page)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = cursor;
        Err(ApiError::Unavailable)
    }
}

/// Load one membership-visible channel detail.
pub async fn load_channel(channel_id: &str) -> Result<ChannelDetail, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = channel_detail_path(channel_id)?;
        let response = Request::get(&path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        response
            .json::<openbot_contracts::command::ChannelDetailResponse>()
            .await
            .map(|response| response.channel)
            .map_err(|_| ApiError::InvalidResponse)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = channel_id;
        Err(ApiError::Unavailable)
    }
}

/// Load the current authenticated user for the sidebar footer.
pub async fn load_current_user() -> Result<CurrentUser, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let response = Request::get("/api/me")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        response
            .json::<CurrentUserResponse>()
            .await
            .map(|response| response.user)
            .map_err(|_| ApiError::InvalidResponse)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(ApiError::Unavailable)
    }
}

/// Load at most the Server-bounded current-actor approval page without using browser cache.
pub async fn list_pending_tool_approvals() -> Result<PendingToolApprovals, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let response = Request::get("/api/tool-approvals")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        let page = response
            .json::<PendingToolApprovals>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        if page.approvals.len() > 100 {
            return Err(ApiError::InvalidResponse);
        }
        Ok(page)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(ApiError::Unavailable)
    }
}

/// Commit one grant or denial for a Server-minted id; no binding fields are accepted here.
pub async fn decide_tool_approval(
    approval_id: &str,
    decision: ToolApprovalDecision,
) -> Result<ToolApprovalResolved, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = approval_decision_path(approval_id)?;
        let body = serde_json::json!({"decision": decision});
        let request = Request::post(&path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .json(&body)
            .map_err(|_| ApiError::InvalidResponse)?;
        let response = request.send().await.map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        let receipt = response
            .json::<ToolApprovalResolved>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        if receipt.approval_id != approval_id || receipt.decision != decision {
            return Err(ApiError::InvalidResponse);
        }
        Ok(receipt)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (approval_id, decision);
        Err(ApiError::Unavailable)
    }
}

/// Read authenticated stored UI preferences; absent fields preserve the host-rendered fallback.
pub async fn load_ui_preferences() -> Result<UiPreferences, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let response = Request::get("/api/me/preferences")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        response
            .json::<UiPreferences>()
            .await
            .map_err(|_| ApiError::InvalidResponse)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(ApiError::Unavailable)
    }
}

/// Merge one or both closed preference fields through the authenticated same-origin API.
pub async fn save_ui_preferences(update: UpdateUiPreferences) -> Result<UiPreferences, ApiError> {
    if update.is_empty() {
        return Err(ApiError::InvalidResponse);
    }
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let request = Request::put("/api/me/preferences")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .json(&update)
            .map_err(|_| ApiError::InvalidResponse)?;
        let response = request.send().await.map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        let stored = response
            .json::<UiPreferences>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        if update.theme.is_some() && stored.theme != update.theme
            || update.locale.is_some() && stored.locale != update.locale
        {
            return Err(ApiError::InvalidResponse);
        }
        Ok(stored)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(ApiError::Unavailable)
    }
}

/// Discover whether the current authenticated host uses one revocable database session.
pub async fn load_session_status() -> Result<SessionStatus, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let response = Request::get("/api/me/session")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        response
            .json::<SessionStatus>()
            .await
            .map_err(|_| ApiError::InvalidResponse)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(ApiError::Unavailable)
    }
}

/// Revoke exactly the current database session; navigation happens only after the 204 receipt.
pub async fn sign_out_current_session() -> Result<(), ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let response = Request::post("/api/auth/sign-out")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if response.status() != 204 {
            return Err(status_error(response.status()));
        }
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(ApiError::Unavailable)
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn approval_decision_path(approval_id: &str) -> Result<String, ApiError> {
    if approval_id.is_empty() || approval_id.len() > 128 || approval_id.as_bytes().contains(&0) {
        return Err(ApiError::InvalidResponse);
    }
    Ok(format!(
        "/api/tool-approvals/{}",
        encode_url_component(approval_id)
    ))
}

#[cfg(any(target_arch = "wasm32", test))]
fn channel_detail_path(channel_id: &str) -> Result<String, ApiError> {
    validate_channel_id(channel_id)?;
    Ok(format!(
        "/api/channels/{}",
        encode_url_component(channel_id)
    ))
}

#[cfg(any(target_arch = "wasm32", test))]
fn thread_run_path(thread_id: &str) -> Result<String, ApiError> {
    validate_thread_id(thread_id)?;
    Ok(format!(
        "/api/threads/{}/runs",
        encode_url_component(thread_id)
    ))
}

#[cfg(any(target_arch = "wasm32", test))]
fn agent_detail_path(agent_id: &str) -> Result<String, ApiError> {
    validate_agent_id(agent_id)?;
    Ok(format!("/api/agents/{}", encode_url_component(agent_id)))
}

/// Build the URL-owned profile-panel route for one validated Agent identity.
pub fn agent_profile_href(agent_id: &str) -> Result<String, ApiError> {
    validate_agent_id(agent_id)?;
    Ok(format!("/agents?agent={}", encode_url_component(agent_id)))
}

fn validate_agent_id(agent_id: &str) -> Result<(), ApiError> {
    if agent_id.is_empty() || agent_id.len() > 512 || agent_id.chars().any(char::is_control) {
        Err(ApiError::InvalidResponse)
    } else {
        Ok(())
    }
}

/// Build one same-origin UI route for a validated channel identity.
pub fn channel_route_href(channel_id: &str) -> Result<String, ApiError> {
    validate_channel_id(channel_id)?;
    Ok(format!("/channel/{}", encode_url_component(channel_id)))
}

/// Build the URL-owned new-channel route for one selected Agent.
pub fn channel_new_href(agent_id: &str) -> Result<String, ApiError> {
    validate_agent_id(agent_id)?;
    Ok(format!(
        "/channel/new?agent={}",
        encode_url_component(agent_id)
    ))
}

fn validate_channel_id(channel_id: &str) -> Result<(), ApiError> {
    if channel_id.is_empty() || channel_id.len() > 512 || channel_id.chars().any(char::is_control) {
        Err(ApiError::InvalidResponse)
    } else {
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_thread_id(thread_id: &str) -> Result<(), ApiError> {
    if thread_id.is_empty() || thread_id.len() > 512 || thread_id.chars().any(char::is_control) {
        Err(ApiError::InvalidResponse)
    } else {
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn channel_list_path(cursor: Option<&str>) -> Result<String, ApiError> {
    let mut path = format!("/api/channels?limit={CHANNEL_PAGE_SIZE}");
    if let Some(cursor) = cursor {
        if cursor.is_empty() || cursor.len() > 4096 || cursor.chars().any(char::is_control) {
            return Err(ApiError::InvalidResponse);
        }
        path.push_str("&cursor=");
        path.push_str(&encode_url_component(cursor));
    }
    Ok(path)
}

fn encode_url_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use core::fmt::Write as _;
            write!(&mut encoded, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    encoded
}

#[cfg(target_arch = "wasm32")]
fn status_error(status: u16) -> ApiError {
    match status {
        401 => ApiError::Unauthorized,
        403 => ApiError::Forbidden,
        404 => ApiError::NotFound,
        409 | 410 | 412 => ApiError::Conflict,
        _ => ApiError::Server,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_path_is_one_encoded_segment_and_rejects_invalid_ids() {
        assert_eq!(
            approval_decision_path("approval/one?x=1").unwrap(),
            "/api/tool-approvals/approval%2Fone%3Fx%3D1"
        );
        assert_eq!(
            approval_decision_path("").unwrap_err(),
            ApiError::InvalidResponse
        );
        assert_eq!(
            approval_decision_path(&"a".repeat(129)).unwrap_err(),
            ApiError::InvalidResponse
        );
    }

    #[test]
    fn channel_paths_are_bounded_and_encode_exactly_one_component() {
        assert_eq!(
            channel_detail_path("channel/one?x=1").unwrap(),
            "/api/channels/channel%2Fone%3Fx%3D1"
        );
        assert_eq!(
            channel_route_href("channel/one?x=1").unwrap(),
            "/channel/channel%2Fone%3Fx%3D1"
        );
        assert_eq!(
            channel_list_path(Some("opaque+/=")).unwrap(),
            "/api/channels?limit=50&cursor=opaque%2B%2F%3D"
        );
        assert_eq!(
            channel_detail_path("").unwrap_err(),
            ApiError::InvalidResponse
        );
        assert_eq!(
            thread_run_path("thread/one?x=1").unwrap(),
            "/api/threads/thread%2Fone%3Fx%3D1/runs"
        );
        assert_eq!(
            channel_list_path(Some("bad\ncursor")).unwrap_err(),
            ApiError::InvalidResponse
        );
    }

    #[test]
    fn agent_paths_are_bounded_and_encode_one_path_or_query_component() {
        assert_eq!(
            agent_detail_path("agent/one?x=1").unwrap(),
            "/api/agents/agent%2Fone%3Fx%3D1"
        );
        assert_eq!(
            agent_profile_href("agent/one?x=1").unwrap(),
            "/agents?agent=agent%2Fone%3Fx%3D1"
        );
        assert_eq!(
            channel_new_href("agent/one?x=1").unwrap(),
            "/channel/new?agent=agent%2Fone%3Fx%3D1"
        );
        assert_eq!(
            agent_detail_path("").unwrap_err(),
            ApiError::InvalidResponse
        );
        assert_eq!(
            agent_profile_href("bad\nagent").unwrap_err(),
            ApiError::InvalidResponse
        );
    }
}
