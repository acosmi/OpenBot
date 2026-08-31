//! Provider-neutral streaming port；vendor DTO 不穿过此模块（v3 §7.3）。

use async_trait::async_trait;
use core::time::Duration;
use openbot_contracts::auth::AuthContext;
use openbot_domain::vault::SecretBytes;
use serde_json::Value;
use std::sync::Arc;
use time::OffsetDateTime;
use url::Url;

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
    authorization: Option<RemoteAguiAuthorization>,
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
            authorization: None,
        })
    }

    /// Attach a Vault-opened Authorization value. The route shares the one zeroizing allocation.
    #[must_use]
    pub fn with_authorization(mut self, authorization: RemoteAguiAuthorization) -> Self {
        self.authorization = Some(authorization);
        self
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

    /// Optional write-only customer Agent Authorization value.
    #[must_use]
    pub const fn authorization(&self) -> Option<&RemoteAguiAuthorization> {
        self.authorization.as_ref()
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
            .field("has_authorization", &self.authorization.is_some())
            .finish()
    }
}

/// Vault-opened remote Agent Authorization value. It is never serde/displayable.
#[derive(Clone)]
pub struct RemoteAguiAuthorization(Arc<SecretBytes>);

impl RemoteAguiAuthorization {
    /// Take ownership of one already validated non-empty header value.
    pub fn new(value: SecretBytes) -> Result<Self, AgentContextError> {
        if value.is_empty()
            || value.len() > 16 * 1_024
            || value.expose().contains(&0)
            || core::str::from_utf8(value.expose()).is_err()
        {
            return Err(AgentContextError::Corrupt {
                field: "remote_authorization",
            });
        }
        Ok(Self(Arc::new(value)))
    }

    /// Explicitly expose UTF-8 only at the SafeDialer request boundary.
    pub fn expose(&self) -> Result<&str, AgentContextError> {
        core::str::from_utf8(self.0.expose()).map_err(|_| AgentContextError::Corrupt {
            field: "remote_authorization",
        })
    }
}

impl core::fmt::Debug for RemoteAguiAuthorization {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("[redacted]")
    }
}

impl PartialEq for RemoteAguiAuthorization {
    fn eq(&self, other: &Self) -> bool {
        self.0.ct_eq(&other.0)
    }
}

impl Eq for RemoteAguiAuthorization {}

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

/// Provider family bound into an operator-attested price snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderBillingFamily {
    /// OpenAI or an explicitly compatible endpoint.
    OpenAiCompatible,
    /// Anthropic Messages.
    Anthropic,
    /// Google Generative AI.
    Google,
}

impl ProviderBillingFamily {
    /// Stable PostgreSQL/audit literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai_compatible",
            Self::Anthropic => "anthropic",
            Self::Google => "google",
        }
    }

    /// Parse the stable storage literal.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "openai_compatible" => Some(Self::OpenAiCompatible),
            "anthropic" => Some(Self::Anthropic),
            "google" => Some(Self::Google),
            _ => None,
        }
    }
}

/// Invalid operator-attested rate-card field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProviderRateCardError {
    /// Provider/model identity is not bounded or canonical.
    #[error("provider_rate_card_identity_invalid")]
    Identity,
    /// Currency must be an explicit three-letter uppercase code.
    #[error("provider_rate_card_currency_invalid")]
    Currency,
    /// Source must be a credential-free HTTPS URL with no query or fragment.
    #[error("provider_rate_card_source_invalid")]
    Source,
    /// Source digest must be lowercase SHA-256.
    #[error("provider_rate_card_digest_invalid")]
    Digest,
    /// Observation time must be at or after the Unix epoch.
    #[error("provider_rate_card_observed_at_invalid")]
    ObservedAt,
    /// A rate cannot fit the PostgreSQL signed-bigint boundary.
    #[error("provider_rate_card_rate_invalid")]
    Rate,
}

