//! Desktop Local background composition from Tauri's current-user app-data authority.
//!
//! The event loop starts with no window. `setup` resolves `AppHandle::path().app_data_dir()`, then
//! a Tauri runtime worker performs the complete authority → PostgreSQL sidecar → fixed database →
//! application key material → shared `ApplicationService` sequence. The custom protocol remains
//! unavailable until that sequence succeeds, and the first window is created last.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::time::Duration;

use openbot_application::tenant::package::{LoadedTenantPackage, TenantPackageError};
use openbot_contracts::auth::AuthContext;
use openbot_infra::application_assembly::{
    ChannelRoutingProviderInput, PostgresApplicationAssembly, PostgresApplicationAssemblyInput,
    assemble_postgres_application,
};
use openbot_infra::auth::single_user::desktop_local::{
    CurrentOsUserAppDataRoot, DesktopLocalAuthority, DesktopLocalAuthorityStore,
};
use openbot_infra::policy::PolicyStore;
use tauri::{App, Builder, Context, Manager, RunEvent, Runtime, Wry};

use crate::desktop_agent_runtime::{
    DesktopAgentHost, DesktopAgentHostInput, start_desktop_agent_host,
};
use crate::desktop_local_bootstrap::{
    DesktopLocalCompositionError, RunningDesktopLocalDataPlane, bootstrap_running_sidecar,
};
use crate::desktop_vault::ReviewedDesktopVaultKeyStoreService;
use crate::os_secret_store::OsSecretStore;
use crate::postgres_sidecar::{
    PostgresSidecarError, PostgresSidecarSupervisor, ReviewedPostgresKeyStoreService,
    VerifiedPostgresBundle,
};
use crate::tauri_host::{
    DesktopTauriProtocol, DesktopTauriProtocolSlot, register_tauri_protocol_slot, valid_scheme,
};
use crate::{
    DesktopAgentBudgets, DesktopOpenAiProviderInput, DesktopUiPreferenceStore,
    DesktopWindowLifecycle, InProcessTransport, VerifiedDesktopWindowAuthority,
};

type PackageFactory = Box<
    dyn FnOnce(&DesktopLocalAuthority) -> Result<LoadedTenantPackage, TenantPackageError>
        + Send
        + 'static,
>;

const DESKTOP_UI_PREFERENCES_FILE: &str = "ui-preferences-v1";

/// Stable startup/shutdown failures with no path, secret, package prose, or platform diagnostics.
#[derive(Debug, thiserror::Error)]
pub enum DesktopLocalRuntimeError {
    /// Caller-supplied scheme, window label, or Desktop network policy was outside the closed shape.
    #[error("desktop_local_runtime_configuration_invalid")]
    Configuration,
    /// Tauri could not resolve the current user's application-data directory.
    #[error("desktop_local_runtime_app_data_unavailable")]
    AppData,
    /// App-instance identity or its private PostgreSQL directory could not be loaded.
    #[error("desktop_local_runtime_authority_failed")]
    Authority,
    /// The release-attested PostgreSQL child did not reach SCRAM readiness.
    #[error("desktop_local_runtime_sidecar_failed")]
    Sidecar,
    /// The release package factory did not return a valid exact-instance package.
    #[error("desktop_local_runtime_package_failed")]
    Package,
    /// Fixed database, migration, principal, or package bootstrap failed.
    #[error("desktop_local_runtime_data_plane_failed")]
    DataPlane,
    /// Per-instance application key material could not be loaded from the OS store.
    #[error("desktop_local_runtime_vault_failed")]
    Vault,
    /// The authoritative action-policy snapshot could not be loaded.
    #[error("desktop_local_runtime_policy_failed")]
    Policy,
    /// Shared PostgreSQL application adapter assembly failed.
    #[error("desktop_local_runtime_application_failed")]
    Application,
    /// Built-in/remote Agent runtime or durable run relay could not start.
    #[error("desktop_local_runtime_agent_failed")]
    Agent,
    /// Local bundle/protocol/window construction failed after application assembly.
    #[error("desktop_local_runtime_host_failed")]
    Host,
    /// Startup failed after a child existed and verified cleanup could not complete.
    #[error("desktop_local_runtime_failure_cleanup_failed")]
    FailureCleanup,
    /// The process-wide background state was poisoned or made an impossible transition.
    #[error("desktop_local_runtime_state_failed")]
    State,
    /// Final authority/transport/reconciler/sidecar shutdown did not fully complete.
    #[error("desktop_local_runtime_shutdown_failed")]
    Shutdown,
    /// Tauri application construction failed before the event loop started.
    #[error("desktop_local_runtime_build_failed")]
    Build,
}

