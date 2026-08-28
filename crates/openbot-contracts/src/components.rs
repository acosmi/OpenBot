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
    vec![CompiledComponentManifestEntry {
        name: SHOW_QUOTE_COMPONENT_NAME.to_owned(),
        title: SHOW_QUOTE_COMPONENT_TITLE.to_owned(),
        kind: CompiledComponentKind::Card,
        description: SHOW_QUOTE_COMPONENT_DESCRIPTION.to_owned(),
    }]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_manifest_and_quote_schema_are_exact_closed_and_stable() {
        let manifest = compiled_component_manifest();
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest[0].name, SHOW_QUOTE_COMPONENT_NAME);
        assert_eq!(manifest[0].kind.as_str(), "card");
        assert_eq!(
            show_quote_parameter_schema()["required"],
            json!(["quote", "attribution"])
        );
        assert_eq!(show_quote_parameter_schema()["additionalProperties"], false);
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