/// Operator-attested immutable maximum-rate provenance for one provider/model pair.
///
/// OpenBot does not ship mutable vendor list prices. The deployment owner records the rate that
/// bounds its contract together with the source document hash and observation time. Maximum rates
/// make the counter conservative when a vendor reports no stable cache-discount breakdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRateCard {
    family: ProviderBillingFamily,
    model: String,
    currency: String,
    max_input_micro_units_per_million_tokens: u64,
    max_output_micro_units_per_million_tokens: u64,
    source_url: String,
    source_sha256: String,
    observed_at: OffsetDateTime,
}

/// Explicit input for one operator-attested maximum-rate snapshot.
pub struct ProviderRateCardInput {
    /// Provider family.
    pub family: ProviderBillingFamily,
    /// Exact provider model id.
    pub model: String,
    /// Three-letter uppercase currency code; OpenBot performs no conversion.
    pub currency: String,
    /// Maximum micro currency units per one million input tokens.
    pub max_input_micro_units_per_million_tokens: u64,
    /// Maximum micro currency units per one million output tokens.
    pub max_output_micro_units_per_million_tokens: u64,
    /// Credential-free HTTPS source URL.
    pub source_url: String,
    /// Lowercase SHA-256 of the source document.
    pub source_sha256: String,
    /// Operator observation time.
    pub observed_at: OffsetDateTime,
}

impl ProviderRateCard {
    /// Validate and canonicalize one explicit maximum-rate snapshot.
    pub fn new(input: ProviderRateCardInput) -> Result<Self, ProviderRateCardError> {
        let ProviderRateCardInput {
            family,
            model,
            currency,
            max_input_micro_units_per_million_tokens,
            max_output_micro_units_per_million_tokens,
            source_url,
            source_sha256,
            observed_at,
        } = input;
        if model.is_empty() || model.len() > 256 || model.as_bytes().contains(&0) {
            return Err(ProviderRateCardError::Identity);
        }
        if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()) {
            return Err(ProviderRateCardError::Currency);
        }
        if max_input_micro_units_per_million_tokens > i64::MAX as u64
            || max_output_micro_units_per_million_tokens > i64::MAX as u64
        {
            return Err(ProviderRateCardError::Rate);
        }
        let source = Url::parse(&source_url).map_err(|_| ProviderRateCardError::Source)?;
        if source.scheme() != "https"
            || source.host_str().is_none()
            || !source.username().is_empty()
            || source.password().is_some()
            || source.query().is_some()
            || source.fragment().is_some()
            || source.as_str().len() > 2048
        {
            return Err(ProviderRateCardError::Source);
        }
        if source_sha256.len() != 64
            || !source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ProviderRateCardError::Digest);
        }
        if observed_at < OffsetDateTime::UNIX_EPOCH {
            return Err(ProviderRateCardError::ObservedAt);
        }
        Ok(Self {
            family,
            model,
            currency,
            max_input_micro_units_per_million_tokens,
            max_output_micro_units_per_million_tokens,
            source_url: source.to_string(),
            source_sha256,
            observed_at,
        })
    }

    /// Provider family.
    #[must_use]
    pub const fn family(&self) -> ProviderBillingFamily {
        self.family
    }

    /// Exact configured model.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Currency code. No conversion is performed.
    #[must_use]
    pub fn currency(&self) -> &str {
        &self.currency
    }

    /// Maximum micro currency units charged per one million input tokens.
    #[must_use]
    pub const fn max_input_rate(&self) -> u64 {
        self.max_input_micro_units_per_million_tokens
    }

    /// Maximum micro currency units charged per one million output tokens.
    #[must_use]
    pub const fn max_output_rate(&self) -> u64 {
        self.max_output_micro_units_per_million_tokens
    }

    /// Canonical credential-free HTTPS provenance URL.
    #[must_use]
    pub fn source_url(&self) -> &str {
        &self.source_url
    }

    /// Lowercase source-document SHA-256.
    #[must_use]
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    /// When the operator observed the attested rate.
    #[must_use]
    pub const fn observed_at(&self) -> OffsetDateTime {
        self.observed_at
    }
}

/// Exact arithmetic upper bound under operator-attested maximum rates in one currency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProviderCostUpperBound {
    micro_units: u64,
    remainder_millionths: u32,
}

