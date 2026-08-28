//! MCP per-user connection DTOs shared by Server, Desktop and Leptos/WASM.

use core::fmt;
use std::sync::Arc;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use subtle::ConstantTimeEq as _;
use time::OffsetDateTime;
use zeroize::Zeroizing;

/// Supported confidential-client authentication at MCP token/revocation endpoints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpOAuthClientAuthMethod {
    /// HTTP Basic per RFC 6749 §2.3.1.
    #[default]
    ClientSecretBasic,
    /// Form-body client authentication for providers that explicitly advertise it.
    ClientSecretPost,
}

/// Deployment OAuth client registration. Clone shares, rather than duplicates, the zeroizing
/// client-secret allocation; Debug never renders it.
#[derive(Clone)]
pub struct McpOAuthClientRegistration {
    client_id: String,
    client_secret: Arc<Zeroizing<String>>,
    issuer: String,
    auth_method: McpOAuthClientAuthMethod,
    resource_metadata_url: Option<String>,
}

impl McpOAuthClientRegistration {
    /// Validate the bounded wire fields and move the secret into a zeroizing allocation.
    pub fn new(
        client_id: String,
        client_secret: String,
        issuer: String,
        auth_method: McpOAuthClientAuthMethod,
        resource_metadata_url: Option<String>,
    ) -> Result<Self, McpOAuthClientRegistrationError> {
        if !valid_component(&client_id, 4 * 1024)
            || !valid_component(&client_secret, 16 * 1024)
            || !valid_component(&issuer, 8 * 1024)
            || resource_metadata_url
                .as_ref()
                .is_some_and(|value| !valid_component(value, 8 * 1024))
        {
            return Err(McpOAuthClientRegistrationError);
        }
        Ok(Self {
            client_id,
            client_secret: Arc::new(Zeroizing::new(client_secret)),
            issuer,
            auth_method,
            resource_metadata_url,
        })
    }

    /// Registered client id (not a secret).
    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Explicit secret exposure only for vault sealing or protocol request construction.
    #[must_use]
    pub fn expose_client_secret(&self) -> &str {
        self.client_secret.as_str()
    }

    /// Authorization-server issuer that this registration is bound to.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Registered token endpoint client-auth method.
    #[must_use]
    pub const fn auth_method(&self) -> McpOAuthClientAuthMethod {
        self.auth_method
    }

    /// Optional administrator-pinned protected-resource metadata URL.
    #[must_use]
    pub fn resource_metadata_url(&self) -> Option<&str> {
        self.resource_metadata_url.as_deref()
    }
}

impl fmt::Debug for McpOAuthClientRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpOAuthClientRegistration")
            .field("client_id_bytes", &self.client_id.len())
            .field("client_secret", &"[redacted]")
            .field("issuer", &"[configured]")
            .field("auth_method", &self.auth_method)
            .field(
                "resource_metadata_url",
                &self.resource_metadata_url.is_some(),
            )
            .finish()
    }
}

impl PartialEq for McpOAuthClientRegistration {
    fn eq(&self, other: &Self) -> bool {
        self.client_id == other.client_id
            && self.issuer == other.issuer
            && self.auth_method == other.auth_method
            && self.resource_metadata_url == other.resource_metadata_url
            && bool::from(
                self.client_secret
                    .as_bytes()
                    .ct_eq(other.client_secret.as_bytes()),
            )
    }
}

impl Eq for McpOAuthClientRegistration {}

impl Serialize for McpOAuthClientRegistration {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire<'a> {
            client_id: &'a str,
            client_secret: &'a str,
            issuer: &'a str,
            token_endpoint_auth_method: McpOAuthClientAuthMethod,
            #[serde(skip_serializing_if = "Option::is_none")]
            resource_metadata_url: Option<&'a str>,
        }
        Wire {
            client_id: self.client_id(),
            client_secret: self.expose_client_secret(),
            issuer: self.issuer(),
            token_endpoint_auth_method: self.auth_method,
            resource_metadata_url: self.resource_metadata_url(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for McpOAuthClientRegistration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            client_id: String,
            client_secret: String,
            issuer: String,
            #[serde(default)]
            token_endpoint_auth_method: McpOAuthClientAuthMethod,
            #[serde(default)]
            resource_metadata_url: Option<String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.client_id,
            wire.client_secret,
            wire.issuer,
            wire.token_endpoint_auth_method,
            wire.resource_metadata_url,
        )
        .map_err(D::Error::custom)
    }
}

/// Registration contained an empty, oversized or NUL-bearing field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("mcp_oauth_client_registration_invalid")]
pub struct McpOAuthClientRegistrationError;

/// Successful OAuth-client registration acknowledgement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpOAuthClientRegistered {
    /// Always true for the successful reply variant.
    pub ok: bool,
}