impl DesktopLocalRuntimeError {
    fn code(&self) -> &'static str {
        match self {
            Self::Configuration => "desktop_local_runtime_configuration_invalid",
            Self::AppData => "desktop_local_runtime_app_data_unavailable",
            Self::Authority => "desktop_local_runtime_authority_failed",
            Self::Sidecar => "desktop_local_runtime_sidecar_failed",
            Self::Package => "desktop_local_runtime_package_failed",
            Self::DataPlane => "desktop_local_runtime_data_plane_failed",
            Self::Vault => "desktop_local_runtime_vault_failed",
            Self::Policy => "desktop_local_runtime_policy_failed",
            Self::Application => "desktop_local_runtime_application_failed",
            Self::Agent => "desktop_local_runtime_agent_failed",
            Self::Host => "desktop_local_runtime_host_failed",
            Self::FailureCleanup => "desktop_local_runtime_failure_cleanup_failed",
            Self::State => "desktop_local_runtime_state_failed",
            Self::Shutdown => "desktop_local_runtime_shutdown_failed",
            Self::Build => "desktop_local_runtime_build_failed",
        }
    }
}

/// Reviewed release resources that remain independent from the current user's app-data path.
pub struct DesktopLocalReleaseInput {
    dist: PathBuf,
    scheme: String,
    window_label: String,
    postgres_bundle: VerifiedPostgresBundle,
    postgres_key_store_service: ReviewedPostgresKeyStoreService,
    vault_key_store_service: ReviewedDesktopVaultKeyStoreService,
    secret_store: Arc<dyn OsSecretStore>,
}

impl DesktopLocalReleaseInput {
    /// Bind verified release assets and reviewed platform key-store identities.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        dist: impl Into<PathBuf>,
        scheme: impl Into<String>,
        window_label: impl Into<String>,
        postgres_bundle: VerifiedPostgresBundle,
        postgres_key_store_service: ReviewedPostgresKeyStoreService,
        vault_key_store_service: ReviewedDesktopVaultKeyStoreService,
        secret_store: Arc<dyn OsSecretStore>,
    ) -> Result<Self, DesktopLocalRuntimeError> {
        let scheme = scheme.into();
        let window_label = window_label.into();
        if !valid_scheme(&scheme) || !valid_window_label(&window_label) {
            return Err(DesktopLocalRuntimeError::Configuration);
        }
        Ok(Self {
            dist: dist.into(),
            scheme,
            window_label,
            postgres_bundle,
            postgres_key_store_service,
            vault_key_store_service,
            secret_store,
        })
    }
}

impl core::fmt::Debug for DesktopLocalReleaseInput {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DesktopLocalReleaseInput")
            .field("dist", &"<reviewed-resource>")
            .field("scheme", &"<reviewed>")
            .field("window_label", &self.window_label)
            .field("postgres_bundle", &self.postgres_bundle)
            .field(
                "postgres_key_store_service",
                &self.postgres_key_store_service,
            )
            .field("vault_key_store_service", &self.vault_key_store_service)
            .field("secret_store", &"current-user OS store")
            .finish()
    }
}

/// Desktop-specific, environment-free inputs to the shared application composition root.
pub struct DesktopLocalApplicationInput {
    provider: DesktopOpenAiProviderInput,
    budgets: DesktopAgentBudgets,
}

impl DesktopLocalApplicationInput {
    /// Bind already-validated environment-free provider and Agent budget inputs.
    pub fn new(
        provider: DesktopOpenAiProviderInput,
        budgets: DesktopAgentBudgets,
    ) -> Result<Self, DesktopLocalRuntimeError> {
        provider
            .remote_transport(budgets.stall_timeout)
            .map_err(|_| DesktopLocalRuntimeError::Configuration)?;
        Ok(Self { provider, budgets })
    }
}

impl core::fmt::Debug for DesktopLocalApplicationInput {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DesktopLocalApplicationInput")
            .field("provider", &self.provider)
            .field("budgets", &self.budgets)
            .finish()
    }
}

/// One-shot setup plan. The package factory receives the elected app-instance authority, so a
/// release template cannot guess or hard-code the per-installation tenant.
pub struct DesktopLocalRuntimeConfig {
    release: DesktopLocalReleaseInput,
    application: DesktopLocalApplicationInput,
    package_factory: PackageFactory,
}

impl DesktopLocalRuntimeConfig {
    /// Construct a one-shot plan from reviewed release inputs and an exact-instance package source.
    #[must_use]
    pub fn new<F>(
        release: DesktopLocalReleaseInput,
        application: DesktopLocalApplicationInput,
        package_factory: F,
    ) -> Self
    where
        F: FnOnce(&DesktopLocalAuthority) -> Result<LoadedTenantPackage, TenantPackageError>
            + Send
            + 'static,
    {
        Self {
            release,
            application,
            package_factory: Box::new(package_factory),
        }
    }
}

