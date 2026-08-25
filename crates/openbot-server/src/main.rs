//! OpenBot Server 生产二进制组装入口。

use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, Mac};
use openbot_agent::{BuiltInAgentConfig, BuiltInAgentRuntime};
use openbot_application::tenant::package::{
    TenantAgentType, TenantPackageAudienceContext, TenantPackageEnvironment,
    synchronize_tenant_package,
};
use openbot_application::{
    ApplicationService, NoRunDispatchConsumer, OpenBotApplication, RunDispatchConsumer, RunRuntime,
};
use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};
use openbot_domain::identity::groups::IdentityProviderId;
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
use openbot_infra::memory_admin::PostgresMemoryAdministration;
use openbot_infra::net::safe_http::{
    CidrAllowlist, EgressPolicy, SafeDialer, SafeHttpBudget, SchemePolicy,
};
use openbot_infra::policy::{PolicyListener, PolicyStore};
use openbot_infra::provider::context::PostgresAgentContextSource;
use openbot_infra::provider::credential::PostgresOpenAiCredentialSource;
use openbot_infra::provider::openai::{
    OpenAiApiKey, OpenAiProtocol, OpenAiProvider, OpenAiProviderConfig,
};
use openbot_infra::repo::ChannelRepo;
use openbot_infra::repo::audit::PostgresAuditReader;
use openbot_infra::repo::people_admin::PostgresPeopleAdministration;
use openbot_infra::run_runtime::{DEFAULT_DISPATCH_CLAIM_DURATION, PostgresRunRuntime, RunRelay};
use openbot_infra::store::plugin_user_credential::PostgresOwnedCredentialRetirer;
use openbot_infra::tenant::{PostgresTenantPackageSynchronizer, load_tenant_package};
use openbot_infra::thread_directory::{DEFAULT_THREAD_LEASE_DURATION, PostgresThreadDirectory};
use openbot_infra::thread_id::mint_thread_id;
use openbot_infra::vault::CredentialRecordVault;
use openbot_server::config::{
    AgentBudgets, DEFAULT_TENANT_PACKAGE_DIR, DeploymentEnvironment, PackageOpenAiProviderConfig,
    ServerConfig, env_map_from_process,
};
use openbot_server::readiness::ReadinessVerdict;
use openbot_server::telemetry::{self, LogFormat};
use openbot_server::{
    AuthResolver, FnReadinessProbe, PostgresSessionAuthResolver, SINGLE_USER_ACTOR_ID,
    SensitiveWriteSecurity, ServerBuilder, SingleUserAuthResolver, install_recorder,
};
use sha2::Sha256;
use url::Url;

type HmacSha256 = Hmac<Sha256>;
const PACKAGE_OPENAI_PROTOCOL: OpenAiProtocol = OpenAiProtocol::Responses;
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
type AgentAssembly = (Arc<dyn RunDispatchConsumer>, Option<BuiltInAgentRuntime>);

struct PackageAgentAssemblyInput {
    pool: deadpool_postgres::Pool,
    deployment: DeploymentId,
    tenant: TenantId,
    runtime: Arc<dyn RunRuntime>,
    required: bool,
    model: String,
    credential_key_id: String,
    provider: PackageOpenAiProviderConfig,
    credential_vault: CredentialRecordVault,
    budgets: AgentBudgets,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    telemetry::init(LogFormat::Json)?;
    let env = env_map_from_process();
    let server = ServerConfig::from_env_map(&env)?;
    if let Some(warning) = server.public_transport().startup_warning() {
        tracing::warn!(code = server.public_transport().as_str(), "{warning}");
    }
    let package_environment =
        TenantPackageEnvironment::from_allowlist(&env, &["MANAGED_AGENT_AG_UI_URL"]);
    let package_directory = tenant_package_directory_for_startup(&server.tenant_package_directory);
    let tenant_package = load_tenant_package(&package_directory, &package_environment)?;
    let requires_built_in_agent = tenant_package
        .package
        .agents
        .iter()
        .any(|agent| agent.agent_type == TenantAgentType::BuiltIn);

    let database_url = openbot_server::config::env::optional(&env, "DATABASE_URL")
        .ok_or_else(|| startup_error("database_url_missing"))?;
    let database: DatabaseConfig = database_url.parse()?;
    let pool = pool::connect(&database).await?;
    openbot_server::database::initialize(&pool).await?;

