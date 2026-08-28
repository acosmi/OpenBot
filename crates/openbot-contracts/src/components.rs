//! Compiled component catalogue and read-only governance projections.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::ids::BotId;

/// Stable Activity report renderer identity.
pub const SHOW_ACTIVITY_REPORT_COMPONENT_NAME: &str = "showActivityReport";
/// Stable Activity report title.
pub const SHOW_ACTIVITY_REPORT_COMPONENT_TITLE: &str = "Activity report";
/// Default model-facing Activity report description.
pub const SHOW_ACTIVITY_REPORT_COMPONENT_DESCRIPTION: &str = "Show what this deployment has actually been doing, read from its own records rather than from anything you know. Use for 'what have the Bots been up to' and 'what has been refused'. You choose the report and the period; the figures are read for you and you will not see them.";
/// Stable data-function identity for per-Bot action counts.
pub const BOT_ACTIVITY_FUNCTION_NAME: &str = "botActivity";
/// Administrator-facing description for the Bot activity data function.
pub const BOT_ACTIVITY_FUNCTION_DESCRIPTION: &str =
    "How many actions each Bot has taken, counted from the audit trail.";
/// Stable data-function identity for recent refusal rows.
pub const RECENT_REFUSALS_FUNCTION_NAME: &str = "recentRefusals";
/// Administrator-facing description for the recent-refusals data function.
pub const RECENT_REFUSALS_FUNCTION_DESCRIPTION: &str =
    "The most recent things this deployment refused, and the reason each was refused.";
/// Stable source description shared by both first-party data functions.
pub const AUDIT_TRAIL_READS_DESCRIPTION: &str = "the audit trail";

/// Stable tool/catalogue identity for the first compiled Rust renderer slice.
pub const SHOW_QUOTE_COMPONENT_NAME: &str = "showQuote";
/// Human-facing title stored when the build first announces the renderer.
pub const SHOW_QUOTE_COMPONENT_TITLE: &str = "Quotation";
/// Model-facing default description promoted on first arrival only.
pub const SHOW_QUOTE_COMPONENT_DESCRIPTION: &str = "Show a quotation with its attribution. Use when the exact words matter, something a person said, or a line from a document you were given.";
/// Stable Record renderer identity.
pub const SHOW_RECORD_COMPONENT_NAME: &str = "showRecord";
/// Stable Record title.
pub const SHOW_RECORD_COMPONENT_TITLE: &str = "Record";
/// Default model-facing Record description.
pub const SHOW_RECORD_COMPONENT_DESCRIPTION: &str = "Show one thing and its fields, an order, a person, a ticket. Use instead of describing a record in prose.";
/// Stable Metrics renderer identity.
pub const SHOW_METRICS_COMPONENT_NAME: &str = "showMetrics";
/// Stable Metrics title.
pub const SHOW_METRICS_COMPONENT_TITLE: &str = "Headline figures";
/// Default model-facing Metrics description.
pub const SHOW_METRICS_COMPONENT_DESCRIPTION: &str = "Show up to six headline figures, each with an optional movement. Use for a summary somebody reads at a glance.";
/// Stable Checklist renderer identity.
pub const SHOW_CHECKLIST_COMPONENT_NAME: &str = "showChecklist";
/// Stable Checklist title.
pub const SHOW_CHECKLIST_COMPONENT_TITLE: &str = "Checklist";
/// Default model-facing Checklist description.
pub const SHOW_CHECKLIST_COMPONENT_DESCRIPTION: &str = "Show a list of things and which are done. Reporting only, the person cannot tick these, so do not use it to ask for anything.";
/// Stable Notice renderer identity.
pub const SHOW_NOTICE_COMPONENT_NAME: &str = "showNotice";
/// Stable Notice title.
pub const SHOW_NOTICE_COMPONENT_TITLE: &str = "Notice";
/// Default model-facing Notice description.
pub const SHOW_NOTICE_COMPONENT_DESCRIPTION: &str = "Show a headline, a short explanation and optional supporting points. Use instead of writing several paragraphs of prose.";
/// Stable Area chart identity/title/description.
pub const SHOW_AREA_CHART_COMPONENT_NAME: &str = "showAreaChart";
/// Area chart title.
pub const SHOW_AREA_CHART_COMPONENT_TITLE: &str = "Area chart";
/// Area chart model-facing description.
pub const SHOW_AREA_CHART_COMPONENT_DESCRIPTION: &str = "The same as showLineChart with the area under each line filled. Use for volume or accumulation rather than for a rate.";
/// Stable Bar chart identity/title/description.
pub const SHOW_BAR_CHART_COMPONENT_NAME: &str = "showBarChart";
/// Bar chart title.
pub const SHOW_BAR_CHART_COMPONENT_TITLE: &str = "Bar chart";
/// Bar chart model-facing description.
pub const SHOW_BAR_CHART_COMPONENT_DESCRIPTION: &str = "Show values as a bar chart. Use when comparing a handful of named things, teams, months, categories. Not for a trend over time, which is showLineChart.";
/// Stable Line chart identity/title/description.
pub const SHOW_LINE_CHART_COMPONENT_NAME: &str = "showLineChart";
/// Line chart title.
pub const SHOW_LINE_CHART_COMPONENT_TITLE: &str = "Line chart";
/// Line chart model-facing description.
pub const SHOW_LINE_CHART_COMPONENT_DESCRIPTION: &str = "Show one or more series over an ordered axis, usually time. Every series must have one value per label.";
/// Stable Donut chart identity/title/description.
pub const SHOW_PIE_CHART_COMPONENT_NAME: &str = "showPieChart";
/// Donut chart title.
pub const SHOW_PIE_CHART_COMPONENT_TITLE: &str = "Donut chart";
/// Donut chart model-facing description.
pub const SHOW_PIE_CHART_COMPONENT_DESCRIPTION: &str = "Show how a whole is divided, as a donut with a legend. Use only when the parts sum to something meaningful, and prefer a bar chart above about six slices.";
/// Stable progress chart identity/title/description.
pub const SHOW_PROGRESS_COMPONENT_NAME: &str = "showProgress";
/// Progress chart title.
pub const SHOW_PROGRESS_COMPONENT_TITLE: &str = "Progress against target";
/// Progress chart model-facing description.
pub const SHOW_PROGRESS_COMPONENT_DESCRIPTION: &str = "Show values against their targets as progress bars. Use for 'are we there yet' questions, budget spent against budget, done against planned.";

