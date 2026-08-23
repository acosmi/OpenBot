//! OpenBot Server 生产二进制组装入口。

use std::error::Error;
use std::sync::Arc;

use hmac::{Hmac, Mac};
use openbot_application::{ApplicationService, OpenBotApplication};
use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};
use openbot_domain::identity::session::TrustedOrigins;
use openbot_infra::auth::config::{
    BindingExposure, ExampleKeyPolicy, KeyEncryptionKey, SingleUserAdmission, auth_config,
    default_session_lifetime, single_user_binding_verdict, single_user_enabled,
};
use openbot_infra::db::compat::{DataMigrationVerdict, check_migration_boundary_on};
use openbot_infra::db::pool::DatabaseConfig;
use openbot_infra::db::{baseline, native, pool};
use openbot_infra::repo::ChannelRepo;
use openbot_infra::repo::people_admin::PostgresPeopleAdministration;
use openbot_server::config::{DeploymentEnvironment, ServerConfig, env_map_from_process};
use openbot_server::readiness::ReadinessVerdict;
use openbot_server::telemetry::{self, LogFormat};
use openbot_server::{
    AuthResolver, FnReadinessProbe, PostgresSessionAuthResolver, SensitiveWriteSecurity,
    ServerBuilder, SingleUserAuthResolver, install_recorder,
};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    telemetry::init(LogFormat::Json)?;
    let env = env_map_from_process();
    let server = ServerConfig::from_env_map(&env)?;
    if let Some(warning) = server.public_transport().startup_warning() {
        tracing::warn!(code = server.public_transport().as_str(), "{warning}");
    }

    let database_url = openbot_server::config::env::optional(&env, "DATABASE_URL")
        .ok_or_else(|| startup_error("database_url_missing"))?;
    let database: DatabaseConfig = database_url.parse()?;
    let pool = pool::connect(&database).await?;
    initialize_database(&pool).await?;

    let key_policy = match server.deployment_environment {
        DeploymentEnvironment::Production => ExampleKeyPolicy::Reject,
        DeploymentEnvironment::Development => ExampleKeyPolicy::Allow,
    };
    let key_encryption = KeyEncryptionKey::from_env_map(&env, key_policy)?;
    if let Some(code) = key_encryption.advisory_code() {
        tracing::warn!(code, "KEY_ENCRYPTION_KEY 建议轮换到 AES-256");
    }
    let raw_master = openbot_server::config::env::optional(&env, "KEY_ENCRYPTION_KEY")
        .ok_or_else(|| startup_error("key_encryption_key_missing"))?;
    let audit_key = derive_audit_key(raw_master.as_bytes());

    let public_url = server.public_url.as_ref().map(|url| url.as_str());
    let auth_config = auth_config(&env, public_url, default_session_lifetime())?;
    let has_provider = auth_config.is_some();
    let single_user = single_user_enabled(&env, has_provider)?;
    // 第一真源的缺省是“tenant package 的 id”，不是目录路径。W-4 尚未加载/校验包，
    // 因而此刻只能要求显式值；拿路径顶替会永久铸造错误的 deployment/thread 身份。
    let deployment = deployment_id_for_startup(server.deployment_id.as_deref())?;
    let tenant = TenantId::new("default");

    if single_user {
        provision_single_user(&pool).await?;
    }

    let (auth, sensitive, floor): (
        Arc<dyn AuthResolver>,
        SensitiveWriteSecurity,
        Option<openbot_domain::identity::roles::AdminFloor>,
    ) = if let Some(config) = auth_config {
        let resolver = PostgresSessionAuthResolver::new(
            pool.clone(),
            config.session_secret.expose().as_bytes(),
            config.session_lifetime,
            deployment.clone(),
            tenant.clone(),
        )?;
        let sensitive =
            SensitiveWriteSecurity::new(config.session_lifetime, config.trusted_origins.clone());
        (Arc::new(resolver), sensitive, Some(config.admin_floor))
    } else {
        if !single_user
            || single_user_binding_verdict(BindingExposure::Loopback) != SingleUserAdmission::Admit
        {
            return Err(startup_error("single_user_binding_refused").into());
        }
        let lifetime = default_session_lifetime();
        let origins = configured_origins(&env)?;
        (
            Arc::new(SingleUserAuthResolver::new(
                deployment.clone(),
                tenant.clone(),
                ActorId::new("single-user"),
                lifetime,
            )),
            SensitiveWriteSecurity::new(lifetime, origins),
            None,
        )
    };

    let people = PostgresPeopleAdministration::new(pool.clone(), floor, audit_key.to_vec())?;
    let application: Arc<dyn ApplicationService> =
        Arc::new(OpenBotApplication::new(ChannelRepo::new(pool.clone())).with_people(people));
    let metrics = install_recorder()?;
    let db_probe_pool = pool.clone();
    let db_probe = FnReadinessProbe::new("database_native_schema", move || {
        let pool = db_probe_pool.clone();
        async move {
            let Ok(client) = pool.get().await else {
                return ReadinessVerdict::NotReady;
            };
            match client
                .query_one(
                    "SELECT EXISTS(SELECT 1 FROM openbot_internal.schema_migrations \
                     WHERE version=$1)",
                    &[&native::NATIVE_LATEST_VERSION],
                )
                .await
                .and_then(|row| row.try_get::<_, bool>(0))
            {
                Ok(true) => ReadinessVerdict::Ready,
                Ok(false) | Err(_) => ReadinessVerdict::NotReady,
            }
        }
    });
    let mut builder = ServerBuilder::new(application, auth)
        .with_sensitive_write_security(sensitive)
        .with_metrics_handle(metrics)
        .with_insecure_transport(server.public_transport().insecure_transport())
        .with_readiness_probe(Arc::new(db_probe));
    if !single_user {
        builder = builder.with_readiness_probe(Arc::new(FnReadinessProbe::new(
            "computer_isolation",
            || async { ReadinessVerdict::NotReady },
        )));
    }

    let address = if single_user {
        format!("127.0.0.1:{}", server.port)
    } else {
        format!("0.0.0.0:{}", server.port)
    };
    let listener = tokio::net::TcpListener::bind(&address).await?;
    tracing::info!(bind = %address, single_user, "OpenBot Server 已启动");
    axum::serve(listener, builder.into_router())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    pool.close();
    Ok(())
}

