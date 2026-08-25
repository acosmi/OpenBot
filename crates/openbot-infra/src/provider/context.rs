//! PostgreSQL run/thread history → provider-neutral bounded context。

use async_trait::async_trait;
use openbot_application::{
    AgentContextError, AgentContextSource, ProviderMessage, ProviderMessageRole, ProviderRequest,
    RunExecutionLease,
};
use openbot_contracts::ids::{DeploymentId, TenantId};
use serde_json::Value;

// SPDX-License-Identifier: MIT
// Source: CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d
// File: shared/bot-prompt.ts::PROVENANCE_GUIDANCE; modified into a Rust static string.
const PROVENANCE_GUIDANCE: &str = concat!(
    "Say where an answer came from. When you read it with one of your tools, cite what you read. ",
    "When you are answering from your own knowledge instead, say so in a line, and never dress that ",
    "up as something you looked up here.",
    "\n\n",
    "This matters most for the answers people act on: a threshold, a deadline, a filing obligation, a ",
    "figure, a rule you are presenting as this organisation's. Never state one of those as established ",
    "here without having read it somewhere you can name. Saying 'I have not checked this against your ",
    "own policy or the current regulation' costs you a sentence. Being confidently wrong about a ",
    "number somebody acts on costs them a great deal more.",
    "\n\n",
    "This is not an instruction to go looking. If nothing you can reach covers the question, answer as ",
    "well as you can and mark it plainly as unverified. Do not go hunting the open web for something ",
    "to cite, and do not keep retrying a page that is not giving you one: an unsourced answer that ",
    "says it is unsourced is honest, and a search that never ends is a Bot that never answers."
);

/// First provider slice refuses oversized history until provenance-aware compression lands。
pub const MAX_AGENT_CONTEXT_MESSAGES: i64 = 4096;
/// Bounded plaintext context bytes；JSON framing/tool schema still faces safe HTTP 8MiB cap。
pub const MAX_AGENT_CONTEXT_BYTES: usize = 6 * 1024 * 1024;

/// Production PostgreSQL context source。
#[derive(Clone, Debug)]
pub struct PostgresAgentContextSource {
    pool: deadpool_postgres::Pool,
    deployment: DeploymentId,
    tenant: TenantId,
    max_output_tokens: Option<u32>,
}

impl PostgresAgentContextSource {
    /// Construct text-only context source。
    pub fn new(
        pool: deadpool_postgres::Pool,
        deployment: DeploymentId,
        tenant: TenantId,
        max_output_tokens: Option<u32>,
    ) -> Result<Self, AgentContextError> {
        if max_output_tokens == Some(0) {
            return Err(AgentContextError::Corrupt {
                field: "max_output_tokens",
            });
        }
        Ok(Self {
            pool,
            deployment,
            tenant,
            max_output_tokens,
        })
    }
}