/// Closed grouping used by Admin and Settings presentation; never read by the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompiledComponentKind {
    /// Data visualization.
    Chart,
    /// Read-only information card.
    Card,
    /// Human-in-the-loop decision surface.
    Decision,
}

impl CompiledComponentKind {
    /// Stable database/wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Chart => "chart",
            Self::Card => "card",
            Self::Decision => "decision",
        }
    }
}

/// Build-owned catalogue entry. A browser may repeat it but cannot choose any field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledComponentManifestEntry {
    /// Stable model tool/catalogue key.
    pub name: String,
    /// Display title.
    pub title: String,
    /// Closed gallery grouping.
    pub kind: CompiledComponentKind,
    /// Initial model-facing description.
    pub description: String,
}

/// Exact upstream catalogue announcement wire, hardened by application identity validation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentCatalogueRequest {
    /// Additive build entries; existing governance rows are never overwritten.
    pub components: Vec<CompiledComponentManifestEntry>,
}

/// Rows inserted by one additive announcement.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentCatalogueAdded {
    /// Stable names actually inserted by this call, not merely requested.
    pub added: Vec<String>,
}

/// One compiled component as the authenticated read surface sees its governance state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComponentRecord {
    /// Stable model tool/catalogue key.
    pub name: String,
    /// Human-facing title.
    pub title: String,
    /// Closed presentation grouping.
    pub kind: CompiledComponentKind,
    /// Current editable model-facing draft.
    pub draft_description: String,
    /// Published model-facing description; absent means the component cannot be offered.
    pub published_description: Option<String>,
    /// Authoritative publication bit.
    pub published: bool,
    /// Database-clock first/last publication time.
    pub published_at: Option<OffsetDateTime>,
    /// Public identifier of the last updater, if present in the compatibility row.
    pub updated_by: Option<String>,
    /// Database-clock row update time.
    pub updated_at: OffsetDateTime,
    /// Derived comparison of draft against the published description or empty string.
    pub has_unpublished_changes: bool,
    /// Stable Agent ids explicitly withheld from this otherwise open component.
    pub withheld_from: Vec<String>,
    /// Data-function names granted to this component; empty means it may call none.
    pub functions: Vec<String>,
}

