//! OpenBot Server 生产二进制组装入口。

use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, Mac};
use openbot_agent::{
    AgentToolInvoker, AuthorizedAgentToolGateway, BuiltInAgentConfig, BuiltInAgentRuntime,
    ProviderRouter, RemoteAgentToolInvoker, RemoteAguiProvider, RetryingProvider,
    RetryingProviderConfig,
};
use openbot_application::tenant::package::{
    BuiltInProviderSource, TenantAgentConfiguration, TenantAgentType, TenantPackageAudienceContext,
    TenantPackageEnvironment, synchronize_tenant_package,
};
use openbot_application::{
    AgentAudit, ApplicationService, NoRunDispatchConsumer, OpenBotApplication, ProviderAdapter,
    RunDispatchConsumer, RunRuntime, remember_provider_tool,
};
use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};
use openbot_domain::identity::groups::IdentityProviderId;
use openbot_domain::identity::session::TrustedOrigins;
use openbot_domain::remote_callback::RemoteRunAssertionSigner;
use openbot_domain::vault::{KeyVersion, WrappingKey};
use openbot_infra::agent_audit::PostgresAgentAudit;
use openbot_infra::agent_callback::{
    PostgresAgentCallbackTokens, PostgresRemoteCallbackAuthenticator,
};
use openbot_infra::agent_tools::{
    PostgresAgentAuthorizationSource, PostgresAgentToolSequence, PostgresBuiltInToolControlPlane,
};
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
use openbot_infra::component_catalogue::PostgresComponentAdministration;
use openbot_infra::db::pool::DatabaseConfig;
use openbot_infra::db::{native, pool};
use openbot_infra::google_drive::GoogleDriveRestTransport;
use openbot_infra::google_drive_oauth::GoogleDriveOAuthClient;
use openbot_infra::mcp::SafeRmcpClient;
use openbot_infra::mcp_catalog::PostgresMcpCatalog;
use openbot_infra::mcp_connections::{McpRevocationReconciler, PostgresMcpConnections};
use openbot_infra::mcp_credentials::PostgresMcpCredentialBroker;
use openbot_infra::mcp_oauth::McpOAuthClient;
use openbot_infra::memory_admin::PostgresMemoryAdministration;
use openbot_infra::net::safe_http::{
    CidrAllowlist, EgressPolicy, SafeDialer, SafeHttpBudget, SchemePolicy,
};
use openbot_infra::policy::{PolicyListener, PolicyStore};
use openbot_infra::provider::anthropic::{
    AnthropicApiKey, AnthropicProvider, AnthropicProviderConfig,
};
use openbot_infra::provider::context::PostgresAgentContextSource;
use openbot_infra::provider::credential::PostgresOpenAiCredentialSource;
use openbot_infra::provider::google::{GoogleApiKey, GoogleProvider, GoogleProviderConfig};
use openbot_infra::provider::openai::{
    OpenAiApiKey, OpenAiProtocol, OpenAiProvider, OpenAiProviderConfig,
};
use openbot_infra::remote_agui::SafeRemoteAguiTransport;
use openbot_infra::repo::audit::PostgresAuditReader;
use openbot_infra::repo::people_admin::PostgresPeopleAdministration;
use openbot_infra::repo::tools::PostgresToolJournal;
use openbot_infra::repo::{ChannelRepo, PostgresAgentDirectory};
use openbot_infra::routing::PostgresChannelRouting;
use openbot_infra::run_runtime::{DEFAULT_DISPATCH_CLAIM_DURATION, PostgresRunRuntime, RunRelay};
use openbot_infra::store::plugin_user_credential::PostgresOwnedCredentialRetirer;
use openbot_infra::tenant::{PostgresTenantPackageSynchronizer, load_tenant_package};
use openbot_infra::thread_directory::{DEFAULT_THREAD_LEASE_DURATION, PostgresThreadDirectory};
use openbot_infra::thread_id::mint_thread_id;
use openbot_infra::tool_approval::PostgresToolApprovalCoordinator;
use openbot_infra::ui_preferences::PostgresUiPreferenceAdministration;
use openbot_infra::vault::CredentialRecordVault;
use openbot_server::config::{
    AgentBudgets, DEFAULT_TENANT_PACKAGE_DIR, DeploymentEnvironment, ManagedProviderConfig,
    ManagedProviderKind, PackageOpenAiProviderConfig, ServerConfig, env_map_from_process,
};
use openbot_server::readiness::ReadinessVerdict;
use openbot_server::telemetry::{self, LogFormat};
use openbot_server::{
    AuthResolver, FnReadinessProbe, PostgresSessionAuthResolver, SINGLE_USER_ACTOR_ID,
    SensitiveWriteSecurity, ServerBuilder, SingleUserAuthResolver, StaticApp, install_recorder,
};
use sha2::Sha256;
use url::Url;

