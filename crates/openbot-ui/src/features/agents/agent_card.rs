//! Compact roster card for one server-authorized Agent projection.

use core::fmt::Write as _;

use leptos::prelude::*;
use openbot_contracts::agent::AgentProfile;
use sha2::{Digest, Sha256};

use crate::api::agent_profile_href;
use crate::primitives::{Avatar, AvatarSize};

/// Render one navigable coworker card using only fields already authorized by the Server.
#[component]
pub fn AgentCard(
    /// Server-authorized, secret-free projection.
    agent: AgentProfile,
) -> impl IntoView {
    let href = agent_profile_href(agent.id.as_str()).expect("server Agent id must be route-safe");
    let dom_id = card_dom_id(agent.id.as_str());
    let avatar_seed = agent.avatar_seed.clone();
    let avatar_name = agent.name.clone();
    let name = agent.name;
    let role_description = agent.role_description;
    view! {
        <a id=dom_id class="ob-agent-card" href=href>
            <span class="ob-agent-card-avatar" aria-hidden="true">
                <Avatar
                    principal_id=avatar_seed
                    name=avatar_name
                    size=AvatarSize::Large
                />
            </span>
            <span class="ob-agent-card-copy">
                <span class="ob-agent-card-name">{name}</span>
                <span class="ob-agent-card-role">{role_description}</span>
            </span>
        </a>
    }
}

fn card_dom_id(agent_id: &str) -> String {
    let mut id = String::from("agent-card-");
    for byte in Sha256::digest(agent_id.as_bytes()) {
        write!(&mut id, "{byte:02x}").expect("writing to String cannot fail");
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_focus_id_is_stable_bounded_and_selector_safe() {
        let first = card_dom_id("agent/one?x=1");
        assert_eq!(first, card_dom_id("agent/one?x=1"));
        assert_ne!(first, card_dom_id("agent-two"));
        assert!(first.len() <= 128);
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        );
    }
}
