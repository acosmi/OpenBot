//! Agent 三张表的类型化 PostgreSQL repositories。

use crate::repo::common::define_table_repo;

define_table_repo!(
    /// `agents` repository。
    AgentRepo,
    table = agents,
    order_by = "\"id\"",
    find = find_by_id(id: &str) where "\"id\" = $1"
);

define_table_repo!(
    /// `agent_profiles` repository。
    AgentProfileRepo,
    table = agent_profiles,
    order_by = "\"agent_id\"",
    find = find_by_agent_id(agent_id: &str) where "\"agent_id\" = $1"
);

define_table_repo!(
    /// `agent_preferences` repository。
    AgentPreferenceRepo,
    table = agent_preferences,
    order_by = "\"user_id\", \"agent_id\"",
    find = find_by_key(user_id: &str, agent_id: &str) where "\"user_id\" = $1 AND \"agent_id\" = $2"
);
