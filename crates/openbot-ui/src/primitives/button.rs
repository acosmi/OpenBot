//! Accessible button primitive.

use leptos::ev::MouseEvent;
use leptos::prelude::*;

/// Visual button variant. Only `Primary` uses a solid inverse surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    /// Neutral chip button.
    #[default]
    Chip,
    /// The page's single primary action treatment.
    Primary,
    /// Low-emphasis navigation/action.
    Ghost,
    /// Destructive intent expressed by text and icon color, never a filled red surface.
    DangerText,
}

impl ButtonVariant {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Chip => "chip",
            Self::Primary => "primary",
            Self::Ghost => "ghost",
            Self::DangerText => "danger-text",
        }
    }
}

/// Button height from the tokenized control scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    /// 28px control.
    Small,
    /// 32px default control.
    #[default]
    Medium,
    /// 36px control.
    Large,
}

impl ButtonSize {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Small => "sm",
            Self::Medium => "md",
            Self::Large => "lg",
        }
    }
}

/// Render a semantic `<button>` with token-only variants and explicit loading/disabled states.
#[component]
pub fn Button(
    /// Visual treatment.
    #[prop(optional)]
    variant: ButtonVariant,
    /// Tokenized control height.
    #[prop(optional)]
    size: ButtonSize,
    /// Reactive disabled state.
    #[prop(optional, into)]
    disabled: MaybeProp<bool>,
    /// Reactive loading state; loading also disables activation.
    #[prop(optional, into)]
    loading: MaybeProp<bool>,
    /// Activation callback.
    #[prop(into)]
    on_click: UnsyncCallback<MouseEvent>,
    /// Visible button content.
    children: Children,
) -> impl IntoView {
    let unavailable =
        Signal::derive(move || disabled.get().unwrap_or(false) || loading.get().unwrap_or(false));
    view! {
        <button
            type="button"
            class="ob-button"
            data-variant=variant.as_str()
            data-size=size.as_str()
            data-state=move || loading.get().unwrap_or(false).then_some("loading")
            aria-busy=move || loading.get().unwrap_or(false).then_some("true")
            disabled=move || unavailable.get()
            on:click=move |event| {
                if !unavailable.get() {
                    on_click.run(event);
                }
            }
        >
            {children()}
        </button>
    }
}
