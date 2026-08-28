//! `/agents` roster destination with URL-owned read-only detail state.

use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_query_map};
use openbot_contracts::agent::{AgentProfile, AgentVisibility};

#[cfg(target_arch = "wasm32")]
use crate::api::list_agents;
use crate::features::layout::{
    DetailPanel, DetailPanelLayout, DetailPanelMain, PageEmpty, PageHeader, PageSection, PageShell,
    PageWidth, StaggerItem,
};
use crate::i18n::{t, t_string, use_i18n};
use crate::primitives::{Button, ButtonSize, ButtonVariant};

use super::{AgentCard, AgentProfilePanel};

/// Current-user coworker roster. Create/edit/hide/delete controls remain absent until their APIs land.
#[component]
pub fn AgentsPage() -> impl IntoView {
    let i18n = use_i18n();
    let query = use_query_map();
    let navigate = use_navigate();
    let selected_agent_id = Memo::new(move |_| query.get().get("agent"));
    let agents = RwSignal::new(Vec::<AgentProfile>::new());
    let loading = RwSignal::new(true);
    let load_error = RwSignal::new(false);
    let reload_generation = RwSignal::new(0_u64);
    install_agent_loader(reload_generation, agents, loading, load_error);

    let mine = Memo::new(move |_| {
        split_agents(&agents.get())
            .0
            .into_iter()
            .enumerate()
            .collect::<Vec<_>>()
    });
    let explore = Memo::new(move |_| {
        split_agents(&agents.get())
            .1
            .into_iter()
            .enumerate()
            .collect::<Vec<_>>()
    });
    let retry =
        move |_| reload_generation.update(|generation| *generation = generation.saturating_add(1));
    let close = move |_| navigate("/agents", Default::default());
    let detail_agent = Signal::derive(move || selected_agent_id.get());
    let panel_open = Signal::derive(move || selected_agent_id.get().is_some());

    view! {
        <DetailPanelLayout>
            <DetailPanelMain>
                <div id="agents-roster-focus" class="ob-agent-roster-focus" tabindex="-1">
                    <PageShell width=PageWidth::Content>
                        <div class="ob-agent-roster-content">
                            <PageHeader
                                heading_id="agents-page-title"
                                title=move || t_string!(i18n, agents.title).to_owned()
                            />
                            <Show when=move || loading.get()>
                                <div class="ob-loading" role="status">
                                    {move || t!(i18n, common.loading)}
                                </div>
                            </Show>
                            <Show when=move || load_error.get()>
                                <div class="ob-alert" role="alert">
                                    <span>{move || t!(i18n, agents.load_error)}</span>
                                    <Button
                                        variant=ButtonVariant::Ghost
                                        size=ButtonSize::Small
                                        on_activate=retry
                                    >
                                        {move || t!(i18n, common.retry)}
                                    </Button>
                                </div>
                            </Show>
                            <Show when=move || !loading.get() && !load_error.get()>
                                <PageSection
                                    heading_id="agents-mine-title"
                                    title=move || t_string!(i18n, agents.your_agents).to_owned()
                                >
                                    <Show
                                        when=move || !mine.get().is_empty()
                                        fallback=move || view! {
                                            <PageEmpty>{move || t!(i18n, agents.mine_empty)}</PageEmpty>
                                        }
                                    >
                                        <div class="ob-agent-grid">
                                            <For
                                                each=move || mine.get()
                                                key=|(_, agent)| agent.id.clone()
                                                children=move |(index, agent)| {
                                                    view! {
                                                        <StaggerItem index=index>
                                                            <AgentCard agent />
                                                        </StaggerItem>
                                                    }
                                                }
                                            />
                                        </div>
                                    </Show>
                                </PageSection>
                                <PageSection
                                    heading_id="agents-explore-title"
                                    title=move || t_string!(i18n, agents.explore_agents).to_owned()
                                >
                                    <Show
                                        when=move || !explore.get().is_empty()
                                        fallback=move || view! {
                                            <PageEmpty>{move || t!(i18n, agents.explore_empty)}</PageEmpty>
                                        }
                                    >
                                        <div class="ob-agent-grid">
                                            <For
                                                each=move || explore.get()
                                                key=|(_, agent)| agent.id.clone()
                                                children=move |(index, agent)| {
                                                    view! {
                                                        <StaggerItem index=index>
                                                            <AgentCard agent />
                                                        </StaggerItem>
                                                    }
                                                }
                                            />
                                        </div>
                                    </Show>
                                </PageSection>
                            </Show>
                        </div>
                    </PageShell>
                </div>
            </DetailPanelMain>
            <DetailPanel
                id="agent-profile-panel"
                title=move || t_string!(i18n, agents.profile).to_owned()
                open=panel_open
                return_focus_id="agents-roster-focus"
                on_close=close
            >
                <AgentProfilePanel agent_id=detail_agent />
            </DetailPanel>
        </DetailPanelLayout>
    }
}

fn split_agents(agents: &[AgentProfile]) -> (Vec<AgentProfile>, Vec<AgentProfile>) {
    let mine = agents.iter().filter(|agent| agent.mine).cloned().collect();
    let explore = agents
        .iter()
        .filter(|agent| !agent.mine && agent.visibility == AgentVisibility::Public)
        .cloned()
        .collect();
    (mine, explore)
}

fn install_agent_loader(
    reload_generation: RwSignal<u64>,
    agents: RwSignal<Vec<AgentProfile>>,
    loading: RwSignal<bool>,
    load_error: RwSignal<bool>,
) {
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        let generation = reload_generation.get();
        loading.set(true);
        load_error.set(false);
        leptos::task::spawn_local_scoped_with_cancellation(async move {
            let outcome = list_agents(false).await;
            if reload_generation.get_untracked() != generation {
                return;
            }
            match outcome {
                Ok(loaded) => agents.set(loaded),
                Err(_) => {
                    agents.set(Vec::new());
                    load_error.set(true);
                }
            }
            loading.set(false);
        });
    });
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (reload_generation, agents, loading, load_error);
}

#[cfg(test)]
mod tests {
    use openbot_contracts::ids::BotId;

    use super::*;

    fn profile(id: &str, mine: bool, visibility: AgentVisibility) -> AgentProfile {
        AgentProfile {
            id: BotId::new(id),
            name: id.to_owned(),
            title: "Title".to_owned(),
            role_description: "Role".to_owned(),
            avatar_seed: id.to_owned(),
            visibility,
            endpoint: None,
            has_auth: false,
            has_callback_token: false,
            hidden: false,
            system_owned: false,
            can_manage: mine,
            mine,
        }
    }

    #[test]
    fn roster_groups_follow_upstream_mine_and_public_explore_predicates() {
        let agents = [
            profile("mine-private", true, AgentVisibility::Private),
            profile("mine-public", true, AgentVisibility::Public),
            profile("explore-public", false, AgentVisibility::Public),
            profile("admin-visible-private", false, AgentVisibility::Private),
        ];
        let (mine, explore) = split_agents(&agents);
        assert_eq!(
            mine.iter()
                .map(|agent| agent.id.as_str())
                .collect::<Vec<_>>(),
            ["mine-private", "mine-public"]
        );
        assert_eq!(
            explore
                .iter()
                .map(|agent| agent.id.as_str())
                .collect::<Vec<_>>(),
            ["explore-public"]
        );
    }
}
