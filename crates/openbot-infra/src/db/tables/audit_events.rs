//! `public.audit_events` 的类型化行 —— 上游 server/src/db/schema/core.ts::auditEvents。
//!
//! 列名、列序（`pg_attribute.attnum`）、可空性与类型逐列对应上游 0012 终态，真源是
//! `fixtures/db/schema-0012.json`；`COLUMN_SPECS` 与真库的一致性由
//! `crate::db::compat::check_migration_boundary` 在活库上机械核对。
//!
//! 主键：PRIMARY KEY (id)。

crate::db::tables::define_table! {
    table = "audit_events";
    id: uuid::Uuid = ("id", "uuid", true),
    actor_user_id: Option<String> = ("actor_user_id", "text", false),
    event_type: String = ("event_type", "text", true),
    target_type: String = ("target_type", "text", true),
    target_id: Option<String> = ("target_id", "text", false),
    payload: serde_json::Value = ("payload", "jsonb", true),
    created_at: time::OffsetDateTime = ("created_at", "timestamp with time zone", true),
}
