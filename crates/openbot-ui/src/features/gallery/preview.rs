//! Inert preview registry for the compiled renderers actually present in this Rust build.

use leptos::prelude::*;
use openbot_contracts::components::SHOW_QUOTE_COMPONENT_NAME;
#[cfg(test)]
use openbot_contracts::components::compiled_component_manifest;

use crate::i18n::{t, use_i18n};

use super::QuoteCard;

const RENDERER_NAMES: [&str; 1] = [SHOW_QUOTE_COMPONENT_NAME];

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
        SHOW_QUOTE_COMPONENT_NAME => view! {
            <QuoteCard
                quote="Meals under $75 need no receipt. Anything above needs one, and anything above $500 needs your manager before you spend it.".to_owned()
                attribution="the expense policy".to_owned()
                context="Last changed in March.".to_owned()
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
        assert_eq!(renderer_names(), [SHOW_QUOTE_COMPONENT_NAME]);
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