/// Authenticated component list response.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentRecords {
    /// All durable compiled-component governance rows in stable kind/title/name order.
    pub components: Vec<ComponentRecord>,
}

/// One published compiled component actually available to one verified Agent runtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantedCompiledComponent {
    /// Stable renderer/tool identity.
    pub name: String,
    /// Published model-facing description; drafts never cross this boundary.
    pub description: String,
}

/// Closed runtime grant snapshot for one Agent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantedCompiledComponents {
    /// Published, non-withheld components in stable name order.
    pub components: Vec<GrantedCompiledComponent>,
}

/// Call-time authorization input for one compiled component invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComponentDecisionRequest {
    /// Untrusted Agent identity; authority is still derived from the verified session.
    pub agent_id: BotId,
    /// Data functions this exact invocation will read; empty for argument-only renderers.
    #[serde(default)]
    pub functions: Vec<String>,
}

/// Stable, localizable reason a component invocation was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentDecisionRefusal {
    /// No durable compiled-component governance row exists for the requested name.
    UnknownComponent,
    /// The component is unpublished or lacks a published model-facing description.
    Unpublished,
    /// This otherwise-open component is explicitly withheld from the Agent.
    WithheldFromAgent,
    /// The component lacks a grant for one data function declared by this invocation.
    FunctionNotGranted {
        /// Stable data-function identity; never arguments or returned data.
        function: String,
    },
    /// The requested function is not shipped by this exact Server build.
    FunctionUnavailable {
        /// Stable requested function identity.
        function: String,
    },
    /// The verified actor lacks the function's underlying data ACL.
    FunctionActorNotAuthorized {
        /// Stable requested function identity.
        function: String,
    },
    /// The current action policy refused this data read.
    FunctionPolicyRefused {
        /// Stable requested function identity.
        function: String,
    },
}

impl ComponentDecisionRefusal {
    /// Stable audit/UI classification without user-facing prose.
    #[must_use]
    pub const fn code_str(&self) -> &'static str {
        match self {
            Self::UnknownComponent => "component_unknown",
            Self::Unpublished => "component_unpublished",
            Self::WithheldFromAgent => "component_withheld",
            Self::FunctionNotGranted { .. } => "component_function_not_granted",
            Self::FunctionUnavailable { .. } => "component_function_unavailable",
            Self::FunctionActorNotAuthorized { .. } => "component_function_actor_not_authorized",
            Self::FunctionPolicyRefused { .. } => "component_function_policy_refused",
        }
    }

    /// Function identity carried by any function-level refusal.
    #[must_use]
    pub fn function(&self) -> Option<&str> {
        match self {
            Self::FunctionNotGranted { function }
            | Self::FunctionUnavailable { function }
            | Self::FunctionActorNotAuthorized { function }
            | Self::FunctionPolicyRefused { function } => Some(function),
            Self::UnknownComponent | Self::Unpublished | Self::WithheldFromAgent => None,
        }
    }
}

/// Result of the mandatory authorization check immediately before one component call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentDecision {
    /// Whether the invocation may continue.
    pub allowed: bool,
    /// Present exactly when `allowed` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<ComponentDecisionRefusal>,
}

impl ComponentDecision {
    /// Construct the only valid allowed shape.
    #[must_use]
    pub const fn allowed() -> Self {
        Self {
            allowed: true,
            refusal: None,
        }
    }

    /// Construct the only valid refused shape.
    #[must_use]
    pub const fn refused(refusal: ComponentDecisionRefusal) -> Self {
        Self {
            allowed: false,
            refusal: Some(refusal),
        }
    }

    /// Whether the two wire fields form one valid closed result.
    #[must_use]
    pub const fn is_consistent(&self) -> bool {
        self.allowed == self.refusal.is_none()
    }
}