async fn initialize_database(pool: &deadpool_postgres::Pool) -> Result<(), Box<dyn Error>> {
    let mut client = pool.get().await?;
    let table_count: i64 = client
        .query_one(
            "SELECT count(*)::bigint FROM information_schema.tables \
             WHERE table_schema='public' AND table_type='BASE TABLE'",
            &[],
        )
        .await?
        .try_get(0)?;
    if table_count == 0 {
        baseline::apply(&client).await?;
    } else {
        let report = check_migration_boundary_on(&client).await?;
        if matches!(
            report.data_migrations,
            DataMigrationVerdict::Unverifiable { .. }
        ) {
            return Err(startup_error("legacy_data_migration_unverifiable").into());
        }
    }
    native::apply(&mut client).await?;
    Ok(())
}

async fn provision_single_user(pool: &deadpool_postgres::Pool) -> Result<(), Box<dyn Error>> {
    let mut client = pool.get().await?;
    let transaction = client.transaction().await?;
    transaction
        .execute(
            "INSERT INTO public.users \
             (id,email,name,email_verified,groups,auth_generation) \
             VALUES('single-user','single-user@localhost','Local Owner',true,'{}',0) \
             ON CONFLICT(id) DO NOTHING",
            &[],
        )
        .await?;
    for role in ["admin", "user"] {
        transaction
            .execute(
                "INSERT INTO public.user_roles(user_id,role) \
                 VALUES('single-user',$1::text::role) ON CONFLICT DO NOTHING",
                &[&role],
            )
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}

fn configured_origins(
    env: &openbot_server::config::EnvMap,
) -> Result<TrustedOrigins, Box<dyn Error>> {
    let configured = openbot_server::config::env::comma_separated(env, "TRUSTED_ORIGINS");
    let entries = if configured.is_empty() {
        vec![openbot_infra::auth::config::DEFAULT_TRUSTED_ORIGIN.to_owned()]
    } else {
        configured
    };
    TrustedOrigins::from_configured(entries).map_err(Into::into)
}

fn derive_audit_key(master: &[u8]) -> [u8; 32] {
    let mut hmac = HmacSha256::new_from_slice(master).expect("HMAC 接受任意非空长度");
    hmac.update(b"openbot:audit-checkpoint:v1");
    hmac.finalize().into_bytes().into()
}

fn deployment_id_for_startup(value: Option<&str>) -> Result<DeploymentId, std::io::Error> {
    value
        .map(DeploymentId::new)
        .ok_or_else(|| startup_error("deployment_id_requires_tenant_package_loader"))
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn startup_error(code: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_key_is_domain_separated_and_deterministic() {
        let first = derive_audit_key(b"master-key");
        assert_eq!(first, derive_audit_key(b"master-key"));
        assert_ne!(first, derive_audit_key(b"other-key"));
        assert_ne!(first, [0; 32]);
    }

    #[test]
    fn deployment_id_is_never_guessed_from_a_package_directory() {
        assert_eq!(
            deployment_id_for_startup(Some("tenant-id")).unwrap(),
            DeploymentId::new("tenant-id"),
        );
        assert_eq!(
            deployment_id_for_startup(None).unwrap_err().to_string(),
            "deployment_id_requires_tenant_package_loader",
        );
    }
}
