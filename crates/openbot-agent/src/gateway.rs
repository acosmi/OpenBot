//! Built-in/remote Agent 进入 application tool pipeline 的唯一入口。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use openbot_application::{
    AgentAuthorizationSource, ApplicationService, RemoteCallbackAuthorization, RunExecutionLease,
    ToolCallSequence, ToolCallSequenceError,
};
use openbot_contracts::auth::AuthContext;
use openbot_contracts::command::{AppCommand, AppReply};
use openbot_contracts::error::AppError;
use openbot_contracts::ids::{BotId, RunId, ToolCallId};
use openbot_contracts::tool::{ToolInvocation, ToolResult};
use serde_json::Value;
use uuid::Uuid;

/// Model-visible, redacted reply paired with a Rust-minted control-plane call id.
#[derive(Clone, PartialEq, Eq)]
pub struct AgentToolReply {
    call_id: ToolCallId,
    content: String,
    error_code: Option<String>,
}

impl AgentToolReply {
    /// Construct from an already-redacted tool-pipeline projection.
    pub fn new(
        call_id: ToolCallId,
        content: String,
        error_code: Option<String>,
    ) -> Result<Self, AgentToolInvokeError> {
        if call_id.as_str().is_empty()
            || content.as_bytes().contains(&0)
            || error_code
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.as_bytes().contains(&0))
        {
            return Err(AgentToolInvokeError::Unavailable);
        }
        Ok(Self {
            call_id,
            content,
            error_code,
        })
    }

    /// Rust-minted tool call id used for durable decision/message identities.
    #[must_use]
    pub const fn call_id(&self) -> &ToolCallId {
        &self.call_id
    }

    /// Redacted model-visible content.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Stable error code; absence means success.
    #[must_use]
    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }
}

impl core::fmt::Debug for AgentToolReply {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AgentToolReply")
            .field("call_id", &self.call_id)
            .field("content_bytes", &self.content.len())
            .field("error_code", &self.error_code)
            .finish()
    }
}

/// Tool invocation cannot safely become another provider sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AgentToolInvokeError {
    /// Auth/application/tool dependency is unavailable or the current scope was revoked.
    #[error("agent_tool_unavailable")]
    Unavailable,
    /// A tool effect/outcome has unknown commit state.
    #[error("agent_tool_reconciliation_required")]
    ReconciliationRequired,
}

/// Host-facing tool boundary. Production implementation always reloads AuthContext first.
#[async_trait]
pub trait AgentToolInvoker: Send + Sync {
    /// Invoke one provider call through ApplicationService's unique tool pipeline.
    async fn invoke(
        &self,
        lease: &RunExecutionLease,
        tool_name: &str,
        arguments: Value,
    ) -> Result<AgentToolReply, AgentToolInvokeError>;

    /// Release bounded per-run sequence state after terminal cleanup.
    fn release(&self, _run_id: &RunId) {}
}

/// Machine callback tool boundary after dual-credential/run/tool authorization.
#[async_trait]
pub trait RemoteAgentToolInvoker: Send + Sync {
    /// Invoke through the same ApplicationService pipeline and durable sequence allocator as the
    /// built-in Agent; no callback-specific executor exists.
    async fn invoke_callback(
        &self,
        authorization: RemoteCallbackAuthorization,
        tool_name: &str,
        arguments: Value,
    ) -> Result<AgentToolReply, AgentToolInvokeError>;
}

/// Explicit fail-closed placeholder for tests/runtime configurations without a real tool plane.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoAgentToolInvoker;

#[async_trait]
impl AgentToolInvoker for NoAgentToolInvoker {
    async fn invoke(
        &self,
        _lease: &RunExecutionLease,
        _tool_name: &str,
        _arguments: Value,
    ) -> Result<AgentToolReply, AgentToolInvokeError> {
        Err(AgentToolInvokeError::Unavailable)
    }
}

/// Agent tool gateway。调用方只能给 run/Bot/tool/args；actor 来自 `AuthContext`，call id 与
/// sequence 由本类型铸造，模型/remote Agent 没有自报三者的参数位。
pub struct AgentToolGateway {
    application: Arc<dyn ApplicationService>,
    sequence: Arc<dyn ToolCallSequence>,
}

impl AgentToolGateway {
    /// 绑定唯一 application 实例。
    #[must_use]
    pub fn new(application: Arc<dyn ApplicationService>) -> Self {
        Self::with_sequence(application, Arc::new(InMemoryToolCallSequence::default()))
    }

