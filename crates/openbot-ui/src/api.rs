//! Same-origin browser transport for typed GUI APIs.
//!
//! This module only performs HTTP framing. Approval binding, actor resolution and the durable
//! decision remain behind `ApplicationService` on the Server.

use openbot_contracts::agent::AgentProfile;
#[cfg(target_arch = "wasm32")]
use openbot_contracts::agent::{AgentProfileResponse, AgentProfilesResponse};
#[cfg(target_arch = "wasm32")]
use openbot_contracts::command::ThreadRunCancellationState;
#[cfg(target_arch = "wasm32")]
use openbot_contracts::command::{
    BeginThreadRunBody, CreateChannelRequest, RouteChannelRequest, ThreadMinted, ThreadRunAnchor,
};
use openbot_contracts::command::{
    ChannelDetail, ChannelPage, ChannelRoutingDecision, ThreadConversationSnapshot,
    ThreadRunCancellation, ThreadRunStarted,
};
use openbot_contracts::components::{
    ComponentCatalogueAdded, ComponentCatalogueRequest, ComponentDecision,
    ComponentDecisionRequest, ComponentRecords, GrantedCompiledComponents,
    compiled_component_manifest,
};
#[cfg(any(target_arch = "wasm32", test))]
use openbot_contracts::components::{ComponentDecisionRefusal, ComponentRecord};
use openbot_contracts::ids::{BotId, ChannelId, RunId, ThreadId};
use openbot_contracts::mcp::{McpConnectionDisconnected, McpConnections, McpOAuthAuthorization};
#[cfg(target_arch = "wasm32")]
use openbot_contracts::memory::UpdateMemoryControl;
use openbot_contracts::memory::{
    CorrectMemory, MemoryControl, MemoryMutation, MemoryPage, MemoryRecord,
};
#[cfg(any(target_arch = "wasm32", test))]
use openbot_contracts::memory::{MemoryScope, MemoryStatus};
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
/// Memory page size; the application owns the authoritative clamp.
pub const MEMORY_PAGE_SIZE: u32 = 50;

/// Announce the exact build-owned compiled component manifest; existing governance is untouched.
pub async fn announce_component_catalogue() -> Result<ComponentCatalogueAdded, ApiError> {
    let request = ComponentCatalogueRequest {
        components: compiled_component_manifest(),
    };
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let response = Request::put("/api/components/catalogue")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .json(&request)
            .map_err(|_| ApiError::InvalidResponse)?
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if response.status() != 200 {
            return Err(status_error(response.status()));
        }
        let added = response
            .json::<ComponentCatalogueAdded>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        validate_catalogue_added(&request, &added)?;
        Ok(added)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = request;
        Err(ApiError::Unavailable)
    }
}

/// Load all durable compiled-component governance rows for authenticated Settings/Admin views.
pub async fn load_components() -> Result<ComponentRecords, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let response = Request::get("/api/components")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if response.status() != 200 {
            return Err(status_error(response.status()));
        }
        let records = response
            .json::<ComponentRecords>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        validate_component_records(&records)?;
        Ok(records)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(ApiError::Unavailable)
    }
}

/// Load the current build's published, non-withheld renderer grants for one Agent.
pub async fn load_components_for_agent(
    agent_id: &BotId,
) -> Result<GrantedCompiledComponents, ApiError> {
    validate_bounded_identifier(agent_id.as_str(), 256)?;
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = format!(
            "/api/components/for-agent/{}",
            encode_url_component(agent_id.as_str())
        );
        let response = Request::get(&path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if response.status() != 200 {
            return Err(status_error(response.status()));
        }
        let granted = response
            .json::<GrantedCompiledComponents>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        validate_granted_components(&granted)?;
        Ok(granted)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(ApiError::Unavailable)
    }
}