impl ProviderCostUpperBound {
    /// Restore a durable counter.
    pub fn from_parts(
        micro_units: u64,
        remainder_millionths: u32,
    ) -> Result<Self, ProviderRateCardError> {
        if remainder_millionths >= 1_000_000 || micro_units > i64::MAX as u64 {
            return Err(ProviderRateCardError::Rate);
        }
        Ok(Self {
            micro_units,
            remainder_millionths,
        })
    }

    /// Add one normalized usage without floating point or per-sampling rounding.
    pub fn accrue(
        self,
        usage: ProviderUsage,
        rate: &ProviderRateCard,
    ) -> Result<Self, ProviderRateCardError> {
        let input = u128::from(usage.input_tokens)
            .checked_mul(u128::from(rate.max_input_rate()))
            .ok_or(ProviderRateCardError::Rate)?;
        let output = u128::from(usage.output_tokens)
            .checked_mul(u128::from(rate.max_output_rate()))
            .ok_or(ProviderRateCardError::Rate)?;
        let numerator = input
            .checked_add(output)
            .and_then(|value| value.checked_add(u128::from(self.remainder_millionths)))
            .ok_or(ProviderRateCardError::Rate)?;
        let whole =
            u64::try_from(numerator / 1_000_000).map_err(|_| ProviderRateCardError::Rate)?;
        let micro_units = self
            .micro_units
            .checked_add(whole)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or(ProviderRateCardError::Rate)?;
        Ok(Self {
            micro_units,
            remainder_millionths: u32::try_from(numerator % 1_000_000)
                .map_err(|_| ProviderRateCardError::Rate)?,
        })
    }

    /// Whole micro currency units already carried from exact arithmetic.
    #[must_use]
    pub const fn micro_units(self) -> u64 {
        self.micro_units
    }

    /// Remaining millionths of one micro currency unit.
    #[must_use]
    pub const fn remainder_millionths(self) -> u32 {
        self.remainder_millionths
    }

    /// Conservative billable amount rounded up only at the aggregate boundary.
    #[must_use]
    pub fn billed_upper_bound_micro_units(self) -> Option<u64> {
        self.micro_units
            .checked_add(u64::from(self.remainder_millionths != 0))
    }
}

