//! Two-locale switch synchronized through `leptos_i18n`.

use leptos::prelude::*;

use crate::i18n::{Locale, t_string, use_i18n};

/// Switch between the two first-release locales without reloading the bundle.
#[component]
pub fn LocaleSwitch() -> impl IntoView {
    let i18n = use_i18n();
    view! {
        <div
            class="ob-segmented"
            role="group"
            aria-label=move || t_string!(i18n, shell.language_label).to_owned()
        >
            <button
                type="button"
                class="ob-segmented-button"
                aria-pressed=move || i18n.get_locale() == Locale::en
                on:click=move |_| i18n.set_locale(Locale::en)
            >
                "EN"
            </button>
            <button
                type="button"
                class="ob-segmented-button"
                aria-pressed=move || i18n.get_locale() == Locale::zh_CN
                on:click=move |_| i18n.set_locale(Locale::zh_CN)
            >
                "简中"
            </button>
        </div>
    }
}