/// One build-owned data function shown on the component administration surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentDataFunctionSummary {
    /// Stable function identity.
    pub name: String,
    /// Administrator-facing purpose; never read by the model.
    pub description: String,
    /// Bounded description of the underlying data source.
    pub reads: String,
}

/// Exact data-function registry for this Server build.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentDataFunctions {
    /// Build-owned summaries in stable name order.
    pub functions: Vec<ComponentDataFunctionSummary>,
}

/// The build-owned data functions shipped by this exact binary.
#[must_use]
pub fn component_data_function_manifest() -> Vec<ComponentDataFunctionSummary> {
    vec![
        ComponentDataFunctionSummary {
            name: BOT_ACTIVITY_FUNCTION_NAME.to_owned(),
            description: BOT_ACTIVITY_FUNCTION_DESCRIPTION.to_owned(),
            reads: AUDIT_TRAIL_READS_DESCRIPTION.to_owned(),
        },
        ComponentDataFunctionSummary {
            name: RECENT_REFUSALS_FUNCTION_NAME.to_owned(),
            description: RECENT_REFUSALS_FUNCTION_DESCRIPTION.to_owned(),
            reads: AUDIT_TRAIL_READS_DESCRIPTION.to_owned(),
        },
    ]
}

/// Untrusted call body for one component-owned data read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComponentFunctionCallRequest {
    /// Agent on whose behalf the component is rendering.
    pub agent_id: BotId,
    /// Stable build-owned function identity.
    pub function: String,
    /// Function arguments; application normalizes and bounds them by the selected registry entry.
    #[serde(default)]
    pub args: Value,
}

/// One Bot's counted actions over a bounded window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotActivityRow {
    /// Stable Bot identity from the audit payload allowlist.
    pub bot: String,
    /// Non-negative number of actions.
    pub actions: u64,
}

/// Data returned by `botActivity`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotActivityReport {
    /// Effective bounded lookback.
    pub days: u16,
    /// At most twelve Bots, ordered by actions descending then identity.
    pub rows: Vec<BotActivityRow>,
}

/// One bounded refusal projection from the audit trail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecentRefusalRow {
    /// Database-clock event time.
    pub at: OffsetDateTime,
    /// Authoritative Bot identity when the event has one.
    pub bot: Option<String>,
    /// Stable audit event type.
    pub what: String,
    /// Stable error/rule classification; never raw policy or user/model prose.
    pub reason: Option<String>,
}

/// Data returned by `recentRefusals`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecentRefusalsReport {
    /// At most fifty rows in reverse chronological order.
    pub rows: Vec<RecentRefusalRow>,
}

/// Typed data returned by one build-owned function while preserving upstream's untagged `data` body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ComponentFunctionData {
    /// `botActivity` result.
    BotActivity(BotActivityReport),
    /// `recentRefusals` result.
    RecentRefusals(RecentRefusalsReport),
}

/// Stable non-authorization failure after the read was permitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentFunctionError {
    /// The bounded local read failed; no vendor/database prose crosses the wire.
    ReadFailed,
}

/// Result of a component-owned data read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentFunctionCall {
    /// False only for an authorization refusal.
    pub allowed: bool,
    /// Present only for a successful read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<ComponentFunctionData>,
    /// Present only when authorization refused before the read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<ComponentDecisionRefusal>,
    /// Present only when an authorized read failed and was audited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ComponentFunctionError>,
}

impl ComponentFunctionCall {
    /// Successful authorized read.
    #[must_use]
    pub const fn succeeded(data: ComponentFunctionData) -> Self {
        Self {
            allowed: true,
            data: Some(data),
            refusal: None,
            error: None,
        }
    }

    /// Authorization refusal before reading data.
    #[must_use]
    pub const fn refused(refusal: ComponentDecisionRefusal) -> Self {
        Self {
            allowed: false,
            data: None,
            refusal: Some(refusal),
            error: None,
        }
    }

    /// Authorized read failed after permission checks.
    #[must_use]
    pub const fn failed(error: ComponentFunctionError) -> Self {
        Self {
            allowed: true,
            data: None,
            refusal: None,
            error: Some(error),
        }
    }

