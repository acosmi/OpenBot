//! Provider-neutral streaming port；vendor DTO 不穿过此模块（v3 §7.3）。

use async_trait::async_trait;
use core::time::Duration;
use openbot_contracts::auth::AuthContext;
use serde_json::Value;

use crate::RunExecutionLease;

/// Loading a fresh authoritative actor context for an asynchronous Agent effect failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AgentAuthorizationError {
    /// Database/ACL source unavailable.
    #[error("agent_authorization_unavailable")]
    Unavailable,
    /// Actor/run/lease no longer authorized. This remains non-enumerating.
    #[error("agent_authorization_refused")]
    Refused,
    /// Durable role/generation data is malformed.
    #[error("agent_authorization_corrupt field={field}")]
    Corrupt {
        /// Static field name only.
        field: &'static str,
    },
}

/// Rebuilds AuthContext from current database ACL before every Agent tool effect.
#[async_trait]
pub trait AgentAuthorizationSource: Send + Sync {
    /// Load a fresh non-serializable context bound to the active run lease.
    async fn load(&self, lease: &RunExecutionLease)
    -> Result<AuthContext, AgentAuthorizationError>;
}

/// Authoritative remote AG-UI route loaded from the Bot row and active run lease.
#[derive(Clone, PartialEq, Eq)]
pub struct RemoteAguiRoute {
    endpoint: String,
    thread_id: String,
    run_id: String,
    bot_id: String,
    run_assertion: Option<String>,
}

impl RemoteAguiRoute {
    /// Construct from trusted PostgreSQL/configuration data.
    pub fn new(
        endpoint: String,
        thread_id: String,
        run_id: String,
        bot_id: String,
        run_assertion: Option<String>,
    ) -> Result<Self, AgentContextError> {
        if [&endpoint, &thread_id, &run_id, &bot_id]
            .into_iter()
            .any(|value| value.is_empty() || value.as_bytes().contains(&0))
            || run_assertion
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.as_bytes().contains(&0))
        {
            return Err(AgentContextError::Corrupt {
                field: "remote_agui_route",
            });
        }
        Ok(Self {
            endpoint,
            thread_id,
            run_id,
            bot_id,
            run_assertion,
        })
    }

    /// Endpoint. Debug never renders it.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Authoritative thread id.
    #[must_use]
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    /// Authoritative run id.
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Authoritative Bot id.
    #[must_use]
    pub fn bot_id(&self) -> &str {
        &self.bot_id
    }

    /// Optional short-lived assertion. Absence means no deployment tool may be offered.
    #[must_use]
    pub fn run_assertion(&self) -> Option<&str> {
        self.run_assertion.as_deref()
    }
}

impl core::fmt::Debug for RemoteAguiRoute {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RemoteAguiRoute")
            .field("endpoint", &"<redacted-origin>")
            .field("thread_id", &self.thread_id)
            .field("run_id", &self.run_id)
            .field("bot_id", &self.bot_id)
            .field("has_run_assertion", &self.run_assertion.is_some())
            .finish()
    }
}

/// Authoritative provider routing class loaded with the Bot row；不是 model 自报字段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderRoute {
    /// Package `model.yaml` 固定 OpenAI。
    PackageOpenAi,
    /// Managed slot 读取 deployment `BOT_PROVIDER/BOT_MODEL` config。
    Managed,
    /// Customer-owned remote AG-UI endpoint.
    RemoteAgUi(RemoteAguiRoute),
}

/// SafeDialer/SSE transport failure without remote body text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RemoteAguiTransportError {
    /// DNS/connect/TLS failed before the request became commit-unknown.
    #[error("remote_agui_unavailable")]
    Unavailable,
    /// Request may have reached the endpoint; replay is unsafe.
    #[error("remote_agui_commit_unknown")]
    CommitUnknown,
    /// Endpoint authentication rejected.
    #[error("remote_agui_authentication")]
    Authentication,
    /// Explicit 429.
    #[error("remote_agui_rate_limited")]
    RateLimited,
    /// Explicit retryable 5xx.
    #[error("remote_agui_server_unavailable")]
    ServerUnavailable,
    /// Status/content-type/SSE framing is invalid.
    #[error("remote_agui_invalid_response")]
    InvalidResponse,
    /// Real response-body read gap exceeded the watchdog.
    #[error("remote_agui_stream_stalled")]
    StreamStalled,
}

