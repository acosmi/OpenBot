//! Three-state token theme switch.

use leptos::prelude::*;

use crate::i18n::{t, t_string, use_i18n};
use crate::icons::Icon;

use super::{IconSize, IconView};

/// Theme preference represented by the `<html>` class contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    /// Follow `prefers-color-scheme`; neither forcing class is present.
    #[default]
    System,
    /// Force the light token block.
    Light,
    /// Force the dark token block.
    Dark,
}

/// Switch the current document among system/light/dark without reloading.
#[component]
pub fn ThemeToggle() -> impl IntoView {
    let i18n = use_i18n();
    let theme = RwSignal::new(current_theme());
    let select = move |next: Theme| {
        apply_theme(next);
        theme.set(next);
    };
    view! {
        <div
            class="ob-segmented"
            role="radiogroup"
            aria-label=move || t_string!(i18n, shell.theme_label).to_owned()
        >
            <button
                type="button"
                class="ob-segmented-button"
                role="radio"
                aria-checked=move || theme.get() == Theme::System
                on:click=move |_| select(Theme::System)
            >
                <IconView icon=Icon::SunMoon size=IconSize::Inline />
                {move || t!(i18n, shell.theme_system)}
            </button>
            <button
                type="button"
                class="ob-segmented-button"
                role="radio"
                aria-checked=move || theme.get() == Theme::Light
                on:click=move |_| select(Theme::Light)
            >
                <IconView icon=Icon::Sun size=IconSize::Inline />
                {move || t!(i18n, shell.theme_light)}
            </button>
            <button
                type="button"
                class="ob-segmented-button"
                role="radio"
                aria-checked=move || theme.get() == Theme::Dark
                on:click=move |_| select(Theme::Dark)
            >
                <IconView icon=Icon::Moon size=IconSize::Inline />
                {move || t!(i18n, shell.theme_dark)}
            </button>
        </div>
    }
}

#[cfg(target_arch = "wasm32")]
fn current_theme() -> Theme {
    let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    else {
        return Theme::System;
    };
    let classes = root.class_list();
    if classes.contains("dark") {
        Theme::Dark
    } else if classes.contains("light") {
        Theme::Light
    } else {
        Theme::System
    }
}

#[cfg(not(target_arch = "wasm32"))]
const fn current_theme() -> Theme {
    Theme::System
}

#[cfg(target_arch = "wasm32")]
fn apply_theme(theme: Theme) {
    let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    else {
        return;
    };
    let classes = root.class_list();
    _ = classes.remove_2("light", "dark");
    match theme {
        Theme::System => {}
        Theme::Light => _ = classes.add_1("light"),
        Theme::Dark => _ = classes.add_1("dark"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
const fn apply_theme(_theme: Theme) {}
