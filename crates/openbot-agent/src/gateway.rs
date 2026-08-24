//! Built-in/remote Agent 进入 application tool pipeline 的唯一入口。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use openbot_application::ApplicationService;
use openbot_contracts::auth::AuthContext;
use openbot_contracts::command::{AppCommand, AppReply};
use openbot_contracts::error::AppError;
use openbot_contracts::ids::{BotId, RunId, ToolCallId};
use openbot_contracts::tool::{ToolInvocation, ToolResult};
use serde_json::Value;
use uuid::Uuid;

/// Agent tool gateway。调用方只能给 run/Bot/tool/args；actor 来自 `AuthContext`，call id 与
/// sequence 由本类型铸造，模型/remote Agent 没有自报三者的参数位。
pub struct AgentToolGateway {
    application: Arc<dyn ApplicationService>,
    next_sequence: Mutex<BTreeMap<RunId, u64>>,
}

impl AgentToolGateway {
    /// 绑定唯一 application 实例。
    #[must_use]
    pub fn new(application: Arc<dyn ApplicationService>) -> Self {
        Self {
            application,
            next_sequence: Mutex::new(BTreeMap::new()),
        }
    }

    /// 铸造调用身份并穿过 `ApplicationService::execute`。
    pub async fn invoke(
        &self,
        auth: AuthContext,
        run_id: RunId,
        bot_id: BotId,
        tool_name: impl Into<String>,
        arguments: Value,
    ) -> Result<ToolResult, AppError> {
        let call_seq = {
            let mut sequences =
                self.next_sequence
                    .lock()
                    .map_err(|_| AppError::DependencyUnavailable {
                        dependency: "agent_tool_sequence",
                    })?;
            let next = sequences.entry(run_id.clone()).or_insert(0);
            let current = *next;
            *next = next.checked_add(1).ok_or(AppError::DependencyUnavailable {
                dependency: "agent_tool_sequence",
            })?;
            current
        };
        let invocation = ToolInvocation {
            call_id: ToolCallId::new(Uuid::now_v7().to_string()),
            run_id,
            bot_id,
            call_seq,
            tool_name: tool_name.into(),
            arguments,
        };
        match self
            .application
            .execute(auth, AppCommand::InvokeTool(invocation))
            .await?
        {
            AppReply::Tool(result) => Ok(result),
            AppReply::Health(_)
            | AppReply::Channels(_)
            | AppReply::CurrentUser(_)
            | AppReply::AdminStatus(_)
            | AppReply::People(_)
            | AppReply::Person(_)
            | AppReply::AuditEvents(_)
            | AppReply::ActionPolicy { .. } => Err(AppError::DependencyUnavailable {
                dependency: "application",
            }),
        }
    }

    /// 借出同一个 application trait object，供组装测试证明没有第二个业务实例。
    #[must_use]
    pub fn application(&self) -> &Arc<dyn ApplicationService> {
        &self.application
    }
}

impl core::fmt::Debug for AgentToolGateway {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AgentToolGateway")
            .field("application", &"<dyn ApplicationService>")
            .field("sequence_state", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use async_trait::async_trait;
    use openbot_application::{AppEventStream, health};
    use openbot_contracts::auth::Role;
    use openbot_contracts::command::{HealthReport, SubscriptionRequest};
    use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};
    use openbot_contracts::tool::{ToolCommitState, ToolResult};
    use serde_json::json;

    use super::*;

    struct FakeApplication {
        calls: Mutex<Vec<(ActorId, ToolInvocation)>>,
        wrong_reply: bool,
    }

    #[async_trait]
    impl ApplicationService for FakeApplication {
        async fn execute(
            &self,
            auth: AuthContext,
            command: AppCommand,
        ) -> Result<AppReply, AppError> {
            let AppCommand::InvokeTool(invocation) = command else {
                return Ok(AppReply::Health(HealthReport { ok: true }));
            };
            self.calls
                .lock()
                .unwrap()
                .push((auth.actor().clone(), invocation.clone()));
            if self.wrong_reply {
                return Ok(AppReply::Health(health()));
            }
            Ok(AppReply::Tool(ToolResult {
                call_id: invocation.call_id,
                content: "ok".to_owned(),
                error_code: None,
                commit_state: ToolCommitState::Committed,
                visible_bytes: 2,
                truncated: false,
            }))
        }

        async fn subscribe(
            &self,
            _auth: AuthContext,
            _request: SubscriptionRequest,
        ) -> Result<AppEventStream, AppError> {
            Ok(openbot_application::use_cases::health_stream(
                Duration::from_secs(1),
            ))
        }
    }

    fn auth(actor: &str) -> AuthContext {
        AuthContext::for_test(
            DeploymentId::new("dep-1"),
            TenantId::new("tenant-1"),
            ActorId::new(actor),
            [Role::User],
            openbot_contracts::auth::AuthGeneration::new(1),
            false,
        )
    }

    fn fake(wrong_reply: bool) -> Arc<FakeApplication> {
        Arc::new(FakeApplication {
            calls: Mutex::new(Vec::new()),
            wrong_reply,
        })
    }

    #[tokio::test]
    async fn actor_call_id_and_sequence_are_all_rust_authoritative() {
        let application = fake(false);
        let gateway = AgentToolGateway::new(application.clone());
        for (run, expected_seq) in [("run-a", 0), ("run-a", 1), ("run-b", 0)] {
            gateway
                .invoke(
                    auth("actor-verified"),
                    RunId::new(run),
                    BotId::new("bot-1"),
                    "computer.write",
                    json!({"claimedActor":"attacker","callSeq":999}),
                )
                .await
                .unwrap();
            let calls = application.calls.lock().unwrap();
            let (actor, invocation) = calls.last().unwrap();
            assert_eq!(actor.as_str(), "actor-verified");
            assert_eq!(invocation.call_seq, expected_seq);
            assert_eq!(
                Uuid::parse_str(invocation.call_id.as_str())
                    .unwrap()
                    .get_version_num(),
                7,
            );
        }
        let calls = application.calls.lock().unwrap();
        assert_ne!(calls[0].1.call_id, calls[1].1.call_id);
        assert_ne!(calls[1].1.call_id, calls[2].1.call_id);
    }

    #[tokio::test]
    async fn concurrent_calls_get_a_gap_free_unique_sequence_per_run() {
        let application = fake(false);
        let gateway = Arc::new(AgentToolGateway::new(application.clone()));
        let mut tasks = Vec::new();
        for _ in 0..32 {
            let gateway = Arc::clone(&gateway);
            tasks.push(tokio::spawn(async move {
                gateway
                    .invoke(
                        auth("actor-1"),
                        RunId::new("run-concurrent"),
                        BotId::new("bot-1"),
                        "computer.write",
                        json!({}),
                    )
                    .await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        let mut sequences: Vec<u64> = application
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, invocation)| invocation.call_seq)
            .collect();
        sequences.sort_unstable();
        assert_eq!(sequences, (0..32).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn a_mismatched_application_reply_is_not_forged_into_tool_success() {
        let application = fake(true);
        let gateway = AgentToolGateway::new(application);
        let error = gateway
            .invoke(
                auth("actor-1"),
                RunId::new("run-1"),
                BotId::new("bot-1"),
                "computer.write",
                json!({}),
            )
            .await
            .expect_err("非 Tool reply 必须报契约破损");
        assert_eq!(error.code().as_str(), "dependency_unavailable");
    }
}