#[async_trait]
impl AgentContextSource for PostgresAgentContextSource {
    async fn load(&self, lease: &RunExecutionLease) -> Result<ProviderRequest, AgentContextError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|_| AgentContextError::Unavailable)?;
        let visible = client
            .query_opt(
                "SELECT a.configuration FROM public.runs r \
                   JOIN public.threads t ON t.thread_id=r.thread_id \
                   JOIN public.thread_memberships tm ON tm.thread_id=t.thread_id \
                   JOIN public.agents a ON a.id=r.bot_id \
                   JOIN public.deployment_packages dp ON dp.id=a.package_id \
                   JOIN public.agent_profiles p ON p.agent_id=a.id \
                   WHERE r.run_id=$1 AND r.thread_id=$2 AND r.bot_id=$3 AND r.actor_id=$4 \
                     AND r.fencing_token=$5 AND r.status='running' \
                     AND t.deployment_id=$6 AND t.tenant_id=$7 \
                     AND dp.tenant_id=$7 \
                     AND t.status='active' AND tm.user_id=$4 AND a.type='built_in' \
                     AND p.deleted_at IS NULL",
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
            .map_err(|_| AgentContextError::Unavailable)?
            .ok_or(AgentContextError::Stale)?;
        let configuration: Value =
            visible
                .try_get("configuration")
                .map_err(|_| AgentContextError::Corrupt {
                    field: "agent_configuration",
                })?;
        let standing_prompt = standing_prompt(&configuration)?;
        let count: i64 = client
            .query_one(
                "SELECT count(*)::bigint FROM public.messages WHERE thread_id=$1",
                &[&lease.thread_id().as_str()],
            )
            .await
            .map_err(|_| AgentContextError::Unavailable)?
            .try_get(0)
            .map_err(|_| AgentContextError::Corrupt {
                field: "message_count",
            })?;
        if count <= 0 || count > MAX_AGENT_CONTEXT_MESSAGES {
            return Err(if count > MAX_AGENT_CONTEXT_MESSAGES {
                AgentContextError::TooLarge
            } else {
                AgentContextError::Corrupt {
                    field: "message_count",
                }
            });
        }
        let rows = client
            .query(
                "SELECT role,content FROM public.messages WHERE thread_id=$1 ORDER BY seq",
                &[&lease.thread_id().as_str()],
            )
            .await
            .map_err(|_| AgentContextError::Unavailable)?;
        let mut messages = Vec::with_capacity(rows.len() + 1);
        let mut total_bytes = standing_prompt.len();
        if total_bytes > MAX_AGENT_CONTEXT_BYTES {
            return Err(AgentContextError::TooLarge);
        }
        messages.push(ProviderMessage {
            role: ProviderMessageRole::System,
            content: standing_prompt,
            tool_call_id: None,
        });
        for row in rows {
            let role: String = row
                .try_get("role")
                .map_err(|_| AgentContextError::Corrupt { field: "role" })?;
            let content: Value = row
                .try_get("content")
                .map_err(|_| AgentContextError::Corrupt { field: "content" })?;
            if role == "tool" || (role == "assistant" && content.get("toolCalls").is_some()) {
                return Err(AgentContextError::ToolHistoryUnsupported);
            }
            let text = text_content(&content).ok_or(AgentContextError::Corrupt {
                field: "message_content",
            })?;
            total_bytes = total_bytes
                .checked_add(text.len())
                .ok_or(AgentContextError::TooLarge)?;
            if total_bytes > MAX_AGENT_CONTEXT_BYTES {
                return Err(AgentContextError::TooLarge);
            }
            let role = match role.as_str() {
                "user" => ProviderMessageRole::User,
                "assistant" => ProviderMessageRole::Assistant,
                "system" | "summary" => ProviderMessageRole::System,
                _ => return Err(AgentContextError::Corrupt { field: "role" }),
            };
            messages.push(ProviderMessage {
                role,
                content: text,
                tool_call_id: None,
            });
        }
        Ok(ProviderRequest {
            messages,
            tools: Vec::new(),
            max_output_tokens: self.max_output_tokens,
        })
    }
}

fn standing_prompt(configuration: &Value) -> Result<String, AgentContextError> {
    let prompt = configuration
        .as_object()
        .and_then(|value| value.get("systemPrompt"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(AgentContextError::Corrupt {
            field: "system_prompt",
        })?;
    if prompt.as_bytes().contains(&0) {
        return Err(AgentContextError::Corrupt {
            field: "system_prompt",
        });
    }
    Ok(format!("{prompt}\n\n{PROVENANCE_GUIDANCE}"))
}

fn text_content(value: &Value) -> Option<String> {
    if let Some(value) = value.as_str() {
        return Some(value.to_owned());
    }
    if let Some(value) = value
        .get("text")
        .or_else(|| value.get("content"))
        .and_then(Value::as_str)
    {
        return Some(value.to_owned());
    }
    value.as_array().map(|parts| {
        parts
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_standing_prompt_is_trimmed_and_always_carries_provenance_guidance() {
        let prompt =
            standing_prompt(&serde_json::json!({"systemPrompt":"  Be helpful.  "})).unwrap();
        assert!(prompt.starts_with("Be helpful.\n\nSay where an answer came from."));
        assert!(prompt.ends_with("a search that never ends is a Bot that never answers."));
        assert_eq!(
            standing_prompt(&serde_json::json!({"systemPrompt":"  "})),
            Err(AgentContextError::Corrupt {
                field: "system_prompt"
            })
        );
    }
}
