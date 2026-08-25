//! Application-owned MCP per-user connection use cases and callback authentication port.

use async_trait::async_trait;
use openbot_contracts::auth::{AuthContext, Role};
use openbot_contracts::error::AppError;
use openbot_contracts::mcp::{
    McpConnectionDisconnected, McpConnections, McpOAuthAuthorization, McpOAuthClientRegistered,
    McpOAuthClientRegistration, McpOAuthReturnTo,
};
use openbot_domain::vault::SecretBytes;

/// Stable connection-flow failure without remote payload, URL, credential or database values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum McpConnectionError {
    /// Server/connection is unknown or outside the actor's scope.
    #[error("mcp_connection_not_visible")]
    NotVisible,
    /// Closed input failed validation.
    #[error("mcp_connection_invalid_input field={field}")]
    InvalidInput {
        /// Static field only.
        field: &'static str,
    },
    /// Deployment lacks public callback or a registered OAuth client.
    #[error("mcp_connection_conflict resource={resource}")]
    Conflict {
        /// Static resource class only.
        resource: &'static str,
    },
    /// Local dependency is unavailable.
    #[error("mcp_connection_unavailable")]
    Unavailable,
    /// Stored row/secret violates the closed schema.
    #[error("mcp_connection_corrupt field={field}")]
    Corrupt {
        /// Static field only.
        field: &'static str,
    },
    /// Vendor metadata/token/revocation endpoint failed.
    #[error("mcp_connection_vendor_failure")]
    VendorFailure,
    /// A local commit result is unknown and must be reconciled.
    #[error("mcp_connection_commit_unknown")]
    CommitUnknown,
}

impl McpConnectionError {
    /// Map into the stable application error taxonomy.
    #[must_use]
    pub const fn into_app_error(self) -> AppError {
        match self {
            Self::NotVisible => AppError::NotVisible,
            Self::InvalidInput { field } => AppError::MalformedPayload { field },
            Self::Conflict { resource } => AppError::RequestConflict { resource },
            Self::Unavailable | Self::Corrupt { .. } => AppError::DependencyUnavailable {
                dependency: "mcp_connections",
            },
            Self::VendorFailure => AppError::VendorFailure {
                vendor: "mcp_oauth",
            },
            Self::CommitUnknown => AppError::ReconciliationRequired { accepted: true },
        }
    }
}

/// Production port for authenticated per-user connection operations.
#[async_trait]
pub trait McpConnectionAdministration: Send + Sync {
    /// List only the current actor's connections.
    async fn list_connections(
        &self,
        auth: &AuthContext,
    ) -> Result<McpConnections, McpConnectionError>;

    /// Mint one-time state + PKCE and return the validated vendor authorization URL.
    async fn begin_oauth(
        &self,
        auth: &AuthContext,
        server_id: &str,
        return_to: McpOAuthReturnTo,
    ) -> Result<McpOAuthAuthorization, McpConnectionError>;

    /// Tombstone locally first, then attempt vendor revocation.
    async fn disconnect(
        &self,
        auth: &AuthContext,
        server_id: &str,
    ) -> Result<McpConnectionDisconnected, McpConnectionError>;

    /// Validate and atomically register/rotate a deployment OAuth client.
    async fn register_oauth_client(
        &self,
        auth: &AuthContext,
        server_id: &str,
        registration: &McpOAuthClientRegistration,
    ) -> Result<McpOAuthClientRegistered, McpConnectionError>;
}

/// Fail-closed default when production assembly has no MCP connection service.
#[derive(Debug, Default)]
pub struct NoMcpConnectionAdministration;

#[async_trait]
impl McpConnectionAdministration for NoMcpConnectionAdministration {
    async fn list_connections(
        &self,
        _auth: &AuthContext,
    ) -> Result<McpConnections, McpConnectionError> {
        Err(McpConnectionError::Unavailable)
    }

    async fn begin_oauth(
        &self,
        _auth: &AuthContext,
        _server_id: &str,
        _return_to: McpOAuthReturnTo,
    ) -> Result<McpOAuthAuthorization, McpConnectionError> {
        Err(McpConnectionError::Unavailable)
    }

    async fn disconnect(
        &self,
        _auth: &AuthContext,
        _server_id: &str,
    ) -> Result<McpConnectionDisconnected, McpConnectionError> {
        Err(McpConnectionError::Unavailable)
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

/// Public callback query. The state is the credential; no browser session is trusted here.
pub struct McpOAuthCallbackInput {
    code: SecretBytes,
    state: SecretBytes,
    issuer: Option<String>,
}

impl McpOAuthCallbackInput {
    /// Move bounded callback values into zeroizing allocations.
    #[must_use]
    pub fn new(code: Vec<u8>, state: Vec<u8>, issuer: Option<String>) -> Self {
        Self {
            code: SecretBytes::new(code),
            state: SecretBytes::new(state),
            issuer,
        }
    }

    /// Expose the authorization code only to the callback coordinator.
    #[must_use]
    pub fn code(&self) -> &[u8] {
        self.code.expose()
    }

    /// Expose opaque state only to the one-time attempt store.
    #[must_use]
    pub fn state(&self) -> &[u8] {
        self.state.expose()
    }

    /// Optional RFC 9207 issuer returned by the authorization server.
    #[must_use]
    pub fn issuer(&self) -> Option<&str> {
        self.issuer.as_deref()
    }
}

impl core::fmt::Debug for McpOAuthCallbackInput {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("McpOAuthCallbackInput")
            .field("code", &"<redacted>")
            .field("state", &"<redacted>")
            .field("issuer_present", &self.issuer.is_some())
            .finish()
    }
}

/// Callback result is always an application-owned absolute/relative in-app redirect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpOAuthCallbackOutcome {
    /// Validated redirect destination; never derived from incoming Host or caller URL input.
    pub redirect_to: String,
}

