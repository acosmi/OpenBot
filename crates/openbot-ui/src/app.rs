//! Shared Server Web/Desktop Leptos application shell and route root.

use leptos::prelude::*;
use leptos_meta::{Title, provide_meta_context};
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;

use crate::features::approvals::ApprovalPage;
use crate::i18n::{I18nContextProvider, t, t_string, use_i18n};
use crate::icons::Icon;
use crate::preferences::{PreferenceSaveStatus, provide_ui_preferences};
use crate::primitives::{IconSize, IconView, LocaleSwitch, ThemeToggle};

/// Single CSR application root used by both supported hosts.
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    view! {
        <I18nContextProvider set_lang_attr_on_html=true enable_cookie=false>
            <Title text="Approvals · OpenBot" />
            <Router>
                <AppShell />
            </Router>
        </I18nContextProvider>
    }
}

#[component]
fn AppShell() -> impl IntoView {
    let i18n = use_i18n();
    provide_ui_preferences(i18n);
    view! {
        <a class="ob-skip-link" href="#main-content">
            {move || t!(i18n, shell.skip_to_content)}
        </a>
        <div class="ob-app-shell">
            <aside class="ob-sidebar">
                <div class="ob-brand">
                    <IconView icon=Icon::Bot size=IconSize::Navigation />
                    <span>{move || t!(i18n, common.app_name)}</span>
                </div>
                <nav
                    class="ob-nav"
                    aria-label=move || t_string!(i18n, shell.nav_admin).to_owned()
                >
                    <a class="ob-nav-link" href="/approvals" aria-current="page">
                        <IconView icon=Icon::ListChecks size=IconSize::Navigation />
                        <span>{move || t!(i18n, admin.nav_approvals)}</span>
                    </a>
                </nav>
                <div class="ob-sidebar-controls">
                    <ThemeToggle />
                    <LocaleSwitch />
                    <PreferenceSaveStatus />
                </div>
            </aside>
            <main id="main-content" class="ob-main" tabindex="-1">
                <Routes fallback=NotFound>
                    <Route path=path!("/") view=ApprovalPage />
                    <Route path=path!("/approvals") view=ApprovalPage />
                </Routes>
            </main>
        </div>
    }
}

#[component]
fn NotFound() -> impl IntoView {
    let i18n = use_i18n();
    view! {
        <section class="ob-page">
            <h1 class="ob-page-title">{move || t!(i18n, errors.not_found_title)}</h1>
            <p class="ob-page-intro">{move || t!(i18n, errors.not_found_body)}</p>
        </section>
    }
}