    /// Whether exactly one valid result shape is represented.
    #[must_use]
    pub const fn is_consistent(&self) -> bool {
        matches!(
            (
                self.allowed,
                self.data.is_some(),
                self.refusal.is_some(),
                self.error.is_some()
            ),
            (true, true, false, false) | (true, false, false, true) | (false, false, true, false)
        )
    }
}

/// The exact compiled renderer manifest for this build.
#[must_use]
pub fn compiled_component_manifest() -> Vec<CompiledComponentManifestEntry> {
    vec![
        manifest_entry(
            SHOW_ACTIVITY_REPORT_COMPONENT_NAME,
            SHOW_ACTIVITY_REPORT_COMPONENT_TITLE,
            CompiledComponentKind::Card,
            SHOW_ACTIVITY_REPORT_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_AREA_CHART_COMPONENT_NAME,
            SHOW_AREA_CHART_COMPONENT_TITLE,
            CompiledComponentKind::Chart,
            SHOW_AREA_CHART_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_BAR_CHART_COMPONENT_NAME,
            SHOW_BAR_CHART_COMPONENT_TITLE,
            CompiledComponentKind::Chart,
            SHOW_BAR_CHART_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_CHECKLIST_COMPONENT_NAME,
            SHOW_CHECKLIST_COMPONENT_TITLE,
            CompiledComponentKind::Card,
            SHOW_CHECKLIST_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_LINE_CHART_COMPONENT_NAME,
            SHOW_LINE_CHART_COMPONENT_TITLE,
            CompiledComponentKind::Chart,
            SHOW_LINE_CHART_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_METRICS_COMPONENT_NAME,
            SHOW_METRICS_COMPONENT_TITLE,
            CompiledComponentKind::Card,
            SHOW_METRICS_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_NOTICE_COMPONENT_NAME,
            SHOW_NOTICE_COMPONENT_TITLE,
            CompiledComponentKind::Card,
            SHOW_NOTICE_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_PIE_CHART_COMPONENT_NAME,
            SHOW_PIE_CHART_COMPONENT_TITLE,
            CompiledComponentKind::Chart,
            SHOW_PIE_CHART_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_PROGRESS_COMPONENT_NAME,
            SHOW_PROGRESS_COMPONENT_TITLE,
            CompiledComponentKind::Chart,
            SHOW_PROGRESS_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_QUOTE_COMPONENT_NAME,
            SHOW_QUOTE_COMPONENT_TITLE,
            CompiledComponentKind::Card,
            SHOW_QUOTE_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_RECORD_COMPONENT_NAME,
            SHOW_RECORD_COMPONENT_TITLE,
            CompiledComponentKind::Card,
            SHOW_RECORD_COMPONENT_DESCRIPTION,
        ),
    ]
}

fn manifest_entry(
    name: &'static str,
    title: &'static str,
    kind: CompiledComponentKind,
    description: &'static str,
) -> CompiledComponentManifestEntry {
    CompiledComponentManifestEntry {
        name: name.to_owned(),
        title: title.to_owned(),
        kind,
        description: description.to_owned(),
    }
}

/// Exact JSON Schema for `showActivityReport`.
#[must_use]
pub fn show_activity_report_parameter_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "report": {
                "type": "string",
                "enum": ["activity", "refusals"],
                "description": "Which report to show: 'activity' for how much each Bot has done, 'refusals' for what this deployment recently refused"
            },
            "title": {
                "type": "string",
                "description": "A heading for the report, in a few words"
            },
            "days": {
                "type": "number",
                "description": "For the activity report: how many days back to count. Defaults to 7"
            }
        },
        "required": ["report"]
    })
}

/// Exact JSON Schema for `showQuote`; renderer/tool registration share this single source.
#[must_use]
pub fn show_quote_parameter_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "quote": {
                "type": "string",
                "description": "The quotation itself, without surrounding quote marks"
            },
            "attribution": {
                "type": "string",
                "description": "Who said or wrote it, e.g. 'Grace Hopper' or 'the 2026 annual report'"
            },
            "context": {
                "type": "string",
                "description": "One short line of context: where it is from, or why it matters here"
            }
        },
        "required": ["quote", "attribution"]
    })
}

