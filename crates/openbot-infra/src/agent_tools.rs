//! Production authorization and first-party tool control plane for the built-in Agent.

use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use openbot_application::{
    AgentAuthorizationError, AgentAuthorizationSource, AuthorizedToolCall, REMEMBER_TOOL_NAME,
    RememberToolMemory, RememberToolMemoryRequest, ResolvedToolScope, ToolApprovalRequest,
    ToolControlPlane, ToolExecutionReport, ToolPolicyEvaluation, ToolPortError,
    parse_remember_tool_arguments, remember_tool_metadata,
};
use openbot_contracts::auth::{AuthContext, AuthContextBuilder, AuthGeneration, Role};
use openbot_contracts::ids::{DeploymentId, TenantId};
use openbot_contracts::memory::MemoryStatus;
use openbot_contracts::tool::ToolInvocation;
use openbot_domain::identity::roles::resolve_effective_role;
use openbot_domain::policy::context::{ActorRef, BotRef, Intent, PageRef, PolicyContext, ToolRef};
use openbot_domain::policy::evaluate;
use openbot_domain::tool::approval::ApprovalTarget;
use openbot_domain::tool::args::ToolArguments;
use openbot_domain::tool::commit::CommitState;
use openbot_domain::tool::metadata::{ToolMetadata, ToolName};
use openbot_domain::tool::pipeline::ApprovalOutcome;
use serde_json::json;

use crate::policy::PolicyStore;

/// Reloads actor roles/access/generation and proves the run lease is still active before a tool.
#[derive(Clone)]
pub struct PostgresAgentAuthorizationSource {
    pool: deadpool_postgres::Pool,
    deployment: DeploymentId,
    tenant: TenantId,
    single_user: bool,
}

impl PostgresAgentAuthorizationSource {
    /// Construct for one deployment/tenant runtime.
    #[must_use]
    pub fn new(
        pool: deadpool_postgres::Pool,
        deployment: DeploymentId,
        tenant: TenantId,
        single_user: bool,
    ) -> Self {
        Self {
            pool,
            deployment,
            tenant,
            single_user,
        }
    }
}

impl core::fmt::Debug for PostgresAgentAuthorizationSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PostgresAgentAuthorizationSource")
            .field("deployment", &self.deployment)
            .field("tenant", &self.tenant)
            .field("single_user", &self.single_user)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl AgentAuthorizationSource for PostgresAgentAuthorizationSource {
    async fn load(
        &self,
        lease: &openbot_application::RunExecutionLease,
    ) -> Result<AuthContext, AgentAuthorizationError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|_| AgentAuthorizationError::Unavailable)?;
        let row = client
            .query_opt(
                "SELECT coalesce(u.auth_generation,0) AS auth_generation, \
                        coalesce(bool_or(ra.email IS NOT NULL),false) AS revoked, \
                        coalesce(array_agg(distinct ur.role::text) \
                          FILTER (WHERE ur.role IS NOT NULL),'{}') AS roles \
                 FROM public.runs r \
                 JOIN public.threads t ON t.thread_id=r.thread_id \
                 JOIN public.thread_memberships tm ON tm.thread_id=t.thread_id \
                 JOIN public.users u ON u.id=r.actor_id \
                 JOIN public.thread_leases l ON l.thread_id=r.thread_id \
                 LEFT JOIN public.user_roles ur ON ur.user_id=u.id \
                 LEFT JOIN public.revoked_access ra ON ra.email=lower(u.email) \
                 WHERE r.run_id=$1 AND r.thread_id=$2 AND r.bot_id=$3 AND r.actor_id=$4 \
                   AND r.status='running' AND r.fencing_token=$5 \
                   AND t.deployment_id=$6 AND t.tenant_id=$7 AND t.status<>'deleted' \
                   AND tm.user_id=$4 AND l.fencing_token=$5 AND l.expires_at>now() \
                 GROUP BY u.id",
                &[
                    &lease.run_id().as_str(),
                    &lease.thread_id().as_str(),
                    &lease.bot_id().as_str(),
                    &lease.actor_id().as_str(),
                    &lease.fencing().get(),
                    &self.deployment.as_str(),
                    &self.tenant.as_str(),
                ],
            )
            .await
            .map_err(|_| AgentAuthorizationError::Unavailable)?
            .ok_or(AgentAuthorizationError::Refused)?;
        let revoked: bool = row
            .try_get("revoked")
            .map_err(|_| AgentAuthorizationError::Corrupt { field: "revoked" })?;
        if revoked {
            return Err(AgentAuthorizationError::Refused);
        }
        let generation: i64 =
            row.try_get("auth_generation")
                .map_err(|_| AgentAuthorizationError::Corrupt {
                    field: "auth_generation",
                })?;
        let generation =
            u64::try_from(generation).map_err(|_| AgentAuthorizationError::Corrupt {
                field: "auth_generation",
            })?;
        let raw_roles: Vec<String> = row
            .try_get("roles")
            .map_err(|_| AgentAuthorizationError::Corrupt { field: "roles" })?;
        let roles = raw_roles
            .iter()
            .map(|value| Role::from_str(value))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AgentAuthorizationError::Corrupt { field: "roles" })?;
        let role = resolve_effective_role(roles).map_err(|_| AgentAuthorizationError::Refused)?;
        Ok(AuthContextBuilder::from_verified_session(
            self.deployment.clone(),
            self.tenant.clone(),
            lease.actor_id().clone(),
            AuthGeneration::new(generation),
            self.single_user,
        )
        .with_role(role)
        .build())
    }
}

