//! Shared 200px secondary navigation for implemented `/settings` destinations.

use leptos::prelude::*;
use leptos_router::hooks::use_location;

use crate::i18n::{t, t_string, use_i18n};
use crate::icons::Icon;
use crate::primitives::{IconSize, IconView};

const GENERAL_PATH: &str = "/settings";
const MEMORY_PATH: &str = "/settings/memory";

/// Settings secondary shell. The global App shell remains the only owner of `<main>`.
#[component]
pub fn SettingsShell(children: Children) -> impl IntoView {
    let i18n = use_i18n();
    let location = use_location();
    let general_location = location.clone();
    let memory_location = location;
    view! {
        <div class="ob-settings-shell">
            <aside class="ob-settings-subnav">
                <nav aria-label=move || t_string!(i18n, settings.title).to_owned()>
                    <a class="ob-settings-back" href="/">
                        <IconView icon=Icon::ArrowLeft size=IconSize::Inline />
                        <span>{move || t!(i18n, settings.back_to_app)}</span>
                    </a>
                    <ul class="ob-settings-subnav-list">
                        <li>
                            <a
                                class="ob-settings-subnav-link"
                                href=GENERAL_PATH
                                data-state=move || {
                                    is_current(&general_location.pathname.get(), GENERAL_PATH)
                                        .then_some("current")
                                }
                                aria-current=move || {
                                    is_current(&general_location.pathname.get(), GENERAL_PATH)
                                        .then_some("page")
                                }
                            >
                                <IconView icon=Icon::Settings size=IconSize::Inline />
                                <span>{move || t!(i18n, settings.nav_general)}</span>
                            </a>
                        </li>
                        <li>
                            <a
                                class="ob-settings-subnav-link"
                                href=MEMORY_PATH
                                data-state=move || {
                                    is_current(&memory_location.pathname.get(), MEMORY_PATH)
                                        .then_some("current")
                                }
                                aria-current=move || {
                                    is_current(&memory_location.pathname.get(), MEMORY_PATH)
                                        .then_some("page")
                                }
                            >
                                <IconView icon=Icon::Brain size=IconSize::Inline />
                                <span>{move || t!(i18n, shell.nav_memory)}</span>
                            </a>
                        </li>
                    </ul>
                </nav>
            </aside>
            <div class="ob-settings-shell-content">
                {children()}
            </div>
        </div>
    }
}

fn is_current(pathname: &str, destination: &str) -> bool {
    pathname == destination
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_settings_destinations_exist_and_general_is_exact() {
        assert_eq!([GENERAL_PATH, MEMORY_PATH].len(), 2);
        assert!(is_current("/settings", GENERAL_PATH));
        assert!(!is_current("/settings/memory", GENERAL_PATH));
        assert!(is_current("/settings/memory", MEMORY_PATH));
        assert!(!is_current("/settings/connected-accounts", MEMORY_PATH));
    }
}
