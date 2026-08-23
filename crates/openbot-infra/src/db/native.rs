//! 本项目自有的 native schema 增量施加器。
//!
//! 上游的 13 条 Drizzle migration 与本项目的增量有两条不同的职责边界：
//!
//! - [`crate::db::baseline`] 把空库一次建成上游 0012 终态；
//! - [`crate::db::compat`] 只读 `drizzle.__drizzle_migrations`，确认旧库至少到 0012；
//! - 本模块从 0012 往后施加 Rust-owned 的 expand-only migration，**绝不写上游账本**。
//!
//! # 自有账本与并发
//!
//! 自有账本是 [`NATIVE_LEDGER_TABLE`]。每条记录绑定 version、名字与 migration SQL 的
//! SHA-256；同版本字节漂移会 fail-closed，不会因为对象“看起来已存在”就跳过。施加全过程在
//! 一个事务里，并先取固定的 transaction-scoped advisory lock，因此多个 replica 同时启动时
//! 恰好一个施加，随后者读到账本后返回 [`ApplyOutcome::AlreadyApplied`]。
//!
//! 真实 migration SQL 刻意没有 `IF NOT EXISTS`：账本缺失而对象已存在是 drift，必须让 DDL
//! 报错并整体回滚。只有账本自身的 bootstrap 使用 `IF NOT EXISTS`，且随后立刻按列读它；一个
//! 同名异形表不会被当作正常账本。
//!
//! # Fresh install 与 upgrade 是同一条终态
//!
//! Fresh install 调用顺序固定为 `baseline::apply` → [`apply`]；既有库先过 compat，再调用
//! [`apply`]。这样 0012 fixture 始终只表达固定上游 oracle，post-0013 另有自己的 fixture，
//! 两份事实不会互相污染。

use openbot_domain::audit::hash::Sha256Digest;
use tokio_postgres::Client;

use crate::db::{InfraError, RowDecodeError};

/// 自有 migration 账本。它不在 `public`，也不是上游 Drizzle 的账本。
pub const NATIVE_LEDGER_TABLE: &str = "openbot_internal.schema_migrations";

/// 第一条 Rust-owned 增量的版本号。
pub const NATIVE_0013_VERSION: i32 = 13;

/// 第一条 Rust-owned 增量的稳定名字。
pub const NATIVE_0013_NAME: &str = "native_0013_audit_tool_pipeline";

/// 第一条 Rust-owned 增量的 SQL 原文。
pub const NATIVE_0013_SQL: &str = include_str!("../../sql/native_0013.sql");

/// 全部署共用的 migration advisory lock key（ASCII `OPENBOT1`）。
const MIGRATION_LOCK_KEY: i64 = 0x4f50_454e_424f_5431;

const LEDGER_ROW_LABEL: &str = "(openbot_internal.schema_migrations)";

const LEDGER_BOOTSTRAP_SQL: &str = r#"
CREATE SCHEMA IF NOT EXISTS openbot_internal;
CREATE TABLE IF NOT EXISTS openbot_internal.schema_migrations (
    version integer PRIMARY KEY,
    name text NOT NULL,
    checksum text NOT NULL,
    applied_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT schema_migrations_checksum_lower_hex
        CHECK (checksum ~ '^[0-9a-f]{64}$')
);
"#;

/// 一次施加的可观察结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// 本次事务施加并记账。
    Applied,
    /// 同字节的 migration 已经记账，本次零 DDL。
    AlreadyApplied,
}

/// 自有 migration 账本的构造性违例。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NativeMigrationViolation {
    /// 同版本已经存在，但名字或 SQL 摘要不同。
    #[error(
        "native migration {version} 账本漂移：期望 name={expected_name} checksum={expected_checksum}，实际 name={actual_name} checksum={actual_checksum}"
    )]
    LedgerDrift {
        /// 版本号。
        version: i32,
        /// 当前二进制钉死的名字。
        expected_name: &'static str,
        /// 当前二进制钉死的 SQL 摘要。
        expected_checksum: String,
        /// 数据库账本里的名字。
        actual_name: String,
        /// 数据库账本里的摘要。
        actual_checksum: String,
    },
    /// 账本已经有更晚版本，却缺当前版本；不能倒序补写。
    #[error(
        "native migration 账本有版本 {future_version}，却缺前置版本 {missing_version}；拒绝倒序施加"
    )]
    MissingBeforeFuture {
        /// 缺失的当前版本。
        missing_version: i32,
        /// 已经存在的更晚版本。
        future_version: i32,
    },
}

/// 当前 0013 SQL 的小写 SHA-256。
#[must_use]
pub fn native_0013_checksum() -> String {
    Sha256Digest::of(NATIVE_0013_SQL.as_bytes()).to_hex()
}