impl core::fmt::Debug for DesktopLocalRuntimeConfig {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DesktopLocalRuntimeConfig")
            .field("release", &self.release)
            .field("application", &self.application)
            .field("package_factory", &"<one-shot>")
            .finish()
    }
}

/// Observable lifecycle phase used by the event-loop exit fence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopLocalRuntimePhase {
    /// `setup` returned with no window and background initialization is running.
    Initializing,
    /// All background owners exist and the first verified window is being created.
    StartingWindow,
    /// At least one verified native window is active.
    Ready,
    /// New work is fenced and shutdown is running.
    Stopping,
    /// Ordered shutdown completed; the next exit request may terminate the process.
    Stopped,
    /// Startup or shutdown failed; the process exits non-zero.
    Failed,
}

struct RuntimeStateInner {
    phase: DesktopLocalRuntimePhase,
    exit_requested: bool,
    lifecycle: Option<Arc<DesktopWindowLifecycle>>,
    owner: Option<Box<dyn RuntimeShutdownOwner>>,
}

/// Managed state shared by Tauri setup, window callbacks, commands, and the run-loop exit fence.
pub struct DesktopLocalRuntimeState {
    inner: Mutex<RuntimeStateInner>,
}

impl DesktopLocalRuntimeState {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(RuntimeStateInner {
                phase: DesktopLocalRuntimePhase::Initializing,
                exit_requested: false,
                lifecycle: None,
                owner: None,
            }),
        })
    }

    /// Read the current closed lifecycle phase without exposing paths or errors.
    #[must_use]
    pub fn phase(&self) -> DesktopLocalRuntimePhase {
        self.inner
            .lock()
            .map(|inner| inner.phase)
            .unwrap_or(DesktopLocalRuntimePhase::Failed)
    }

    fn note_exit_requested(&self) -> Result<bool, DesktopLocalRuntimeError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| DesktopLocalRuntimeError::State)?;
        inner.exit_requested = true;
        Ok(inner.phase == DesktopLocalRuntimePhase::Ready)
    }

    fn exit_requested(&self) -> Result<bool, DesktopLocalRuntimeError> {
        self.inner
            .lock()
            .map(|inner| inner.exit_requested)
            .map_err(|_| DesktopLocalRuntimeError::State)
    }

    fn install_owner(
        &self,
        owner: &mut Option<Box<dyn RuntimeShutdownOwner>>,
        lifecycle: Arc<DesktopWindowLifecycle>,
    ) -> Result<bool, DesktopLocalRuntimeError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| DesktopLocalRuntimeError::State)?;
        if inner.phase != DesktopLocalRuntimePhase::Initializing || inner.owner.is_some() {
            return Err(DesktopLocalRuntimeError::State);
        }
        if owner.is_none() {
            return Err(DesktopLocalRuntimeError::State);
        }
        inner.phase = DesktopLocalRuntimePhase::StartingWindow;
        inner.lifecycle = Some(lifecycle);
        inner.owner = owner.take();
        Ok(inner.exit_requested)
    }

    fn mark_ready(&self) -> Result<bool, DesktopLocalRuntimeError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| DesktopLocalRuntimeError::State)?;
        if inner.phase != DesktopLocalRuntimePhase::StartingWindow {
            return Err(DesktopLocalRuntimeError::State);
        }
        inner.phase = DesktopLocalRuntimePhase::Ready;
        Ok(inner.exit_requested)
    }

    fn lifecycle(&self) -> Option<Arc<DesktopWindowLifecycle>> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.lifecycle.clone())
    }

    fn begin_shutdown(
        &self,
    ) -> Result<Option<Box<dyn RuntimeShutdownOwner>>, DesktopLocalRuntimeError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| DesktopLocalRuntimeError::State)?;
        match inner.phase {
            DesktopLocalRuntimePhase::StartingWindow | DesktopLocalRuntimePhase::Ready => {
                inner.phase = DesktopLocalRuntimePhase::Stopping;
                inner.lifecycle = None;
                Ok(inner.owner.take())
            }
            DesktopLocalRuntimePhase::Stopping
            | DesktopLocalRuntimePhase::Stopped
            | DesktopLocalRuntimePhase::Failed
            | DesktopLocalRuntimePhase::Initializing => Ok(None),
        }
    }

    fn finish_shutdown(&self, succeeded: bool) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.phase = if succeeded {
                DesktopLocalRuntimePhase::Stopped
            } else {
                DesktopLocalRuntimePhase::Failed
            };
            inner.lifecycle = None;
            inner.owner = None;
        }
    }

    fn fail_startup(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.phase = DesktopLocalRuntimePhase::Failed;
            inner.lifecycle = None;
            inner.owner = None;
        }
    }
}

