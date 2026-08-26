//! Current-actor Agent roster and detail reads.

use openbot_contracts::agent::AgentProfile;
use openbot_contracts::auth::{AuthContext, Role};
use openbot_contracts::error::AppError;
use openbot_contracts::ids::BotId;

use crate::ports::{AgentDirectory, AgentReadScope, PortError};

/// List visible or hidden coworkers using only authoritative actor/role scope.
pub async fn list_visible_agents(
    directory: &dyn AgentDirectory,
    auth: &AuthContext,
    hidden: bool,
) -> Result<Vec<AgentProfile>, AppError> {
    directory
        .list_visible_agents(&scope(auth), hidden)
        .await
        .map_err(PortError::into_app_error)
}

/// Read one accessible profile; missing/invisible/deleted collapse to NotVisible.
pub async fn get_visible_agent(
    directory: &dyn AgentDirectory,
    auth: &AuthContext,
    agent_id: BotId,
) -> Result<AgentProfile, AppError> {
    if agent_id.as_str().is_empty()
        || agent_id.as_str().len() > 512
        || agent_id.as_str().chars().any(char::is_control)
    {
        return Err(AppError::MalformedPayload { field: "agent_id" });
    }
    directory
        .get_visible_agent(&scope(auth), &agent_id)
        .await
        .map_err(PortError::into_app_error)?
        .ok_or(AppError::NotVisible)
}

fn scope(auth: &AuthContext) -> AgentReadScope {
    AgentReadScope {
        tenant: auth.tenant().clone(),
        actor: auth.actor().clone(),
        admin: auth.has_role(Role::Admin),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use openbot_contracts::agent::AgentVisibility;
    use openbot_contracts::auth::AuthGeneration;
    use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};

    use super::*;

    struct FakeAgents {
        profile: AgentProfile,
        calls: Mutex<Vec<(AgentReadScope, Option<BotId>, bool)>>,
    }

    #[async_trait]
    impl AgentDirectory for FakeAgents {
        async fn list_visible_agents(
            &self,
            scope: &AgentReadScope,
            hidden: bool,
        ) -> Result<Vec<AgentProfile>, PortError> {
            self.calls
                .lock()
                .unwrap()
                .push((scope.clone(), None, hidden));
            Ok(vec![self.profile.clone()])
        }

        async fn get_visible_agent(
            &self,
            scope: &AgentReadScope,
            agent_id: &BotId,
        ) -> Result<Option<AgentProfile>, PortError> {
            self.calls
                .lock()
                .unwrap()
                .push((scope.clone(), Some(agent_id.clone()), false));
            Ok((agent_id == &self.profile.id).then(|| self.profile.clone()))
        }
    }

    fn auth(admin: bool) -> AuthContext {
        AuthContext::for_test(
            DeploymentId::new("dep"),
            TenantId::new("tenant"),
            ActorId::new("actor"),
            if admin {
                vec![Role::Admin, Role::User]
            } else {
                vec![Role::User]
            },
            AuthGeneration::new(1),
            false,
        )
    }

    fn fake() -> FakeAgents {
        FakeAgents {
            profile: AgentProfile {
                id: BotId::new("agent-1"),
                name: "Agent".to_owned(),
                title: "Title".to_owned(),
                role_description: "Role".to_owned(),
                avatar_seed: "seed".to_owned(),
                visibility: AgentVisibility::Public,
                endpoint: None,
                has_auth: false,
                has_callback_token: false,
                hidden: false,
                system_owned: false,
                can_manage: true,
                mine: true,
            },
            calls: Mutex::new(Vec::new()),
        }
    }

    #[tokio::test]
    async fn list_and_detail_inject_actor_admin_and_hidden_without_self_report() {
        let fake = fake();
        assert_eq!(
            list_visible_agents(&fake, &auth(true), true)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            get_visible_agent(&fake, &auth(false), BotId::new("agent-1"))
                .await
                .unwrap()
                .id
                .as_str(),
            "agent-1"
        );
        assert_eq!(
            fake.calls.lock().unwrap().as_slice(),
            [
                (
                    AgentReadScope {
                        tenant: TenantId::new("tenant"),
                        actor: ActorId::new("actor"),
                        admin: true,
                    },
                    None,
                    true,
                ),
                (
                    AgentReadScope {
                        tenant: TenantId::new("tenant"),
                        actor: ActorId::new("actor"),
                        admin: false,
                    },
                    Some(BotId::new("agent-1")),
                    false,
                ),
            ]
        );
    }

    #[tokio::test]
    async fn malformed_or_missing_detail_never_becomes_an_empty_success() {
        let fake = fake();
        assert_eq!(
            get_visible_agent(&fake, &auth(false), BotId::new("missing")).await,
            Err(AppError::NotVisible)
        );
        let before = fake.calls.lock().unwrap().len();
        assert_eq!(
            get_visible_agent(&fake, &auth(false), BotId::new("bad\nagent")).await,
            Err(AppError::MalformedPayload { field: "agent_id" })
        );
        assert_eq!(fake.calls.lock().unwrap().len(), before);
    }
}