/// Exact JSON Schema for `showRecord`.
#[must_use]
pub fn show_record_parameter_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "title": {"type": "string", "description": "What this record is, e.g. a person or an order"},
            "subtitle": {"type": "string", "description": "One line of context under the title"},
            "status": {"type": "string", "description": "A short status word, e.g. Approved"},
            "statusTone": tone_schema(),
            "fields": {
                "type": "array",
                "description": "The fields, in the order they should be read",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "label": {"type": "string"},
                        "value": {"type": "string", "description": "Already formatted for a person to read"}
                    },
                    "required": ["label", "value"]
                }
            }
        },
        "required": ["title", "fields"]
    })
}

/// Exact JSON Schema for `showMetrics`.
#[must_use]
pub fn show_metrics_parameter_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "title": {"type": "string", "description": "What these figures are about"},
            "caption": {"type": "string"},
            "metrics": {
                "type": "array",
                "maxItems": 6,
                "description": "Up to six figures. More than that wanted a table",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "label": {"type": "string"},
                        "value": {"type": "string", "description": "Already formatted, including any unit or currency"},
                        "change": {"type": "string", "description": "The movement, e.g. '+12% on last month'"},
                        "changeTone": tone_schema()
                    },
                    "required": ["label", "value"]
                }
            }
        },
        "required": ["title", "metrics"]
    })
}

/// Exact JSON Schema for `showChecklist`.
#[must_use]
pub fn show_checklist_parameter_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "title": {"type": "string", "description": "What this list is"},
            "caption": {"type": "string"},
            "items": {
                "type": "array",
                "description": "The items, in the order they should be done",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "text": {"type": "string"},
                        "done": {"type": "boolean", "description": "Whether this one is already finished"},
                        "note": {"type": "string", "description": "A short aside, e.g. who it is waiting on"}
                    },
                    "required": ["text", "done"]
                }
            }
        },
        "required": ["title", "items"]
    })
}

/// Exact JSON Schema for `showNotice`.
#[must_use]
pub fn show_notice_parameter_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "title": {"type": "string", "description": "The headline, in a few words"},
            "body": {"type": "string", "description": "The explanation, in one or two sentences"},
            "tone": tone_schema(),
            "points": {
                "type": "array",
                "description": "Supporting points, if there are any",
                "items": {"type": "string"}
            }
        },
        "required": ["title", "body"]
    })
}

fn tone_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["neutral", "positive", "caution", "negative"],
        "description": "How this reads at a glance. Use negative and caution sparingly, for a refusal, a breach or a failure, not for anything merely notable"
    })
}

/// Exact JSON Schema for `showBarChart`.
#[must_use]
pub fn show_bar_chart_parameter_schema() -> Value {
    point_chart_schema("One bar per point, in the order they should appear", false)
}

/// Exact JSON Schema for `showPieChart`.
#[must_use]
pub fn show_pie_chart_parameter_schema() -> Value {
    point_chart_schema(
        "One slice per point. Values are summed to make the whole",
        false,
    )
}

/// Exact shared JSON Schema for `showLineChart` and `showAreaChart`.
#[must_use]
pub fn show_line_chart_parameter_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "title": {"type": "string", "description": "A short title for the chart"},
            "caption": {"type": "string", "description": "One line under the title saying what the reader should take from it"},
            "labels": {"type": "array", "description": "The x axis, e.g. months", "items": {"type": "string"}},
            "series": {
                "type": "array",
                "description": "One line per series",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "name": {"type": "string", "description": "What this line is called"},
                        "values": {"type": "array", "description": "One value per label, in the same order", "items": {"type": "number"}}
                    },
                    "required": ["name", "values"]
                }
            }
        },
        "required": ["title", "labels", "series"]
    })
}

/// Area chart intentionally reuses the exact Line chart schema.
#[must_use]
pub fn show_area_chart_parameter_schema() -> Value {
    show_line_chart_parameter_schema()
}

/// Exact JSON Schema for `showProgress`.
#[must_use]
pub fn show_progress_parameter_schema() -> Value {
    point_chart_schema("One row per thing being tracked", true)
}