/// Complete SSE `data:` values from the unique safe transport.
#[async_trait]
pub trait RemoteAguiEventStream: Send {
    /// Read the next complete event payload.
    async fn next_data(&mut self) -> Result<Option<String>, RemoteAguiTransportError>;
}

/// Raw HTTP/SSE port. Semantic decoding remains in `openbot-agent`.
#[async_trait]
pub trait RemoteAguiTransport: Send + Sync {
    /// POST one encoded RunAgentInput to a trusted endpoint.
    async fn start(
        &self,
        endpoint: &str,
        body: Vec<u8>,
    ) -> Result<Box<dyn RemoteAguiEventStream>, RemoteAguiTransportError>;
}

/// Provider input message role。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderMessageRole {
    /// Standing/system instruction。
    System,
    /// User。
    User,
    /// Assistant history。
    Assistant,
    /// Tool result history。
    Tool,
}

/// A normalized assistant tool call retained as part of the next sampling history.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderToolCall {
    /// Vendor call id used only to pair the subsequent tool result.
    pub call_id: String,
    /// Authoritative catalog name after application validation.
    pub name: String,
    /// Validated object arguments. Debug never renders them.
    pub arguments: Value,
}

impl core::fmt::Debug for ProviderToolCall {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProviderToolCall")
            .field("call_id", &self.call_id)
            .field("name", &self.name)
            .field("arguments", &"[redacted]")
            .finish()
    }
}

/// Provider-neutral input message；Debug 只显示长度。
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderMessage {
    /// Role。
    pub role: ProviderMessageRole,
    /// Plain text projection. It may be empty on an assistant tool-call turn.
    pub content: String,
    /// Tool result 的 call id。
    pub tool_call_id: Option<String>,
    /// Tool result 的 authoritative catalog name；Google functionResponse 必填。
    pub tool_name: Option<String>,
    /// Assistant tool-call blocks. Non-assistant roles must leave this empty.
    pub tool_calls: Vec<ProviderToolCall>,
}

impl core::fmt::Debug for ProviderMessage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProviderMessage")
            .field("role", &self.role)
            .field("content_bytes", &self.content.len())
            .field("has_tool_call_id", &self.tool_call_id.is_some())
            .field("has_tool_name", &self.tool_name.is_some())
            .field("tool_call_count", &self.tool_calls.len())
            .finish()
    }
}

/// 权威 catalog 投影到 provider 的 function tool。
#[derive(Clone, PartialEq)]
pub struct ProviderToolDefinition {
    /// Tool name。
    pub name: String,
    /// Model-visible description。
    pub description: String,
    /// JSON Schema；application tool catalog 已先验证。
    pub input_schema: Value,
}

impl core::fmt::Debug for ProviderToolDefinition {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProviderToolDefinition")
            .field("name", &self.name)
            .field("description_bytes", &self.description.len())
            .field("schema", &"[redacted-structure]")
            .finish()
    }
}

/// 一次 sampling request；actor/run/API key 不在 model request 自报面。
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderRequest {
    /// Provider route from authoritative Agent configuration。
    pub route: ProviderRoute,
    /// Ordered conversation/context。
    pub messages: Vec<ProviderMessage>,
    /// Granted tools；空即 text-only。
    pub tools: Vec<ProviderToolDefinition>,
    /// Optional output token cap from authoritative budget。
    pub max_output_tokens: Option<u32>,
}

/// Provider output item kind。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderOutputKind {
    /// Assistant text/refusal。
    Message,
    /// Reasoning stream。
    Reasoning,
    /// Function call。
    FunctionCall,
    /// Vendor extension；accepted but not treated as a tool/effect。
    Extension,
}