/// First-party catalog/scope/policy/executor implementation. Initial catalog contains `remember`.
#[derive(Clone)]
pub struct PostgresBuiltInToolControlPlane<M> {
    pool: deadpool_postgres::Pool,
    deployment: DeploymentId,
    tenant: TenantId,
    policy: PolicyStore,
    memory: Arc<M>,
}

impl<M> PostgresBuiltInToolControlPlane<M> {
    /// Construct with the same policy hot cache and memory adapter used by ApplicationService.
    #[must_use]
    pub fn new(
        pool: deadpool_postgres::Pool,
        deployment: DeploymentId,
        tenant: TenantId,
        policy: PolicyStore,
        memory: Arc<M>,
    ) -> Self {
        Self {
            pool,
            deployment,
            tenant,
            policy,
            memory,
        }
    }
}

impl<M> core::fmt::Debug for PostgresBuiltInToolControlPlane<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PostgresBuiltInToolControlPlane")
            .field("deployment", &self.deployment)
            .field("tenant", &self.tenant)
            .field("policy", &self.policy)
            .field("memory", &"<remember-tool-store>")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<M> ToolControlPlane for PostgresBuiltInToolControlPlane<M>
where
    M: RememberToolMemory + 'static,
{
    async fn metadata(&self, name: &ToolName) -> Result<ToolMetadata, ToolPortError> {
        if name.as_str() == REMEMBER_TOOL_NAME {
            Ok(remember_tool_metadata())
        } else {
            Err(ToolPortError::NotVisible)
        }
    }

    async fn resolve_scope(
        &self,
        auth: &AuthContext,
        invocation: &ToolInvocation,
        arguments: &ToolArguments,
        metadata: &ToolMetadata,
    ) -> Result<ResolvedToolScope, ToolPortError> {
        if auth.deployment() != &self.deployment
            || auth.tenant() != &self.tenant
            || metadata.name.as_str() != REMEMBER_TOOL_NAME
        {
            return Err(ToolPortError::NotVisible);
        }
        let parsed = parse_remember_tool_arguments(arguments.as_value())
            .map_err(|_| ToolPortError::InvalidInput { field: "arguments" })?;
        let client = self
            .pool
            .get()
            .await
            .map_err(|_| ToolPortError::Unavailable {
                dependency: "database",
            })?;
        let auth_generation =
            i64::try_from(auth.auth_generation().get()).map_err(|_| ToolPortError::Corrupt {
                field: "auth_generation",
            })?;
        let row = client
            .query_opt(
                "SELECT r.thread_id FROM public.runs r \
                 JOIN public.threads t ON t.thread_id=r.thread_id \
                 JOIN public.thread_memberships tm ON tm.thread_id=t.thread_id \
                 JOIN public.agent_profiles ap ON ap.agent_id=r.bot_id \
                 JOIN public.users u ON u.id=r.actor_id \
                 WHERE r.run_id=$1 AND r.bot_id=$2 AND r.actor_id=$3 AND r.status='running' \
                   AND t.deployment_id=$4 AND t.tenant_id=$5 AND t.status<>'deleted' \
                   AND tm.user_id=$3 AND ap.deleted_at IS NULL \
                   AND (ap.visibility='public' OR ap.owner_user_id=$3) \
                   AND coalesce(u.auth_generation,0)=$7 \
                   AND NOT EXISTS(SELECT 1 FROM public.revoked_access ra \
                                  WHERE ra.email=lower(u.email)) \
                   AND EXISTS(SELECT 1 FROM public.user_roles ur WHERE ur.user_id=u.id) \
                   AND NOT EXISTS(SELECT 1 FROM public.tool_calls c \
                                  WHERE c.run_id=r.run_id AND c.call_seq=$6)",
                &[
                    &invocation.run_id.as_str(),
                    &invocation.bot_id.as_str(),
                    &auth.actor().as_str(),
                    &self.deployment.as_str(),
                    &self.tenant.as_str(),
                    &i64::try_from(invocation.call_seq)
                        .map_err(|_| ToolPortError::InvalidInput { field: "call_seq" })?,
                    &auth_generation,
                ],
            )
            .await
            .map_err(|_| ToolPortError::Unavailable {
                dependency: "database",
            })?
            .ok_or(ToolPortError::NotVisible)?;
        let thread_id: String = row
            .try_get("thread_id")
            .map_err(|_| ToolPortError::Corrupt { field: "thread_id" })?;
        let thread_id = openbot_contracts::ids::ThreadId::new(thread_id);
        let target = match parsed.scope() {
            openbot_application::RememberToolScope::User => ApprovalTarget {
                kind: "memory_user",
                id: auth.actor().as_str().to_owned(),
            },
            openbot_application::RememberToolScope::Bot => ApprovalTarget {
                kind: "memory_bot",
                id: invocation.bot_id.as_str().to_owned(),
            },
            openbot_application::RememberToolScope::Thread => ApprovalTarget {
                kind: "memory_thread",
                id: thread_id.as_str().to_owned(),
            },
        };
        Ok(ResolvedToolScope {
            tenant_id: self.tenant.clone(),
            run_id: invocation.run_id.clone(),
            thread_id,
            bot_id: invocation.bot_id.clone(),
            call_seq: invocation.call_seq,
            target,
            policy_context: PolicyContext {
                tool: ToolRef {
                    name: REMEMBER_TOOL_NAME.to_owned(),
                },
                bot: BotRef {
                    id: invocation.bot_id.as_str().to_owned(),
                },
                page: PageRef {
                    url: "openbot://memory".to_owned(),
                    host: "memory".to_owned(),
                },
                actor: ActorRef {
                    id: auth.actor().as_str().to_owned(),
                },
                element: None,
                key: None,
                intent: Some(Intent::WriteTool),
                file: None,
                mcp: None,
                command: None,
            },
            idempotency_key: None,
        })
    }

    async fn evaluate_policy(
        &self,
        context: &PolicyContext,
    ) -> Result<ToolPolicyEvaluation, ToolPortError> {
        let compiled = self.policy.compiled();
        Ok(ToolPolicyEvaluation::from_domain(&evaluate(
            &compiled, context,
        )))
    }

    async fn approval(
        &self,
        _request: &ToolApprovalRequest,
    ) -> Result<ApprovalOutcome, ToolPortError> {
        Err(ToolPortError::Corrupt {
            field: "remember_approval",
        })
    }

    async fn execute(&self, call: AuthorizedToolCall) -> ToolExecutionReport {
        let started = Instant::now();
        let (call, redeemed) = call.redeem();
        let timeout = call.metadata().timeout;
        let arguments = match parse_remember_tool_arguments(call.arguments().as_value()) {
            Ok(arguments) if call.metadata().name.as_str() == REMEMBER_TOOL_NAME => arguments,
            Ok(_) | Err(_) => {
                return ToolExecutionReport::new(
                    redeemed,
                    r#"{"status":"not_remembered","error":"invalid_arguments"}"#.to_owned(),
                    CommitState::NotCommitted,
                    started.elapsed(),
                    Some("invalid_arguments"),
                );
            }
        };
        let request = RememberToolMemoryRequest {
            tenant: call.tenant().clone(),
            actor: call.actor().actor().clone(),
            auth_generation: call.auth_generation(),
            run: call.run().clone(),
            bot: call.actor().bot().clone(),
            thread: call.thread().clone(),
            arguments,
        };
        let result = tokio::time::timeout(timeout, self.memory.remember_from_tool(request)).await;
        match result {
            Ok(Ok(record)) if record.status == MemoryStatus::Active => ToolExecutionReport::new(
                redeemed,
                json!({"status":"remembered","memoryId":record.memory_id}).to_string(),
                CommitState::Committed,
                started.elapsed(),
                None,
            ),
            Ok(Ok(_)) => ToolExecutionReport::new(
                redeemed,
                r#"{"status":"not_remembered","error":"memory_state_invalid"}"#.to_owned(),
                CommitState::NotCommitted,
                started.elapsed(),
                Some("memory_state_invalid"),
            ),
            Ok(Err(error)) => {
                let (commit, code) = match error {
                    openbot_application::MemoryAdministrationError::CommitUnknown => {
                        (CommitState::Unknown, "memory_commit_unknown")
                    }
                    openbot_application::MemoryAdministrationError::InvalidInput { .. } => {
                        (CommitState::NotCommitted, "invalid_arguments")
                    }
                    openbot_application::MemoryAdministrationError::NotVisible => {
                        (CommitState::NotCommitted, "not_visible")
                    }
                    openbot_application::MemoryAdministrationError::Conflict => {
                        (CommitState::NotCommitted, "memory_conflict")
                    }
                    openbot_application::MemoryAdministrationError::Unavailable
                    | openbot_application::MemoryAdministrationError::Corrupt { .. } => {
                        (CommitState::NotCommitted, "dependency_unavailable")
                    }
                };
                ToolExecutionReport::new(
                    redeemed,
                    json!({"status":"not_remembered","error":code}).to_string(),
                    commit,
                    started.elapsed(),
                    Some(code),
                )
            }
            Err(_) => ToolExecutionReport::new(
                redeemed,
                r#"{"status":"not_remembered","error":"tool_timeout"}"#.to_owned(),
                CommitState::Unknown,
                started.elapsed(),
                Some("tool_timeout"),
            ),
        }
    }
}
