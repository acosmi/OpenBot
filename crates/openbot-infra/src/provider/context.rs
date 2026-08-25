//! PostgreSQL run/thread history → provider-neutral bounded context。

use std::collections::BTreeMap;

use async_trait::async_trait;
use openbot_application::{
    AgentContextError, AgentContextSource, ProviderMessage, ProviderMessageRole, ProviderRequest,
    ProviderRoute, ProviderToolCall, ProviderToolDefinition, RemoteAguiRoute, RunExecutionLease,
};
use openbot_contracts::ids::{DeploymentId, TenantId};
use openbot_domain::remote_callback::{RemoteRunAssertionSigner, RemoteRunScope, RemoteToolSet};
use serde_json::Value;

use crate::mcp_catalog::{GrantedMcpTool, McpCatalogError, PostgresMcpCatalog};

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
    tools: Vec<ProviderToolDefinition>,
    remote_assertions: Option<std::sync::Arc<RemoteRunAssertionSigner>>,
    mcp_catalog: Option<std::sync::Arc<PostgresMcpCatalog>>,
}

impl PostgresAgentContextSource {
    /// Construct a bounded context source; first-party tools may be attached with [`Self::with_tools`].
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
            tools: Vec::new(),
            remote_assertions: None,
            mcp_catalog: None,
        })
    }

    /// Attach the first-party catalog projected to model-visible JSON Schema.
    #[must_use]
    pub fn with_tools(mut self, tools: Vec<ProviderToolDefinition>) -> Self {
        self.tools = tools;
        self
    }

    /// Attach the deployment run-assertion signer. A remote route without it fails closed.
    #[must_use]
    pub fn with_remote_assertions(
        mut self,
        signer: std::sync::Arc<RemoteRunAssertionSigner>,
    ) -> Self {
        self.remote_assertions = Some(signer);
        self
    }

    /// Attach the authoritative per-Bot MCP grant/catalog projection.
    #[must_use]
    pub fn with_mcp_catalog(mut self, catalog: std::sync::Arc<PostgresMcpCatalog>) -> Self {
        self.mcp_catalog = Some(catalog);
        self
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
                "SELECT a.type::text AS agent_type,a.name,a.configuration,p.title,p.role_description, \
                        floor(extract(epoch FROM clock_timestamp())*1000)::bigint \
                          AS assertion_issued_at_millis \
                   FROM public.runs r \
                   JOIN public.threads t ON t.thread_id=r.thread_id \
                   JOIN public.thread_memberships tm ON tm.thread_id=t.thread_id \
                   JOIN public.agents a ON a.id=r.bot_id \
                   JOIN public.deployment_packages dp ON dp.id=a.package_id \
                   JOIN public.agent_profiles p ON p.agent_id=a.id \
                   WHERE r.run_id=$1 AND r.thread_id=$2 AND r.bot_id=$3 AND r.actor_id=$4 \
                     AND r.fencing_token=$5 AND r.status='running' \
                     AND t.deployment_id=$6 AND t.tenant_id=$7 \
                     AND dp.tenant_id=$7 \
                     AND t.status='active' AND tm.user_id=$4 \
                     AND a.type IN ('built_in','remote_ag_ui') \
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
        let agent_type: String =
            visible
                .try_get("agent_type")
                .map_err(|_| AgentContextError::Corrupt {
                    field: "agent_type",
                })?;
        let granted_mcp = match &self.mcp_catalog {
            Some(catalog) => catalog
                .granted_tools_on_client(&client, lease.bot_id(), lease.actor_id())
                .await
                .map_err(map_mcp_catalog_error)?,
            None => Vec::new(),
        };
        let (route, standing_prompt, tools, max_output_tokens) = match agent_type.as_str() {
            "built_in" => {
                let mut tools = self.tools.clone();
                tools.extend(granted_mcp.iter().map(|tool| tool.provider_definition()));
                (
                    provider_route(&configuration)?,
                    append_granted_tool_guidance(standing_prompt(&configuration)?, &granted_mcp),
                    tools,
                    self.max_output_tokens,
                )
            }
            "remote_ag_ui" => {
                let endpoint = configuration
                    .as_object()
                    .and_then(|value| value.get("endpoint"))
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .ok_or(AgentContextError::Corrupt {
                        field: "remote_endpoint",
                    })?
                    .trim()
                    .to_owned();
                let name: String =
                    visible
                        .try_get("name")
                        .map_err(|_| AgentContextError::Corrupt {
                            field: "agent_name",
                        })?;
                let title: String =
                    visible
                        .try_get("title")
                        .map_err(|_| AgentContextError::Corrupt {
                            field: "agent_title",
                        })?;
                let role_description: String =
                    visible.try_get("role_description").map_err(|_| {
                        AgentContextError::Corrupt {
                            field: "role_description",
                        }
                    })?;
                let issued_at_millis: i64 =
                    visible.try_get("assertion_issued_at_millis").map_err(|_| {
                        AgentContextError::Corrupt {
                            field: "assertion_issued_at_millis",
                        }
                    })?;
                let remote_tools =
                    RemoteToolSet::new(granted_mcp.iter().map(|tool| tool.model_name.clone()))
                        .map_err(|_| AgentContextError::Corrupt {
                            field: "remote_tool_set",
                        })?;
                let assertion = self
                    .remote_assertions
                    .as_ref()
                    .ok_or(AgentContextError::Unavailable)?
                    .mint(
                        RemoteRunScope {
                            deployment: self.deployment.clone(),
                            tenant: self.tenant.clone(),
                            bot: lease.bot_id().clone(),
                            actor: lease.actor_id().clone(),
                            run: lease.run_id().clone(),
                        },
                        &remote_tools,
                        issued_at_millis,
                    )
                    .map_err(|_| AgentContextError::Corrupt {
                        field: "remote_run_assertion",
                    })?;
                (
                    ProviderRoute::RemoteAgUi(RemoteAguiRoute::new(
                        endpoint,
                        lease.thread_id().as_str().to_owned(),
                        lease.run_id().as_str().to_owned(),
                        lease.bot_id().as_str().to_owned(),
                        Some(assertion),
                    )?),
                    append_granted_tool_guidance(
                        remote_standing_prompt(&name, &title, &role_description)?,
                        &granted_mcp,
                    ),
                    granted_mcp
                        .iter()
                        .map(|tool| tool.provider_definition())
                        .collect(),
                    None,
                )
            }
            _ => {
                return Err(AgentContextError::Corrupt {
                    field: "agent_type",
                });
            }
        };
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
            content: standing_prompt.clone(),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        });
        let mut pending_tool_calls = BTreeMap::<String, String>::new();
        for row in rows {
            let role: String = row
                .try_get("role")
                .map_err(|_| AgentContextError::Corrupt { field: "role" })?;
            let content: Value = row
                .try_get("content")
                .map_err(|_| AgentContextError::Corrupt { field: "content" })?;
            let text = text_content(&content).ok_or(AgentContextError::Corrupt {
                field: "message_content",
            })?;
            if role == "system" && text == standing_prompt {
                continue;
            }
            let tool_calls = if role == "assistant" {
                if !pending_tool_calls.is_empty() {
                    return Err(AgentContextError::Corrupt {
                        field: "unfinished_tool_pair",
                    });
                }
                provider_tool_calls(&content)?
            } else {
                if content.get("toolCalls").is_some() {
                    return Err(AgentContextError::Corrupt {
                        field: "tool_calls_role",
                    });
                }
                Vec::new()
            };
            let tool_call_id = if role == "tool" {
                Some(required_content_string(&content, "toolCallId")?)
            } else {
                None
            };
            let tool_name = if role == "tool" {
                Some(required_content_string(&content, "toolName")?)
            } else {
                None
            };
            if role == "assistant" {
                for call in &tool_calls {
                    if pending_tool_calls
                        .insert(call.call_id.clone(), call.name.clone())
                        .is_some()
                    {
                        return Err(AgentContextError::Corrupt {
                            field: "duplicate_tool_call_id",
                        });
                    }
                }
            } else if role == "tool" {
                let (Some(call_id), Some(name)) = (tool_call_id.as_deref(), tool_name.as_deref())
                else {
                    return Err(AgentContextError::Corrupt {
                        field: "tool_pair_binding",
                    });
                };
                if pending_tool_calls.remove(call_id).as_deref() != Some(name) {
                    return Err(AgentContextError::Corrupt {
                        field: "tool_pair_binding",
                    });
                }
            } else if !pending_tool_calls.is_empty() {
                return Err(AgentContextError::Corrupt {
                    field: "unfinished_tool_pair",
                });
            }
            let tool_bytes = tool_calls.iter().try_fold(0_usize, |total, call| {
                serde_json::to_vec(&call.arguments)
                    .ok()
                    .and_then(|arguments| {
                        total
                            .checked_add(call.call_id.len())?
                            .checked_add(call.name.len())?
                            .checked_add(arguments.len())
                    })
                    .ok_or(AgentContextError::TooLarge)
            })?;
            let result_pair_bytes = tool_call_id
                .as_ref()
                .zip(tool_name.as_ref())
                .map_or(0, |(call_id, name)| {
                    call_id.len().saturating_add(name.len())
                });
            total_bytes = total_bytes
                .checked_add(text.len())
                .and_then(|value| value.checked_add(tool_bytes))
                .and_then(|value| value.checked_add(result_pair_bytes))
                .ok_or(AgentContextError::TooLarge)?;
            if total_bytes > MAX_AGENT_CONTEXT_BYTES {
                return Err(AgentContextError::TooLarge);
            }
            let role = match role.as_str() {
                "user" => ProviderMessageRole::User,
                "assistant" => ProviderMessageRole::Assistant,
                "system" | "summary" => ProviderMessageRole::System,
                "tool" => ProviderMessageRole::Tool,
                _ => return Err(AgentContextError::Corrupt { field: "role" }),
            };
            messages.push(ProviderMessage {
                role,
                content: text,
                tool_call_id,
                tool_name,
                tool_calls,
            });
        }
        if !pending_tool_calls.is_empty() {
            return Err(AgentContextError::ToolHistoryUnsupported);
        }
        Ok(ProviderRequest {
            route,
            messages,
            tools,
            max_output_tokens,
        })
    }
}

