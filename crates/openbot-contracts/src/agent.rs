//! Remote Agent administration wire types.

use core::fmt;

use serde::de::Error as _;
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use subtle::ConstantTimeEq as _;
use zeroize::Zeroizing;

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
