//! Authority-owned engine role and scope models from v4 §10.1 / §10.6.

use openbot_contracts::ids::{
    ActorId, BotId, ChannelId, CredentialPrincipalId, TenantId, ThreadId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Workspace anchor; Browser profiles may persist, workspace roots never cross this boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceScope {
    /// Channel-owned workspace.
    Channel(ChannelId),
    /// Direct-thread-owned workspace.
    Thread(ThreadId),
}

/// `ProfileScope + WorkspaceScope`; `bot_id` alone is deliberately insufficient.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComputerSecurityScope {
    tenant_id: TenantId,
    bot_id: BotId,
    credential_principal_id: CredentialPrincipalId,
    workspace: WorkspaceScope,
}

impl ComputerSecurityScope {
    /// Construct the complete browser isolation scope.
    #[must_use]
    pub fn new(
        tenant_id: TenantId,
        bot_id: BotId,
        credential_principal_id: CredentialPrincipalId,
        workspace: WorkspaceScope,
    ) -> Self {
        Self {
            tenant_id,
            bot_id,
            credential_principal_id,
            workspace,
        }
    }
}

/// Desktop application-window session ID. It is internal and never accepted from renderer input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopWindowSessionId(String);

impl DesktopWindowSessionId {
    /// Construct a bounded non-empty session ID minted by the Rust Desktop host.
    pub fn new(value: impl Into<String>) -> Result<Self, ScopeError> {
        let value = value.into();
        if value.is_empty() || value.len() > 256 || value.contains('\0') {
            return Err(ScopeError::InvalidWindowSessionId);
        }
        Ok(Self(value))
    }
}

/// Temporary Desktop component engine scope; it deliberately has no persistent profile principal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentRenderScope {
    tenant_id: TenantId,
    actor_id: ActorId,
    desktop_window_session_id: DesktopWindowSessionId,
}

impl ComponentRenderScope {
    /// Construct one Desktop application-window component scope.
    #[must_use]
    pub fn new(
        tenant_id: TenantId,
        actor_id: ActorId,
        desktop_window_session_id: DesktopWindowSessionId,
    ) -> Self {
        Self {
            tenant_id,
            actor_id,
            desktop_window_session_id,
        }
    }
}

/// Closed role tag carried in the one-shot boot capability.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineRoleKind {
    /// Browser Computer role.
    BrowserComputer,
    /// Desktop sandboxed component role.
    SandboxedComponent,
}

/// Full authority-owned role. Only its closed tag and opaque scope digest cross into the shim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineRole {
    /// Persistent browser profile + workspace scope.
    BrowserComputer(ComputerSecurityScope),
    /// Temporary per-window component scope.
    SandboxedComponent(ComponentRenderScope),
}

impl EngineRole {
    /// Closed wire tag.
    #[must_use]
    pub const fn kind(&self) -> EngineRoleKind {
        match self {
            Self::BrowserComputer(_) => EngineRoleKind::BrowserComputer,
            Self::SandboxedComponent(_) => EngineRoleKind::SandboxedComponent,
        }
    }

    /// Opaque deterministic digest used for partition naming without exposing actor/tenant IDs.
    #[must_use]
    pub fn scope_digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        match self {
            Self::BrowserComputer(scope) => {
                push(&mut hash, b"browser-computer-v1");
                push(&mut hash, scope.tenant_id.as_str().as_bytes());
                push(&mut hash, scope.bot_id.as_str().as_bytes());
                push(&mut hash, scope.credential_principal_id.as_str().as_bytes());
                match &scope.workspace {
                    WorkspaceScope::Channel(id) => {
                        push(&mut hash, b"channel");
                        push(&mut hash, id.as_str().as_bytes());
                    }
                    WorkspaceScope::Thread(id) => {
                        push(&mut hash, b"thread");
                        push(&mut hash, id.as_str().as_bytes());
                    }
                }
            }
            Self::SandboxedComponent(scope) => {
                push(&mut hash, b"sandboxed-component-v1");
                push(&mut hash, scope.tenant_id.as_str().as_bytes());
                push(&mut hash, scope.actor_id.as_str().as_bytes());
                push(&mut hash, scope.desktop_window_session_id.0.as_bytes());
            }
        }
        hash.finalize().into()
    }
}

/// Scope construction failures.
#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScopeError {
    /// Desktop window session ID was empty, over 256 bytes, or contained NUL.
    #[error("engine_window_session_id_invalid")]
    InvalidWindowSessionId,
}

fn push(hash: &mut Sha256, value: &[u8]) {
    hash.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_le_bytes());
    hash.update(value);
}

#[cfg(test)]
mod tests {
    use openbot_contracts::ids::{
        ActorId, BotId, ChannelId, CredentialPrincipalId, TenantId, ThreadId,
    };

    use super::{
        ComponentRenderScope, ComputerSecurityScope, DesktopWindowSessionId, EngineRole,
        WorkspaceScope,
    };

    #[test]
    fn every_scope_axis_changes_the_opaque_partition_digest() {
        let browser = |tenant: &str, bot: &str, principal: &str, workspace: WorkspaceScope| {
            EngineRole::BrowserComputer(ComputerSecurityScope::new(
                TenantId::new(tenant),
                BotId::new(bot),
                CredentialPrincipalId::new(principal),
                workspace,
            ))
            .scope_digest()
        };
        let baseline = browser(
            "tenant-a",
            "bot-a",
            "principal-a",
            WorkspaceScope::Channel(ChannelId::new("channel-a")),
        );
        for changed in [
            browser(
                "tenant-b",
                "bot-a",
                "principal-a",
                WorkspaceScope::Channel(ChannelId::new("channel-a")),
            ),
            browser(
                "tenant-a",
                "bot-b",
                "principal-a",
                WorkspaceScope::Channel(ChannelId::new("channel-a")),
            ),
            browser(
                "tenant-a",
                "bot-a",
                "principal-b",
                WorkspaceScope::Channel(ChannelId::new("channel-a")),
            ),
            browser(
                "tenant-a",
                "bot-a",
                "principal-a",
                WorkspaceScope::Thread(ThreadId::new("channel-a")),
            ),
        ] {
            assert_ne!(baseline, changed);
        }
    }

    #[test]
    fn component_digest_binds_window_actor_and_tenant() {
        let role = EngineRole::SandboxedComponent(ComponentRenderScope::new(
            TenantId::new("tenant-a"),
            ActorId::new("actor-a"),
            DesktopWindowSessionId::new("window-a").expect("session"),
        ));
        let changed = EngineRole::SandboxedComponent(ComponentRenderScope::new(
            TenantId::new("tenant-a"),
            ActorId::new("actor-a"),
            DesktopWindowSessionId::new("window-b").expect("session"),
        ));
        assert_ne!(role.scope_digest(), changed.scope_digest());
    }
}
