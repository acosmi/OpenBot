//! Bounded control/boot wire for the Rust-owned engine host.

use std::path::Path;

use openbot_contracts::engine::{
    ENGINE_PROTOCOL_VERSION, ENGINE_RELEASE_EPOCH, MAX_ENGINE_BOOT_BYTES,
};
use openbot_contracts::ids::{ComputerGeneration, ComputerId, TabId};
use serde::{Deserialize, Serialize};

use super::scope::{EngineRole, EngineRoleKind};

/// Rust-minted operation identifier. Renderer/shim input can only echo it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineOperationId(String);

impl EngineOperationId {
    /// Construct a bounded operation ID from authority-owned state.
    pub fn new(value: impl Into<String>) -> Result<Self, EngineProtocolError> {
        let value = value.into();
        validate_string(&value, 128)
            .then_some(Self(value))
            .ok_or(EngineProtocolError::InvalidOperationId)
    }

    /// Borrow the wire value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct BootCapability {
    protocol_version: u16,
    release_epoch: String,
    control_pipe: String,
    frame_pipe: String,
    token: String,
    role: EngineRoleKind,
    scope_digest: String,
    computer_id: String,
    generation: String,
}

impl BootCapability {
    pub(crate) fn new(
        control_pipe: &Path,
        frame_pipe: &Path,
        token: &BootToken,
        role: &EngineRole,
        computer_id: &ComputerId,
        generation: ComputerGeneration,
    ) -> Result<Self, EngineProtocolError> {
        let control_pipe = control_pipe
            .to_str()
            .filter(|value| validate_string(value, 512))
            .ok_or(EngineProtocolError::InvalidPipePath)?
            .to_owned();
        let frame_pipe = frame_pipe
            .to_str()
            .filter(|value| validate_string(value, 512))
            .ok_or(EngineProtocolError::InvalidPipePath)?
            .to_owned();
        if !validate_string(computer_id.as_str(), 256) {
            return Err(EngineProtocolError::InvalidComputerId);
        }
        Ok(Self {
            protocol_version: ENGINE_PROTOCOL_VERSION,
            release_epoch: ENGINE_RELEASE_EPOCH.to_string(),
            control_pipe,
            frame_pipe,
            token: token.hex(),
            role: role.kind(),
            scope_digest: hex(&role.scope_digest()),
            computer_id: computer_id.as_str().to_owned(),
            generation: generation.get().to_string(),
        })
    }

    pub(crate) fn line(&self) -> Result<Vec<u8>, EngineProtocolError> {
        let mut line = serde_json::to_vec(self).map_err(|_| EngineProtocolError::EncodeFailed)?;
        line.push(b'\n');
        if line.len() > MAX_ENGINE_BOOT_BYTES {
            return Err(EngineProtocolError::BootTooLarge);
        }
        Ok(line)
    }
}

#[derive(Clone)]
pub(crate) struct BootToken([u8; 16]);

impl BootToken {
    pub(crate) fn random() -> Result<Self, EngineProtocolError> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| EngineProtocolError::RandomFailed)?;
        Ok(Self(bytes))
    }

    pub(crate) fn bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub(crate) fn hex(&self) -> String {
        hex(&self.0)
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum EngineCommandWire<'a> {
    Start {
        operation_id: &'a str,
        computer_id: &'a str,
        generation: String,
        tab_id: &'a str,
    },
    Stop {
        operation_id: &'a str,
        computer_id: &'a str,
        generation: String,
        tab_id: &'a str,
    },
    Shutdown {
        operation_id: &'a str,
    },
}

impl<'a> EngineCommandWire<'a> {
    pub(crate) fn start(
        operation: &'a EngineOperationId,
        computer: &'a ComputerId,
        generation: ComputerGeneration,
        tab: &'a TabId,
    ) -> Self {
        Self::Start {
            operation_id: operation.as_str(),
            computer_id: computer.as_str(),
            generation: generation.get().to_string(),
            tab_id: tab.as_str(),
        }
    }