#[async_trait::async_trait]
trait RuntimeShutdownOwner: Send {
    async fn shutdown(self: Box<Self>) -> Result<(), DesktopLocalRuntimeError>;
}

struct DesktopLocalBackgroundOwner {
    lifecycle: Arc<DesktopWindowLifecycle>,
    transport: Arc<InProcessTransport>,
    agent_host: Option<DesktopAgentHost>,
    assembly: Option<PostgresApplicationAssembly>,
    data_plane: Option<RunningDesktopLocalDataPlane>,
}

#[async_trait::async_trait]
impl RuntimeShutdownOwner for DesktopLocalBackgroundOwner {
    async fn shutdown(mut self: Box<Self>) -> Result<(), DesktopLocalRuntimeError> {
        let authority_ok = self.lifecycle.shutdown_authority().is_ok();
        let _transport = self.transport.shutdown().await;
        if let Some(agent_host) = self.agent_host.take() {
            agent_host.stop().await;
        }
        if let Some(assembly) = self.assembly.take() {
            assembly.shutdown().await;
        }
        let data_plane_ok = match self.data_plane.take() {
            Some(data_plane) => data_plane.shutdown().await.is_ok(),
            None => false,
        };
        if authority_ok && data_plane_ok {
            Ok(())
        } else {
            Err(DesktopLocalRuntimeError::Shutdown)
        }
    }
}

pub(crate) struct PreparedDesktopLocalRuntime {
    auth: AuthContext,
    protocol: Arc<DesktopTauriProtocol>,
    lifecycle: Arc<DesktopWindowLifecycle>,
    owner: Box<dyn RuntimeShutdownOwner>,
    #[cfg(test)]
    application: Arc<dyn openbot_application::ApplicationService>,
    #[cfg(test)]
    pool: openbot_infra::db::pool::DatabasePool,
}

#[cfg(test)]
impl PreparedDesktopLocalRuntime {
    pub(crate) fn auth_context(&self) -> &AuthContext {
        &self.auth
    }

    pub(crate) fn application(&self) -> &Arc<dyn openbot_application::ApplicationService> {
        &self.application
    }

    pub(crate) fn active_window_count(&self) -> usize {
        self.lifecycle.active_window_count()
    }

    pub(crate) fn pool(&self) -> &openbot_infra::db::pool::DatabasePool {
        &self.pool
    }

    pub(crate) async fn shutdown(self) -> Result<(), DesktopLocalRuntimeError> {
        self.owner.shutdown().await
    }
}

/// Builder wrapper that makes the exit-fencing run callback non-optional.
pub struct DesktopLocalTauriBuilder {
    builder: Builder<Wry>,
}

impl DesktopLocalTauriBuilder {
    /// Build with the caller's reviewed external Tauri context, then run with ordered exit fencing.
    pub fn run(self, context: Context<Wry>) -> Result<(), DesktopLocalRuntimeError> {
        validate_tauri_context(&context)?;
        let app = self
            .builder
            .build(context)
            .map_err(|_| DesktopLocalRuntimeError::Build)?;
        run_desktop_local_app(app);
        Ok(())
    }
}

fn validate_tauri_context<R: Runtime>(
    context: &Context<R>,
) -> Result<(), DesktopLocalRuntimeError> {
    if context
        .config()
        .app
        .windows
        .iter()
        .any(|window| window.create)
    {
        return Err(DesktopLocalRuntimeError::Configuration);
    }
    Ok(())
}

