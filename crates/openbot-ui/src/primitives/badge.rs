//! Text-and-dot status badge primitive.

use leptos::prelude::*;

/// Semantic badge tone. Color is never the only carrier because text is always present.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BadgeTone {
    /// Neutral status.
    #[default]
    Neutral,
    /// Caution status.
    Caution,
    /// Success status.
    Success,
    /// Danger status.
    Danger,
}

impl BadgeTone {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Caution => "caution",
            Self::Success => "success",
            Self::Danger => "danger",
        }
    }
}

/// Render a compact status badge with a decorative dot and visible text.
#[component]
pub fn Badge(
    /// Semantic tone.
    #[prop(optional)]
    tone: BadgeTone,
    /// Visible status text.
    children: Children,
) -> impl IntoView {
    view! {
        <span class="ob-badge" data-tone=tone.as_str()>
            <span class="ob-badge-dot" aria-hidden="true"></span>
            {children()}
        </span>
    }
}