    pub(crate) fn stop(
        operation: &'a EngineOperationId,
        computer: &'a ComputerId,
        generation: ComputerGeneration,
        tab: &'a TabId,
    ) -> Self {
        Self::Stop {
            operation_id: operation.as_str(),
            computer_id: computer.as_str(),
            generation: generation.get().to_string(),
            tab_id: tab.as_str(),
        }
    }

    pub(crate) fn shutdown(operation: &'a EngineOperationId) -> Self {
        Self::Shutdown {
            operation_id: operation.as_str(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum EngineEventWire {
    Hello {
        token: String,
    },
    Ready {
        main_pid: u32,
        main_creation_time: f64,
        protocol_version: u16,
    },
    Started {
        operation_id: String,
        tab_id: String,
        renderer_pid: u32,
        renderer_creation_time: f64,
        renderer_sandboxed: bool,
        node_exposed: bool,
        origin: String,
    },
    Stopped {
        operation_id: String,
    },
    ShutdownComplete {
        operation_id: String,
    },
    Error {
        #[serde(default)]
        operation_id: Option<String>,
        code: String,
    },
}

/// Stable protocol construction/parsing failures.
#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
pub enum EngineProtocolError {
    /// Operation ID was empty, too long, or contained NUL.
    #[error("engine_operation_id_invalid")]
    InvalidOperationId,
    /// Boot pipe path was not bounded UTF-8.
    #[error("engine_pipe_path_invalid")]
    InvalidPipePath,
    /// Computer ID cannot fit the bounded wire.
    #[error("engine_computer_id_invalid")]
    InvalidComputerId,
    /// OS CSPRNG failed.
    #[error("engine_boot_random_failed")]
    RandomFailed,
    /// JSON encoding failed.
    #[error("engine_protocol_encode_failed")]
    EncodeFailed,
    /// Boot capability exceeded 4 KiB.
    #[error("engine_boot_too_large")]
    BootTooLarge,
}

pub(crate) fn encode_command(
    command: &EngineCommandWire<'_>,
) -> Result<Vec<u8>, EngineProtocolError> {
    let mut bytes = serde_json::to_vec(command).map_err(|_| EngineProtocolError::EncodeFailed)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn validate_string(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.contains('\0')
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use openbot_contracts::ids::{
        BotId, ChannelId, ComputerGeneration, ComputerId, CredentialPrincipalId, TenantId,
    };

    use super::{BootCapability, BootToken, EngineOperationId};
    use crate::engine::{ComputerSecurityScope, EngineRole, WorkspaceScope};

    #[test]
    fn boot_line_is_one_bounded_line_and_contains_no_actor_policy_or_intent() {
        let role = EngineRole::BrowserComputer(ComputerSecurityScope::new(
            TenantId::new("tenant"),
            BotId::new("bot"),
            CredentialPrincipalId::new("principal"),
            WorkspaceScope::Channel(ChannelId::new("channel")),
        ));
        let boot = BootCapability::new(
            std::path::Path::new("/tmp/control.sock"),
            std::path::Path::new("/tmp/frame.sock"),
            &BootToken([7; 16]),
            &role,
            &ComputerId::new("computer"),
            ComputerGeneration::new(3),
        )
        .expect("boot");
        let line = String::from_utf8(boot.line().expect("line")).expect("utf8");
        assert_eq!(line.matches('\n').count(), 1);
        for forbidden in ["actor_id", "policy", "intent", "decision"] {
            assert!(!line.contains(forbidden));
        }
    }

    #[test]
    fn operation_id_is_bounded() {
        assert!(EngineOperationId::new("op-1").is_ok());
        assert!(EngineOperationId::new("").is_err());
        assert!(EngineOperationId::new("x".repeat(129)).is_err());
    }
}
