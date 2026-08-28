//! Deployment-scoped user preferences at `/settings`.

use leptos::prelude::*;

use crate::features::layout::{PageHeader, PageSection, PageShell, PageWidth};
use crate::i18n::{t, t_string, use_i18n};
use crate::preferences::PreferenceSaveStatus;
use crate::primitives::{LocaleSwitch, ThemeToggle};

/// User-owned appearance and language settings backed by the shared preference context.
#[component]
pub fn SettingsPage() -> impl IntoView {
    let i18n = use_i18n();
    view! {
        <PageShell width=PageWidth::Content>
            <PageHeader
                heading_id="settings-page-title"
                title=move || t_string!(i18n, settings.preferences_title).to_owned()
                description=move || t_string!(i18n, settings.preferences_description).to_owned()
            />
            <PageSection
                heading_id="settings-general-title"
                title=move || t_string!(i18n, settings.nav_general).to_owned()
            >
                <div class="ob-settings-preference-list">
                    <div class="ob-settings-preference-row">
                        <div class="ob-settings-preference-copy">
                            <h3>{move || t!(i18n, settings.appearance_theme_label)}</h3>
                            <p>{move || t!(i18n, settings.appearance_theme_help)}</p>
                        </div>
                        <div class="ob-settings-preference-actions">
                            <ThemeToggle />
                        </div>
                    </div>
                    <div class="ob-settings-preference-row">
                        <div class="ob-settings-preference-copy">
                            <h3>{move || t!(i18n, settings.nav_language)}</h3>
                            <p>{move || t!(i18n, settings.language_help)}</p>
                        </div>
                        <div class="ob-settings-preference-actions">
                            <LocaleSwitch id="settings-locale-switch" />
                        </div>
                    </div>
                </div>
                <div class="ob-settings-preference-status">
                    <PreferenceSaveStatus />
                </div>
            </PageSection>
        </PageShell>
    }
}