    /// Bind an authoritative cross-replica sequence allocator for production assembly.
    #[must_use]
    pub fn with_sequence(
        application: Arc<dyn ApplicationService>,
        sequence: Arc<dyn ToolCallSequence>,
    ) -> Self {
        Self {
            application,
            sequence,
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
        self.invoke_captured(auth, run_id, bot_id, tool_name.into(), arguments)
            .await
            .1
    }

    async fn invoke_captured(
        &self,
        auth: AuthContext,
        run_id: RunId,
        bot_id: BotId,
        tool_name: String,
        arguments: Value,
    ) -> (ToolCallId, Result<ToolResult, AppError>) {
        let call_seq = match self.sequence.next(&run_id).await {
            Ok(sequence) => sequence,
            Err(ToolCallSequenceError::Unavailable | ToolCallSequenceError::Exhausted) => {
                return (
                    ToolCallId::new("unavailable"),
                    Err(AppError::DependencyUnavailable {
                        dependency: "agent_tool_sequence",
                    }),
                );
            }
        };
        let call_id = ToolCallId::new(Uuid::now_v7().to_string());
        let invocation = ToolInvocation {
            call_id: call_id.clone(),
            run_id,
            bot_id,
            call_seq,
            tool_name,
            arguments,
        };
        let result = match self
            .application
            .execute(auth, AppCommand::InvokeTool(invocation))
            .await
        {
            Ok(AppReply::Tool(result)) => Ok(result),
            Ok(
                AppReply::Health(_)
                | AppReply::Channels(_)
                | AppReply::Channel(_)
                | AppReply::ChannelRouting(_)
                | AppReply::Agents(_)
                | AppReply::Agent(_)
                | AppReply::CurrentUser(_)
                | AppReply::AdminStatus(_)
                | AppReply::People(_)
                | AppReply::Person(_)
                | AppReply::AuditEvents(_)
                | AppReply::ActionPolicy { .. }
                | AppReply::ThreadMinted(_)
                | AppReply::ThreadStatus(_)
                | AppReply::ThreadRunStarted(_)
                | AppReply::ThreadRunCancellation(_)
                | AppReply::ThreadHistory(_)
                | AppReply::ThreadConversation(_)
                | AppReply::Memory(_)
                | AppReply::MemoryControl(_)
                | AppReply::Memories(_)
                | AppReply::MemoryRecall(_)
                | AppReply::AgentCallbackToken(_)
                | AppReply::AgentCallbackTokenRevoked(_)
                | AppReply::McpConnections(_)
                | AppReply::McpOAuthAuthorization(_)
                | AppReply::McpConnectionDisconnected(_)
                | AppReply::McpOAuthClientRegistered(_)
                | AppReply::McpServerMutation(_)
                | AppReply::PendingToolApprovals(_)
                | AppReply::ToolApprovalResolved(_)
                | AppReply::UiPreferences(_),
            ) => Err(AppError::DependencyUnavailable {
                dependency: "application",
            }),
            Err(error) => Err(error),
        };
        (call_id, result)
    }

    /// Drop per-run sequence state after the host has committed a terminal.
    pub fn release(&self, run_id: &RunId) {
        self.sequence.release(run_id);
    }

    /// 借出同一个 application trait object，供组装测试证明没有第二个业务实例。
    #[must_use]
    pub fn application(&self) -> &Arc<dyn ApplicationService> {
        &self.application
    }
}

/// Production wrapper that reloads current DB authorization before each tool effect.
pub struct AuthorizedAgentToolGateway {
    gateway: AgentToolGateway,
    authorization: Arc<dyn AgentAuthorizationSource>,
}

impl AuthorizedAgentToolGateway {
    /// Bind one ApplicationService and one authoritative authorization source.
    #[must_use]
    pub fn new(
        application: Arc<dyn ApplicationService>,
        authorization: Arc<dyn AgentAuthorizationSource>,
    ) -> Self {
        Self::with_sequence(
            application,
            authorization,
            Arc::new(InMemoryToolCallSequence::default()),
        )
    }