type HmacSha256 = Hmac<Sha256>;
const PACKAGE_OPENAI_PROTOCOL: OpenAiProtocol = OpenAiProtocol::Responses;
const CHANNEL_ROUTING_OPENAI_PROTOCOL: OpenAiProtocol = OpenAiProtocol::ChatCompletions;
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

struct BuiltInAgentAssemblyInput {
    pool: deadpool_postgres::Pool,
    deployment: DeploymentId,
    tenant: TenantId,
    runtime: Arc<dyn RunRuntime>,
    required: bool,
    requires_managed: bool,
    model: String,
    credential_key_id: String,
    provider: PackageOpenAiProviderConfig,
    managed_provider: Option<ManagedProviderConfig>,
    credential_vault: CredentialRecordVault,
    tools: Arc<dyn AgentToolInvoker>,
    audit: Arc<dyn AgentAudit>,
    remote_assertions: Arc<RemoteRunAssertionSigner>,
    mcp_catalog: Arc<PostgresMcpCatalog>,
    budgets: AgentBudgets,
}

struct ChannelRoutingAssemblyInput {
    pool: deadpool_postgres::Pool,
    model: String,
    credential_key_id: String,
    provider: PackageOpenAiProviderConfig,
    credential_vault: CredentialRecordVault,
    audit_key: Vec<u8>,
    stall_timeout: Option<Duration>,
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
    let requires_agent_runtime = tenant_package.package.agents.iter().any(|agent| {
        matches!(
            agent.agent_type,
            TenantAgentType::BuiltIn | TenantAgentType::RemoteAgUi
        )
    });
    let requires_managed_agent = tenant_package.package.agents.iter().any(|agent| {
        matches!(
            &agent.configuration,
            TenantAgentConfiguration::BuiltIn {
                provider_source: BuiltInProviderSource::Managed,
                ..
            }
        )
    });

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
    let remote_assertions = Arc::new(RemoteRunAssertionSigner::new(
        raw_master.as_bytes().to_vec(),
    )?);

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
    let thread_directory = PostgresThreadDirectory::with_runtime(
        pool.clone(),
        database
            .clone()
            .with_application_name("openbot-thread-events"),
        thread_runtime_owner,
        DEFAULT_THREAD_LEASE_DURATION,
    )?;
    let memory = PostgresMemoryAdministration::new(pool.clone());
    // MCP defaults to public HTTPS only. Private/reserved endpoints remain fail-closed until an
    // MCP-specific administrator CIDR configuration is added; provider CIDRs are not reused.
    let mcp_client = SafeRmcpClient::new(
        SafeDialer::new(EgressPolicy::default()),
        SchemePolicy::HttpsOnly,
        server.agent_budgets.stall_timeout,
    );
    let mcp_catalog = Arc::new(PostgresMcpCatalog::new(
        pool.clone(),
        mcp_client.clone(),
        audit_key.to_vec(),
    )?);
    let drive_oauth = GoogleDriveOAuthClient::new(SafeDialer::new(EgressPolicy::default()))?;
    let drive_transport = GoogleDriveRestTransport::new(SafeDialer::new(EgressPolicy::default()))?;
    let mcp_credentials = Arc::new(
        PostgresMcpCredentialBroker::new(pool.clone(), model_credential_vault.clone())
            .with_user_oauth(
                SafeDialer::new(EgressPolicy::default()),
                SchemePolicy::HttpsOnly,
                audit_key.to_vec(),
            )?
            .with_google_drive_oauth(drive_oauth.clone()),
    );
    match tokio::time::timeout(
        Duration::from_secs(30),
        mcp_catalog.refresh_startup_servers(&mcp_credentials),
    )
    .await
    {
        Ok(Ok(mcp_sweep)) => tracing::info!(
            refreshed = mcp_sweep.refreshed,
            failed = mcp_sweep.failed,
            authenticated_deferred = mcp_sweep.authenticated_deferred,
            "MCP startup catalog sweep 已完成"
        ),
        Ok(Err(error)) => return Err(error.into()),
        Err(_) => tracing::warn!(
            code = "mcp_startup_sweep_timeout",
            "MCP startup catalog sweep 超过新增 30s 总预算；Server 继续且未刷新项保持不可见"
        ),
    }
    let oauth_public_url = server
        .public_url
        .as_ref()
        .filter(|url| Url::parse(url.as_str()).is_ok_and(|parsed| parsed.scheme() == "https"));
    if server.public_url.is_some() && oauth_public_url.is_none() {
        tracing::warn!(
            code = "mcp_oauth_https_callback_required",
            "MCP OAuth Server callback 要求 HTTPS public URL；connect 保持不可用"
        );
    }
    let mcp_connections = Arc::new(
        PostgresMcpConnections::new(
            pool.clone(),
            model_credential_vault.clone(),
            McpOAuthClient::new(
                SafeDialer::new(EgressPolicy::default()),
                SchemePolicy::HttpsOnly,
            ),
            mcp_catalog.clone(),
            deployment.clone(),
            tenant.clone(),
            derive_mcp_oauth_state_key(raw_master.as_bytes()).to_vec(),
            audit_key.to_vec(),
            oauth_public_url.map(|url| url.as_str()),
            server.app_url.as_deref(),
            SchemePolicy::HttpsOnly,
        )?
        .with_google_drive_oauth(drive_oauth),
    );
    let tool_approvals = Arc::new(PostgresToolApprovalCoordinator::new(
        pool.clone(),
        deployment.clone(),
        tenant.clone(),
        audit_key.to_vec(),
    )?);
    let tool_control = PostgresBuiltInToolControlPlane::new(
        pool.clone(),
        deployment.clone(),
        tenant.clone(),
        policy_store.clone(),
        Arc::new(memory.clone()),
    )
    .with_mcp(mcp_catalog.clone(), mcp_client)
    .with_google_drive(drive_transport)
    .with_tool_approvals(tool_approvals.clone())
    .with_mcp_credentials(mcp_credentials);
    let tool_journal = PostgresToolJournal::new(pool.clone(), audit_key.to_vec())?;
    let callback_tokens = PostgresAgentCallbackTokens::new(
        pool.clone(),
        deployment.clone(),
        tenant.clone(),
        audit_key.to_vec(),
    )?;
    let remote_callback_auth = Arc::new(
        PostgresRemoteCallbackAuthenticator::new(
            pool.clone(),
            deployment.clone(),
            tenant.clone(),
            single_user,
            remote_assertions.clone(),
            audit_key.to_vec(),
        )?
        .with_mcp_catalog(mcp_catalog.clone()),
    );
    let channel_routing = build_channel_routing(ChannelRoutingAssemblyInput {
        pool: pool.clone(),
        model: tenant_package.package.model.default_model.clone(),
        credential_key_id: tenant_package.package.model.credential_secret_ref.clone(),
        provider: server.package_openai_provider.clone(),
        credential_vault: model_credential_vault.clone(),
        audit_key: audit_key.to_vec(),
        stall_timeout: server.agent_budgets.stall_timeout,
    })?;
    let channels = ChannelRepo::new(pool.clone());
    let components = Arc::new(PostgresComponentAdministration::new(
        pool.clone(),
        audit_key.to_vec(),
    )?);
    let application = OpenBotApplication::new(channels.clone())
        .with_people(people)
        .with_audit(PostgresAuditReader::new(pool.clone()))
        .with_policy(policy_store)
        .with_tools(tool_control, tool_journal)
        .with_threads(thread_directory)
        .with_memory(memory)
        .with_agent_callback_tokens(callback_tokens)
        .with_channel_administration(Arc::new(channels))
        .with_channel_routing(Arc::new(channel_routing));
    let application = application
        .with_agent_directory(Arc::new(PostgresAgentDirectory::new(pool.clone())))
        .with_component_administration(components)
        .with_mcp_connections(mcp_connections.clone())
        .with_tool_approvals(tool_approvals)
        .with_ui_preferences(Arc::new(PostgresUiPreferenceAdministration::new(
            pool.clone(),
        )));
    let mcp_revocation_reconciler = McpRevocationReconciler::start(mcp_connections.clone());
    let application: Arc<dyn ApplicationService> = Arc::new(application);
    let governed_tools = Arc::new(AuthorizedAgentToolGateway::with_sequence(
        application.clone(),
        Arc::new(PostgresAgentAuthorizationSource::new(
            pool.clone(),
            deployment.clone(),
            tenant.clone(),
            single_user,
        )),
        Arc::new(PostgresAgentToolSequence::new(pool.clone())),
    ));
    let agent_tools: Arc<dyn AgentToolInvoker> = governed_tools.clone();
    let remote_callback_tools: Arc<dyn RemoteAgentToolInvoker> = governed_tools;
    let managed_provider = managed_provider_for_slot(&server);
    let agent_audit: Arc<dyn AgentAudit> =
        Arc::new(PostgresAgentAudit::new(pool.clone(), audit_key.to_vec())?);
    let (run_consumer, built_in_agent) = build_built_in_agent(BuiltInAgentAssemblyInput {
        pool: pool.clone(),
        deployment: deployment.clone(),
        tenant: tenant.clone(),
        runtime: run_runtime.clone(),
        required: requires_agent_runtime,
        requires_managed: requires_managed_agent,
        model: tenant_package.package.model.default_model.clone(),
        credential_key_id: tenant_package.package.model.credential_secret_ref.clone(),
        provider: server.package_openai_provider.clone(),
        managed_provider,
        credential_vault: model_credential_vault,
        tools: agent_tools,
        audit: agent_audit,
        remote_assertions: remote_assertions.clone(),
        mcp_catalog,
        budgets: server.agent_budgets,
    })?;
    let built_in_agent_ready = built_in_agent.is_some() || !requires_agent_runtime;
    let run_relay = RunRelay::start_with_database(
        run_runtime,
        run_consumer,
        database
            .clone()
            .with_application_name("openbot-run-control"),
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
        .with_remote_callback_authenticator(remote_callback_auth)
        .with_remote_callback_tools(remote_callback_tools)
        .with_readiness_probe(Arc::new(db_probe));
    if let Some(dist) = server.app_dist_dir.as_deref() {
        builder = builder.with_static_app(StaticApp::open(dist)?);
    }
    builder = builder.with_mcp_oauth_callback(mcp_connections);
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
    mcp_revocation_reconciler.stop().await;
    policy_listener.stop().await;
    serve_result?;
    pool.close();
    Ok(())
}

fn build_built_in_agent(input: BuiltInAgentAssemblyInput) -> Result<AgentAssembly, Box<dyn Error>> {
    let BuiltInAgentAssemblyInput {
        pool,
        deployment,
        tenant,
        runtime,
        required,
        requires_managed,
        model,
        credential_key_id,
        provider,
        managed_provider,
        credential_vault,
        tools,
        audit,
        remote_assertions,
        mcp_catalog,
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
    let egress = CidrAllowlist::parse_exact(egress_allow_cidrs.iter().map(String::as_str))?;
    let egress_policy = EgressPolicy::new(egress);
    let package_provider: Arc<dyn ProviderAdapter> =
        Arc::new(OpenAiProvider::new_with_credential_source(
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
            SafeDialer::new(egress_policy.clone()),
        ));
    let managed = if requires_managed {
        Some(build_managed_provider(
            managed_provider.ok_or_else(|| startup_error("managed_provider_key_missing"))?,
            budgets,
        )?)
    } else {
        None
    };
    let remote_transport = Arc::new(SafeRemoteAguiTransport::new(
        SafeDialer::new(egress_policy),
        SafeHttpBudget::new(64 * 1024 * 1024, Duration::from_secs(30))?,
        budgets.stall_timeout,
        if allow_http {
            SchemePolicy::HttpOrHttps
        } else {
            SchemePolicy::HttpsOnly
        },
    )?);
    let remote_provider: Arc<dyn ProviderAdapter> =
        Arc::new(RemoteAguiProvider::new(remote_transport));
    let provider = Arc::new(RetryingProvider::new(
        Arc::new(ProviderRouter::new(package_provider, managed).with_remote_agui(remote_provider)),
        RetryingProviderConfig::default(),
    )?);
    let context = Arc::new(
        PostgresAgentContextSource::new(pool, deployment, tenant, Some(budgets.max_output_tokens))?
            .with_tools(vec![remember_provider_tool()])
            .with_remote_assertions(remote_assertions)
            .with_mcp_catalog(mcp_catalog),
    );
    let agent = BuiltInAgentRuntime::start(
        runtime,
        context,
        provider,
        tools,
        audit,
        BuiltInAgentConfig {
            run_deadline: budgets.run_deadline,
            ..BuiltInAgentConfig::default()
        },
    )
    .map_err(|_| startup_error("agent_runtime_config_invalid"))?;
    let consumer = agent.consumer();
    Ok((consumer, Some(agent)))
}

fn build_channel_routing(
    input: ChannelRoutingAssemblyInput,
) -> Result<PostgresChannelRouting, Box<dyn Error>> {
    let ChannelRoutingAssemblyInput {
        pool,
        model,
        credential_key_id,
        provider,
        credential_vault,
        audit_key,
        stall_timeout,
    } = input;
    let PackageOpenAiProviderConfig {
        base_url,
        environment_api_key,
        egress_allow_cidrs,
        allow_http,
    } = provider;
    let protocol = CHANNEL_ROUTING_OPENAI_PROTOCOL;
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
    let egress = EgressPolicy::new(CidrAllowlist::parse_exact(
        egress_allow_cidrs.iter().map(String::as_str),
    )?);
    let provider: Arc<dyn ProviderAdapter> = Arc::new(OpenAiProvider::new_with_credential_source(
        OpenAiProviderConfig::new_with_transport_policy(
            endpoint,
            model,
            protocol,
            SafeHttpBudget::new(16 * 1024 * 1024, Duration::from_secs(10))?,
            stall_timeout,
            if allow_http {
                SchemePolicy::HttpOrHttps
            } else {
                SchemePolicy::HttpsOnly
            },
        )?,
        credentials,
        SafeDialer::new(egress),
    ));
    Ok(PostgresChannelRouting::new(pool, audit_key, provider)?)
}

fn managed_provider_for_slot(server: &ServerConfig) -> Option<ManagedProviderConfig> {
    server.managed_provider.clone().or_else(|| {
        let package = &server.package_openai_provider;
        package
            .environment_api_key
            .clone()
            .map(|api_key| ManagedProviderConfig {
                provider: ManagedProviderKind::OpenAi,
                model: "gpt-5.5".to_owned(),
                api_key,
                base_url: package.base_url.clone(),
                use_responses_api: false,
                egress_allow_cidrs: package.egress_allow_cidrs.clone(),
                allow_http: package.allow_http,
            })
    })
}

fn build_managed_provider(
    config: ManagedProviderConfig,
    budgets: AgentBudgets,
) -> Result<Arc<dyn ProviderAdapter>, Box<dyn Error>> {
    let ManagedProviderConfig {
        provider,
        model,
        api_key,
        base_url,
        use_responses_api,
        egress_allow_cidrs,
        allow_http,
    } = config;
    let scheme = if allow_http {
        SchemePolicy::HttpOrHttps
    } else {
        SchemePolicy::HttpsOnly
    };
    let dialer = SafeDialer::new(EgressPolicy::new(CidrAllowlist::parse_exact(
        egress_allow_cidrs.iter().map(String::as_str),
    )?));
    let connect_budget = SafeHttpBudget::new(64 * 1024 * 1024, Duration::from_secs(30))?;
    match provider {
        ManagedProviderKind::OpenAi => {
            let protocol = if use_responses_api {
                OpenAiProtocol::Responses
            } else {
                OpenAiProtocol::ChatCompletions
            };
            Ok(Arc::new(OpenAiProvider::new(
                OpenAiProviderConfig::new_with_transport_policy(
                    openai_endpoint(base_url.as_str(), protocol)?,
                    model,
                    protocol,
                    connect_budget,
                    budgets.stall_timeout,
                    scheme,
                )?,
                OpenAiApiKey::from_bytes(api_key.expose().as_bytes().to_vec())?,
                dialer,
            )))
        }
        ManagedProviderKind::Anthropic => Ok(Arc::new(AnthropicProvider::new(
            AnthropicProviderConfig::new_with_transport_policy(
                anthropic_endpoint(base_url.as_str())?,
                model,
                AnthropicApiKey::from_bytes(api_key.expose().as_bytes().to_vec())?,
                connect_budget,
                budgets.stall_timeout,
                scheme,
            )?,
            dialer,
        ))),
        ManagedProviderKind::Google => Ok(Arc::new(GoogleProvider::new(
            GoogleProviderConfig::new_with_transport_policy(
                google_endpoint(base_url.as_str(), &model)?,
                model,
                GoogleApiKey::from_bytes(api_key.expose().as_bytes().to_vec())?,
                connect_budget,
                budgets.stall_timeout,
                scheme,
            )?,
            dialer,
        ))),
    }
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

fn anthropic_endpoint(base: &str) -> Result<Url, std::io::Error> {
    provider_base(base, "anthropic_base_url_invalid")?
        .join("v1/messages")
        .map_err(|_| startup_error("anthropic_base_url_invalid"))
}

fn google_endpoint(base: &str, model: &str) -> Result<Url, std::io::Error> {
    let model = model.strip_prefix("models/").unwrap_or(model);
    if model.is_empty()
        || model.starts_with('/')
        || model.ends_with('/')
        || model.split('/').any(|segment| {
            segment.is_empty()
                || segment == "."
                || segment == ".."
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return Err(startup_error("google_model_invalid"));
    }
    let model_path = if model.contains('/') {
        model.to_owned()
    } else {
        format!("models/{model}")
    };
    provider_base(base, "google_base_url_invalid")?
        .join(&format!(
            "v1beta/{model_path}:streamGenerateContent?alt=sse"
        ))
        .map_err(|_| startup_error("google_base_url_invalid"))
}

fn provider_base(base: &str, code: &'static str) -> Result<Url, std::io::Error> {
    let mut base = Url::parse(base).map_err(|_| startup_error(code))?;
    if !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err(startup_error(code));
    }
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    Ok(base)
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

fn derive_mcp_oauth_state_key(master: &[u8]) -> [u8; 32] {
    let mut hmac = HmacSha256::new_from_slice(master).expect("HMAC 接受任意非空长度");
    hmac.update(b"openbot:mcp-oauth-state:v1");
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
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn audit_key_is_domain_separated_and_deterministic() {
        let first = derive_audit_key(b"master-key");
        let oauth = derive_mcp_oauth_state_key(b"master-key");
        assert_eq!(first, derive_audit_key(b"master-key"));
        assert_eq!(oauth, derive_mcp_oauth_state_key(b"master-key"));
        assert_ne!(first, derive_audit_key(b"other-key"));
        assert_ne!(oauth, derive_mcp_oauth_state_key(b"other-key"));
        assert_ne!(first, oauth);
        assert_ne!(first, [0; 32]);
        assert_ne!(oauth, [0; 32]);
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
            CHANNEL_ROUTING_OPENAI_PROTOCOL,
            OpenAiProtocol::ChatCompletions
        );
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

    #[test]
    fn managed_provider_defaults_from_openai_environment_only_when_the_slot_needs_it() {
        let server = ServerConfig::from_env_map(&BTreeMap::from([(
            "OPENAI_API_KEY".to_owned(),
            " managed-key ".to_owned(),
        )]))
        .unwrap();
        assert!(server.managed_provider.is_none());
        let managed = managed_provider_for_slot(&server).unwrap();
        assert_eq!(managed.provider, ManagedProviderKind::OpenAi);
        assert_eq!(managed.model, "gpt-5.5");
        assert_eq!(managed.api_key.expose(), "managed-key");
        assert!(!managed.use_responses_api);

        let no_key = ServerConfig::from_env_map(&BTreeMap::new()).unwrap();
        assert!(managed_provider_for_slot(&no_key).is_none());
    }

    #[test]
    fn anthropic_and_google_base_urls_map_to_locked_sdk_endpoints_without_model_injection() {
        assert_eq!(
            anthropic_endpoint("https://api.anthropic.com")
                .unwrap()
                .as_str(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            google_endpoint(
                "https://generativelanguage.googleapis.com",
                "gemini-2.5-flash"
            )
            .unwrap()
            .as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
        );
        assert_eq!(
            google_endpoint("https://gateway.example/google/", "models/gemini-custom")
                .unwrap()
                .as_str(),
            "https://gateway.example/google/v1beta/models/gemini-custom:streamGenerateContent?alt=sse"
        );
        assert!(google_endpoint("https://gateway.example", "../admin?key=secret").is_err());
    }

    #[test]
    fn production_managed_factory_constructs_all_three_provider_families() {
        let cases = [
            (
                ManagedProviderKind::OpenAi,
                "gpt-5.5",
                "https://api.openai.com/v1",
                false,
            ),
            (
                ManagedProviderKind::Anthropic,
                "claude-sonnet-4-5",
                "https://api.anthropic.com",
                false,
            ),
            (
                ManagedProviderKind::Google,
                "gemini-2.5-flash",
                "https://generativelanguage.googleapis.com",
                false,
            ),
        ];
        let mut constructed = 0;
        for (provider, model, base, use_responses_api) in cases {
            let adapter = build_managed_provider(
                ManagedProviderConfig {
                    provider,
                    model: model.to_owned(),
                    api_key: openbot_server::config::Secret::new("test-key"),
                    base_url: openbot_server::config::DeploymentAddress::parse(base).unwrap(),
                    use_responses_api,
                    egress_allow_cidrs: Vec::new(),
                    allow_http: false,
                },
                AgentBudgets {
                    stall_timeout: None,
                    run_deadline: Some(Duration::from_secs(1800)),
                    max_output_tokens: 16_384,
                },
            )
            .unwrap();
            constructed += 1;
            drop(adapter);
        }
        assert_eq!(constructed, 3);
    }
}
