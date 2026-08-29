//! Shared Server Web/Desktop Leptos application shell and route root.

use leptos::prelude::*;
use leptos_meta::{Title, provide_meta_context};
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;

use crate::features::admin::{
    AdminAuditPage, AdminBoundariesPage, AdminHomePage, AdminIdentityProvidersPage,
    AdminPeoplePage, AdminShell, SandboxPlaygroundPage,
};
use crate::features::agents::AgentsPage;
use crate::features::approvals::ApprovalPage;
use crate::features::channels::{ChannelDetailPage, ChannelNewPage};
use crate::features::memory::MemoryPage;
use crate::features::settings::{
    ComponentGalleryDetailPage, ComponentsGalleryPage, ConnectedAccountDetailPage,
    ConnectedAccountsPage, SettingsPage, SettingsShell,
};
use crate::i18n::{I18nContextProvider, t, t_string, use_i18n};
use crate::preferences::provide_ui_preferences;
use crate::primitives::{Sidebar, SidebarProvider, SidebarTrigger};
use crate::shell::{AppSidebar, AuthenticatedBoundary, HomePage};

/// Single CSR application root used by both supported hosts.
#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();
    view! {
        <I18nContextProvider set_lang_attr_on_html=true enable_cookie=false>
            <Title text="OpenBot" />
            <Router>
                <AppShell />
            </Router>
        </I18nContextProvider>
    }
}

#[component]
fn AppShell() -> impl IntoView {
    view! {
        <AuthenticatedBoundary>
            <AuthenticatedShell />
        </AuthenticatedBoundary>
    }
}

#[component]
fn AuthenticatedShell() -> impl IntoView {
    let i18n = use_i18n();
    provide_ui_preferences(i18n);
    let collapsed = RwSignal::new(false);
    view! {
        <a class="ob-skip-link" href="#main-content">
            {move || t!(i18n, shell.skip_to_content)}
        </a>
        <SidebarProvider
            id="app-sidebar".to_owned()
            collapsed
            aria_label=move || t_string!(i18n, shell.nav_channels).to_owned()
            mobile_title=move || t_string!(i18n, shell.sidebar_mobile_title).to_owned()
            mobile_description=move || t_string!(i18n, shell.sidebar_mobile_description).to_owned()
        >
            <div class="ob-app-shell">
                <Sidebar>
                    <AppSidebar />
                </Sidebar>
                <div class="ob-app-stage">
                    <header class="ob-shell-topbar">
                        <SidebarTrigger aria_label=move || t_string!(i18n, shell.sidebar_toggle).to_owned() />
                        <span>{move || t!(i18n, common.app_name)}</span>
                    </header>
                    <main id="main-content" class="ob-main" tabindex="-1">
                        <AppRoutes />
                    </main>
                </div>
            </div>
        </SidebarProvider>
    }
}

