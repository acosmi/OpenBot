//! Inert preview registry for the compiled renderers actually present in this Rust build.

use leptos::prelude::*;
#[cfg(test)]
use openbot_contracts::components::compiled_component_manifest;
use openbot_contracts::components::{
    SHOW_CHECKLIST_COMPONENT_NAME, SHOW_METRICS_COMPONENT_NAME, SHOW_NOTICE_COMPONENT_NAME,
    SHOW_QUOTE_COMPONENT_NAME, SHOW_RECORD_COMPONENT_NAME,
};

use crate::i18n::{t, use_i18n};

use super::{
    ChecklistCard, ChecklistItem, GalleryTone, HeadlineMetric, MetricsCard, NoticeCard, QuoteCard,
    RecordCard, RecordField,
};

const RENDERER_NAMES: [&str; 5] = [
    SHOW_CHECKLIST_COMPONENT_NAME,
    SHOW_METRICS_COMPONENT_NAME,
    SHOW_NOTICE_COMPONENT_NAME,
    SHOW_QUOTE_COMPONENT_NAME,
    SHOW_RECORD_COMPONENT_NAME,
];

/// Stable names this build can actually draw.
#[must_use]
pub const fn renderer_names() -> &'static [&'static str] {
    &RENDERER_NAMES
}

/// Whether one durable catalogue row has a renderer in this exact build.
#[must_use]
pub fn component_has_renderer(name: &str) -> bool {
    renderer_names().contains(&name)
}

/// Draw one inert component sample or an honest stale-row fallback.
#[component]
pub fn ComponentPreview(name: String) -> AnyView {
    let i18n = use_i18n();
    let content = match name.as_str() {
        SHOW_CHECKLIST_COMPONENT_NAME => view! {
            <ChecklistCard
                title="Before the release".to_owned()
                caption=None
                items=vec![
                    ChecklistItem { text: "Migrations applied".to_owned(), done: true, note: None },
                    ChecklistItem { text: "Changelog written".to_owned(), done: true, note: None },
                    ChecklistItem { text: "Load test".to_owned(), done: false, note: Some("Waiting on staging".to_owned()) },
                ]
            />
        }
        .into_any(),
        SHOW_METRICS_COMPONENT_NAME => view! {
            <MetricsCard
                title="This month".to_owned()
                caption=None
                metrics=vec![
                    HeadlineMetric { label: "Revenue".to_owned(), value: "$412k".to_owned(), change: Some("+12% on last month".to_owned()), change_tone: GalleryTone::Positive },
                    HeadlineMetric { label: "Open deals".to_owned(), value: "38".to_owned(), change: None, change_tone: GalleryTone::Neutral },
                    HeadlineMetric { label: "Churn".to_owned(), value: "1.4%".to_owned(), change: Some("+0.3pt".to_owned()), change_tone: GalleryTone::Caution },
                ]
            />
        }
        .into_any(),
        SHOW_NOTICE_COMPONENT_NAME => view! {
            <NoticeCard
                title="Certificate expires in 30 days".to_owned()
                body="The checkout certificate has an owner now, and this is the first of the new alerts.".to_owned()
                tone=GalleryTone::Caution
                points=vec!["Owner: Platform".to_owned(), "Renews automatically once approved".to_owned()]
            />
        }
        .into_any(),
        SHOW_QUOTE_COMPONENT_NAME => view! {
            <QuoteCard
                quote="Meals under $75 need no receipt. Anything above needs one, and anything above $500 needs your manager before you spend it.".to_owned()
                attribution="the expense policy".to_owned()
                context="Last changed in March.".to_owned()
            />
        }
        .into_any(),
        SHOW_RECORD_COMPONENT_NAME => view! {
            <RecordCard
                title="Invoice 2043".to_owned()
                subtitle=Some("Northwind Traders".to_owned())
                status=Some("Approved".to_owned())
                status_tone=GalleryTone::Neutral
                fields=vec![
                    RecordField { label: "Amount".to_owned(), value: "$4,280.00".to_owned() },
                    RecordField { label: "Raised".to_owned(), value: "12 March".to_owned() },
                    RecordField { label: "Owner".to_owned(), value: "Priya Raman".to_owned() },
                ]
            />
        }
        .into_any(),
        _ => view! {
            <p class="ob-gallery-preview-unavailable">
                {move || t!(i18n, gallery.renderer_unavailable)}
            </p>
        }
        .into_any(),
    };
    view! {
        <div class="ob-gallery-preview" aria-hidden="true">
            {content}
        </div>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_registry_is_unique_and_exactly_matches_the_current_manifest() {
        let manifest = compiled_component_manifest();
        assert_eq!(
            renderer_names(),
            [
                SHOW_CHECKLIST_COMPONENT_NAME,
                SHOW_METRICS_COMPONENT_NAME,
                SHOW_NOTICE_COMPONENT_NAME,
                SHOW_QUOTE_COMPONENT_NAME,
                SHOW_RECORD_COMPONENT_NAME,
            ]
        );
        assert_eq!(
            manifest
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            renderer_names()
        );
        assert!(component_has_renderer(SHOW_QUOTE_COMPONENT_NAME));
        assert!(!component_has_renderer("showLegacyWidget"));
    }
}
