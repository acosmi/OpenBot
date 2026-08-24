//! OpenBot Server 生产二进制组装入口。

use std::error::Error;
use std::sync::Arc;

use hmac::{Hmac, Mac};
use openbot_application::{ApplicationService, OpenBotApplication};
use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};
use openbot_domain::identity::session::TrustedOrigins;
use openbot_domain::vault::{KeyVersion, WrappingKey};
use openbot_infra::auth::config::{
    AuthConfig, BindingExposure, ExampleKeyPolicy, KeyEncryptionKey, SingleUserAdmission,
    auth_config_with_dynamic_provider, default_session_lifetime, single_user_binding_verdict,
    single_user_enabled,
};
use openbot_infra::auth::oidc::coordinator::{DEFAULT_IDP_TIMEOUT, DEFAULT_METADATA_MAX_BYTES};
use openbot_infra::auth::oidc::{
    FetchBudget, OidcLoginCoordinator, OidcProviderRuntime, PostgresLoginAttemptStore,
    PostgresOidcRateLimiter, PostgresOidcSessionIssuer, PreAuthSurface, ProviderRegistry,
    configured_oidc_providers,
};
use openbot_infra::auth::single_user::initialize_single_user;
use openbot_infra::auth::sso::DynamicSsoService;
use openbot_infra::db::pool::DatabaseConfig;
use openbot_infra::db::{native, pool};
use openbot_infra::net::safe_http::{EgressPolicy, SafeDialer};
use openbot_infra::repo::ChannelRepo;
use openbot_infra::repo::people_admin::PostgresPeopleAdministration;
use openbot_server::config::{DeploymentEnvironment, ServerConfig, env_map_from_process};
use openbot_server::readiness::ReadinessVerdict;
use openbot_server::telemetry::{self, LogFormat};
use openbot_server::{
    AuthResolver, FnReadinessProbe, PostgresSessionAuthResolver, SINGLE_USER_ACTOR_ID,
    SensitiveWriteSecurity, ServerBuilder, SingleUserAuthResolver, install_recorder,
};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;
type OidcLoginAssembly = (
    Arc<OidcLoginCoordinator>,
    Arc<DynamicSsoService>,
    PreAuthSurface,
    TrustedOrigins,
    bool,
);
type AuthAssembly = (
    Arc<dyn AuthResolver>,
    SensitiveWriteSecurity,
    Option<openbot_domain::identity::roles::AdminFloor>,
    Option<OidcLoginAssembly>,
);

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
    openbot_server::database::initialize(&pool).await?;

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

    let has_dynamic_provider = database_has_dynamic_provider(&pool).await?;
    let public_url = server.public_url.as_ref().map(|url| url.as_str());
    let auth_config = auth_config_with_dynamic_provider(
        &env,
        public_url,
        default_session_lifetime(),
        has_dynamic_provider,
    )?;
    let has_provider = auth_config.is_some();
    let single_user = single_user_enabled(&env, has_provider)?;
    // 第一真源的缺省是“tenant package 的 id”，不是目录路径。W-4 尚未加载/校验包，
    // 因而此刻只能要求显式值；拿路径顶替会永久铸造错误的 deployment/thread 身份。
    let deployment = deployment_id_for_startup(server.deployment_id.as_deref())?;
    let tenant = TenantId::new("default");

    initialize_single_user(&pool, single_user).await?;

    let (auth, sensitive, floor, oidc_login): AuthAssembly = if let Some(config) = auth_config {
        let oidc = build_oidc_login(
            &config,
            &pool,
            &tenant,
            audit_key.as_slice(),
            key_encryption.into_wrapping_key(),
            server.public_transport().cookie_secure(),
        )
        .await?;
        let resolver = PostgresSessionAuthResolver::new(
            pool.clone(),
            config.session_secret.expose().as_bytes(),
            config.session_lifetime,
            deployment.clone(),
            tenant.clone(),
        )?;
        let sensitive =
            SensitiveWriteSecurity::new(config.session_lifetime, config.trusted_origins.clone());
        (
            Arc::new(resolver),
            sensitive,
            Some(config.admin_floor),
            Some(oidc),
        )
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
                ActorId::new(SINGLE_USER_ACTOR_ID),
                lifetime,
            )),
            SensitiveWriteSecurity::new(lifetime, origins),
            None,
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
    if let Some((coordinator, dynamic_sso, preauth, origins, secure_cookie)) = oidc_login {
        builder = builder.with_login_security(origins, secure_cookie);
        builder = builder.with_oidc_login(coordinator, preauth);
        builder = builder.with_dynamic_sso(dynamic_sso);
    }
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
    axum::serve(
        listener,
        builder
            .into_router()
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    pool.close();
    Ok(())
}

async fn build_oidc_login(
    config: &AuthConfig,
    pool: &deadpool_postgres::Pool,
    tenant: &TenantId,
    audit_key: &[u8],
    wrapping_key: WrappingKey,
    secure_cookie: bool,
) -> Result<OidcLoginAssembly, Box<dyn Error>> {
    let configured = configured_oidc_providers(config)?;
    let registry =
        ProviderRegistry::build(configured.iter().map(|provider| provider.config.clone()))?;
    let preauth = PreAuthSurface::project(&registry);
    let environment_provider_ids: Vec<String> = configured
        .iter()
        .map(|provider| provider.config.id().as_str().to_owned())
        .collect();
    let dialer = SafeDialer::new(EgressPolicy::default());
    let budget = FetchBudget::new(DEFAULT_METADATA_MAX_BYTES, DEFAULT_IDP_TIMEOUT);
    let mut runtimes = Vec::with_capacity(configured.len());
    for provider in configured {
        runtimes.push(
            OidcProviderRuntime::discover(
                provider.config,
                Some(provider.client_secret),
                None,
                &dialer,
                budget,
            )
            .await?,
        );
    }
    let attempts = PostgresLoginAttemptStore::new(
        pool.clone(),
        config.session_secret.expose().as_bytes(),
        tenant,
        4096,
    )?;
    let sessions = PostgresOidcSessionIssuer::new(
        pool.clone(),
        config.session_secret.expose().as_bytes(),
        config.session_lifetime,
        config.admin_floor.clone(),
        audit_key,
    )?;
    let rate_limiter = PostgresOidcRateLimiter::new(
        pool.clone(),
        config.session_secret.expose().as_bytes(),
        tenant,
    )?;
    let coordinator =
        OidcLoginCoordinator::new(runtimes, attempts, sessions, rate_limiter, dialer.clone())?;
    let dynamic_sso = DynamicSsoService::new(
        pool.clone(),
        tenant,
        config.session_secret.expose().as_bytes(),
        config.session_secret.expose().as_bytes(),
        audit_key,
        wrapping_key,
        KeyVersion::new(1),
        config.session_lifetime,
        config.admin_floor.clone(),
        environment_provider_ids,
        dialer,
        config.public_url.clone(),
    )?;
    dynamic_sso
        .preflight_all(time::OffsetDateTime::now_utc())
        .await?;
    Ok((
        Arc::new(coordinator),
        Arc::new(dynamic_sso),
        preauth,
        config.trusted_origins.clone(),
        secure_cookie,
    ))
}

async fn database_has_dynamic_provider(
    pool: &deadpool_postgres::Pool,
) -> Result<bool, Box<dyn Error>> {
    let client = pool.get().await?;
    Ok(client
        .query_one("SELECT EXISTS(SELECT 1 FROM public.sso_providers)", &[])
        .await?
        .try_get(0)?)
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
