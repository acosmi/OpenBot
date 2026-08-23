//! `public.users` 的类型化行 —— 上游 server/src/db/schema/core.ts::users。
//!
//! 列名、列序（`pg_attribute.attnum`）、可空性与类型逐列对应上游 0012 终态，真源是
//! `fixtures/db/schema-0012.json`；`COLUMN_SPECS` 与真库的一致性由
//! `crate::db::compat::check_migration_boundary` 在活库上机械核对。
//!
//! 主键：PRIMARY KEY (id)。
//! 唯一：UNIQUE (email)。

crate::db::tables::define_table! {
    table = "users";
    id: String = ("id", "text", true),
    email: String = ("email", "text", true),
    name: Option<String> = ("name", "text", false),
    image: Option<String> = ("image", "text", false),
    email_verified: bool = ("email_verified", "boolean", true),
    groups: Vec<Option<String>> = ("groups", "text[]", true),
    created_at: time::OffsetDateTime = ("created_at", "timestamp with time zone", true),
    updated_at: time::OffsetDateTime = ("updated_at", "timestamp with time zone", true),
}