/// Re-authorize one compiled renderer immediately before accepting its tool call.
pub async fn decide_component(
    name: &str,
    agent_id: &BotId,
    functions: &[String],
) -> Result<ComponentDecision, ApiError> {
    validate_component_name(name)?;
    validate_bounded_identifier(agent_id.as_str(), 256)?;
    if functions.len() > 1024 {
        return Err(ApiError::InvalidResponse);
    }
    let mut previous = None::<&str>;
    for function in functions {
        validate_component_name(function)?;
        if previous.is_some_and(|previous| previous >= function.as_str()) {
            return Err(ApiError::InvalidResponse);
        }
        previous = Some(function);
    }
    let request = ComponentDecisionRequest {
        agent_id: agent_id.clone(),
        functions: functions.to_vec(),
    };
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = format!("/api/components/{}/decision", encode_url_component(name));
        let response = Request::post(&path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .json(&request)
            .map_err(|_| ApiError::InvalidResponse)?
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if response.status() != 200 {
            return Err(status_error(response.status()));
        }
        let decision = response
            .json::<ComponentDecision>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        validate_component_decision(&decision, functions)?;
        Ok(decision)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = request;
        Err(ApiError::Unavailable)
    }
}

/// Load reviewed user-OAuth servers and the current actor's connection rows.
pub async fn load_mcp_connections() -> Result<McpConnections, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let response = Request::get("/api/plugins/connections")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if response.status() != 200 {
            return Err(status_error(response.status()));
        }
        let page = response
            .json::<McpConnections>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        validate_mcp_connections(&page)?;
        Ok(page)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(ApiError::Unavailable)
    }
}

/// Ask the Server to mint OAuth state/PKCE and validate its navigation receipt.
pub async fn begin_mcp_connection(server_id: &str) -> Result<McpOAuthAuthorization, ApiError> {
    validate_mcp_server_id(server_id)?;
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = mcp_connect_path(server_id)?;
        let response = Request::post(&path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if response.status() != 200 {
            return Err(status_error(response.status()));
        }
        let authorization = response
            .json::<McpOAuthAuthorization>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        validate_authorization_target(&authorization.authorization_url)?;
        Ok(authorization)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = server_id;
        Err(ApiError::Unavailable)
    }
}

/// Tombstone one actor-owned connection and verify the exact Server receipt.
pub async fn disconnect_mcp_connection(
    server_id: &str,
) -> Result<McpConnectionDisconnected, ApiError> {
    validate_mcp_server_id(server_id)?;
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = mcp_disconnect_path(server_id)?;
        let response = Request::delete(&path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if response.status() != 200 {
            return Err(status_error(response.status()));
        }
        let receipt = response
            .json::<McpConnectionDisconnected>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        if receipt.server_id != server_id {
            return Err(ApiError::InvalidResponse);
        }
        Ok(receipt)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = server_id;
        Err(ApiError::Unavailable)
    }
}

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

/// Persist cancellation for the exact active native run; terminal still arrives via snapshot/SSE.
pub async fn cancel_thread_run(
    thread_id: &ThreadId,
    run_id: &RunId,
) -> Result<ThreadRunCancellation, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = thread_cancel_path(thread_id.as_str(), run_id.as_str())?;
        let response = Request::post(&path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        let status = response.status();
        if !matches!(status, 200 | 202) {
            return Err(status_error(status));
        }
        let reply = response
            .json::<ThreadRunCancellation>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        let status_matches = matches!(
            (status, reply.state),
            (
                202,
                ThreadRunCancellationState::Requested
                    | ThreadRunCancellationState::AlreadyRequested
            ) | (200, ThreadRunCancellationState::AlreadyTerminal)
        );
        if reply.thread_id != *thread_id || reply.run_id != *run_id || !status_matches {
            return Err(ApiError::InvalidResponse);
        }
        Ok(reply)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (thread_id, run_id);
        Err(ApiError::Unavailable)
    }
}

/// Mint one browser-CSPRNG-backed durable idempotency key.
#[must_use]
pub fn mint_run_id() -> RunId {
    RunId::new(uuid::Uuid::now_v7().to_string())
}

/// Ask the Server to mint a deployment-owned native thread identity.
pub async fn mint_thread_id() -> Result<ThreadId, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let response = Request::post("/api/threads/mint")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if response.status() != 200 {
            return Err(status_error(response.status()));
        }
        response
            .json::<ThreadMinted>()
            .await
            .map(|minted| minted.thread_id)
            .map_err(|_| ApiError::InvalidResponse)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(ApiError::Unavailable)
    }
}