/// Result of adding or refreshing one reviewed server catalogue entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerMutation {
    /// Stable curated/server id.
    pub server_id: String,
    /// Monotonic catalog generation committed by PostgreSQL.
    pub catalog_generation: u64,
    /// Number of tools currently present in the refreshed server catalogue.
    pub tool_count: u32,
    /// Existing grants suspended because identity/schema/effect no longer matched.
    pub suspended_grants: u32,
}

impl McpOAuthClientRegistered {
    /// Construct the only successful acknowledgement.
    #[must_use]
    pub const fn success() -> Self {
        Self { ok: true }
    }
}

fn valid_component(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.as_bytes().contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_client_wire_round_trips_but_debug_never_contains_the_secret() {
        let raw = r#"{"clientId":"client","clientSecret":"CLIENT-SECRET-CANARY","issuer":"https://issuer.example","tokenEndpointAuthMethod":"client_secret_basic"}"#;
        let registration: McpOAuthClientRegistration = serde_json::from_str(raw).unwrap();
        assert!(!format!("{registration:?}").contains("CLIENT-SECRET-CANARY"));
        let shared = registration.clone();
        drop(registration);
        assert_eq!(shared.expose_client_secret(), "CLIENT-SECRET-CANARY");
        let encoded = serde_json::to_value(&shared).unwrap();
        assert_eq!(encoded["clientSecret"], "CLIENT-SECRET-CANARY");
        assert_eq!(encoded["tokenEndpointAuthMethod"], "client_secret_basic");
    }

    #[test]
    fn empty_or_nul_client_fields_are_rejected() {
        assert!(
            McpOAuthClientRegistration::new(
                String::new(),
                "secret".to_owned(),
                "https://issuer.example".to_owned(),
                McpOAuthClientAuthMethod::ClientSecretBasic,
                None,
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<McpOAuthClientRegistration>(
                r#"{"clientId":"client","clientSecret":"bad\u0000secret","issuer":"https://issuer.example"}"#
            )
            .is_err()
        );
    }
}

/// Closed screen that may receive the browser after an OAuth callback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpOAuthReturnTo {
    /// Personal connected-accounts settings.
    #[default]
    Settings,
    /// Deployment plugin administration detail.
    Admin,
}

/// One actor-owned OAuth connection. Credential identifiers and secrets are deliberately absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpConnection {
    /// Stable MCP server/catalog identifier.
    pub server_id: String,
    /// Exact scope string returned by the authorization server.
    pub scope: String,
    /// Database-clock connection timestamp.
    pub connected_at: OffsetDateTime,
}

/// Personal connection page payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpConnections {
    /// Stable ids for compile-time reviewed user-OAuth servers enabled by this deployment.
    /// Display metadata is intentionally absent and remains a localized UI concern.
    pub available_server_ids: Vec<String>,
    /// Connections owned by the authenticated actor only.
    pub connections: Vec<McpConnection>,
    /// Exact registered Server callback URI, or `None` when this deployment cannot run OAuth.
    pub redirect_uri: Option<String>,
}

/// Authorization URL created from validated vendor metadata and one-time state/PKCE.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpOAuthAuthorization {
    /// URL the browser may deliberately navigate to.
    pub authorization_url: String,
}

/// Vendor-side status after local access has already been tombstoned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpVendorRevocationStatus {
    /// Vendor confirmed token revocation.
    Revoked,
    /// Local deny is final, while vendor revocation remains queued for reconciliation.
    Pending,
}

/// Disconnect receipt. It never implies vendor revocation when only local retirement succeeded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpConnectionDisconnected {
    /// Stable server identifier that was disconnected.
    pub server_id: String,
    /// Truthful vendor reconciliation state.
    pub vendor_revocation: McpVendorRevocationStatus,
}

#[cfg(test)]
mod personal_connection_tests {
    use super::*;

    #[test]
    fn personal_connections_wire_is_closed_and_contains_no_server_metadata_or_credentials() {
        let page = McpConnections {
            available_server_ids: vec!["google-drive".to_owned()],
            connections: vec![McpConnection {
                server_id: "google-drive".to_owned(),
                scope: "drive.readonly".to_owned(),
                connected_at: OffsetDateTime::UNIX_EPOCH,
            }],
            redirect_uri: Some("https://app.example.test/api/plugins/oauth/callback".to_owned()),
        };
        assert_eq!(page.available_server_ids, ["google-drive"]);
        assert_eq!(page.connections.len(), 1);
        let encoded = serde_json::to_value(&page).unwrap();
        assert_eq!(
            encoded
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["availableServerIds", "connections", "redirectUri"]
        );
        assert_eq!(
            serde_json::from_value::<McpConnections>(encoded).unwrap(),
            page
        );
        assert!(
            serde_json::from_str::<McpConnections>(
                r#"{"availableServerIds":[],"connections":[],"redirectUri":null,"clientSecret":"forbidden"}"#
            )
            .is_err()
        );
    }
}