fn map_mcp_catalog_error(error: McpCatalogError) -> AgentContextError {
    match error {
        McpCatalogError::Unavailable => AgentContextError::Unavailable,
        McpCatalogError::NotVisible => AgentContextError::Stale,
        McpCatalogError::Corrupt { .. } => AgentContextError::Corrupt {
            field: "mcp_catalog",
        },
    }
}

// SPDX-License-Identifier: MIT
// Source: CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d
// File: server/src/plugins/tools.ts::grantedToolGuidance; translated to validated catalog facts.
fn append_granted_tool_guidance(mut prompt: String, tools: &[GrantedMcpTool]) -> String {
    if tools.is_empty() {
        return prompt;
    }
    let mut by_system = BTreeMap::<&str, Vec<&str>>::new();
    for tool in tools {
        by_system
            .entry(tool.server_id.as_str())
            .or_default()
            .push(tool.raw_name.as_str());
    }
    let mut guidance = vec![
        "You can reach these systems directly, as the person asking, with their own access:"
            .to_owned(),
    ];
    for (system, names) in by_system {
        guidance.push(format!("- {system}: {}", names.join(", ")));
    }
    guidance.extend([
        "Use them for anything about those systems. Do NOT browse to one of their websites instead: your".to_owned(),
        "browser is signed in as nobody, so it sees less than these tools do and will meet a sign-in wall".to_owned(),
        "that connecting an account has already solved.".to_owned(),
        "If one of these systems is involved and no tool above covers the part you need, that is a".to_owned(),
        "missing grant and not something to work around. Say so plainly, name the capability you would".to_owned(),
        "need, and say an administrator can grant it on that connector. Do not reach for the browser, do".to_owned(),
        "not ask the person to sign in, and do not ask them to fetch it for you: they already have the".to_owned(),
        "access, and the thing that is missing is yours, not theirs.".to_owned(),
    ]);
    prompt.push_str("\n\n");
    prompt.push_str(&guidance.join("\n"));
    prompt
}

