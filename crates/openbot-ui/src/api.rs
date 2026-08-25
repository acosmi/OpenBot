//! Same-origin browser transport for typed GUI APIs.
//!
//! This module only performs HTTP framing. Approval binding, actor resolution and the durable
//! decision remain behind `ApplicationService` on the Server.

use openbot_contracts::tool::{PendingToolApprovals, ToolApprovalDecision, ToolApprovalResolved};

/// Stable, payload-free failure categories suitable for localized presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalApiError {
    /// The browser could not complete the request.
    Network,
    /// The current session is absent or expired.
    Unauthorized,
    /// The actor is not allowed to access the approval.
    Forbidden,
    /// The approval was concurrently resolved or invalidated.
    Conflict,
    /// The Server response did not match the closed contract.
    InvalidResponse,
    /// The Server returned another unsuccessful status.
    Server,
    /// The browser-only API was called by a non-WASM target.
    Unavailable,
}

/// Load at most the Server-bounded current-actor approval page without using browser cache.
pub async fn list_pending_tool_approvals() -> Result<PendingToolApprovals, ApprovalApiError> {
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
            .map_err(|_| ApprovalApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        let page = response
            .json::<PendingToolApprovals>()
            .await
            .map_err(|_| ApprovalApiError::InvalidResponse)?;
        if page.approvals.len() > 100 {
            return Err(ApprovalApiError::InvalidResponse);
        }
        Ok(page)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Err(ApprovalApiError::Unavailable)
    }
}

/// Commit one grant or denial for a Server-minted id; no binding fields are accepted here.
pub async fn decide_tool_approval(
    approval_id: &str,
    decision: ToolApprovalDecision,
) -> Result<ToolApprovalResolved, ApprovalApiError> {
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
            .map_err(|_| ApprovalApiError::InvalidResponse)?;
        let response = request
            .send()
            .await
            .map_err(|_| ApprovalApiError::Network)?;
        if !response.ok() {
            return Err(status_error(response.status()));
        }
        let receipt = response
            .json::<ToolApprovalResolved>()
            .await
            .map_err(|_| ApprovalApiError::InvalidResponse)?;
        if receipt.approval_id != approval_id || receipt.decision != decision {
            return Err(ApprovalApiError::InvalidResponse);
        }
        Ok(receipt)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (approval_id, decision);
        Err(ApprovalApiError::Unavailable)
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn approval_decision_path(approval_id: &str) -> Result<String, ApprovalApiError> {
    if approval_id.is_empty() || approval_id.len() > 128 || approval_id.as_bytes().contains(&0) {
        return Err(ApprovalApiError::InvalidResponse);
    }
    let mut encoded = String::with_capacity(approval_id.len());
    for byte in approval_id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use core::fmt::Write as _;
            write!(&mut encoded, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    Ok(format!("/api/tool-approvals/{encoded}"))
}

#[cfg(target_arch = "wasm32")]
fn status_error(status: u16) -> ApprovalApiError {
    match status {
        401 => ApprovalApiError::Unauthorized,
        403 => ApprovalApiError::Forbidden,
        404 | 409 | 410 | 412 => ApprovalApiError::Conflict,
        _ => ApprovalApiError::Server,
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
            ApprovalApiError::InvalidResponse
        );
        assert_eq!(
            approval_decision_path(&"a".repeat(129)).unwrap_err(),
            ApprovalApiError::InvalidResponse
        );
    }
}