/// 在一个已到 0012 的数据库上施加 Rust-owned 0013。
///
/// # Errors
///
/// - 连接/DDL/账本查询失败返回脱敏的 [`InfraError::Query`]；
/// - 同版本账本漂移或出现版本空洞返回 [`InfraError::NativeMigration`]；
/// - commit 失败同样返回查询错误，事务由 PostgreSQL 回滚。
pub async fn apply(client: &mut Client) -> Result<ApplyOutcome, InfraError> {
    let transaction = client
        .transaction()
        .await
        .map_err(|source| InfraError::query("开始 native schema migration 事务", source))?;

    transaction
        .query_one("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK_KEY])
        .await
        .map_err(|source| InfraError::query("获取 native schema migration 锁", source))?;

    transaction
        .batch_execute(LEDGER_BOOTSTRAP_SQL)
        .await
        .map_err(|source| InfraError::query("初始化 native schema migration 账本", source))?;

    let checksum = native_0013_checksum();
    let existing = transaction
        .query_opt(
            "SELECT name, checksum FROM openbot_internal.schema_migrations WHERE version = $1",
            &[&NATIVE_0013_VERSION],
        )
        .await
        .map_err(|source| InfraError::query("读取 native schema migration 账本", source))?;

    if let Some(row) = existing {
        let actual_name: String = row
            .try_get("name")
            .map_err(|source| RowDecodeError::column(LEDGER_ROW_LABEL, "name", source))?;
        let actual_checksum: String = row
            .try_get("checksum")
            .map_err(|source| RowDecodeError::column(LEDGER_ROW_LABEL, "checksum", source))?;
        if actual_name != NATIVE_0013_NAME || actual_checksum != checksum {
            return Err(NativeMigrationViolation::LedgerDrift {
                version: NATIVE_0013_VERSION,
                expected_name: NATIVE_0013_NAME,
                expected_checksum: checksum,
                actual_name,
                actual_checksum,
            }
            .into());
        }
        transaction
            .commit()
            .await
            .map_err(|source| InfraError::query("提交 native schema migration 只读事务", source))?;
        return Ok(ApplyOutcome::AlreadyApplied);
    }

    let future = transaction
        .query_opt(
            "SELECT version FROM openbot_internal.schema_migrations \
             WHERE version > $1 ORDER BY version LIMIT 1",
            &[&NATIVE_0013_VERSION],
        )
        .await
        .map_err(|source| InfraError::query("检查 native schema migration 版本空洞", source))?;
    if let Some(row) = future {
        let future_version: i32 = row
            .try_get("version")
            .map_err(|source| RowDecodeError::column(LEDGER_ROW_LABEL, "version", source))?;
        return Err(NativeMigrationViolation::MissingBeforeFuture {
            missing_version: NATIVE_0013_VERSION,
            future_version,
        }
        .into());
    }

    transaction
        .batch_execute(NATIVE_0013_SQL)
        .await
        .map_err(|source| InfraError::query("应用 native_0013.sql", source))?;
    transaction
        .execute(
            "INSERT INTO openbot_internal.schema_migrations (version, name, checksum) \
             VALUES ($1, $2, $3)",
            &[&NATIVE_0013_VERSION, &NATIVE_0013_NAME, &checksum],
        )
        .await
        .map_err(|source| InfraError::query("记录 native_0013.sql 账本", source))?;
    transaction
        .commit()
        .await
        .map_err(|source| InfraError::query("提交 native_0013.sql", source))?;
    Ok(ApplyOutcome::Applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statement_lines() -> impl Iterator<Item = &'static str> {
        NATIVE_0013_SQL
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("--"))
    }

    #[test]
    fn migration_sql_is_mechanically_expand_only() {
        let forbidden_prefixes = ["DROP ", "TRUNCATE ", "DELETE ", "UPDATE "];
        for line in statement_lines() {
            let uppercase = line.to_ascii_uppercase();
            assert!(
                !forbidden_prefixes
                    .iter()
                    .any(|prefix| uppercase.starts_with(prefix)),
                "0013 出现 destructive 语句：{line}",
            );
            for forbidden in [" RENAME ", "ALTER COLUMN", "SET NOT NULL"] {
                assert!(
                    !uppercase.contains(forbidden),
                    "0013 出现兼容期禁令 `{forbidden}`：{line}",
                );
            }
        }

        assert_eq!(NATIVE_0013_SQL.matches("CREATE TABLE public.").count(), 3);
        assert_eq!(
            NATIVE_0013_SQL
                .matches("ALTER TABLE public.audit_events\n    ADD COLUMN")
                .count(),
            2,
        );
        assert!(NATIVE_0013_SQL.contains("ADD COLUMN prev_hash text"));
        assert!(NATIVE_0013_SQL.contains("ADD COLUMN row_hash text"));
    }

    #[test]
    fn real_migration_uses_the_ledger_not_object_existence_as_idempotency() {
        assert!(!statement_lines().any(|line| line.contains("IF NOT EXISTS")));
        assert!(LEDGER_BOOTSTRAP_SQL.contains("IF NOT EXISTS"));
        assert!(!LEDGER_BOOTSTRAP_SQL.contains("drizzle"));
        assert_eq!(NATIVE_LEDGER_TABLE, "openbot_internal.schema_migrations");
    }

    #[test]
    fn checksum_is_lowercase_sha256_and_changes_with_the_sql() {
        let checksum = native_0013_checksum();
        assert_eq!(checksum.len(), 64);
        assert!(
            checksum
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        );
        assert_ne!(checksum, Sha256Digest::of(b"different migration").to_hex());
    }
}
