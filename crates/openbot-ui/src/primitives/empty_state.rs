//! Empty-state primitive.

use leptos::prelude::*;

/// Render a titled empty state without inventing a disabled action.
#[component]
pub fn EmptyState(
    /// Empty-state heading.
    #[prop(into)]
    title: String,
    /// Explanatory body.
    #[prop(into)]
    body: String,
) -> impl IntoView {
    view! {
        <section class="ob-empty-state" aria-labelledby="approval-empty-title">
            <h2 id="approval-empty-title" class="ob-empty-title">{title}</h2>
            <p class="ob-empty-body">{body}</p>
        </section>
    }
}
