//! Remote Agent administration wire types.

use core::fmt;

use serde::de::Error as _;
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use subtle::ConstantTimeEq as _;
use zeroize::Zeroizing;

use crate::ids::BotId;

/// Browser-visible coworker visibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentVisibility {
    /// Visible to every authenticated actor.
    Public,
    /// Visible only to the owner and administrators.
    Private,
}

/// Closed coworker projection; no credential value or raw configuration crosses this boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentProfile {
    /// Agent identity.
    pub id: BotId,
    /// Display name.
    pub name: String,
    /// Short title.
    pub title: String,
    /// Standing role description.
    pub role_description: String,
    /// Deterministic avatar seed.
    pub avatar_seed: String,
    /// Public/private visibility.
    pub visibility: AgentVisibility,
    /// Remote AG-UI endpoint, or `None` for built-in.
    pub endpoint: Option<String>,
    /// Whether a write-only outbound auth reference exists; never the key.
    pub has_auth: bool,
    /// Whether a hash-only callback credential exists; never the token.
    pub has_callback_token: bool,
    /// Per-current-user hidden state.
    pub hidden: bool,
    /// Whether the profile came from a deployment package.
    pub system_owned: bool,
    /// Server-decided current-actor management permission.
    pub can_manage: bool,
    /// Whether the current actor owns this profile.
    pub mine: bool,
}

/// Exact `GET /api/agents` response envelope.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfilesResponse {
    /// Visible profiles.
    pub agents: Vec<AgentProfile>,
}

/// Exact `GET /api/agents/{agent_id}` response envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileResponse {
    /// Visible profile.
    pub agent: AgentProfile,
}

/// One-time callback credential response.
///
/// The value must cross the response boundary once, so it implements serde. It deliberately does
/// not implement `Clone` or `Display`; `Debug` is redacted and the owned allocation is zeroized on
/// drop. PostgreSQL stores only its SHA-256 digest.
pub struct CallbackTokenIssued {
    token: Zeroizing<String>,
}

impl CallbackTokenIssued {
    /// Take ownership of an already validated freshly minted token.
    pub fn new(token: String) -> Result<Self, CallbackTokenWireError> {
        if token.is_empty() || token.len() > 4096 || token.as_bytes().contains(&0) {
            return Err(CallbackTokenWireError::Invalid);
        }
        Ok(Self {
            token: Zeroizing::new(token),
        })
    }

    /// Explicitly expose the token only to hashing or response serialization call sites.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.token.as_str()
    }
}

impl fmt::Debug for CallbackTokenIssued {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallbackTokenIssued")
            .field("token", &"[redacted]")
            .finish()
    }
}

impl PartialEq for CallbackTokenIssued {
    fn eq(&self, other: &Self) -> bool {
        self.token.as_bytes().ct_eq(other.token.as_bytes()).into()
    }
}

impl Eq for CallbackTokenIssued {}

impl Serialize for CallbackTokenIssued {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("CallbackTokenIssued", 1)?;
        state.serialize_field("token", self.expose())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for CallbackTokenIssued {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            token: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.token).map_err(D::Error::custom)
    }
}

/// Callback token wire value is empty, oversized, or contains NUL.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CallbackTokenWireError {
    /// Invalid token response value.
    #[error("callback_token_wire_invalid")]
    Invalid,
}

/// Successful idempotent callback-token revocation acknowledgement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallbackTokenRevoked;

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> AgentProfile {
        AgentProfile {
            id: BotId::new("agent-1"),
            name: "Research".to_owned(),
            title: "Research coworker".to_owned(),
            role_description: "Find cited facts.".to_owned(),
            avatar_seed: "research-seed".to_owned(),
            visibility: AgentVisibility::Private,
            endpoint: Some("https://agent.example.test/ag-ui".to_owned()),
            has_auth: true,
            has_callback_token: true,
            hidden: false,
            system_owned: false,
            can_manage: true,
            mine: true,
        }
    }

    #[test]
    fn agent_projection_envelopes_are_closed_camel_case_and_secret_free() {
        let list = AgentProfilesResponse {
            agents: vec![profile()],
        };
        let value = serde_json::to_value(&list).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "agents": [{
                    "id": "agent-1",
                    "name": "Research",
                    "title": "Research coworker",
                    "roleDescription": "Find cited facts.",
                    "avatarSeed": "research-seed",
                    "visibility": "private",
                    "endpoint": "https://agent.example.test/ag-ui",
                    "hasAuth": true,
                    "hasCallbackToken": true,
                    "hidden": false,
                    "systemOwned": false,
                    "canManage": true,
                    "mine": true
                }]
            })
        );
        assert!(value.to_string().find("credential").is_none());
        assert!(
            serde_json::from_value::<AgentProfileResponse>(serde_json::json!({
                "agent": serde_json::to_value(profile()).unwrap(),
                "ownerUserId": "must-not-cross"
            }))
            .is_err()
        );
        assert_eq!(
            serde_json::from_value::<AgentProfilesResponse>(value).unwrap(),
            list
        );
    }

    #[test]
    fn one_time_token_serializes_but_debug_redacts_and_clone_is_not_available() {
        let token = CallbackTokenIssued::new("obot_agt_secret-value".to_owned()).unwrap();
        assert_eq!(
            serde_json::to_string(&token).unwrap(),
            r#"{"token":"obot_agt_secret-value"}"#
        );
        let debug = format!("{token:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("secret-value"));
        let round_trip: CallbackTokenIssued =
            serde_json::from_str(r#"{"token":"obot_agt_secret-value"}"#).unwrap();
        assert_eq!(round_trip, token);
    }
}