    let policy_store = PolicyStore::postgres(
        pool.clone(),
        server
            .computer
            .as_ref()
            .and_then(|computer| computer.action_policy.clone()),
    );
    let policy_origin = policy_store.load().await?;
    let compiled_policy = policy_store.compiled();
    tracing::info!(
        origin = ?policy_origin,
        mode = %compiled_policy.mode(),
        version = %compiled_policy.version(),
        configured = compiled_policy.is_configured(),
        "action policy 已从权威来源加载"
    );
    let policy_listener = PolicyListener::start(
        database
            .clone()
            .with_application_name("openbot-action-policy-listener"),
        Arc::new(policy_store.clone()),
    )
    .await?;

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
    let environment_provider_ids = auth_config
        .as_ref()
        .map(|config| {
            config
                .configured_providers()
                .into_iter()
                .map(|provider| IdentityProviderId::new(provider.as_str()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let deployment = deployment_id_for_startup(
        server.deployment_id.as_deref(),
        &tenant_package.package.tenant_id,
    );
    let tenant = TenantId::new(&tenant_package.package.tenant_id);
    let model_credential_vault = CredentialRecordVault::single_key(
        tenant.clone(),
        KeyVersion::new(1),
        KeyEncryptionKey::from_env_map(&env, key_policy)?.into_wrapping_key(),
    );

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

    let audience_context = if single_user {
        TenantPackageAudienceContext::single_user(ActorId::new(SINGLE_USER_ACTOR_ID))?
    } else {
        let mut providers = environment_provider_ids;
        let mut mappings = Vec::new();
        if let Some((_, dynamic_sso, _, _, _)) = &oidc_login {
            let (dynamic_providers, dynamic_mappings) = dynamic_sso.group_audience_inputs().await?;
            providers.extend(dynamic_providers);
            mappings.extend(dynamic_mappings);
        }
        TenantPackageAudienceContext::multi_user(providers, mappings)?
    };
    let package_report = synchronize_tenant_package(
        &PostgresTenantPackageSynchronizer::new(pool.clone()),
        &tenant_package,
        &audience_context,
    )
    .await?;
    tracing::info!(
        tenant = %package_report.tenant_id,
        agents = package_report.agents,
        channels = package_report.channels,
        memberships_granted = package_report.memberships_granted,
        memberships_revoked = package_report.memberships_revoked,
        generations_advanced = package_report.generations_advanced,
        single_user_groups_ignored = package_report.single_user_groups_ignored,
        knowledge_sources_compatibility_only = package_report.knowledge_sources_compatibility_only,
        runtime_theme_ignored = package_report.runtime_theme_ignored,
        "tenant package 已经由 Application use case 同步"
    );

    let owned_credentials = Arc::new(PostgresOwnedCredentialRetirer::new(
        pool.clone(),
        audit_key.to_vec(),
    )?);
    let people = PostgresPeopleAdministration::new(pool.clone(), floor, audit_key.to_vec())?
        .with_owned_credential_retirer(owned_credentials);
    let thread_runtime_owner = format!("runtime:{}", mint_thread_id(&deployment)?);
    let run_runtime: Arc<dyn RunRuntime> = Arc::new(PostgresRunRuntime::new(
        pool.clone(),
        thread_runtime_owner.clone(),
        DEFAULT_THREAD_LEASE_DURATION,
        DEFAULT_DISPATCH_CLAIM_DURATION,
    )?);
    let (run_consumer, built_in_agent) = build_package_agent(PackageAgentAssemblyInput {
        pool: pool.clone(),
        deployment: deployment.clone(),
        tenant: tenant.clone(),
        runtime: run_runtime.clone(),
        required: requires_built_in_agent,
        model: tenant_package.package.model.default_model.clone(),
        credential_key_id: tenant_package.package.model.credential_secret_ref.clone(),
        provider: server.package_openai_provider.clone(),
        credential_vault: model_credential_vault,
        budgets: server.agent_budgets,
    })?;
    let built_in_agent_ready = built_in_agent.is_some() || !requires_built_in_agent;
    let run_relay = RunRelay::start(run_runtime, run_consumer);
    let thread_directory = PostgresThreadDirectory::with_runtime(
        pool.clone(),
        database
            .clone()
            .with_application_name("openbot-thread-events"),
        thread_runtime_owner,
        DEFAULT_THREAD_LEASE_DURATION,
    )?;
    let application: Arc<dyn ApplicationService> = Arc::new(
        OpenBotApplication::new(ChannelRepo::new(pool.clone()))
            .with_people(people)
            .with_audit(PostgresAuditReader::new(pool.clone()))
            .with_policy(policy_store)
            .with_threads(thread_directory)
            .with_memory(PostgresMemoryAdministration::new(pool.clone())),
    );
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
    if !built_in_agent_ready {
        builder = builder.with_readiness_probe(Arc::new(FnReadinessProbe::new(
            "built_in_agent_provider",
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
    let serve_result = axum::serve(
        listener,
        builder
            .into_router()
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await;
    run_relay.stop().await;
    if let Some(agent) = built_in_agent {
        agent.stop().await;
    }
    policy_listener.stop().await;
    serve_result?;
    pool.close();
    Ok(())
}

fn build_package_agent(input: PackageAgentAssemblyInput) -> Result<AgentAssembly, Box<dyn Error>> {
    let PackageAgentAssemblyInput {
        pool,
        deployment,
        tenant,
        runtime,
        required,
        model,
        credential_key_id,
        provider,
        credential_vault,
        budgets,
    } = input;
    if !required {
        return Ok((Arc::new(NoRunDispatchConsumer), None));
    }
    let PackageOpenAiProviderConfig {
        base_url,
        environment_api_key,
        egress_allow_cidrs,
        allow_http,
    } = provider;
    // Fixed upstream @ai-sdk/openai 3.0.99 `createLanguageModel` delegates to Responses.
    let protocol = PACKAGE_OPENAI_PROTOCOL;
    let endpoint = openai_endpoint(base_url.as_str(), protocol)?;
    let environment_fallback = environment_api_key
        .map(|key| OpenAiApiKey::from_bytes(key.expose().as_bytes().to_vec()))
        .transpose()?;
    let credentials = Arc::new(PostgresOpenAiCredentialSource::new(
        pool.clone(),
        credential_vault,
        credential_key_id,
        environment_fallback,
    )?);
    let provider = Arc::new(OpenAiProvider::new_with_credential_source(
        OpenAiProviderConfig::new_with_transport_policy(
            endpoint,
            model,
            protocol,
            SafeHttpBudget::new(64 * 1024 * 1024, Duration::from_secs(30))?,
            budgets.stall_timeout,
            if allow_http {
                SchemePolicy::HttpOrHttps
            } else {
                SchemePolicy::HttpsOnly
            },
        )?,
        credentials,
        SafeDialer::new(EgressPolicy::new(CidrAllowlist::parse_exact(
            egress_allow_cidrs.iter().map(String::as_str),
        )?)),
    ));
    let context = Arc::new(PostgresAgentContextSource::new(
        pool, deployment, tenant, None,
    )?);
    let agent = BuiltInAgentRuntime::start(
        runtime,
        context,
        provider,
        BuiltInAgentConfig {
            run_deadline: budgets.run_deadline,
            ..BuiltInAgentConfig::default()
        },
    )
    .map_err(|_| startup_error("agent_runtime_config_invalid"))?;
    let consumer = agent.consumer();
    Ok((consumer, Some(agent)))
}

fn openai_endpoint(base: &str, protocol: OpenAiProtocol) -> Result<Url, std::io::Error> {
    let mut base = Url::parse(base).map_err(|_| startup_error("openai_base_url_invalid"))?;
    if !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err(startup_error("openai_base_url_invalid"));
    }
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.join(match protocol {
        OpenAiProtocol::Responses => "responses",
        OpenAiProtocol::ChatCompletions => "chat/completions",
    })
    .map_err(|_| startup_error("openai_base_url_invalid"))
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

fn deployment_id_for_startup(value: Option<&str>, package_tenant_id: &str) -> DeploymentId {
    DeploymentId::new(value.unwrap_or(package_tenant_id))
}

fn tenant_package_directory_for_startup(configured: &str) -> PathBuf {
    let direct = PathBuf::from(configured);
    if direct.is_dir() || configured != DEFAULT_TENANT_PACKAGE_DIR {
        return direct;
    }
    let workspace_default = PathBuf::from("examples/fintech");
    if workspace_default.is_dir() {
        workspace_default
    } else {
        direct
    }
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
    fn deployment_id_uses_explicit_value_or_validated_package_tenant_not_a_directory() {
        assert_eq!(
            deployment_id_for_startup(Some("tenant-id"), "package-id"),
            DeploymentId::new("tenant-id"),
        );
        assert_eq!(
            deployment_id_for_startup(None, "package-id"),
            DeploymentId::new("package-id"),
        );
        assert_ne!(
            deployment_id_for_startup(None, "package-id"),
            DeploymentId::new("../examples/fintech")
        );
    }

    #[test]
    fn default_package_path_resolves_to_the_workspace_fixture_without_rewriting_custom_paths() {
        assert!(
            tenant_package_directory_for_startup(DEFAULT_TENANT_PACKAGE_DIR)
                .ends_with("examples/fintech")
        );
        assert_eq!(
            tenant_package_directory_for_startup("/mounted/customer-package"),
            PathBuf::from("/mounted/customer-package")
        );
    }

    #[test]
    fn openai_base_url_is_sdk_style_base_and_protocol_selects_exact_endpoint() {
        assert_eq!(PACKAGE_OPENAI_PROTOCOL, OpenAiProtocol::Responses);
        assert_eq!(
            openai_endpoint("https://api.openai.com/v1", OpenAiProtocol::Responses)
                .unwrap()
                .as_str(),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            openai_endpoint(
                "https://gateway.example/openai/v1/",
                OpenAiProtocol::ChatCompletions,
            )
            .unwrap()
            .as_str(),
            "https://gateway.example/openai/v1/chat/completions"
        );
        assert!(
            openai_endpoint("https://user@gateway.example/v1", OpenAiProtocol::Responses).is_err()
        );
        assert!(
            openai_endpoint(
                "https://gateway.example/v1?tenant=x",
                OpenAiProtocol::Responses
            )
            .is_err()
        );
    }
}