/// Register deferred custom-protocol framing and the one authoritative Desktop Local setup.
pub fn register_desktop_local_runtime(
    builder: Builder<Wry>,
    config: DesktopLocalRuntimeConfig,
) -> Result<DesktopLocalTauriBuilder, DesktopLocalRuntimeError> {
    let scheme = config.release.scheme.clone();
    let window_label = config.release.window_label.clone();
    let protocol_slot = DesktopTauriProtocolSlot::pending();
    let runtime_state = DesktopLocalRuntimeState::new();

    let setup_state = Arc::clone(&runtime_state);
    let setup_slot = Arc::clone(&protocol_slot);
    let builder = register_tauri_protocol_slot(builder, &scheme, protocol_slot)
        .map_err(|_| DesktopLocalRuntimeError::Configuration)?
        .manage(Arc::clone(&runtime_state))
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let app_data_root = match resolve_current_user_app_data_root(app) {
                Ok(root) => root,
                Err(_) => {
                    setup_state.fail_startup();
                    app_handle.exit(1);
                    return Ok(());
                }
            };
            tauri::async_runtime::spawn(async move {
                let prepared = prepare_desktop_local_runtime(app_data_root, config).await;
                let prepared = match prepared {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        tracing::error!(code = error.code(), "Desktop Local startup failed");
                        setup_state.fail_startup();
                        app_handle.exit(1);
                        return;
                    }
                };
                if setup_slot.install(Arc::clone(&prepared.protocol)).is_err() {
                    let _ = prepared.owner.shutdown().await;
                    setup_state.fail_startup();
                    app_handle.exit(1);
                    return;
                }
                let lifecycle = Arc::clone(&prepared.lifecycle);
                let auth = prepared.auth.clone();
                let mut owner = Some(prepared.owner);
                if setup_state
                    .install_owner(&mut owner, Arc::clone(&lifecycle))
                    .is_err()
                {
                    if let Some(owner) = owner {
                        let _ = owner.shutdown().await;
                    }
                    setup_state.fail_startup();
                    app_handle.exit(1);
                    return;
                }
                if setup_state.exit_requested().unwrap_or(true) {
                    request_shutdown(Arc::clone(&setup_state), app_handle, 0);
                    return;
                }
                let window = lifecycle.create_verified_window(
                    &app_handle,
                    window_label,
                    VerifiedDesktopWindowAuthority::from_verified_session(auth, None),
                );
                if window.is_err() {
                    request_shutdown(Arc::clone(&setup_state), app_handle, 1);
                    return;
                }
                let exit_requested = setup_state.mark_ready().unwrap_or(true);
                if exit_requested || lifecycle.active_window_count() == 0 {
                    request_shutdown(Arc::clone(&setup_state), app_handle, 0);
                }
            });
            Ok(())
        });

    let window_state = Arc::clone(&runtime_state);
    let builder = builder.on_window_event(move |window, event| {
        let Some(lifecycle) = window_state.lifecycle() else {
            return;
        };
        match lifecycle.handle_window_event(window.label(), event) {
            Ok(true) if lifecycle.active_window_count() == 0 => {
                request_shutdown(Arc::clone(&window_state), window.app_handle().clone(), 0)
            }
            Ok(_) => {}
            Err(_) => request_shutdown(Arc::clone(&window_state), window.app_handle().clone(), 1),
        }
    });
    Ok(DesktopLocalTauriBuilder { builder })
}

fn resolve_current_user_app_data_root<R, M>(
    manager: &M,
) -> Result<CurrentOsUserAppDataRoot, DesktopLocalRuntimeError>
where
    R: Runtime,
    M: Manager<R>,
{
    let path = manager
        .path()
        .app_data_dir()
        .map_err(|_| DesktopLocalRuntimeError::AppData)?;
    CurrentOsUserAppDataRoot::from_current_os_user_app_data(path)
        .map_err(|_| DesktopLocalRuntimeError::AppData)
}

fn run_desktop_local_app(app: App<Wry>) {
    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            let state = app_handle.state::<Arc<DesktopLocalRuntimeState>>();
            match state.phase() {
                DesktopLocalRuntimePhase::Initializing
                | DesktopLocalRuntimePhase::StartingWindow => {
                    api.prevent_exit();
                    if state.note_exit_requested().is_err() {
                        app_handle.exit(1);
                    }
                }
                DesktopLocalRuntimePhase::Ready => {
                    api.prevent_exit();
                    let _ = state.note_exit_requested();
                    request_shutdown(Arc::clone(state.inner()), app_handle.clone(), 0);
                }
                DesktopLocalRuntimePhase::Stopping => api.prevent_exit(),
                DesktopLocalRuntimePhase::Stopped | DesktopLocalRuntimePhase::Failed => {}
            }
        }
    });
}

fn request_shutdown(
    state: Arc<DesktopLocalRuntimeState>,
    app_handle: tauri::AppHandle<Wry>,
    requested_exit_code: i32,
) {
    let owner = match state.begin_shutdown() {
        Ok(Some(owner)) => owner,
        Ok(None) => return,
        Err(_) => {
            state.finish_shutdown(false);
            app_handle.exit(1);
            return;
        }
    };
    tauri::async_runtime::spawn(async move {
        let succeeded = owner.shutdown().await.is_ok();
        state.finish_shutdown(succeeded);
        app_handle.exit(if succeeded { requested_exit_code } else { 1 });
    });
}

