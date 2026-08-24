//! `public.runs` typed row（native 0016）。

crate::db::tables::define_table! {
    table = "runs";
    run_id: String = ("run_id", "text", true),
    thread_id: String = ("thread_id", "text", true),
    bot_id: String = ("bot_id", "text", true),
    actor_id: String = ("actor_id", "text", true),
    foreground: bool = ("foreground", "boolean", true),
    status: String = ("status", "text", true),
    fencing_token: i64 = ("fencing_token", "bigint", true),
    next_event_seq: i64 = ("next_event_seq", "bigint", true),
    terminal_event_seq: Option<i64> = ("terminal_event_seq", "bigint", false),
    error_code: Option<String> = ("error_code", "text", false),
    created_at: time::OffsetDateTime = ("created_at", "timestamp with time zone", true),
    started_at: Option<time::OffsetDateTime> = ("started_at", "timestamp with time zone", false),
    finished_at: Option<time::OffsetDateTime> = ("finished_at", "timestamp with time zone", false),
}