/// Authentication coordinator for the public OAuth callback.
#[async_trait]
pub trait McpOAuthCallback: Send + Sync {
    /// Consume state before validation/network and complete or fail to a uniform redirect.
    async fn complete(&self, input: McpOAuthCallbackInput) -> McpOAuthCallbackOutcome;
}

/// Application use case: list actor-owned connections.
pub async fn list_mcp_connections(
    port: &dyn McpConnectionAdministration,
    auth: &AuthContext,
) -> Result<McpConnections, AppError> {
    port.list_connections(auth)
        .await
        .map_err(McpConnectionError::into_app_error)
}

/// Application use case: begin actor-owned OAuth.
pub async fn begin_mcp_oauth(
    port: &dyn McpConnectionAdministration,
    auth: &AuthContext,
    server_id: &str,
    return_to: McpOAuthReturnTo,
) -> Result<McpOAuthAuthorization, AppError> {
    port.begin_oauth(auth, server_id, return_to)
        .await
        .map_err(McpConnectionError::into_app_error)
}

/// Application use case: local-first disconnect.
pub async fn disconnect_mcp_connection(
    port: &dyn McpConnectionAdministration,
    auth: &AuthContext,
    server_id: &str,
) -> Result<McpConnectionDisconnected, AppError> {
    port.disconnect(auth, server_id)
        .await
        .map_err(McpConnectionError::into_app_error)
}

/// Application use case: admin-only deployment OAuth-client registration.
pub async fn register_mcp_oauth_client(
    port: &dyn McpConnectionAdministration,
    auth: &AuthContext,
    server_id: &str,
    registration: &McpOAuthClientRegistration,
) -> Result<McpOAuthClientRegistered, AppError> {
    if !auth.has_role(Role::Admin) {
        return Err(AppError::ForbiddenRole {
            required: Role::Admin,
        });
    }
    port.register_oauth_client(auth, server_id, registration)
        .await
        .map_err(McpConnectionError::into_app_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openbot_contracts::auth::AuthGeneration;
    use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};
    use openbot_contracts::mcp::{McpOAuthClientAuthMethod, McpOAuthClientRegistration};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct FakePort {
        registrations: AtomicUsize,
    }

    #[async_trait]
    impl McpConnectionAdministration for FakePort {
        async fn list_connections(
            &self,
            _auth: &AuthContext,
        ) -> Result<McpConnections, McpConnectionError> {
            Err(McpConnectionError::Unavailable)
        }

        async fn begin_oauth(
            &self,
            _auth: &AuthContext,
            _server_id: &str,
            _return_to: McpOAuthReturnTo,
        ) -> Result<McpOAuthAuthorization, McpConnectionError> {
            Err(McpConnectionError::Unavailable)
        }

        async fn disconnect(
            &self,
            _auth: &AuthContext,
            _server_id: &str,
        ) -> Result<McpConnectionDisconnected, McpConnectionError> {
            Err(McpConnectionError::Unavailable)
        }

        async fn register_oauth_client(
            &self,
            _auth: &AuthContext,
            _server_id: &str,
            _registration: &McpOAuthClientRegistration,
        ) -> Result<McpOAuthClientRegistered, McpConnectionError> {
            self.registrations.fetch_add(1, Ordering::SeqCst);
            Ok(McpOAuthClientRegistered::success())
        }
    }

    fn auth(role: Role) -> AuthContext {
        AuthContext::for_test(
            DeploymentId::new("dep"),
            TenantId::new("tenant"),
            ActorId::new("actor"),
            [role],
            AuthGeneration::new(1),
            false,
        )
    }

    fn registration() -> McpOAuthClientRegistration {
        McpOAuthClientRegistration::new(
            "client".to_owned(),
            "secret".to_owned(),
            "https://issuer.example".to_owned(),
            McpOAuthClientAuthMethod::ClientSecretBasic,
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn oauth_client_registration_is_admin_gated_before_the_port() {
        let port = FakePort::default();
        assert!(matches!(
            register_mcp_oauth_client(&port, &auth(Role::User), "notes", &registration()).await,
            Err(AppError::ForbiddenRole {
                required: Role::Admin
            })
        ));
        assert_eq!(port.registrations.load(Ordering::SeqCst), 0);
        assert_eq!(
            register_mcp_oauth_client(&port, &auth(Role::Admin), "notes", &registration()).await,
            Ok(McpOAuthClientRegistered::success())
        );
        assert_eq!(port.registrations.load(Ordering::SeqCst), 1);
    }
}