fn provider_tool_calls(content: &Value) -> Result<Vec<ProviderToolCall>, AgentContextError> {
    let Some(calls) = content.get("toolCalls") else {
        return Ok(Vec::new());
    };
    let calls = calls.as_array().ok_or(AgentContextError::Corrupt {
        field: "tool_calls",
    })?;
    if calls.len() > 256 {
        return Err(AgentContextError::TooLarge);
    }
    calls
        .iter()
        .map(|call| {
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or(AgentContextError::Corrupt {
                    field: "tool_call_id",
                })?
                .to_owned();
            let function = call.get("function").and_then(Value::as_object).ok_or(
                AgentContextError::Corrupt {
                    field: "tool_call_function",
                },
            )?;
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or(AgentContextError::Corrupt {
                    field: "tool_call_name",
                })?
                .to_owned();
            let arguments = match function.get("arguments") {
                Some(value) if value.is_object() => value.clone(),
                Some(Value::String(value)) => serde_json::from_str(value)
                    .ok()
                    .filter(Value::is_object)
                    .ok_or(AgentContextError::Corrupt {
                        field: "tool_call_arguments",
                    })?,
                _ => {
                    return Err(AgentContextError::Corrupt {
                        field: "tool_call_arguments",
                    });
                }
            };
            Ok(ProviderToolCall {
                call_id,
                name,
                arguments,
            })
        })
        .collect()
}

