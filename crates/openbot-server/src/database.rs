//! Server 启动期的 fresh / legacy / Rust-managed 数据库分流。

use deadpool_postgres::Pool;

use openbot_infra::db::InfraError;
use openbot_infra::db::compat::{DataMigrationVerdict, check_migration_boundary_on};
use openbot_infra::db::{fresh, native};

/// 本次启动识别出的数据库来源。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseOrigin {
    /// public schema 为空，本次原子施加 Rust baseline + native migrations。
    Fresh,
    /// 已有且通过 checksum 校验的 Rust native ledger。
    RustManaged,
    /// 有完整 Drizzle 账本的上游 legacy 数据库，本次接上 native migrations。
    LegacyUpgraded,
}

/// 启动数据库失败；不带连接串、行值或 PostgreSQL 自由文本。
#[derive(Debug)]
pub enum DatabaseInitializationError {
    /// infra 的脱敏连接/schema/migration 错误。
    Infra(InfraError),
    /// 已有 public schema 却既无完整 Drizzle 账本、也无可信 native 账本。
    LegacyDataMigrationUnverifiable,
}

impl core::fmt::Display for DatabaseInitializationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Infra(error) => core::fmt::Display::fmt(error, formatter),
            Self::LegacyDataMigrationUnverifiable => {
                formatter.write_str("legacy_data_migration_unverifiable")
            }
        }
    }
}

impl std::error::Error for DatabaseInitializationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Infra(error) => Some(error),
            Self::LegacyDataMigrationUnverifiable => None,
        }
    }
}

impl From<InfraError> for DatabaseInitializationError {
    fn from(error: InfraError) -> Self {
        Self::Infra(error)
    }
}

/// 初始化或验证 Server 数据库，并施加当前 native migration。
///
/// Rust fresh 库以 native ledger 作为后续重启的来源证明；未知无账本 legacy 库继续拒绝，
/// 不能因为 schema 看起来相同就猜纯数据 migration 0003 已运行。
///
/// # Errors
///
/// 见 [`DatabaseInitializationError`]。
pub async fn initialize(pool: &Pool) -> Result<DatabaseOrigin, DatabaseInitializationError> {
    let mut client = pool
        .get()
        .await
        .map_err(|source| InfraError::connect("取数据库初始化连接", source))?;
    let public_tables: i64 = client
        .query_one(
            "SELECT count(*)::bigint FROM information_schema.tables \
             WHERE table_schema='public' AND table_type='BASE TABLE'",
            &[],
        )
        .await
        .map_err(|source| InfraError::query("检查 public schema 是否为空", source))?
        .try_get(0)
        .map_err(|source| {
            openbot_infra::db::RowDecodeError::column(
                "(information_schema.tables)",
                "count",
                source,
            )
        })
        .map_err(InfraError::from)?;

    if public_tables == 0 {
        match fresh::apply(&mut client).await? {
            fresh::FreshApplyOutcome::Applied(_) => return Ok(DatabaseOrigin::Fresh),
            fresh::FreshApplyOutcome::AlreadyInitialized => {}
        }
    }

    let rust_managed = native::ledger_exists(&client).await?;
    let report = check_migration_boundary_on(&client).await?;
    if matches!(
        report.data_migrations,
        DataMigrationVerdict::Unverifiable { .. }
    ) && !rust_managed
    {
        return Err(DatabaseInitializationError::LegacyDataMigrationUnverifiable);
    }

    native::apply(&mut client).await?;
    Ok(if rust_managed {
        DatabaseOrigin::RustManaged
    } else {
        DatabaseOrigin::LegacyUpgraded
    })
}
