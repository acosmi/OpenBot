//! `public.mcp_servers` 的类型化行 —— 上游 server/src/db/schema/plugins.ts::mcpServers。
//!
//! 列名、列序（`pg_attribute.attnum`）、可空性与类型逐列对应上游 0012 终态，真源是
//! `fixtures/db/schema-0012.json`；`COLUMN_SPECS` 与真库的一致性由
//! `crate::db::compat::check_migration_boundary` 在活库上机械核对。
//!
//! 主键：PRIMARY KEY (id)。
//!
//! 外键：
//!
//! - FOREIGN KEY (credential_id) REFERENCES credentials(id) ON DELETE RESTRICT

crate::db::tables::define_table! {
    table = "mcp_servers";
    id: String = ("id", "text", true),
    title: String = ("title", "text", true),
    vendor: String = ("vendor", "text", true),
    url: String = ("url", "text", true),
    provenance: String = ("provenance", "text", true),
    credential_id: Option<uuid::Uuid> = ("credential_id", "uuid", false),
    tools_refreshed_at: Option<time::OffsetDateTime> = ("tools_refreshed_at", "timestamp with time zone", false),
    last_error: Option<String> = ("last_error", "text", false),
    added_by: Option<String> = ("added_by", "text", false),
    created_at: time::OffsetDateTime = ("created_at", "timestamp with time zone", true),
    updated_at: time::OffsetDateTime = ("updated_at", "timestamp with time zone", true),
}