fn required_content_string(
    content: &Value,
    field: &'static str,
) -> Result<String, AgentContextError> {
    content
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or(AgentContextError::Corrupt { field })
}

fn provider_route(configuration: &Value) -> Result<ProviderRoute, AgentContextError> {
    match configuration
        .as_object()
        .and_then(|value| value.get("providerSource"))
    {
        None | Some(Value::Null) => Ok(ProviderRoute::PackageOpenAi),
        Some(Value::String(value)) if value == "package" => Ok(ProviderRoute::PackageOpenAi),
        Some(Value::String(value)) if value == "managed" => Ok(ProviderRoute::Managed),
        _ => Err(AgentContextError::Corrupt {
            field: "provider_source",
        }),
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

fn remote_standing_prompt(
    name: &str,
    title: &str,
    role_description: &str,
) -> Result<String, AgentContextError> {
    let name = name.trim();
    let title = title.trim();
    let role_description = role_description.trim();
    if name.is_empty() || title.is_empty() || role_description.is_empty() {
        return Err(AgentContextError::Corrupt {
            field: "remote_standing_role",
        });
    }
    let prompt = format!(
        "You are {name}, {title}.\n\n{role_description}\n\n\
         This standing role applies in every channel. Treat channel messages as task-specific instructions within it.\n\n\
         {PROVENANCE_GUIDANCE}"
    );
    if prompt.len() > MAX_AGENT_CONTEXT_BYTES {
        return Err(AgentContextError::TooLarge);
    }
    Ok(prompt)
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
        assert_eq!(
            provider_route(&serde_json::json!({"systemPrompt":"x","providerSource":"managed"})),
            Ok(ProviderRoute::Managed)
        );
        assert_eq!(
            provider_route(&serde_json::json!({"systemPrompt":"x","providerSource":"unknown"})),
            Err(AgentContextError::Corrupt {
                field: "provider_source"
            })
        );
    }
}