#[component]
fn AppRoutes() -> impl IntoView {
    #[cfg(feature = "design-gallery")]
    {
        view! {
            <Routes fallback=NotFound>
                <Route path=path!("/_design") view=crate::design_gallery::DesignGallery />
                <Route path=path!("/") view=HomePage />
                <Route path=path!("/approvals") view=ApprovalPage />
                <Route path=path!("/agents") view=AgentsPage />
                <Route path=path!("/admin") view=AdminHomeRoute />
                <Route path=path!("/admin/audit") view=AdminAuditRoute />
                <Route path=path!("/admin/boundaries") view=AdminBoundariesRoute />
                <Route path=path!("/admin/identity-providers") view=AdminIdentityProvidersRoute />
                <Route path=path!("/admin/people") view=AdminPeopleRoute />
                <Route path=path!("/admin/playground") view=AdminPlaygroundRoute />
                <Route path=path!("/channel/new") view=ChannelNewPage />
                <Route path=path!("/channel/:channel_id") view=ChannelDetailPage />
                <Route path=path!("/settings/connected-accounts/:server_id") view=SettingsConnectedAccountDetailRoute />
                <Route path=path!("/settings/connected-accounts") view=SettingsConnectedAccountsRoute />
                <Route path=path!("/settings/components-gallery/:name") view=SettingsComponentGalleryDetailRoute />
                <Route path=path!("/settings/components-gallery") view=SettingsComponentsGalleryRoute />
                <Route path=path!("/settings/memory") view=SettingsMemoryRoute />
                <Route path=path!("/settings") view=SettingsPreferencesRoute />
            </Routes>
        }
    }
    #[cfg(not(feature = "design-gallery"))]
    {
        view! {
            <Routes fallback=NotFound>
                <Route path=path!("/") view=HomePage />
                <Route path=path!("/approvals") view=ApprovalPage />
                <Route path=path!("/agents") view=AgentsPage />
                <Route path=path!("/admin") view=AdminHomeRoute />
                <Route path=path!("/admin/audit") view=AdminAuditRoute />
                <Route path=path!("/admin/boundaries") view=AdminBoundariesRoute />
                <Route path=path!("/admin/identity-providers") view=AdminIdentityProvidersRoute />
                <Route path=path!("/admin/people") view=AdminPeopleRoute />
                <Route path=path!("/admin/playground") view=AdminPlaygroundRoute />
                <Route path=path!("/channel/new") view=ChannelNewPage />
                <Route path=path!("/channel/:channel_id") view=ChannelDetailPage />
                <Route path=path!("/settings/connected-accounts/:server_id") view=SettingsConnectedAccountDetailRoute />
                <Route path=path!("/settings/connected-accounts") view=SettingsConnectedAccountsRoute />
                <Route path=path!("/settings/components-gallery/:name") view=SettingsComponentGalleryDetailRoute />
                <Route path=path!("/settings/components-gallery") view=SettingsComponentsGalleryRoute />
                <Route path=path!("/settings/memory") view=SettingsMemoryRoute />
                <Route path=path!("/settings") view=SettingsPreferencesRoute />
            </Routes>
        }
    }
}

#[component]
fn AdminHomeRoute() -> impl IntoView {
    view! { <AdminShell><AdminHomePage /></AdminShell> }
}

#[component]
fn AdminAuditRoute() -> impl IntoView {
    view! { <AdminShell><AdminAuditPage /></AdminShell> }
}

#[component]
fn AdminBoundariesRoute() -> impl IntoView {
    view! { <AdminShell><AdminBoundariesPage /></AdminShell> }
}

#[component]
fn AdminIdentityProvidersRoute() -> impl IntoView {
    view! { <AdminShell><AdminIdentityProvidersPage /></AdminShell> }
}

#[component]
fn AdminPeopleRoute() -> impl IntoView {
    view! { <AdminShell><AdminPeoplePage /></AdminShell> }
}

#[component]
fn AdminPlaygroundRoute() -> impl IntoView {
    view! { <AdminShell><SandboxPlaygroundPage /></AdminShell> }
}

#[component]
fn SettingsPreferencesRoute() -> impl IntoView {
    view! {
        <SettingsShell>
            <SettingsPage />
        </SettingsShell>
    }
}

#[component]
fn SettingsMemoryRoute() -> impl IntoView {
    view! {
        <SettingsShell>
            <MemoryPage />
        </SettingsShell>
    }
}

#[component]
fn SettingsConnectedAccountsRoute() -> impl IntoView {
    view! {
        <SettingsShell>
            <ConnectedAccountsPage />
        </SettingsShell>
    }
}

#[component]
fn SettingsConnectedAccountDetailRoute() -> impl IntoView {
    view! {
        <SettingsShell>
            <ConnectedAccountDetailPage />
        </SettingsShell>
    }
}

#[component]
fn SettingsComponentsGalleryRoute() -> impl IntoView {
    view! {
        <SettingsShell>
            <ComponentsGalleryPage />
        </SettingsShell>
    }
}

#[component]
fn SettingsComponentGalleryDetailRoute() -> impl IntoView {
    view! {
        <SettingsShell>
            <ComponentGalleryDetailPage />
        </SettingsShell>
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