fn point_chart_schema(description: &'static str, target: bool) -> Value {
    let mut properties = serde_json::Map::from_iter([
        (
            "label".to_owned(),
            json!({"type": "string", "description": "What this point is called, e.g. a month or a team"}),
        ),
        (
            "value".to_owned(),
            json!({"type": "number", "description": "Its value"}),
        ),
    ]);
    let mut required = vec!["label", "value"];
    if target {
        properties.insert(
            "target".to_owned(),
            json!({"type": "number", "description": "What the value is measured against"}),
        );
        required.push("target");
    }
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "title": {"type": "string", "description": "A short title for the chart"},
            "caption": {"type": "string", "description": "One line under the title saying what the reader should take from it"},
            "points": {
                "type": "array",
                "description": description,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": properties,
                    "required": required
                }
            }
        },
        "required": ["title", "points"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_and_compiled_schemas_are_exact_closed_and_stable() {
        let manifest = compiled_component_manifest();
        assert_eq!(manifest.len(), 11);
        assert_eq!(
            manifest
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            [
                SHOW_ACTIVITY_REPORT_COMPONENT_NAME,
                SHOW_AREA_CHART_COMPONENT_NAME,
                SHOW_BAR_CHART_COMPONENT_NAME,
                SHOW_CHECKLIST_COMPONENT_NAME,
                SHOW_LINE_CHART_COMPONENT_NAME,
                SHOW_METRICS_COMPONENT_NAME,
                SHOW_NOTICE_COMPONENT_NAME,
                SHOW_PIE_CHART_COMPONENT_NAME,
                SHOW_PROGRESS_COMPONENT_NAME,
                SHOW_QUOTE_COMPONENT_NAME,
                SHOW_RECORD_COMPONENT_NAME,
            ]
        );
        assert_eq!(
            manifest
                .iter()
                .filter(|entry| entry.kind == CompiledComponentKind::Chart)
                .count(),
            5
        );
        assert_eq!(
            show_activity_report_parameter_schema()["properties"]["report"]["enum"],
            json!(["activity", "refusals"])
        );
        assert_eq!(
            show_activity_report_parameter_schema()["required"],
            json!(["report"])
        );
        assert_eq!(
            show_quote_parameter_schema()["required"],
            json!(["quote", "attribution"])
        );
        assert_eq!(show_quote_parameter_schema()["additionalProperties"], false);
        assert_eq!(
            show_metrics_parameter_schema()["properties"]["metrics"]["maxItems"],
            6
        );
        assert_eq!(
            show_record_parameter_schema()["properties"]["statusTone"]["enum"],
            json!(["neutral", "positive", "caution", "negative"])
        );
        assert_eq!(
            show_checklist_parameter_schema()["required"],
            json!(["title", "items"])
        );
        assert_eq!(
            show_notice_parameter_schema()["required"],
            json!(["title", "body"])
        );
        assert_eq!(
            show_area_chart_parameter_schema(),
            show_line_chart_parameter_schema()
        );
        assert_eq!(
            show_bar_chart_parameter_schema()["properties"]["points"]["items"]["required"],
            json!(["label", "value"])
        );
        assert_eq!(
            show_progress_parameter_schema()["properties"]["points"]["items"]["required"],
            json!(["label", "value", "target"])
        );
        assert!(
            serde_json::from_str::<ComponentCatalogueRequest>(
                r#"{"components":[],"renderer":"untrusted"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn component_record_wire_contains_governance_facts_but_no_source_or_secret() {
        let record = ComponentRecord {
            name: SHOW_QUOTE_COMPONENT_NAME.to_owned(),
            title: SHOW_QUOTE_COMPONENT_TITLE.to_owned(),
            kind: CompiledComponentKind::Card,
            draft_description: "draft".to_owned(),
            published_description: Some("published".to_owned()),
            published: true,
            published_at: Some(OffsetDateTime::UNIX_EPOCH),
            updated_by: Some("the build".to_owned()),
            updated_at: OffsetDateTime::UNIX_EPOCH,
            has_unpublished_changes: true,
            withheld_from: vec!["agent-one".to_owned()],
            functions: vec!["readOrder".to_owned()],
        };
        let encoded = serde_json::to_value(&record).unwrap();
        assert_eq!(encoded["name"], SHOW_QUOTE_COMPONENT_NAME);
        assert_eq!(encoded["hasUnpublishedChanges"], true);
        assert!(encoded.get("source").is_none());
        assert!(encoded.get("arguments").is_none());
        assert!(encoded.get("secret").is_none());
    }

    #[test]
    fn runtime_grants_and_decisions_have_closed_payload_free_wire_shapes() {
        let request = ComponentDecisionRequest {
            agent_id: BotId::new("agent-one"),
            functions: vec!["recentRefusals".to_owned()],
        };
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"agentId":"agent-one","functions":["recentRefusals"]}"#
        );
        assert!(
            serde_json::from_str::<ComponentDecisionRequest>(
                r#"{"agentId":"agent-one","functions":[],"actor":"admin"}"#
            )
            .is_err()
        );

        let allowed = ComponentDecision::allowed();
        assert!(allowed.is_consistent());
        assert_eq!(
            serde_json::to_string(&allowed).unwrap(),
            r#"{"allowed":true}"#
        );
        let refused = ComponentDecision::refused(ComponentDecisionRefusal::FunctionNotGranted {
            function: "recentRefusals".to_owned(),
        });
        assert!(refused.is_consistent());
        assert_eq!(
            serde_json::to_string(&refused).unwrap(),
            r#"{"allowed":false,"refusal":{"code":"function_not_granted","function":"recentRefusals"}}"#
        );
        assert_eq!(
            refused.refusal.as_ref().unwrap().code_str(),
            "component_function_not_granted"
        );

        let grants = GrantedCompiledComponents {
            components: vec![GrantedCompiledComponent {
                name: SHOW_QUOTE_COMPONENT_NAME.to_owned(),
                description: SHOW_QUOTE_COMPONENT_DESCRIPTION.to_owned(),
            }],
        };
        let encoded = serde_json::to_value(grants).unwrap();
        assert_eq!(encoded["components"][0]["name"], SHOW_QUOTE_COMPONENT_NAME);
        assert!(encoded["components"][0].get("draft").is_none());
        assert!(encoded["components"][0].get("arguments").is_none());
    }

    #[test]
    fn component_data_function_registry_and_call_wire_are_exact_and_consistent() {
        let functions = component_data_function_manifest();
        assert_eq!(
            functions
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            [BOT_ACTIVITY_FUNCTION_NAME, RECENT_REFUSALS_FUNCTION_NAME]
        );
        assert!(
            functions
                .iter()
                .all(|entry| entry.reads == AUDIT_TRAIL_READS_DESCRIPTION)
        );
        let request = ComponentFunctionCallRequest {
            agent_id: BotId::new("agent-one"),
            function: BOT_ACTIVITY_FUNCTION_NAME.to_owned(),
            args: json!({"days": 7}),
        };
        assert!(
            serde_json::from_value::<ComponentFunctionCallRequest>(json!({
                "agentId": "agent-one",
                "function": BOT_ACTIVITY_FUNCTION_NAME,
                "args": {},
                "actor": "admin"
            }))
            .is_err()
        );
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({"agentId":"agent-one","function":"botActivity","args":{"days":7}})
        );

        let succeeded = ComponentFunctionCall::succeeded(ComponentFunctionData::BotActivity(
            BotActivityReport {
                days: 7,
                rows: vec![BotActivityRow {
                    bot: "agent-one".to_owned(),
                    actions: 3,
                }],
            },
        ));
        let refused =
            ComponentFunctionCall::refused(ComponentDecisionRefusal::FunctionActorNotAuthorized {
                function: BOT_ACTIVITY_FUNCTION_NAME.to_owned(),
            });
        let failed = ComponentFunctionCall::failed(ComponentFunctionError::ReadFailed);
        assert!(succeeded.is_consistent());
        assert!(refused.is_consistent());
        assert!(failed.is_consistent());
        assert_eq!(
            serde_json::to_value(&failed).unwrap(),
            json!({"allowed":true,"error":"read_failed"})
        );
        assert!(
            !ComponentFunctionCall {
                allowed: true,
                data: None,
                refusal: None,
                error: None,
            }
            .is_consistent()
        );
    }
}
