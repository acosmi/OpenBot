//! Compiled component catalogue and read-only governance projections.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;

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

/// The exact compiled renderer manifest for this build.
#[must_use]
pub fn compiled_component_manifest() -> Vec<CompiledComponentManifestEntry> {
    vec![
        manifest_entry(
            SHOW_CHECKLIST_COMPONENT_NAME,
            SHOW_CHECKLIST_COMPONENT_TITLE,
            SHOW_CHECKLIST_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_METRICS_COMPONENT_NAME,
            SHOW_METRICS_COMPONENT_TITLE,
            SHOW_METRICS_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_NOTICE_COMPONENT_NAME,
            SHOW_NOTICE_COMPONENT_TITLE,
            SHOW_NOTICE_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_QUOTE_COMPONENT_NAME,
            SHOW_QUOTE_COMPONENT_TITLE,
            SHOW_QUOTE_COMPONENT_DESCRIPTION,
        ),
        manifest_entry(
            SHOW_RECORD_COMPONENT_NAME,
            SHOW_RECORD_COMPONENT_TITLE,
            SHOW_RECORD_COMPONENT_DESCRIPTION,
        ),
    ]
}

fn manifest_entry(
    name: &'static str,
    title: &'static str,
    description: &'static str,
) -> CompiledComponentManifestEntry {
    CompiledComponentManifestEntry {
        name: name.to_owned(),
        title: title.to_owned(),
        kind: CompiledComponentKind::Card,
        description: description.to_owned(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_manifest_and_quote_schema_are_exact_closed_and_stable() {
        let manifest = compiled_component_manifest();
        assert_eq!(manifest.len(), 5);
        assert_eq!(
            manifest
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            [
                SHOW_CHECKLIST_COMPONENT_NAME,
                SHOW_METRICS_COMPONENT_NAME,
                SHOW_NOTICE_COMPONENT_NAME,
                SHOW_QUOTE_COMPONENT_NAME,
                SHOW_RECORD_COMPONENT_NAME,
            ]
        );
        assert!(manifest.iter().all(|entry| entry.kind.as_str() == "card"));
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
}
