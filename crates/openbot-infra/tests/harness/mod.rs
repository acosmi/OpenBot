//! 需要真实 PostgreSQL 的集成测试共用的临时库 harness。
//!
//! 抽成一份而不是每个测试文件各抄一份：两份 harness 会各自漂移，而漂移的那一刻
//! 两边的测试仍然是绿的 —— 那正是本仓点名过的「测抄件」反模式。
//!
//! 每个用例自己 `CREATE DATABASE` 一个带随机后缀的临时库，跑完
//! `DROP DATABASE ... WITH (FORCE)` 删掉。用例**从不**改动环境变量指向的那个库，
//! 也从不碰参照库 `openbot_ref_0012`。

use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use openbot_infra::db::pool::{self, DatabaseConfig};

/// 管理连接的 libpq keyword 串（`host=... port=... user=... password=... dbname=...`）。
pub(crate) const ENV_KEY: &str = "OPENBOT_TEST_DATABASE_URL";

pub(crate) static SEQUENCE: AtomicU32 = AtomicU32::new(0);

/// 从环境变量取管理连接配置；没设就**硬失败**。
///
/// 为什么不是"打印一行 SKIP 然后放行"：`#[ignore]` 已经在默认路径上给了可见的跳过
/// （`cargo test` 会逐条打印 `ignored, 需要真实 PostgreSQL：…`）。加 `--include-ignored`
/// 就是调用方在明说"我要跑真库测试"，这时候静默通过会让"我跑了集成测试且全绿"变成一句假话 ——
/// CLAUDE.md 反复点名过：跳过自己却报 ok 的用例不是闸门。
pub(crate) fn admin_config(test_name: &str) -> DatabaseConfig {
    let Ok(url) = std::env::var(ENV_KEY) else {
        panic!(
            "{test_name} 需要真实 PostgreSQL，但环境变量 {ENV_KEY} 未设置。\
             默认 `cargo test` 会按 #[ignore] 跳过本用例并打印理由；\
             调用方显式传了 --include-ignored，所以这里不静默放行。\
             设法：OPENBOT_TEST_DATABASE_URL=\"host=… port=… user=… password=… dbname=postgres\""
        );
    };
    let parsed = tokio_postgres::Config::from_str(&url)
        .unwrap_or_else(|e| panic!("{ENV_KEY} 不是合法的 libpq 连接串：{e}"));
    let host = match parsed.get_hosts().first() {
        Some(tokio_postgres::config::Host::Tcp(host)) => host.clone(),
        other => panic!("{ENV_KEY} 必须给一个 TCP host，实际是 {other:?}"),
    };
    let port = parsed.get_ports().first().copied().unwrap_or(5432);
    let user = parsed
        .get_user()
        .unwrap_or_else(|| panic!("{ENV_KEY} 缺少 user"));
    let dbname = parsed
        .get_dbname()
        .unwrap_or_else(|| panic!("{ENV_KEY} 缺少 dbname"));
    let mut config = DatabaseConfig::new(host, port, user, dbname)
        .with_application_name("openbot-infra-schema-parity-test")
        .with_max_pool_size(2);
    if let Some(password) = parsed.get_password() {
        let password = std::str::from_utf8(password).expect("口令必须是 UTF-8");
        config = config.with_password(password);
    }
    config
}

/// 带随机后缀的临时库名。进程号 + 纳秒 + 进程内序号，三者一起避免并行跑时撞名。
pub(crate) fn unique_database_name(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时钟应当晚于 UNIX 纪元")
        .as_nanos();
    let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("openbot_it_{tag}_{}_{nanos}_{seq}", std::process::id())
}

/// 用管理连接跑一条 utility 语句。
///
/// `CREATE DATABASE` / `DROP DATABASE` 不能在事务块里跑，所以走简单查询协议的单条语句。
pub(crate) async fn run_utility(admin: &DatabaseConfig, sql: &str) -> Result<(), String> {
    let pool = pool::connect(admin)
        .await
        .map_err(|e| format!("连接管理库失败：{e}"))?;
    let outcome = async {
        let client = pool
            .get()
            .await
            .map_err(|e| format!("取管理连接失败：{e}"))?;
        client
            .simple_query(sql)
            .await
            .map(|_| ())
            .map_err(|e| format!("执行 `{sql}` 失败：{e}"))
    }
    .await;
    pool.close();
    outcome
}

/// 建临时库 → 跑 `body` → 无论成败都删临时库 → 再把 `body` 的结果抛出来。
///
/// `body` 只返回 `Result`、绝不 panic：panic 会跳过下面的删库，把临时库留在开发机上。
pub(crate) async fn with_temp_database<F, Fut>(admin: &DatabaseConfig, tag: &str, body: F)
where
    F: FnOnce(DatabaseConfig) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let name = unique_database_name(tag);
    run_utility(admin, &format!("CREATE DATABASE \"{name}\""))
        .await
        .unwrap_or_else(|e| panic!("建临时库 {name} 失败：{e}"));

    let outcome = body(admin.clone().with_dbname(&name)).await;

    // `WITH (FORCE)`（PostgreSQL 13+）踢掉可能还没完全关闭的连接：连接池关闭是异步的，
    // 不加这个偶尔会撞上 "database is being accessed by other users"。
    let dropped = run_utility(admin, &format!("DROP DATABASE \"{name}\" WITH (FORCE)")).await;

    if let Err(message) = outcome {
        panic!("{message}");
    }
    dropped.unwrap_or_else(|e| panic!("删临时库 {name} 失败：{e}"));
}