    /// Production constructor using one durable allocator for built-in and remote callbacks.
    #[must_use]
    pub fn with_sequence(
        application: Arc<dyn ApplicationService>,
        authorization: Arc<dyn AgentAuthorizationSource>,
        sequence: Arc<dyn ToolCallSequence>,
    ) -> Self {
        Self {
            gateway: AgentToolGateway::with_sequence(application, sequence),
            authorization,
        }
    }
}

impl core::fmt::Debug for AuthorizedAgentToolGateway {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AuthorizedAgentToolGateway")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl AgentToolInvoker for AuthorizedAgentToolGateway {
    async fn invoke(
        &self,
        lease: &RunExecutionLease,
        tool_name: &str,
        arguments: Value,
    ) -> Result<AgentToolReply, AgentToolInvokeError> {
        let auth = self
            .authorization
            .load(lease)
            .await
            .map_err(|_| AgentToolInvokeError::Unavailable)?;
        let (call_id, result) = self
            .gateway
            .invoke_captured(
                auth,
                lease.run_id().clone(),
                lease.bot_id().clone(),
                tool_name.to_owned(),
                arguments,
            )
            .await;
        map_application_reply(call_id, result)
    }

    fn release(&self, run_id: &RunId) {
        self.gateway.release(run_id);
    }
}

#[async_trait]
impl RemoteAgentToolInvoker for AuthorizedAgentToolGateway {
    async fn invoke_callback(
        &self,
        authorization: RemoteCallbackAuthorization,
        tool_name: &str,
        arguments: Value,
    ) -> Result<AgentToolReply, AgentToolInvokeError> {
        let (call_id, result) = self
            .gateway
            .invoke_captured(
                authorization.auth().clone(),
                authorization.run().clone(),
                authorization.bot().clone(),
                tool_name.to_owned(),
                arguments,
            )
            .await;
        map_application_reply(call_id, result)
    }
}

fn map_application_reply(
    call_id: ToolCallId,
    result: Result<ToolResult, AppError>,
) -> Result<AgentToolReply, AgentToolInvokeError> {
    match result {
        Ok(result) => Ok(AgentToolReply {
            call_id,
            content: result.content,
            error_code: result.error_code.or_else(|| {
                (result.commit_state == openbot_contracts::tool::ToolCommitState::NotCommitted)
                    .then(|| "tool_not_committed".to_owned())
            }),
        }),
        Err(AppError::PolicyRefused { .. }) => Ok(normalized_error_reply(
            call_id,
            "policy_refused",
            "Refused. Tool call was refused by policy.",
        )),
        Err(AppError::MalformedPayload { .. }) => Ok(normalized_error_reply(
            call_id,
            "invalid_arguments",
            "Tool arguments were invalid.",
        )),
        Err(AppError::NotVisible | AppError::ForbiddenRole { .. }) => Ok(normalized_error_reply(
            call_id,
            "tool_not_available",
            "Tool is not available.",
        )),
        Err(AppError::ReconciliationRequired { .. }) => {
            Err(AgentToolInvokeError::ReconciliationRequired)
        }
        Err(
            AppError::Unauthenticated
            | AppError::DependencyUnavailable { .. }
            | AppError::VendorFailure { .. }
            | AppError::StaleGeneration { .. }
            | AppError::RequestConflict { .. }
            | AppError::LeaseConflict { .. }
            | AppError::IdentityConflict { .. }
            | AppError::SensitiveWriteRefused { .. },
        ) => Err(AgentToolInvokeError::Unavailable),
    }
}

fn normalized_error_reply(call_id: ToolCallId, error_code: &str, content: &str) -> AgentToolReply {
    AgentToolReply::new(call_id, content.to_owned(), Some(error_code.to_owned()))
        .expect("static normalized tool reply is valid")
}

impl core::fmt::Debug for AgentToolGateway {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AgentToolGateway")
            .field("application", &"<dyn ApplicationService>")
            .field("sequence", &"<dyn ToolCallSequence>")
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct InMemoryToolCallSequence {
    next: Mutex<BTreeMap<RunId, u64>>,
}

#[async_trait]
impl ToolCallSequence for InMemoryToolCallSequence {
    async fn next(&self, run: &RunId) -> Result<u64, ToolCallSequenceError> {
        let mut sequences = self
            .next
            .lock()
            .map_err(|_| ToolCallSequenceError::Unavailable)?;
        let next = sequences.entry(run.clone()).or_insert(0);
        let current = *next;
        *next = next
            .checked_add(1)
            .ok_or(ToolCallSequenceError::Exhausted)?;
        Ok(current)
    }

    fn release(&self, run: &RunId) {
        if let Ok(mut sequences) = self.next.lock() {
            sequences.remove(run);
        }
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