/// Normalized usage。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProviderUsage {
    /// Input tokens。
    pub input_tokens: u64,
    /// Output tokens。
    pub output_tokens: u64,
    /// Total tokens；must equal/safely dominate known components。
    pub total_tokens: u64,
}

/// Stable provider failure category；vendor message/body never crosses this boundary。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderFailure {
    /// API key/auth rejected。
    Authentication,
    /// 429；Retry-After 只保留规范化 duration，不保留 header/body。
    RateLimited {
        /// HTTP delta-seconds/date normalized against receive time。
        retry_after: Option<Duration>,
    },
    /// Explicit retryable 5xx。
    ServerUnavailable {
        /// Optional Retry-After。
        retry_after: Option<Duration>,
    },
    /// Schema/sequence/UTF-8/SSE invalid。
    InvalidResponse,
    /// Real body read gap exceeded AGENT_STALL_TIMEOUT_MS。
    StreamStalled,
    /// DNS/connect/TLS/protocol transport failure。
    Transport,
    /// Provider reported failed/incomplete output。
    GenerationFailed,
}

/// Unified normalized event（v3 §7.3）。
#[derive(Clone, PartialEq)]
pub enum ProviderEvent {
    /// Response identity became available。
    ResponseStarted {
        /// Vendor response id；只作 trace/correlation，不作授权。
        response_id: String,
    },
    /// Skeleton item；name/arguments may arrive later。
    OutputItemAdded {
        /// Stable output index。
        index: u32,
        /// Normalized item kind。
        kind: ProviderOutputKind,
    },
    /// Text/refusal delta。
    TextDelta {
        /// Output index。
        index: u32,
        /// Complete UTF-8 delta。
        delta: String,
    },
    /// Reasoning delta。
    ReasoningDelta {
        /// Output index。
        index: u32,
        /// Complete UTF-8 delta。
        delta: String,
    },
    /// Tool skeleton。
    ToolCallStarted {
        /// Stable output/tool index。
        index: u32,
        /// Provider call id。
        call_id: String,
        /// Name may arrive later。
        name: Option<String>,
    },
    /// Partial JSON string。
    ToolArgumentsDelta {
        /// Stable output/tool index。
        index: u32,
        /// Provider call id。
        call_id: String,
        /// Partial JSON string。
        delta: String,
    },
    /// Complete parsed arguments。
    ToolCallCompleted {
        /// Stable output/tool index。
        index: u32,
        /// Provider call id。
        call_id: String,
        /// Final tool name。
        name: String,
        /// Parsed JSON object arguments。
        arguments: Value,
    },
    /// Token usage。
    Usage(ProviderUsage),
    /// Exactly one normal terminal from provider stream。
    Completed,
    /// Exactly one normalized failure terminal。
    Failed(ProviderFailure),
}

impl core::fmt::Debug for ProviderEvent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ResponseStarted { response_id } => f
                .debug_struct("ResponseStarted")
                .field("response_id", response_id)
                .finish(),
            Self::OutputItemAdded { index, kind } => f
                .debug_struct("OutputItemAdded")
                .field("index", index)
                .field("kind", kind)
                .finish(),
            Self::TextDelta { index, delta } => f
                .debug_struct("TextDelta")
                .field("index", index)
                .field("bytes", &delta.len())
                .finish(),
            Self::ReasoningDelta { index, delta } => f
                .debug_struct("ReasoningDelta")
                .field("index", index)
                .field("bytes", &delta.len())
                .finish(),
            Self::ToolCallStarted {
                index,
                call_id,
                name,
            } => f
                .debug_struct("ToolCallStarted")
                .field("index", index)
                .field("call_id", call_id)
                .field("name", name)
                .finish(),
            Self::ToolArgumentsDelta {
                index,
                call_id,
                delta,
            } => f
                .debug_struct("ToolArgumentsDelta")
                .field("index", index)
                .field("call_id", call_id)
                .field("bytes", &delta.len())
                .finish(),
            Self::ToolCallCompleted {
                index,
                call_id,
                name,
                ..
            } => f
                .debug_struct("ToolCallCompleted")
                .field("index", index)
                .field("call_id", call_id)
                .field("name", name)
                .field("arguments", &"[redacted]")
                .finish(),
            Self::Usage(usage) => f.debug_tuple("Usage").field(usage).finish(),
            Self::Completed => f.write_str("Completed"),
            Self::Failed(failure) => f.debug_tuple("Failed").field(failure).finish(),
        }
    }
}

