//! `public.runs` typed row（native 0016 + expand-only 0024 usage suffix）。

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
    next_tool_call_seq: Option<i64> = ("next_tool_call_seq", "bigint", false),
    terminal_event_seq: Option<i64> = ("terminal_event_seq", "bigint", false),
    error_code: Option<String> = ("error_code", "text", false),
    created_at: time::OffsetDateTime = ("created_at", "timestamp with time zone", true),
    started_at: Option<time::OffsetDateTime> = ("started_at", "timestamp with time zone", false),
    finished_at: Option<time::OffsetDateTime> = ("finished_at", "timestamp with time zone", false),
    budget_max_output_tokens: Option<i64> = ("budget_max_output_tokens", "bigint", false),
    usage_input_tokens: i64 = ("usage_input_tokens", "bigint", true),
    usage_output_tokens: i64 = ("usage_output_tokens", "bigint", true),
    usage_total_tokens: i64 = ("usage_total_tokens", "bigint", true),
    usage_next_sampling: i32 = ("usage_next_sampling", "integer", true),
    usage_last_sampling: Option<i32> = ("usage_last_sampling", "integer", false),
    usage_last_input_tokens: Option<i64> = ("usage_last_input_tokens", "bigint", false),
    usage_last_output_tokens: Option<i64> = ("usage_last_output_tokens", "bigint", false),
    usage_last_total_tokens: Option<i64> = ("usage_last_total_tokens", "bigint", false),
}