/// SafeDialer/SSE transport failure without remote body text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RemoteAguiTransportError {
    /// URL, scheme, or destination policy rejected the request before any socket was opened.
    #[error("remote_agui_destination_rejected")]
    DestinationRejected,
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
    /// Resolve and apply current scheme/egress policy without sending a request. Runtime still
    /// repeats the same decision immediately before every connection and redirect.
    async fn validate_endpoint(&self, _endpoint: &str) -> Result<(), RemoteAguiTransportError> {
        Err(RemoteAguiTransportError::Unavailable)
    }

    /// POST one encoded RunAgentInput to a trusted endpoint.
    async fn start(
        &self,
        endpoint: &str,
        authorization: Option<&RemoteAguiAuthorization>,
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
    /// Optional operator-attested price snapshot; `None` means explicitly unpriced, never zero.
    pub rate_card: Option<ProviderRateCard>,
    /// User cap frozen onto this run when it was created; never supplied by the model/renderer.
    pub cost_cap: Option<crate::run_cost_budget::RunCostCap>,
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
    /// Frozen cap has no operator-attested rate snapshot.
    RunCostBudgetUnpriced,
    /// Frozen cap and operator-attested rate use different currencies.
    RunCostBudgetCurrencyMismatch,
    /// Durable cost upper bound exceeded the frozen cap.
    RunCostBudgetExceeded,
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

#[cfg(test)]
mod rate_card_tests {
    use super::*;
    use time::macros::datetime;

    fn card(input: u64, output: u64) -> ProviderRateCard {
        ProviderRateCard::new(ProviderRateCardInput {
            family: ProviderBillingFamily::OpenAiCompatible,
            model: "model-1".to_owned(),
            currency: "USD".to_owned(),
            max_input_micro_units_per_million_tokens: input,
            max_output_micro_units_per_million_tokens: output,
            source_url: "https://prices.example.test/archive/2026-08-30".to_owned(),
            source_sha256: "a".repeat(64),
            observed_at: datetime!(2026-08-30 12:00 UTC),
        })
        .unwrap()
    }

    #[test]
    fn explicit_provenance_is_closed_and_credential_free() {
        let rate = card(1_500_000, 2_000_000);
        assert_eq!(rate.family().as_str(), "openai_compatible");
        assert_eq!(rate.model(), "model-1");
        assert_eq!(rate.currency(), "USD");
        assert_eq!(rate.source_sha256(), "a".repeat(64));
        assert_eq!(rate.observed_at(), datetime!(2026-08-30 12:00 UTC));
        let bad_currency = ProviderRateCardInput {
            family: ProviderBillingFamily::OpenAiCompatible,
            model: "model-1".to_owned(),
            currency: "usd".to_owned(),
            max_input_micro_units_per_million_tokens: 1,
            max_output_micro_units_per_million_tokens: 1,
            source_url: "https://prices.example.test/rates".to_owned(),
            source_sha256: "a".repeat(64),
            observed_at: datetime!(2026-08-30 12:00 UTC),
        };
        assert_eq!(
            ProviderRateCard::new(bad_currency),
            Err(ProviderRateCardError::Currency),
        );
        for source in [
            "http://prices.example.test/rates",
            "https://user@prices.example.test/rates",
            "https://prices.example.test/rates?contract=secret",
            "https://prices.example.test/rates#today",
        ] {
            assert_eq!(
                ProviderRateCard::new(ProviderRateCardInput {
                    family: ProviderBillingFamily::Anthropic,
                    model: "model-1".to_owned(),
                    currency: "USD".to_owned(),
                    max_input_micro_units_per_million_tokens: 1,
                    max_output_micro_units_per_million_tokens: 1,
                    source_url: source.to_owned(),
                    source_sha256: "a".repeat(64),
                    observed_at: datetime!(2026-08-30 12:00 UTC),
                }),
                Err(ProviderRateCardError::Source),
            );
        }
        assert_eq!(
            ProviderRateCard::new(ProviderRateCardInput {
                family: ProviderBillingFamily::Google,
                model: "model-1".to_owned(),
                currency: "USD".to_owned(),
                max_input_micro_units_per_million_tokens: 1,
                max_output_micro_units_per_million_tokens: 1,
                source_url: "https://prices.example.test/rates".to_owned(),
                source_sha256: "A".repeat(64),
                observed_at: datetime!(2026-08-30 12:00 UTC),
            }),
            Err(ProviderRateCardError::Digest),
        );
    }

    #[test]
    fn exact_cost_carries_fraction_across_samplings_before_rounding() {
        let rate = card(1_500_000, 2_000_000);
        let one = ProviderCostUpperBound::default()
            .accrue(
                ProviderUsage {
                    input_tokens: 1,
                    output_tokens: 1,
                    total_tokens: 2,
                },
                &rate,
            )
            .unwrap();
        assert_eq!(one.micro_units(), 3);
        assert_eq!(one.remainder_millionths(), 500_000);
        assert_eq!(one.billed_upper_bound_micro_units(), Some(4));
        let two = one
            .accrue(
                ProviderUsage {
                    input_tokens: 1,
                    output_tokens: 1,
                    total_tokens: 2,
                },
                &rate,
            )
            .unwrap();
        assert_eq!(two.micro_units(), 7);
        assert_eq!(two.remainder_millionths(), 0);
        assert_eq!(two.billed_upper_bound_micro_units(), Some(7));
    }

    #[test]
    fn arithmetic_overflow_and_invalid_durable_parts_fail_closed() {
        assert_eq!(
            ProviderCostUpperBound::from_parts(0, 1_000_000),
            Err(ProviderRateCardError::Rate),
        );
        assert_eq!(
            ProviderCostUpperBound::default().accrue(
                ProviderUsage {
                    input_tokens: u64::MAX,
                    output_tokens: u64::MAX,
                    total_tokens: u64::MAX,
                },
                &card(i64::MAX as u64, i64::MAX as u64),
            ),
            Err(ProviderRateCardError::Rate),
        );
    }
}