/// Provider start/stream transport error；在 event terminal 之前发生。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProviderPortError {
    /// Config/request invalid before network。
    #[error("provider_request_invalid field={field}")]
    InvalidRequest {
        /// Static field。
        field: &'static str,
    },
    /// Transport/status unavailable；细分类由 returned Failed event 表达时可用。
    #[error("provider_unavailable")]
    Unavailable,
    /// Request may have left the process before headers became knowable；不得自动重试。
    #[error("provider_commit_unknown")]
    CommitUnknown,
}

/// 一条已打开的 provider stream。
#[async_trait]
pub trait ProviderSession: Send {
    /// 下一 normalized event；`None` 只允许发生在 terminal event 之后。
    async fn next_event(&mut self) -> Result<Option<ProviderEvent>, ProviderPortError>;
}

/// Built-in Agent sampling port。
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    /// Start sampling；API key/endpoint/model 由 adapter verified config 持有。
    async fn start(
        &self,
        request: ProviderRequest,
    ) -> Result<Box<dyn ProviderSession>, ProviderPortError>;
}

/// Authoritative thread/Bot context load failure。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AgentContextError {
    /// PostgreSQL unavailable。
    #[error("agent_context_unavailable")]
    Unavailable,
    /// Run/thread/membership/fencing no longer visible。
    #[error("agent_context_stale")]
    Stale,
    /// Stored row malformed。
    #[error("agent_context_corrupt field={field}")]
    Corrupt {
        /// Static field。
        field: &'static str,
    },
    /// Context exceeds the bounded first slice; compression is not silently faked。
    #[error("agent_context_too_large")]
    TooLarge,
    /// Existing tool history ends with an unfinished assistant/tool pair.
    #[error("agent_context_tool_history_unsupported")]
    ToolHistoryUnsupported,
}

/// PostgreSQL context/catalog projection port for a verified run lease。
#[async_trait]
pub trait AgentContextSource: Send + Sync {
    /// Load provider-neutral request；scope must be revalidated in the query。
    async fn load(&self, lease: &RunExecutionLease) -> Result<ProviderRequest, AgentContextError>;
}

/// Built-in Agent lifecycle audit facts；不携带 prompt/delta/key/vendor body。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentAuditKind {
    /// Dispatch 已 durable activate、即将读取 context。
    Invoked,
    /// Provider real body read gap exceeded the configured watchdog。
    StreamStalled,
    /// Absolute run deadline fired after child cancellation。
    RunDeadlineExceeded,
}

/// Audit chain append failure；底层错误不跨 Agent boundary。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AgentAuditError {
    /// PostgreSQL/hash-chain unavailable or commit unknown。
    #[error("agent_audit_unavailable")]
    Unavailable,
}

/// Agent lifecycle audit port；production 必须注入 hash-chain implementation。
#[async_trait]
pub trait AgentAudit: Send + Sync {
    /// Append one allowlisted lifecycle fact for the authoritative lease。
    async fn record(
        &self,
        lease: &RunExecutionLease,
        kind: AgentAuditKind,
    ) -> Result<(), AgentAuditError>;
}

/// Explicit no-op used by narrow unit/integration harnesses；production main never constructs it。
#[derive(Clone, Copy, Debug, Default)]
pub struct NoAgentAudit;

#[async_trait]
impl AgentAudit for NoAgentAudit {
    async fn record(
        &self,
        _lease: &RunExecutionLease,
        _kind: AgentAuditKind,
    ) -> Result<(), AgentAuditError> {
        Ok(())
    }
}