pub(crate) async fn prepare_desktop_local_runtime(
    app_data_root: CurrentOsUserAppDataRoot,
    config: DesktopLocalRuntimeConfig,
) -> Result<PreparedDesktopLocalRuntime, DesktopLocalRuntimeError> {
    let DesktopLocalRuntimeConfig {
        release,
        application,
        package_factory,
    } = config;
    let DesktopLocalReleaseInput {
        dist,
        scheme,
        window_label: _,
        postgres_bundle,
        postgres_key_store_service,
        vault_key_store_service,
        secret_store,
    } = release;
    let DesktopLocalApplicationInput { provider, budgets } = application;
    let remote_agent_probe = provider
        .remote_transport(budgets.stall_timeout)
        .map_err(|_| DesktopLocalRuntimeError::Configuration)?;
    let channel_routing_provider = ChannelRoutingProviderInput {
        endpoint: provider
            .channel_endpoint()
            .map_err(|_| DesktopLocalRuntimeError::Configuration)?,
        environment_api_key: None,
        egress_allow_cidrs: provider.egress_allow_cidrs(),
        allow_http: false,
    };

    let authority_store = DesktopLocalAuthorityStore::new(app_data_root.clone());
    let installation = authority_store
        .load_or_create_installation()
        .map_err(|_| DesktopLocalRuntimeError::Authority)?;
    let package =
        package_factory(installation.authority()).map_err(|_| DesktopLocalRuntimeError::Package)?;
    let sidecar = PostgresSidecarSupervisor::start(
        postgres_bundle,
        app_data_root.as_path(),
        installation.authority().instance_id(),
        installation.sidecar_data_dir(),
        secret_store.as_ref(),
        &postgres_key_store_service,
    )
    .await
    .map_err(map_sidecar_error)?;
    let data_plane = bootstrap_running_sidecar(installation, sidecar, &package)
        .await
        .map_err(map_data_plane_error)?;

    let listener_database = match data_plane.thread_listener_database() {
        Ok(listener) => listener,
        Err(_) => {
            return Err(cleanup_data_plane(data_plane, DesktopLocalRuntimeError::DataPlane).await);
        }
    };
    let key_material = match data_plane
        .load_application_key_material(secret_store.as_ref(), &vault_key_store_service)
    {
        Ok(material) => material,
        Err(_) => {
            return Err(cleanup_data_plane(data_plane, DesktopLocalRuntimeError::Vault).await);
        }
    };
    let pool = data_plane.pool().clone();
    #[cfg(test)]
    let test_pool = pool.clone();
    let policy_store = PolicyStore::postgres(pool.clone(), None);
    if policy_store.load().await.is_err() {
        return Err(cleanup_data_plane(data_plane, DesktopLocalRuntimeError::Policy).await);
    }
    let auth = data_plane.auth_context().clone();
    let (credential_vault, audit_key, remote_assertions, mcp_oauth_state_key) =
        key_material.into_assembly_parts();
    let agent_credential_vault = credential_vault.clone();
    let agent_audit_key = audit_key.expose().to_vec();
    let agent_remote_assertions = Arc::clone(&remote_assertions);
    let agent_listener_database = listener_database.clone();
    let assembly = match assemble_postgres_application(PostgresApplicationAssemblyInput {
        pool: pool.clone(),
        listener_database: listener_database.clone(),
        deployment: auth.deployment().clone(),
        tenant: auth.tenant().clone(),
        single_user: true,
        admin_floor: None,
        model: package.package.model.default_model.clone(),
        credential_key_id: package.package.model.credential_secret_ref.clone(),
        credential_vault,
        audit_key,
        remote_assertions,
        mcp_oauth_state_key,
        policy_store,
        ui_preferences: Arc::new(DesktopUiPreferenceStore::new(
            app_data_root.as_path().join(DESKTOP_UI_PREFERENCES_FILE),
        )),
        remote_agent_probe: remote_agent_probe.clone(),
        managed_slot_available: false,
        channel_routing_provider,
        stall_timeout: budgets.stall_timeout,
        oauth_public_url: None,
        app_url: None,
    })
    .await
    {
        Ok(assembly) => assembly,
        Err(_) => {
            return Err(
                cleanup_data_plane(data_plane, DesktopLocalRuntimeError::Application).await,
            );
        }
    };
    let agent_host = match start_desktop_agent_host(DesktopAgentHostInput {
        pool,
        listener_database: agent_listener_database,
        deployment: auth.deployment().clone(),
        tenant: auth.tenant().clone(),
        package: package.clone(),
        application: Arc::clone(&assembly.application),
        runtime: Arc::clone(&assembly.run_runtime),
        credential_vault: agent_credential_vault,
        audit_key: agent_audit_key,
        remote_assertions: agent_remote_assertions,
        mcp_catalog: Arc::clone(&assembly.mcp_catalog),
        components: Arc::clone(&assembly.components),
        sandboxed_components: Arc::clone(&assembly.sandboxed_components),
        provider,
        remote_transport: remote_agent_probe,
        budgets,
    }) {
        Ok(host) => host,
        Err(_) => {
            return Err(
                cleanup_assembly(data_plane, assembly, DesktopLocalRuntimeError::Agent).await,
            );
        }
    };
    let transport = Arc::new(InProcessTransport::new(Arc::clone(&assembly.application)));
    let protocol = match DesktopTauriProtocol::open(dist, Arc::clone(&transport)) {
        Ok(protocol) => Arc::new(protocol),
        Err(_) => {
            return Err(cleanup_agent_host(
                data_plane,
                assembly,
                agent_host,
                DesktopLocalRuntimeError::Host,
            )
            .await);
        }
    };
    let lifecycle = match DesktopWindowLifecycle::new(&scheme, Arc::clone(&protocol)) {
        Ok(lifecycle) => Arc::new(lifecycle),
        Err(_) => {
            return Err(cleanup_agent_host(
                data_plane,
                assembly,
                agent_host,
                DesktopLocalRuntimeError::Host,
            )
            .await);
        }
    };
    #[cfg(test)]
    let application = Arc::clone(&assembly.application);
    let owner: Box<dyn RuntimeShutdownOwner> = Box::new(DesktopLocalBackgroundOwner {
        lifecycle: Arc::clone(&lifecycle),
        transport,
        agent_host: Some(agent_host),
        assembly: Some(assembly),
        data_plane: Some(data_plane),
    });
    Ok(PreparedDesktopLocalRuntime {
        auth,
        protocol,
        lifecycle,
        owner,
        #[cfg(test)]
        application,
        #[cfg(test)]
        pool: test_pool,
    })
}