/// Load one atomic durable history/active-run/event-cursor snapshot.
pub async fn load_thread_conversation(
    thread_id: &ThreadId,
) -> Result<ThreadConversationSnapshot, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = thread_conversation_path(thread_id.as_str())?;
        let response = Request::get(&path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        if response.status() != 200 {
            return Err(status_error(response.status()));
        }
        let snapshot = response
            .json::<ThreadConversationSnapshot>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        let active_shape_matches =
            snapshot.active_run_id.is_some() == snapshot.active_run_state.is_some();
        let cancellable_matches = !snapshot.active_run_cancellable
            || matches!(
                snapshot.active_run_state,
                Some(openbot_contracts::command::ThreadForegroundRunState::Running)
            );
        if !active_shape_matches
            || !cancellable_matches
            || (snapshot.active_run_id.is_none() && !snapshot.active_run_text.is_empty())
        {
            return Err(ApiError::InvalidResponse);
        }
        Ok(snapshot)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = thread_id;
        Err(ApiError::Unavailable)
    }
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

/// Load the current actor's runtime memory write control.
pub async fn load_memory_control() -> Result<MemoryControl, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let response = Request::get("/api/memories/control")
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
            .json::<MemoryControl>()
            .await
            .map_err(|_| ApiError::InvalidResponse)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(ApiError::Unavailable)
    }
}

/// Persist and verify the current actor's runtime memory write control.
pub async fn save_memory_control(writes_enabled: bool) -> Result<MemoryControl, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let request = Request::put("/api/memories/control")
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .json(&UpdateMemoryControl { writes_enabled })
            .map_err(|_| ApiError::InvalidResponse)?;
        let response = request.send().await.map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        let control = response
            .json::<MemoryControl>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        if control.writes_enabled != writes_enabled {
            return Err(ApiError::InvalidResponse);
        }
        Ok(control)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = writes_enabled;
        Err(ApiError::Unavailable)
    }
}

/// Load one owner-scoped memory keyset page.
pub async fn list_memories(cursor: Option<&str>) -> Result<MemoryPage, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = memory_list_path(cursor)?;
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
            .json::<MemoryPage>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        validate_memory_page(&page)?;
        Ok(page)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = cursor;
        Err(ApiError::Unavailable)
    }
}

/// Correct one active memory; the Server creates a replacement and preserves provenance.
pub async fn correct_memory_record(
    memory_id: &str,
    correction: CorrectMemory,
) -> Result<MemoryRecord, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = memory_detail_path(memory_id)?;
        let request = Request::put(&path)
            .cache(RequestCache::NoStore)
            .credentials(RequestCredentials::SameOrigin)
            .redirect(RequestRedirect::Error)
            .json(&correction)
            .map_err(|_| ApiError::InvalidResponse)?;
        let response = request.send().await.map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        let record = response
            .json::<MemoryRecord>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        validate_memory_record(&record)?;
        if record.status != MemoryStatus::Active
            || record.supersedes_id.as_deref() != Some(memory_id)
        {
            return Err(ApiError::InvalidResponse);
        }
        Ok(record)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (memory_id, correction);
        Err(ApiError::Unavailable)
    }
}

