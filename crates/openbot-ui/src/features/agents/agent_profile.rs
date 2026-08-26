//! Read-only Agent profile panel backed by the authoritative detail API.

use leptos::prelude::*;
use openbot_contracts::agent::{AgentProfile as AgentProfileDto, AgentVisibility};

#[cfg(target_arch = "wasm32")]
use crate::api::load_agent;
use crate::i18n::{t, t_string, use_i18n};
use crate::primitives::{Avatar, AvatarSize, Badge};

/// Load and render the selected coworker. Mutation controls land only with their real APIs.
#[component]
pub fn AgentProfilePanel(
    /// URL-owned selected Agent identity; `None` clears pending profile state.
    #[prop(into)]
    agent_id: Signal<Option<String>>,
) -> impl IntoView {
    let i18n = use_i18n();
    let profile = RwSignal::new(None::<AgentProfileDto>);
    let loading = RwSignal::new(false);
    let load_error = RwSignal::new(false);
    let generation = StoredValue::new(0_u64);

    Effect::new(move |_| {
        let selected = agent_id.get();
        generation.update_value(|value| *value = value.wrapping_add(1));
        let request_generation = generation.get_value();
        profile.set(None);
        load_error.set(false);
        let Some(agent_id) = selected else {
            loading.set(false);
            return;
        };
        loading.set(true);
        #[cfg(target_arch = "wasm32")]
        leptos::task::spawn_local_scoped_with_cancellation(async move {
            let outcome = load_agent(&agent_id).await;
            if generation.get_value() != request_generation {
                return;
            }
            match outcome {
                Ok(agent) => profile.set(Some(agent)),
                Err(_) => load_error.set(true),
            }
            loading.set(false);
        });
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (agent_id, request_generation);
            loading.set(false);
        }
    });

    view! {
        <Show when=move || loading.get()>
            <div class="ob-loading" role="status">{move || t!(i18n, common.loading)}</div>
        </Show>
        <Show when=move || load_error.get()>
            <p class="ob-agent-profile-error" role="alert">
                {move || t!(i18n, agents.detail_load_error)}
            </p>
        </Show>
        {move || profile.get().map(|profile| {
            let avatar_seed = profile.avatar_seed.clone();
            let avatar_name = profile.name.clone();
            let name = profile.name;
            let title = profile.title;
            let role_description = profile.role_description;
            let visibility = profile.visibility;
            let system_owned = profile.system_owned;
            view! {
                <article class="ob-agent-profile">
                    <header class="ob-agent-profile-header">
                        <span class="ob-agent-profile-avatar" aria-hidden="true">
                            <Avatar
                                principal_id=avatar_seed
                                name=avatar_name
                                size=AvatarSize::Large
                            />
                        </span>
                        <div class="ob-agent-profile-identity">
                            <h3>{name}</h3>
                            <p>{title}</p>
                        </div>
                        <div class="ob-agent-profile-badges">
                            <Badge>
                                {move || match visibility {
                                    AgentVisibility::Public => {
                                        t_string!(i18n, agents.visibility_public).to_owned()
                                    }
                                    AgentVisibility::Private => {
                                        t_string!(i18n, agents.visibility_private).to_owned()
                                    }
                                }}
                            </Badge>
                            <Show when=move || system_owned>
                                <Badge>{move || t!(i18n, agents.system_owned)}</Badge>
                            </Show>
                        </div>
                    </header>
                    <section class="ob-agent-profile-role" aria-labelledby="agent-profile-role-title">
                        <h4 id="agent-profile-role-title">
                            {move || t!(i18n, agents.role_label)}
                        </h4>
                        <p>{role_description}</p>
                    </section>
                </article>
            }
        })}
    }
}