async fn cleanup_data_plane(
    data_plane: RunningDesktopLocalDataPlane,
    original: DesktopLocalRuntimeError,
) -> DesktopLocalRuntimeError {
    match data_plane.shutdown().await {
        Ok(()) => original,
        Err(_) => DesktopLocalRuntimeError::FailureCleanup,
    }
}

async fn cleanup_assembly(
    data_plane: RunningDesktopLocalDataPlane,
    assembly: PostgresApplicationAssembly,
    original: DesktopLocalRuntimeError,
) -> DesktopLocalRuntimeError {
    assembly.shutdown().await;
    cleanup_data_plane(data_plane, original).await
}

async fn cleanup_agent_host(
    data_plane: RunningDesktopLocalDataPlane,
    assembly: PostgresApplicationAssembly,
    agent_host: DesktopAgentHost,
    original: DesktopLocalRuntimeError,
) -> DesktopLocalRuntimeError {
    agent_host.stop().await;
    cleanup_assembly(data_plane, assembly, original).await
}

fn map_sidecar_error(_error: PostgresSidecarError) -> DesktopLocalRuntimeError {
    DesktopLocalRuntimeError::Sidecar
}

fn map_data_plane_error(_error: DesktopLocalCompositionError) -> DesktopLocalRuntimeError {
    DesktopLocalRuntimeError::DataPlane
}