/// Forbid or delete one owner memory and verify that retained content is erased.
pub async fn mutate_memory_record(
    memory_id: &str,
    mutation: MemoryMutation,
) -> Result<MemoryRecord, ApiError> {
    #[cfg(target_arch = "wasm32")]
    {
        use gloo_net::http::Request;
        use web_sys::{RequestCache, RequestCredentials, RequestRedirect};

        let path = match mutation {
            MemoryMutation::Forbid => format!("{}/forbid", memory_detail_path(memory_id)?),
            MemoryMutation::Delete => memory_detail_path(memory_id)?,
        };
        let request = match mutation {
            MemoryMutation::Forbid => Request::post(&path),
            MemoryMutation::Delete => Request::delete(&path),
        }
        .cache(RequestCache::NoStore)
        .credentials(RequestCredentials::SameOrigin)
        .redirect(RequestRedirect::Error);
        let response = request.send().await.map_err(|_| ApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        let record = response
            .json::<MemoryRecord>()
            .await
            .map_err(|_| ApiError::InvalidResponse)?;
        validate_memory_record(&record)?;
        let expected = match mutation {
            MemoryMutation::Forbid => MemoryStatus::Forbidden,
            MemoryMutation::Delete => MemoryStatus::Deleted,
        };
        if record.memory_id != memory_id || record.status != expected || record.content.is_some() {
            return Err(ApiError::InvalidResponse);
        }
        Ok(record)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (memory_id, mutation);
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

/// Build one same-origin Settings gallery detail URL for a closed component name.
pub fn component_gallery_href(name: &str) -> Result<String, ApiError> {
    validate_component_name(name)?;
    Ok(format!(
        "/settings/components-gallery/{}",
        encode_url_component(name)
    ))
}

fn validate_component_name(name: &str) -> Result<(), ApiError> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Err(ApiError::InvalidResponse)
    } else {
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_catalogue_added(
    request: &ComponentCatalogueRequest,
    added: &ComponentCatalogueAdded,
) -> Result<(), ApiError> {
    if added.added.len() > request.components.len() {
        return Err(ApiError::InvalidResponse);
    }
    let requested = request
        .components
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut unique = std::collections::BTreeSet::new();
    for name in &added.added {
        validate_component_name(name)?;
        if !requested.contains(name.as_str()) || !unique.insert(name.as_str()) {
            return Err(ApiError::InvalidResponse);
        }
    }
    Ok(())
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_component_records(records: &ComponentRecords) -> Result<(), ApiError> {
    if records.components.len() > 256 {
        return Err(ApiError::InvalidResponse);
    }
    let mut names = std::collections::BTreeSet::new();
    let mut previous = None::<(&str, &str, &str)>;
    for record in &records.components {
        validate_component_record(record)?;
        if !names.insert(record.name.as_str()) {
            return Err(ApiError::InvalidResponse);
        }
        let key = (
            record.kind.as_str(),
            record.title.as_str(),
            record.name.as_str(),
        );
        if previous.is_some_and(|previous| previous > key) {
            return Err(ApiError::InvalidResponse);
        }
        previous = Some(key);
    }
    Ok(())
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_component_record(record: &ComponentRecord) -> Result<(), ApiError> {
    validate_component_name(&record.name)?;
    validate_component_identifier(&record.title, 512)?;
    validate_component_description(&record.draft_description)?;
    if let Some(description) = record.published_description.as_deref() {
        validate_component_description(description)?;
    }
    if record.published && (record.published_description.is_none() || record.published_at.is_none())
    {
        return Err(ApiError::InvalidResponse);
    }
    if record.has_unpublished_changes
        != (record.draft_description != record.published_description.as_deref().unwrap_or(""))
    {
        return Err(ApiError::InvalidResponse);
    }
    if let Some(updated_by) = record.updated_by.as_deref() {
        validate_component_identifier(updated_by, 512)?;
    }
    validate_component_identifier_list(&record.withheld_from)?;
    validate_component_identifier_list(&record.functions)?;
    Ok(())
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_granted_components(granted: &GrantedCompiledComponents) -> Result<(), ApiError> {
    let renderers = compiled_component_manifest()
        .into_iter()
        .map(|entry| entry.name)
        .collect::<std::collections::BTreeSet<_>>();
    if granted.components.len() > renderers.len() {
        return Err(ApiError::InvalidResponse);
    }
    let mut previous = None::<&str>;
    for component in &granted.components {
        validate_component_name(&component.name)?;
        validate_component_description(&component.description)?;
        if !renderers.contains(&component.name)
            || previous.is_some_and(|previous| previous >= component.name.as_str())
        {
            return Err(ApiError::InvalidResponse);
        }
        previous = Some(&component.name);
    }
    Ok(())
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_component_decision(
    decision: &ComponentDecision,
    requested_functions: &[String],
) -> Result<(), ApiError> {
    if !decision.is_consistent() {
        return Err(ApiError::InvalidResponse);
    }
    if let Some(ComponentDecisionRefusal::FunctionNotGranted { function }) = &decision.refusal
        && requested_functions.binary_search(function).is_err()
    {
        return Err(ApiError::InvalidResponse);
    }
    Ok(())
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_component_description(value: &str) -> Result<(), ApiError> {
    if value.is_empty() || value.len() > 64 * 1024 || value.as_bytes().contains(&0) {
        Err(ApiError::InvalidResponse)
    } else {
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_component_identifier(value: &str, max: usize) -> Result<(), ApiError> {
    validate_bounded_identifier(value, max)
}

fn validate_bounded_identifier(value: &str, max: usize) -> Result<(), ApiError> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        Err(ApiError::InvalidResponse)
    } else {
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_component_identifier_list(values: &[String]) -> Result<(), ApiError> {
    if values.len() > 1024 {
        return Err(ApiError::InvalidResponse);
    }
    let mut previous = None::<&str>;
    for value in values {
        validate_component_identifier(value, 512)?;
        if previous.is_some_and(|previous| previous >= value.as_str()) {
            return Err(ApiError::InvalidResponse);
        }
        previous = Some(value);
    }
    Ok(())
}

fn validate_mcp_server_id(server_id: &str) -> Result<(), ApiError> {
    if server_id.is_empty()
        || server_id.len() > 64
        || server_id.contains("__")
        || server_id.contains('/')
        || !server_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Err(ApiError::InvalidResponse)
    } else {
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn mcp_connect_path(server_id: &str) -> Result<String, ApiError> {
    validate_mcp_server_id(server_id)?;
    Ok(format!(
        "/api/plugins/servers/{}/connect?returnTo=settings",
        encode_url_component(server_id)
    ))
}

#[cfg(any(target_arch = "wasm32", test))]
fn mcp_disconnect_path(server_id: &str) -> Result<String, ApiError> {
    validate_mcp_server_id(server_id)?;
    Ok(format!(
        "/api/plugins/connections/{}",
        encode_url_component(server_id)
    ))
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_mcp_connections(page: &McpConnections) -> Result<(), ApiError> {
    if page.available_server_ids.len() > 64 || page.connections.len() > 256 {
        return Err(ApiError::InvalidResponse);
    }
    let mut available = std::collections::BTreeSet::new();
    for server_id in &page.available_server_ids {
        validate_mcp_server_id(server_id)?;
        if !available.insert(server_id.as_str()) {
            return Err(ApiError::InvalidResponse);
        }
    }
    let mut connected = std::collections::BTreeSet::new();
    for connection in &page.connections {
        validate_mcp_server_id(&connection.server_id)?;
        if !connected.insert(connection.server_id.as_str())
            || connection.scope.is_empty()
            || connection.scope.len() > 16 * 1024
            || connection.scope.chars().any(char::is_control)
        {
            return Err(ApiError::InvalidResponse);
        }
    }
    if let Some(redirect_uri) = page.redirect_uri.as_deref() {
        validate_callback_uri(redirect_uri)?;
    }
    Ok(())
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_callback_uri(value: &str) -> Result<(), ApiError> {
    if value.len() > 16 * 1024 || value.chars().any(char::is_control) {
        return Err(ApiError::InvalidResponse);
    }
    let parsed = url::Url::parse(value).map_err(|_| ApiError::InvalidResponse)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ApiError::InvalidResponse);
    }
    Ok(())
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_authorization_target(value: &str) -> Result<(), ApiError> {
    if value.is_empty()
        || value.len() > 16 * 1024
        || value.chars().any(char::is_control)
        || value.contains('\\')
        || value.contains('#')
    {
        return Err(ApiError::InvalidResponse);
    }
    if value.starts_with('/') {
        return if value.starts_with("//") {
            Err(ApiError::InvalidResponse)
        } else {
            Ok(())
        };
    }
    let parsed = url::Url::parse(value).map_err(|_| ApiError::InvalidResponse)?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ApiError::InvalidResponse);
    }
    Ok(())
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
fn memory_list_path(cursor: Option<&str>) -> Result<String, ApiError> {
    let mut path = format!("/api/memories?limit={MEMORY_PAGE_SIZE}");
    if let Some(cursor) = cursor {
        validate_memory_id(cursor)?;
        path.push_str("&cursor=");
        path.push_str(&encode_url_component(cursor));
    }
    Ok(path)
}

#[cfg(any(target_arch = "wasm32", test))]
fn memory_detail_path(memory_id: &str) -> Result<String, ApiError> {
    validate_memory_id(memory_id)?;
    Ok(format!("/api/memories/{}", encode_url_component(memory_id)))
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_memory_id(memory_id: &str) -> Result<(), ApiError> {
    if memory_id.is_empty() || memory_id.len() > 512 || memory_id.chars().any(char::is_control) {
        Err(ApiError::InvalidResponse)
    } else {
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_memory_page(page: &MemoryPage) -> Result<(), ApiError> {
    if page.memories.len() > MEMORY_PAGE_SIZE as usize {
        return Err(ApiError::InvalidResponse);
    }
    let mut ids = std::collections::BTreeSet::new();
    for record in &page.memories {
        validate_memory_record(record)?;
        if !ids.insert(record.memory_id.as_str()) {
            return Err(ApiError::InvalidResponse);
        }
    }
    if let Some(cursor) = page.next_cursor.as_deref() {
        validate_memory_id(cursor)?;
    }
    Ok(())
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_memory_record(record: &MemoryRecord) -> Result<(), ApiError> {
    validate_memory_id(&record.memory_id)?;
    if record.owner_user_id.is_empty()
        || record.owner_user_id.chars().any(char::is_control)
        || record.created_by.is_empty()
        || record.created_by.chars().any(char::is_control)
        || record.tags.len() > 32
        || record
            .tags
            .iter()
            .any(|tag| tag.is_empty() || tag.len() > 64 || tag.chars().any(char::is_control))
    {
        return Err(ApiError::InvalidResponse);
    }
    match record.status {
        MemoryStatus::Active | MemoryStatus::Superseded
            if record.content.as_deref().is_none_or(str::is_empty) =>
        {
            return Err(ApiError::InvalidResponse);
        }
        MemoryStatus::Forbidden | MemoryStatus::Deleted if record.content.is_some() => {
            return Err(ApiError::InvalidResponse);
        }
        _ => {}
    }
    match &record.scope {
        MemoryScope::User => {}
        MemoryScope::Bot { bot_id } if bot_id.as_str().is_empty() => {
            return Err(ApiError::InvalidResponse);
        }
        MemoryScope::Thread { thread_id } if thread_id.as_str().is_empty() => {
            return Err(ApiError::InvalidResponse);
        }
        MemoryScope::Bot { .. } | MemoryScope::Thread { .. } => {}
    }
    if let Some(source) = &record.source
        && (source.thread_id.as_str().is_empty()
            || source.message_id.is_empty()
            || source.message_id.chars().any(char::is_control))
    {
        return Err(ApiError::InvalidResponse);
    }
    Ok(())
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
fn thread_cancel_path(thread_id: &str, run_id: &str) -> Result<String, ApiError> {
    validate_thread_id(thread_id)?;
    validate_run_id(run_id)?;
    Ok(format!(
        "/api/threads/{}/runs/{}/cancel",
        encode_url_component(thread_id),
        encode_url_component(run_id)
    ))
}

#[cfg(any(target_arch = "wasm32", test))]
fn thread_conversation_path(thread_id: &str) -> Result<String, ApiError> {
    validate_thread_id(thread_id)?;
    Ok(format!(
        "/api/threads/{}/conversation",
        encode_url_component(thread_id)
    ))
}

/// Build the EventSource URL whose optional cursor is only the initial durable replay boundary.
#[cfg(any(target_arch = "wasm32", test))]
pub fn thread_event_stream_path(
    thread_id: &ThreadId,
    cursor: Option<u64>,
) -> Result<String, ApiError> {
    validate_thread_id(thread_id.as_str())?;
    let mut path = format!(
        "/api/threads/{}/events",
        encode_url_component(thread_id.as_str())
    );
    if let Some(cursor) = cursor {
        path.push_str("?cursor=");
        path.push_str(&cursor.to_string());
    }
    Ok(path)
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
fn validate_run_id(run_id: &str) -> Result<(), ApiError> {
    if run_id.is_empty() || run_id.len() > 512 || run_id.chars().any(char::is_control) {
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
    use openbot_contracts::components::{CompiledComponentKind, SHOW_QUOTE_COMPONENT_NAME};
    use openbot_contracts::memory::{MemoryKind, MemoryOrigin, MemorySensitivity};
    use time::OffsetDateTime;

    use super::*;

    fn memory_record(id: &str, content: Option<&str>, status: MemoryStatus) -> MemoryRecord {
        MemoryRecord {
            memory_id: id.to_owned(),
            owner_user_id: "actor".to_owned(),
            scope: MemoryScope::User,
            memory_kind: MemoryKind::Preference,
            content: content.map(str::to_owned),
            tags: vec!["preference".to_owned()],
            sensitivity: MemorySensitivity::Normal,
            source: None,
            origin: MemoryOrigin::UserAction,
            created_by: "actor".to_owned(),
            supersedes_id: None,
            status,
            expires_at: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn mcp_page() -> McpConnections {
        McpConnections {
            available_server_ids: vec!["google-drive".to_owned()],
            connections: vec![openbot_contracts::mcp::McpConnection {
                server_id: "google-drive".to_owned(),
                scope: "https://www.googleapis.com/auth/drive.readonly".to_owned(),
                connected_at: OffsetDateTime::UNIX_EPOCH,
            }],
            redirect_uri: Some("http://127.0.0.1:39015/api/plugins/oauth/callback".to_owned()),
        }
    }

    fn component_record(name: &str, title: &str, published: bool) -> ComponentRecord {
        ComponentRecord {
            name: name.to_owned(),
            title: title.to_owned(),
            kind: CompiledComponentKind::Card,
            draft_description: "Show one compiled component.".to_owned(),
            published_description: published.then(|| "Show one compiled component.".to_owned()),
            published,
            published_at: published.then_some(OffsetDateTime::UNIX_EPOCH),
            updated_by: Some("the build".to_owned()),
            updated_at: OffsetDateTime::UNIX_EPOCH,
            has_unpublished_changes: !published,
            withheld_from: Vec::new(),
            functions: Vec::new(),
        }
    }

    #[test]
    fn component_records_catalogue_receipts_and_routes_are_closed() {
        assert_eq!(
            component_gallery_href(SHOW_QUOTE_COMPONENT_NAME).unwrap(),
            "/settings/components-gallery/showQuote"
        );
        assert_eq!(
            component_gallery_href("bad/name").unwrap_err(),
            ApiError::InvalidResponse
        );
        let records = ComponentRecords {
            components: vec![component_record(
                SHOW_QUOTE_COMPONENT_NAME,
                "Quotation",
                true,
            )],
        };
        assert!(validate_component_records(&records).is_ok());

        let request = ComponentCatalogueRequest {
            components: compiled_component_manifest(),
        };
        assert!(
            validate_catalogue_added(
                &request,
                &ComponentCatalogueAdded {
                    added: vec![SHOW_QUOTE_COMPONENT_NAME.to_owned()],
                },
            )
            .is_ok()
        );
        assert_eq!(
            validate_catalogue_added(
                &request,
                &ComponentCatalogueAdded {
                    added: vec!["showUnknown".to_owned()],
                },
            )
            .unwrap_err(),
            ApiError::InvalidResponse
        );

        let mut inconsistent = records.components[0].clone();
        inconsistent.published_description = None;
        assert_eq!(
            validate_component_record(&inconsistent).unwrap_err(),
            ApiError::InvalidResponse
        );
        let mut duplicate_functions = records.components[0].clone();
        duplicate_functions.functions = vec!["read".to_owned(), "read".to_owned()];
        assert_eq!(
            validate_component_record(&duplicate_functions).unwrap_err(),
            ApiError::InvalidResponse
        );

        let grants = GrantedCompiledComponents {
            components: vec![openbot_contracts::components::GrantedCompiledComponent {
                name: SHOW_QUOTE_COMPONENT_NAME.to_owned(),
                description: "published quote".to_owned(),
            }],
        };
        assert!(validate_granted_components(&grants).is_ok());
        let mut stale = grants.clone();
        stale.components[0].name = "showStale".to_owned();
        assert_eq!(
            validate_granted_components(&stale).unwrap_err(),
            ApiError::InvalidResponse
        );

        let requested = vec!["recentRefusals".to_owned()];
        assert!(validate_component_decision(&ComponentDecision::allowed(), &requested).is_ok());
        assert!(
            validate_component_decision(
                &ComponentDecision::refused(ComponentDecisionRefusal::FunctionNotGranted {
                    function: "recentRefusals".to_owned(),
                },),
                &requested,
            )
            .is_ok()
        );
        assert_eq!(
            validate_component_decision(
                &ComponentDecision::refused(ComponentDecisionRefusal::FunctionNotGranted {
                    function: "botActivity".to_owned(),
                },),
                &requested,
            )
            .unwrap_err(),
            ApiError::InvalidResponse
        );
        assert_eq!(
            validate_component_decision(
                &ComponentDecision {
                    allowed: true,
                    refusal: Some(ComponentDecisionRefusal::Unpublished),
                },
                &[],
            )
            .unwrap_err(),
            ApiError::InvalidResponse
        );
    }

    #[test]
    fn mcp_paths_projection_and_navigation_receipts_are_closed() {
        assert_eq!(
            mcp_connect_path("google-drive").unwrap(),
            "/api/plugins/servers/google-drive/connect?returnTo=settings"
        );
        assert_eq!(
            mcp_disconnect_path("google-drive").unwrap(),
            "/api/plugins/connections/google-drive"
        );
        assert!(validate_mcp_connections(&mcp_page()).is_ok());
        assert!(
            validate_authorization_target("/settings/connected-accounts?connected=google-drive")
                .is_ok()
        );
        assert!(
            validate_authorization_target(
                "https://accounts.google.com/o/oauth2/v2/auth?state=opaque"
            )
            .is_ok()
        );

        for invalid in [
            "//attacker.example/path",
            "http://accounts.google.com/authorize",
            "https://user@accounts.google.com/authorize",
            "https://accounts.google.com/authorize#token",
            "javascript:alert(1)",
            "/bad\\path",
        ] {
            assert_eq!(
                validate_authorization_target(invalid).unwrap_err(),
                ApiError::InvalidResponse
            );
        }
        assert_eq!(
            validate_mcp_server_id("google__drive").unwrap_err(),
            ApiError::InvalidResponse
        );
        let mut duplicate = mcp_page();
        duplicate
            .available_server_ids
            .push("google-drive".to_owned());
        assert_eq!(
            validate_mcp_connections(&duplicate).unwrap_err(),
            ApiError::InvalidResponse
        );
        let mut credential_like_scope = mcp_page();
        credential_like_scope.connections[0].scope = "bad\nscope".to_owned();
        assert_eq!(
            validate_mcp_connections(&credential_like_scope).unwrap_err(),
            ApiError::InvalidResponse
        );
    }

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
            thread_conversation_path("thread/one?x=1").unwrap(),
            "/api/threads/thread%2Fone%3Fx%3D1/conversation"
        );
        assert_eq!(
            thread_cancel_path("thread/one?x=1", "run/one?x=2").unwrap(),
            "/api/threads/thread%2Fone%3Fx%3D1/runs/run%2Fone%3Fx%3D2/cancel"
        );
        assert_eq!(
            thread_event_stream_path(&ThreadId::new("thread/one?x=1"), Some(7)).unwrap(),
            "/api/threads/thread%2Fone%3Fx%3D1/events?cursor=7"
        );
        assert_eq!(
            channel_list_path(Some("bad\ncursor")).unwrap_err(),
            ApiError::InvalidResponse
        );
    }

    #[test]
    fn memory_paths_and_pages_are_closed_bounded_and_owner_safe() {
        assert_eq!(
            memory_list_path(Some("memory/one?x=1")).unwrap(),
            "/api/memories?limit=50&cursor=memory%2Fone%3Fx%3D1"
        );
        assert_eq!(
            memory_detail_path("memory/one?x=1").unwrap(),
            "/api/memories/memory%2Fone%3Fx%3D1"
        );
        assert_eq!(
            memory_detail_path("bad\nmemory").unwrap_err(),
            ApiError::InvalidResponse
        );

        let active = memory_record("memory-1", Some("tea"), MemoryStatus::Active);
        assert!(validate_memory_record(&active).is_ok());
        let mut erased_active = active.clone();
        erased_active.content = None;
        assert_eq!(
            validate_memory_record(&erased_active).unwrap_err(),
            ApiError::InvalidResponse
        );
        let duplicate = MemoryPage {
            memories: vec![active.clone(), active],
            next_cursor: None,
        };
        assert_eq!(
            validate_memory_page(&duplicate).unwrap_err(),
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
