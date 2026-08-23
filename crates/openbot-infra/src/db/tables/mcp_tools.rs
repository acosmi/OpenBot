//! `public.mcp_tools` 的类型化行 —— 上游 server/src/db/schema/plugins.ts::mcpTools。
//!
//! 列名、列序（`pg_attribute.attnum`）、可空性与类型逐列对应上游 0012 终态，真源是
//! `fixtures/db/schema-0012.json`；`COLUMN_SPECS` 与真库的一致性由
//! `crate::db::compat::check_migration_boundary` 在活库上机械核对。
//!
//! 主键：PRIMARY KEY (server_id, name)。
//!
//! 外键：
//!
//! - FOREIGN KEY (server_id) REFERENCES mcp_servers(id) ON DELETE CASCADE

crate::db::tables::define_table! {
    table = "mcp_tools";
    server_id: String = ("server_id", "text", true),
    name: String = ("name", "text", true),
    description: String = ("description", "text", true),
    input_schema: serde_json::Value = ("input_schema", "jsonb", true),
    created_at: time::OffsetDateTime = ("created_at", "timestamp with time zone", true),
}