fn valid_window_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use core::pin::Pin;
    use core::task::{Context as TaskContext, Poll};
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use openbot_application::{AppEventStream, ApplicationService};
    use openbot_contracts::command::{
        AppCommand, AppEvent, AppReply, HealthReport, SubscriptionRequest,
    };
    use openbot_contracts::error::AppError;
    use tauri::WindowEvent;

    use super::*;
    use crate::testing::auth_for;

    static TEST_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    struct EmptyStream(VecDeque<AppEvent>);

    impl futures_core::Stream for EmptyStream {
        type Item = AppEvent;

        fn poll_next(
            mut self: Pin<&mut Self>,
            _context: &mut TaskContext<'_>,
        ) -> Poll<Option<Self::Item>> {
            Poll::Ready(self.0.pop_front())
        }
    }

    struct NoopService;

    #[async_trait]
    impl ApplicationService for NoopService {
        async fn execute(
            &self,
            _auth: AuthContext,
            _command: AppCommand,
        ) -> Result<AppReply, AppError> {
            Ok(AppReply::Health(HealthReport { ok: true }))
        }

        async fn subscribe(
            &self,
            _auth: AuthContext,
            _request: SubscriptionRequest,
        ) -> Result<AppEventStream, AppError> {
            Ok(Box::pin(EmptyStream(VecDeque::new())))
        }
    }

    struct CountingOwner(Arc<AtomicUsize>);

    #[async_trait]
    impl RuntimeShutdownOwner for CountingOwner {
        async fn shutdown(self: Box<Self>) -> Result<(), DesktopLocalRuntimeError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn protocol_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "openbot-tauri-background-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("index.html"),
            "<!doctype html><html lang=\"en\"><head><script type=\"module\" src=\"/openbot-bootstrap.mjs\"></script></head><body></body></html>",
        )
        .unwrap();
        fs::write(root.join("openbot-bootstrap.mjs"), "export {};").unwrap();
        root
    }

    fn lifecycle() -> (
        Arc<DesktopWindowLifecycle>,
        Arc<DesktopTauriProtocol>,
        PathBuf,
    ) {
        let root = protocol_root();
        let transport = Arc::new(InProcessTransport::new(Arc::new(NoopService)));
        let protocol = Arc::new(DesktopTauriProtocol::open(&root, transport).unwrap());
        (
            Arc::new(DesktopWindowLifecycle::new("openbot", Arc::clone(&protocol)).unwrap()),
            protocol,
            root,
        )
    }

    #[test]
    fn phase_fences_exit_until_background_is_terminal() {
        let state = DesktopLocalRuntimeState::new();
        assert_eq!(state.phase(), DesktopLocalRuntimePhase::Initializing);
        assert!(!state.note_exit_requested().unwrap());
        assert_eq!(state.phase(), DesktopLocalRuntimePhase::Initializing);
        state.fail_startup();
        assert_eq!(state.phase(), DesktopLocalRuntimePhase::Failed);
    }

    #[test]
    fn release_label_and_application_network_policy_are_closed() {
        assert!(valid_window_label("main"));
        assert!(!valid_window_label(""));
        assert!(!valid_window_label("main/window"));
        assert!(!valid_window_label(&"a".repeat(65)));

        assert!(
            DesktopLocalApplicationInput::new(
                DesktopOpenAiProviderInput::new(
                    "https://api.example.test/v1",
                    vec!["203.0.113.0/24".to_owned()],
                )
                .unwrap(),
                DesktopAgentBudgets::new(
                    Some(Duration::from_secs(2)),
                    Some(Duration::from_secs(1_800)),
                    16_384,
                )
                .unwrap(),
            )
            .is_ok()
        );
    }

    #[test]
    fn app_data_authority_is_exactly_tauri_path_resolver_output() {
        let app = tauri::test::mock_app();
        let expected = app.path().app_data_dir().unwrap();
        let actual = resolve_current_user_app_data_root(&app).unwrap();
        assert_eq!(actual.as_path(), expected);
    }

    #[test]
    fn external_tauri_context_cannot_precreate_a_window_before_setup() {
        let mut context: Context<tauri::test::MockRuntime> =
            tauri::test::mock_context(tauri::test::noop_assets());
        assert!(validate_tauri_context(&context).is_ok());
        context.config_mut().app.windows.push(Default::default());
        assert!(matches!(
            validate_tauri_context(&context),
            Err(DesktopLocalRuntimeError::Configuration)
        ));
        context.config_mut().app.windows[0].create = false;
        assert!(validate_tauri_context(&context).is_ok());
    }

    #[tokio::test]
    async fn destroyed_last_window_releases_the_single_background_owner() {
        let (lifecycle, protocol, root) = lifecycle();
        let app = tauri::test::mock_app();
        lifecycle
            .create_verified_window(
                &app,
                "main",
                VerifiedDesktopWindowAuthority::from_verified_session(auth_for("actor-1"), None),
            )
            .unwrap();
        assert_eq!(lifecycle.active_window_count(), 1);
        assert!(protocol.is_window_bound("main").unwrap());

        let state = DesktopLocalRuntimeState::new();
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let mut owner: Option<Box<dyn RuntimeShutdownOwner>> =
            Some(Box::new(CountingOwner(Arc::clone(&shutdowns))));
        assert!(
            !state
                .install_owner(&mut owner, Arc::clone(&lifecycle))
                .unwrap()
        );
        assert!(owner.is_none());
        assert!(!state.mark_ready().unwrap());
        assert_eq!(state.phase(), DesktopLocalRuntimePhase::Ready);

        assert!(
            lifecycle
                .handle_window_event("main", &WindowEvent::Destroyed)
                .unwrap()
        );
        assert_eq!(lifecycle.active_window_count(), 0);
        assert!(!protocol.is_window_bound("main").unwrap());
        let owner = state.begin_shutdown().unwrap().unwrap();
        assert_eq!(state.phase(), DesktopLocalRuntimePhase::Stopping);
        owner.shutdown().await.unwrap();
        state.finish_shutdown(true);
        assert_eq!(state.phase(), DesktopLocalRuntimePhase::Stopped);
        assert_eq!(shutdowns.load(Ordering::SeqCst), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
