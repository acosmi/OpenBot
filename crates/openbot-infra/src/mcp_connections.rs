//! PostgreSQL/Vault implementation of MCP OAuth connect, callback and local-first disconnect.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use aes_gcm::aead::{Aead as _, KeyInit as _, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use deadpool_postgres::Pool;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use openbot_application::{
    McpConnectionAdministration, McpConnectionError, McpOAuthCallback, McpOAuthCallbackInput,
    McpOAuthCallbackOutcome,
};
use openbot_contracts::auth::{AuthContext, Role};
use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};
use openbot_contracts::mcp::{
    McpConnection, McpConnectionDisconnected, McpConnections, McpOAuthAuthorization,
    McpOAuthClientAuthMethod, McpOAuthClientRegistered, McpOAuthClientRegistration,
    McpOAuthReturnTo, McpServerMutation, McpVendorRevocationStatus,
};
use openbot_domain::audit::event::{AuditEvent, AuditEventType};
use openbot_domain::audit::payload::{AuditFact, AuditIdentifier, AuditLabel, AuditPayload};
use openbot_domain::vault::{SecretBytes, SecretKind, SecretPrincipal, ServiceId};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use time::{Duration, OffsetDateTime};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use url::Url;
use uuid::Uuid;
use zeroize::{Zeroize as _, Zeroizing};

use crate::google_drive::{
    GOOGLE_DRIVE_API_BASE, GOOGLE_DRIVE_PROVENANCE, GOOGLE_DRIVE_READONLY_SCOPE,
    GOOGLE_DRIVE_SERVER_ID, GOOGLE_DRIVE_TRANSPORT, GOOGLE_DRIVE_VENDOR,
};
use crate::google_drive_oauth::{GoogleDriveOAuthClient, GoogleDriveOAuthError};
use crate::mcp::McpBearerToken;
use crate::mcp_catalog::{
    McpCatalogError, McpCatalogRefresh, PostgresMcpCatalog, VendorTransportKind,
};
use crate::mcp_oauth::{McpOAuthClient, McpOAuthError};
use crate::net::safe_http::SchemePolicy;
use crate::repo::audit::{append_event_in_transaction, next_event_coordinates};
use crate::vault::CredentialRecordVault;

type HmacSha256 = Hmac<Sha256>;

const CALLBACK_PATH: &str = "/api/plugins/oauth/callback";
const ATTEMPT_TTL: Duration = Duration::minutes(10);
const ATTEMPT_LOCK_KEY: i64 = 0x4f50_4d43_504f_4131; // `OPMCPOA1`
// v2 binds the reviewed vendor transport into the sealed one-time attempt. Pre-v2 attempts fail
// closed after an upgrade instead of being reinterpreted under a different protocol adapter.
const ATTEMPT_VERSION: u8 = 2;
const MAX_ATTEMPT_VALUE_BYTES: usize = 32 * 1024;
const DEFAULT_ATTEMPT_CAPACITY: usize = 4096;
const REVOCATION_BATCH: i64 = 32;
const REVOCATION_RETRY_INTERVAL: StdDuration = StdDuration::from_secs(30);

/// Production MCP OAuth connection coordinator.
#[derive(Clone)]
pub struct PostgresMcpConnections {
    pool: Pool,
    vault: CredentialRecordVault,
    oauth: McpOAuthClient,
    drive_oauth: Option<GoogleDriveOAuthClient>,
    catalog: Arc<PostgresMcpCatalog>,
    deployment: DeploymentId,
    tenant: TenantId,
    state_key: Arc<SecretBytes>,
    checkpoint_key: Arc<SecretBytes>,
    tenant_scope: String,
    callback_uri: Option<String>,
    app_origin: Option<String>,
    capacity: usize,
}

/// One pending-revocation sweep summary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct McpRevocationSweep {
    /// Tombstones claimed for this sweep.
    pub attempted: usize,
    /// Vendor revocations confirmed and durably audited.
    pub revoked: usize,
    /// Claims returned to pending for a later retry.
    pub pending: usize,
}

/// Lifecycle handle for periodic pending vendor-revocation reconciliation.
pub struct McpRevocationReconciler {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl McpRevocationReconciler {
    /// Start an immediate sweep followed by bounded 30-second retry intervals.
    #[must_use]
    pub fn start(connections: Arc<PostgresMcpConnections>) -> Self {
        let (stop, stop_rx) = watch::channel(false);
        let task = tokio::spawn(supervise_revocations(connections, stop_rx));
        Self { stop, task }
    }

    /// Stop claiming new tombstones and wait for the current bounded sweep.
    pub async fn stop(self) {
        self.stop.send_replace(true);
        let _ = self.task.await;
    }
}

impl core::fmt::Debug for McpRevocationReconciler {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("McpRevocationReconciler")
            .finish_non_exhaustive()
    }
}

impl PostgresMcpConnections {
    /// Construct from production network, vault, catalog and deployment-owned addresses/keys.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: Pool,
        vault: CredentialRecordVault,
        oauth: McpOAuthClient,
        catalog: Arc<PostgresMcpCatalog>,
        deployment: DeploymentId,
        tenant: TenantId,
        state_key: Vec<u8>,
        checkpoint_key: Vec<u8>,
        public_url: Option<&str>,
        app_url: Option<&str>,
        callback_scheme_policy: SchemePolicy,
    ) -> Result<Self, McpConnectionError> {
        if state_key.len() < 32 || checkpoint_key.is_empty() {
            return Err(McpConnectionError::Corrupt { field: "oauth_key" });
        }
        let callback_uri = public_url
            .map(|value| callback_uri(value, callback_scheme_policy))
            .transpose()?;
        let app_origin = app_url
            .map(validate_app_origin)
            .transpose()?
            .map(|value| value.trim_end_matches('/').to_owned());
        let tenant_scope = hex(&Sha256::digest(
            [
                b"openbot-mcp-oauth-attempt-tenant-v1\0".as_slice(),
                deployment.as_str().as_bytes(),
                b"\0",
                tenant.as_str().as_bytes(),
            ]
            .concat(),
        ));
        Ok(Self {
            pool,
            vault,
            oauth,
            drive_oauth: None,
            catalog,
            deployment,
            tenant,
            state_key: Arc::new(SecretBytes::new(state_key)),
            checkpoint_key: Arc::new(SecretBytes::new(checkpoint_key)),
            tenant_scope,
            callback_uri,
            app_origin,
            capacity: DEFAULT_ATTEMPT_CAPACITY,
        })
    }

    /// Attach the curated Google Drive web-server OAuth adapter. Without it, Drive connect,
    /// callback and vendor-revocation operations fail closed while remote MCP remains available.
    #[must_use]
    pub fn with_google_drive_oauth(mut self, oauth: GoogleDriveOAuthClient) -> Self {
        self.drive_oauth = Some(oauth);
        self
    }

    async fn load_server_client(
        &self,
        server_id: &str,
    ) -> Result<ServerClientMaterial, McpConnectionError> {
        validate_server_id(server_id)?;
        let client = self.pool.get().await.map_err(unavailable)?;
        let row = client
            .query_opt(
                "SELECT s.url,s.vendor,s.provenance,coalesce(s.transport,'mcp') AS transport,
                        s.credential_id,c.kind,c.provider,c.encrypted_value,c.revoked_at
                   FROM public.mcp_servers s
                   LEFT JOIN public.credentials c ON c.id=s.credential_id
                  WHERE s.id=$1",
                &[&server_id],
            )
            .await
            .map_err(query_unavailable)?
            .ok_or(McpConnectionError::NotVisible)?;
        let endpoint: String = row.try_get("url").map_err(|_| corrupt("server_endpoint"))?;
        let vendor: String = row.try_get("vendor").map_err(|_| corrupt("vendor"))?;
        let provenance: String = row
            .try_get("provenance")
            .map_err(|_| corrupt("provenance"))?;
        let transport = VendorTransportKind::parse(
            &row.try_get::<_, String>("transport")
                .map_err(|_| corrupt("transport"))?,
        )
        .map_err(|_| corrupt("transport"))?;
        if transport == VendorTransportKind::GoogleDriveRest
            && (server_id != GOOGLE_DRIVE_SERVER_ID
                || endpoint != GOOGLE_DRIVE_API_BASE
                || vendor != GOOGLE_DRIVE_VENDOR
                || provenance != GOOGLE_DRIVE_PROVENANCE)
        {
            return Err(corrupt("transport_identity"));
        }
        let credential_id: Option<Uuid> = row
            .try_get("credential_id")
            .map_err(|_| corrupt("oauth_client_id"))?;
        let kind: Option<crate::db::types::CredentialKind> = row
            .try_get("kind")
            .map_err(|_| corrupt("oauth_client_kind"))?;
        let provider: Option<String> = row
            .try_get("provider")
            .map_err(|_| corrupt("oauth_client_provider"))?;
        let encrypted: Option<String> = row
            .try_get("encrypted_value")
            .map_err(|_| corrupt("oauth_client_value"))?;
        let revoked: Option<OffsetDateTime> = row
            .try_get("revoked_at")
            .map_err(|_| corrupt("oauth_client_revoked_at"))?;
        let credential_id = credential_id.ok_or(McpConnectionError::Conflict {
            resource: "mcp_oauth_client",
        })?;
        if kind != Some(crate::db::types::CredentialKind::McpOauthClient)
            || provider.as_deref() != Some(server_id)
            || revoked.is_some()
        {
            return Err(McpConnectionError::Conflict {
                resource: "mcp_oauth_client",
            });
        }
        let secret = self
            .vault
            .open(
                &credential_id,
                SecretKind::McpOauthClient,
                SecretPrincipal::Deployment,
                SecretPrincipal::Service(ServiceId::new(server_id)),
                encrypted
                    .as_deref()
                    .ok_or_else(|| corrupt("oauth_client_value"))?,
            )
            .map_err(|error| {
                tracing::error!(code = %error, "MCP OAuth client 密文被拒");
                corrupt("oauth_client_value")
            })?
            .into_secret();
        Ok(ServerClientMaterial {
            credential_id,
            endpoint,
            client: secret,
            transport,
        })
    }

    async fn ensure_auth_current(&self, auth: &AuthContext) -> Result<(), McpConnectionError> {
        if auth.deployment() != &self.deployment || auth.tenant() != &self.tenant {
            return Err(McpConnectionError::NotVisible);
        }
        let generation =
            i64::try_from(auth.auth_generation().get()).map_err(|_| corrupt("auth_generation"))?;
        let client = self.pool.get().await.map_err(unavailable)?;
        let current: bool = client
            .query_one(
                "SELECT EXISTS(
                    SELECT 1 FROM public.users u
                     WHERE u.id=$1 AND coalesce(u.auth_generation,0)=$2
                       AND EXISTS(SELECT 1 FROM public.user_roles ur WHERE ur.user_id=u.id)
                       AND NOT EXISTS(SELECT 1 FROM public.revoked_access ra
                                       WHERE ra.email=lower(u.email)))",
                &[&auth.actor().as_str(), &generation],
            )
            .await
            .map_err(query_unavailable)?
            .try_get(0)
            .map_err(|_| corrupt("actor_scope"))?;
        if current {
            Ok(())
        } else {
            Err(McpConnectionError::NotVisible)
        }
    }

    async fn insert_attempt(
        &self,
        state: &str,
        attempt: &StoredAttempt,
    ) -> Result<(), McpConnectionError> {
        let identifier = self.state_identifier(state)?;
        let value = self.seal_attempt(attempt)?;
        let expires_at = OffsetDateTime::from_unix_timestamp(attempt.expires_at_unix_seconds)
            .map_err(|_| corrupt("attempt_expiry"))?;
        let mut client = self.pool.get().await.map_err(unavailable)?;
        let transaction = client.transaction().await.map_err(query_unavailable)?;
        transaction
            .query_one("SELECT pg_advisory_xact_lock($1)", &[&ATTEMPT_LOCK_KEY])
            .await
            .map_err(query_unavailable)?;
        let now: OffsetDateTime = transaction
            .query_one("SELECT clock_timestamp()", &[])
            .await
            .map_err(query_unavailable)?
            .try_get(0)
            .map_err(|_| corrupt("clock"))?;
        let prefix = format!("mcp-oauth-attempt:{}:%", self.tenant_scope);
        transaction
            .execute(
                "DELETE FROM public.verifications WHERE identifier LIKE $1 AND expires_at<=$2",
                &[&prefix, &now],
            )
            .await
            .map_err(query_unavailable)?;
        let count: i64 = transaction
            .query_one(
                "SELECT count(*)::bigint FROM public.verifications WHERE identifier LIKE $1",
                &[&prefix],
            )
            .await
            .map_err(query_unavailable)?
            .try_get(0)
            .map_err(|_| corrupt("attempt_count"))?;
        if usize::try_from(count).map_err(|_| corrupt("attempt_count"))? >= self.capacity {
            return Err(McpConnectionError::Conflict {
                resource: "mcp_oauth_attempt_capacity",
            });
        }
        let current: bool = transaction
            .query_one(
                "SELECT EXISTS(
                    SELECT 1 FROM public.users u
                    JOIN public.mcp_servers s ON s.id=$3
                     WHERE u.id=$1 AND coalesce(u.auth_generation,0)=$2
                       AND s.url=$4 AND s.credential_id=$5
                       AND coalesce(s.transport,'mcp')=$6
                       AND EXISTS(SELECT 1 FROM public.user_roles ur WHERE ur.user_id=u.id)
                       AND NOT EXISTS(SELECT 1 FROM public.revoked_access ra
                                       WHERE ra.email=lower(u.email)))",
                &[
                    &attempt.actor_id,
                    &attempt.auth_generation,
                    &attempt.server_id,
                    &attempt.resource,
                    &attempt.client_credential_id,
                    &attempt.transport,
                ],
            )
            .await
            .map_err(query_unavailable)?
            .try_get(0)
            .map_err(|_| corrupt("attempt_scope"))?;
        if !current {
            return Err(McpConnectionError::NotVisible);
        }
        let id = Uuid::now_v7().to_string();
        transaction
            .execute(
                "INSERT INTO public.verifications(id,identifier,value,expires_at,created_at,updated_at)
                 VALUES($1,$2,$3,$4,$5,$5)",
                &[&id, &identifier, &value, &expires_at, &now],
            )
            .await
            .map_err(query_unavailable)?;
        transaction.commit().await.map_err(|error| {
            tracing::error!(error = %error, "MCP OAuth attempt commit 结果未知");
            McpConnectionError::CommitUnknown
        })
    }

    async fn consume_attempt(&self, state: &[u8]) -> Result<StoredAttempt, McpConnectionError> {
        let state = core::str::from_utf8(state).map_err(|_| McpConnectionError::NotVisible)?;
        let identifier = self.state_identifier(state)?;
        let mut client = self.pool.get().await.map_err(unavailable)?;
        let transaction = client.transaction().await.map_err(query_unavailable)?;
        let rows = transaction
            .query(
                "DELETE FROM public.verifications WHERE identifier=$1
                 RETURNING value,expires_at,clock_timestamp() AS consumed_at",
                &[&identifier],
            )
            .await
            .map_err(query_unavailable)?;
        transaction.commit().await.map_err(|error| {
            tracing::error!(error = %error, "MCP OAuth state consume commit 结果未知");
            McpConnectionError::CommitUnknown
        })?;
        if rows.len() != 1 {
            return Err(McpConnectionError::NotVisible);
        }
        let value: String = rows[0]
            .try_get("value")
            .map_err(|_| corrupt("attempt_value"))?;
        let column_expiry: OffsetDateTime = rows[0]
            .try_get("expires_at")
            .map_err(|_| corrupt("attempt_expiry"))?;
        let consumed_at: OffsetDateTime = rows[0]
            .try_get("consumed_at")
            .map_err(|_| corrupt("clock"))?;
        let attempt = self.open_attempt(&value)?;
        if attempt.version != ATTEMPT_VERSION
            || attempt.deployment_id != self.deployment.as_str()
            || attempt.tenant_id != self.tenant.as_str()
            || self.callback_uri.as_deref() != Some(attempt.redirect_uri.as_str())
            || attempt.expires_at_unix_seconds != column_expiry.unix_timestamp()
            || attempt_is_expired(attempt.expires_at_unix_seconds, consumed_at)
            || !valid_server_id(&attempt.server_id)
            || !valid_pkce(&attempt.code_verifier)
        {
            return Err(corrupt("attempt"));
        }
        Ok(attempt)
    }

    fn state_identifier(&self, state: &str) -> Result<String, McpConnectionError> {
        if state.is_empty()
            || state.len() > 512
            || !state
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(McpConnectionError::NotVisible);
        }
        let mut mac = <HmacSha256 as Mac>::new_from_slice(self.state_key.expose())
            .map_err(|_| corrupt("state_key"))?;
        mac.update(b"openbot-mcp-oauth-state-v1\0");
        mac.update(self.tenant_scope.as_bytes());
        mac.update(b"\0");
        mac.update(state.as_bytes());
        Ok(format!(
            "mcp-oauth-attempt:{}:{}",
            self.tenant_scope,
            hex(&mac.finalize().into_bytes())
        ))
    }

    fn seal_attempt(&self, attempt: &StoredAttempt) -> Result<String, McpConnectionError> {
        let key = self.attempt_encryption_key()?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| corrupt("state_key"))?;
        let mut nonce = [0_u8; 12];
        getrandom::fill(&mut nonce).map_err(|_| McpConnectionError::Unavailable)?;
        let plaintext =
            Zeroizing::new(serde_json::to_vec(attempt).map_err(|_| corrupt("attempt_value"))?);
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: self.tenant_scope.as_bytes(),
                },
            )
            .map_err(|_| corrupt("attempt_value"))?;
        let mut encoded = Vec::with_capacity(nonce.len() + ciphertext.len());
        encoded.extend_from_slice(&nonce);
        encoded.extend_from_slice(&ciphertext);
        Ok(URL_SAFE_NO_PAD.encode(encoded))
    }

    fn open_attempt(&self, value: &str) -> Result<StoredAttempt, McpConnectionError> {
        if value.is_empty() || value.len() > MAX_ATTEMPT_VALUE_BYTES {
            return Err(corrupt("attempt_value"));
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| corrupt("attempt_value"))?;
        if decoded.len() <= 12 {
            return Err(corrupt("attempt_value"));
        }
        let key = self.attempt_encryption_key()?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| corrupt("state_key"))?;
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&decoded[..12]),
                    Payload {
                        msg: &decoded[12..],
                        aad: self.tenant_scope.as_bytes(),
                    },
                )
                .map_err(|_| corrupt("attempt_value"))?,
        );
        serde_json::from_slice(&plaintext).map_err(|_| corrupt("attempt_value"))
    }

    fn attempt_encryption_key(&self) -> Result<Zeroizing<[u8; 32]>, McpConnectionError> {
        let mut key = Zeroizing::new([0_u8; 32]);
        Hkdf::<Sha256>::new(
            Some(b"openbot-mcp-oauth-attempt-aead-v1"),
            self.state_key.expose(),
        )
        .expand(self.tenant_scope.as_bytes(), &mut *key)
        .map_err(|_| corrupt("state_key"))?;
        Ok(key)
    }

    async fn complete_inner(
        &self,
        input: &McpOAuthCallbackInput,
    ) -> Result<String, McpConnectionError> {
        // State is burned before code/issuer validation or any vendor request.
        let attempt = self.consume_attempt(input.state()).await?;
        if input.code().is_empty() {
            return Err(McpConnectionError::VendorFailure);
        }
        match input.issuer() {
            Some(issuer) if issuer == attempt.issuer => {}
            Some(_) => return Err(McpConnectionError::VendorFailure),
            None if attempt.issuer_required => return Err(McpConnectionError::VendorFailure),
            None => {}
        }
        self.ensure_stored_actor_current(&attempt).await?;
        let material = self.load_server_client(&attempt.server_id).await?;
        if material.credential_id != attempt.client_credential_id
            || material.endpoint != attempt.resource
            || material.transport.as_str() != attempt.transport
        {
            return Err(McpConnectionError::NotVisible);
        }
        let (access, refresh, scope) = match material.transport {
            VendorTransportKind::Mcp => self
                .oauth
                .exchange_authorization_code(
                    &material.endpoint,
                    material.client.expose(),
                    input.code(),
                    &attempt.redirect_uri,
                    attempt.code_verifier.as_bytes(),
                    &attempt.requested_scope,
                )
                .await
                .map_err(map_oauth_vendor)?
                .into_parts(),
            VendorTransportKind::GoogleDriveRest => self
                .drive_oauth
                .as_ref()
                .ok_or(McpConnectionError::Conflict {
                    resource: "google_drive_oauth_runtime",
                })?
                .exchange_authorization_code(
                    material.client.expose(),
                    input.code(),
                    &attempt.redirect_uri,
                    attempt.code_verifier.as_bytes(),
                )
                .await
                .map_err(map_drive_oauth_vendor)?
                .into_parts(),
        };
        let old = self
            .persist_connection(&attempt, &material, &refresh, &scope)
            .await?;

        // Catalog failure does not roll back a valid connection; it remains invisible until a later
        // refresh rather than lying that consent failed after the credential transaction committed.
        let catalog_bearer = match material.transport {
            VendorTransportKind::Mcp => McpBearerToken::from_secret(access).map(Some).ok(),
            VendorTransportKind::GoogleDriveRest => Some(None),
        };
        if let Some(bearer) = catalog_bearer
            && let Err(error) = self.catalog.refresh(&attempt.server_id, bearer).await
        {
            tracing::warn!(code = %error, "MCP OAuth callback 后 catalog refresh 失败");
        }
        if let Some(old) = old {
            self.try_vendor_revoke(&attempt.actor_id, &attempt.server_id, &material, old)
                .await;
        }
        Ok(self.success_redirect(&attempt.server_id, attempt.return_to))
    }

    async fn ensure_stored_actor_current(
        &self,
        attempt: &StoredAttempt,
    ) -> Result<(), McpConnectionError> {
        let client = self.pool.get().await.map_err(unavailable)?;
        let current: bool = client
            .query_one(
                "SELECT EXISTS(
                    SELECT 1 FROM public.users u
                     WHERE u.id=$1 AND coalesce(u.auth_generation,0)=$2
                       AND EXISTS(SELECT 1 FROM public.user_roles ur WHERE ur.user_id=u.id)
                       AND NOT EXISTS(SELECT 1 FROM public.revoked_access ra
                                       WHERE ra.email=lower(u.email)))",
                &[&attempt.actor_id, &attempt.auth_generation],
            )
            .await
            .map_err(query_unavailable)?
            .try_get(0)
            .map_err(|_| corrupt("actor_scope"))?;
        if current {
            Ok(())
        } else {
            Err(McpConnectionError::NotVisible)
        }
    }

    async fn persist_connection(
        &self,
        attempt: &StoredAttempt,
        material: &ServerClientMaterial,
        refresh: &SecretBytes,
        scope: &str,
    ) -> Result<Option<OldRefresh>, McpConnectionError> {
        if scope.len() > 16 * 1024 || scope.as_bytes().contains(&0) {
            return Err(corrupt("scope"));
        }
        if material.transport == VendorTransportKind::GoogleDriveRest
            && scope != GOOGLE_DRIVE_READONLY_SCOPE
        {
            return Err(corrupt("scope"));
        }
        let credential_id = Uuid::now_v7();
        let actor = ActorId::new(&attempt.actor_id);
        let encrypted = self
            .vault
            .seal(
                &credential_id,
                SecretKind::McpUserToken,
                SecretPrincipal::Actor(actor.clone()),
                SecretPrincipal::Service(ServiceId::new(&attempt.server_id)),
                refresh,
            )
            .map_err(|_| corrupt("refresh_token"))?;
        let mut client = self.pool.get().await.map_err(unavailable)?;
        let transaction = client.transaction().await.map_err(query_unavailable)?;
        let now: OffsetDateTime = transaction
            .query_one("SELECT clock_timestamp()", &[])
            .await
            .map_err(query_unavailable)?
            .try_get(0)
            .map_err(|_| corrupt("clock"))?;
        let current: bool = transaction
            .query_one(
                "SELECT EXISTS(
                    SELECT 1 FROM public.users u JOIN public.mcp_servers s ON s.id=$3
                     WHERE u.id=$1 AND coalesce(u.auth_generation,0)=$2
                       AND s.url=$4 AND s.credential_id=$5
                       AND coalesce(s.transport,'mcp')=$6
                       AND EXISTS(SELECT 1 FROM public.user_roles ur WHERE ur.user_id=u.id)
                       AND NOT EXISTS(SELECT 1 FROM public.revoked_access ra
                                       WHERE ra.email=lower(u.email)))",
                &[
                    &attempt.actor_id,
                    &attempt.auth_generation,
                    &attempt.server_id,
                    &attempt.resource,
                    &attempt.client_credential_id,
                    &attempt.transport,
                ],
            )
            .await
            .map_err(query_unavailable)?
            .try_get(0)
            .map_err(|_| corrupt("callback_scope"))?;
        if !current {
            return Err(McpConnectionError::NotVisible);
        }
        let old_row = transaction
            .query_opt(
                "SELECT uc.credential_id,c.kind,c.provider,c.key_id,
                        c.encrypted_value,c.revoked_at
                   FROM public.mcp_user_credentials uc
                   JOIN public.credentials c ON c.id=uc.credential_id
                  WHERE uc.server_id=$1 AND uc.user_id=$2 FOR UPDATE OF uc,c",
                &[&attempt.server_id, &attempt.actor_id],
            )
            .await
            .map_err(query_unavailable)?;
        let old = if let Some(row) = old_row {
            let old_id: Uuid = row
                .try_get("credential_id")
                .map_err(|_| corrupt("old_credential_id"))?;
            let old_encrypted: String = row
                .try_get("encrypted_value")
                .map_err(|_| corrupt("old_credential_value"))?;
            let old_kind: crate::db::types::CredentialKind = row
                .try_get("kind")
                .map_err(|_| corrupt("old_credential_kind"))?;
            let old_provider: String = row
                .try_get("provider")
                .map_err(|_| corrupt("old_credential_provider"))?;
            let old_owner: String = row
                .try_get("key_id")
                .map_err(|_| corrupt("old_credential_owner"))?;
            let old_revoked: Option<OffsetDateTime> = row
                .try_get("revoked_at")
                .map_err(|_| corrupt("old_credential_revoked_at"))?;
            if old_revoked.is_some()
                || old_kind != crate::db::types::CredentialKind::McpUserToken
                || old_provider != attempt.server_id
                || old_owner != attempt.actor_id
            {
                return Err(corrupt("old_credential_binding"));
            }
            let old_refresh = self
                .vault
                .open(
                    &old_id,
                    SecretKind::McpUserToken,
                    SecretPrincipal::Actor(actor.clone()),
                    SecretPrincipal::Service(ServiceId::new(&attempt.server_id)),
                    &old_encrypted,
                )
                .map_err(|_| corrupt("old_credential_value"))?
                .into_secret();
            Some(OldRefresh {
                credential_id: old_id,
                refresh: old_refresh,
            })
        } else {
            None
        };
        let metadata = serde_json::json!({
            "server":attempt.server_id,
            "scope":scope,
            "resource":attempt.resource,
            "issuer":attempt.issuer,
            "transport":attempt.transport,
            "credential_generation":1,
            "revocation_status":"active"
        });
        transaction
            .execute(
                "INSERT INTO public.credentials(
                   id,kind,provider,encrypted_value,key_id,metadata,created_at,updated_at
                 ) VALUES($1,'mcp_user_token',$2,$3,$4,$5,$6,$6)",
                &[
                    &credential_id,
                    &attempt.server_id,
                    &encrypted,
                    &attempt.actor_id,
                    &metadata,
                    &now,
                ],
            )
            .await
            .map_err(query_unavailable)?;
        transaction
            .execute(
                "INSERT INTO public.mcp_user_credentials(
                   server_id,user_id,credential_id,scope,connected_at,updated_at
                 ) VALUES($1,$2,$3,$4,$5,$5)
                 ON CONFLICT(server_id,user_id) DO UPDATE SET
                   credential_id=excluded.credential_id,scope=excluded.scope,
                   connected_at=excluded.connected_at,updated_at=excluded.updated_at",
                &[
                    &attempt.server_id,
                    &attempt.actor_id,
                    &credential_id,
                    &scope,
                    &now,
                ],
            )
            .await
            .map_err(query_unavailable)?;
        if let Some(old) = &old {
            transaction
                .execute(
                    "UPDATE public.credentials SET revoked_at=$2,updated_at=$2,
                       metadata=coalesce(metadata,'{}'::jsonb)||
                                jsonb_build_object('revocation_status','pending',
                                                   'revocation_reason','reconnected')
                     WHERE id=$1 AND revoked_at IS NULL",
                    &[&old.credential_id, &now],
                )
                .await
                .map_err(query_unavailable)?;
        }
        self.append_connection_audit(
            &transaction,
            &actor,
            &attempt.server_id,
            "mcp.account_connected",
            None,
        )
        .await?;
        transaction.commit().await.map_err(|error| {
            tracing::error!(error = %error, "MCP OAuth connection commit 结果未知");
            McpConnectionError::CommitUnknown
        })?;
        Ok(old)
    }

    async fn append_connection_audit(
        &self,
        transaction: &tokio_postgres::Transaction<'_>,
        actor: &ActorId,
        server_id: &str,
        event_type: &'static str,
        revocation: Option<(AuditLabel, bool)>,
    ) -> Result<(), McpConnectionError> {
        let mut facts = vec![AuditFact::CredentialOwner(
            AuditIdentifier::new(actor.as_str()).map_err(|_| corrupt("actor_id"))?,
        )];
        if let Some((reason, vendor_revoked)) = revocation {
            facts.push(AuditFact::RevocationReason(reason));
            facts.push(AuditFact::VendorRevoked(vendor_revoked));
        }
        let payload = AuditPayload::from_facts(facts).map_err(|_| corrupt("audit_payload"))?;
        let (id, created_at) = next_event_coordinates(transaction)
            .await
            .map_err(query_unavailable)?;
        let event = AuditEvent {
            id,
            actor: Some(actor.clone()),
            event_type: AuditEventType::parse(event_type).ok_or_else(|| corrupt("audit_event"))?,
            target_kind: AuditLabel::new("mcp_server"),
            target_id: Some(AuditIdentifier::new(server_id).map_err(|_| corrupt("server_id"))?),
            payload,
            created_at,
        };
        append_event_in_transaction(transaction, &event, self.checkpoint_key.expose())
            .await
            .map(|_| ())
            .map_err(query_unavailable)
    }

    async fn try_vendor_revoke(
        &self,
        actor_id: &str,
        server_id: &str,
        material: &ServerClientMaterial,
        old: OldRefresh,
    ) -> bool {
        let revoked = match material.transport {
            VendorTransportKind::Mcp => self
                .oauth
                .revoke_refresh_token(
                    &material.endpoint,
                    material.client.expose(),
                    old.refresh.expose(),
                )
                .await
                .is_ok(),
            VendorTransportKind::GoogleDriveRest => match &self.drive_oauth {
                Some(oauth) => oauth
                    .revoke_refresh_token(old.refresh.expose())
                    .await
                    .is_ok(),
                None => false,
            },
        };
        if !revoked {
            return false;
        }
        self.mark_vendor_revoked(actor_id, server_id, old.credential_id)
            .await
            .is_ok()
    }

    async fn mark_vendor_revoked(
        &self,
        actor_id: &str,
        server_id: &str,
        credential_id: Uuid,
    ) -> Result<(), McpConnectionError> {
        let actor = ActorId::new(actor_id);
        let mut client = self.pool.get().await.map_err(unavailable)?;
        let transaction = client.transaction().await.map_err(query_unavailable)?;
        let updated = transaction
            .execute(
                "UPDATE public.credentials SET
                   metadata=coalesce(metadata,'{}'::jsonb)||
                            jsonb_build_object('revocation_status','revoked',
                                               'vendor_revoked_at',clock_timestamp()),
                   updated_at=clock_timestamp()
                 WHERE id=$1 AND revoked_at IS NOT NULL
                   AND metadata->>'revocation_status' IN ('pending','revoking')",
                &[&credential_id],
            )
            .await
            .map_err(query_unavailable)?;
        if updated != 1 {
            return Err(McpConnectionError::NotVisible);
        }
        self.append_connection_audit(
            &transaction,
            &actor,
            server_id,
            "mcp.account_disconnected",
            Some((AuditLabel::new("vendor_revoke_confirmed"), true)),
        )
        .await?;
        transaction.commit().await.map_err(|error| {
            tracing::error!(error = %error, "vendor revoke confirmation commit 结果未知");
            McpConnectionError::CommitUnknown
        })
    }

    fn success_redirect(&self, server_id: &str, return_to: McpOAuthReturnTo) -> String {
        let origin = self.app_origin.as_deref().unwrap_or_default();
        match return_to {
            McpOAuthReturnTo::Admin => format!("{origin}/admin/plugins/{server_id}"),
            McpOAuthReturnTo::Settings => {
                format!("{origin}/settings/connected-accounts/{server_id}")
            }
        }
    }

    fn failure_redirect(&self) -> String {
        let origin = self.app_origin.as_deref().unwrap_or_default();
        format!("{origin}/settings/connected-accounts?connected=failed")
    }

    /// Claim and reconcile a bounded batch of local tombstones. Vendor revocation is idempotent;
    /// local access is already absent regardless of this method's outcome.
    pub async fn reconcile_pending_revocations(
        &self,
    ) -> Result<McpRevocationSweep, McpConnectionError> {
        let claims = self.claim_pending_revocations().await?;
        let mut sweep = McpRevocationSweep {
            attempted: claims.len(),
            ..McpRevocationSweep::default()
        };
        for claim in claims {
            let actor = ActorId::new(&claim.actor_id);
            let refresh = self
                .vault
                .open(
                    &claim.credential_id,
                    SecretKind::McpUserToken,
                    SecretPrincipal::Actor(actor),
                    SecretPrincipal::Service(ServiceId::new(&claim.server_id)),
                    &claim.encrypted_value,
                )
                .ok()
                .map(|value| value.into_secret());
            let material = self.load_server_client(&claim.server_id).await.ok();
            let revoked = match (refresh, material) {
                (Some(refresh), Some(material)) => {
                    self.try_vendor_revoke(
                        &claim.actor_id,
                        &claim.server_id,
                        &material,
                        OldRefresh {
                            credential_id: claim.credential_id,
                            refresh,
                        },
                    )
                    .await
                }
                _ => false,
            };
            if revoked {
                sweep.revoked = sweep.revoked.saturating_add(1);
            } else {
                self.return_revocation_pending(claim.credential_id).await?;
                sweep.pending = sweep.pending.saturating_add(1);
            }
        }
        Ok(sweep)
    }

    async fn claim_pending_revocations(
        &self,
    ) -> Result<Vec<PendingRevocation>, McpConnectionError> {
        let mut client = self.pool.get().await.map_err(unavailable)?;
        let transaction = client.transaction().await.map_err(query_unavailable)?;
        let rows = transaction
            .query(
                "WITH candidates AS (
                    SELECT id FROM public.credentials
                     WHERE kind='mcp_user_token' AND revoked_at IS NOT NULL
                       AND ((metadata->>'revocation_status'='pending' AND
                             updated_at<clock_timestamp()-interval '30 seconds') OR
                            (metadata->>'revocation_status'='revoking' AND
                             updated_at<clock_timestamp()-interval '2 minutes'))
                     ORDER BY updated_at,id
                     LIMIT $1 FOR UPDATE SKIP LOCKED
                 )
                 UPDATE public.credentials c SET
                   metadata=coalesce(c.metadata,'{}'::jsonb)||
                            jsonb_build_object(
                              'revocation_status','revoking',
                              'revocation_attempts',
                                CASE WHEN coalesce(c.metadata->>'revocation_attempts','')
                                               ~ '^[0-9]{1,17}$'
                                     THEN (c.metadata->>'revocation_attempts')::bigint+1
                                     ELSE 1 END),
                   updated_at=clock_timestamp()
                  FROM candidates WHERE c.id=candidates.id
                 RETURNING c.id,c.provider,c.key_id,c.encrypted_value",
                &[&REVOCATION_BATCH],
            )
            .await
            .map_err(query_unavailable)?;
        let claims = rows
            .iter()
            .map(|row| {
                Ok(PendingRevocation {
                    credential_id: row.try_get("id").map_err(|_| corrupt("credential_id"))?,
                    server_id: row
                        .try_get("provider")
                        .map_err(|_| corrupt("credential_provider"))?,
                    actor_id: row
                        .try_get("key_id")
                        .map_err(|_| corrupt("credential_owner"))?,
                    encrypted_value: row
                        .try_get("encrypted_value")
                        .map_err(|_| corrupt("credential_value"))?,
                })
            })
            .collect::<Result<Vec<_>, McpConnectionError>>()?;
        transaction.commit().await.map_err(|error| {
            tracing::error!(error = %error, "pending vendor revoke claim commit 结果未知");
            McpConnectionError::CommitUnknown
        })?;
        Ok(claims)
    }

    async fn return_revocation_pending(
        &self,
        credential_id: Uuid,
    ) -> Result<(), McpConnectionError> {
        self.pool
            .get()
            .await
            .map_err(unavailable)?
            .execute(
                "UPDATE public.credentials SET
                   metadata=coalesce(metadata,'{}'::jsonb)||
                            jsonb_build_object('revocation_status','pending'),
                   updated_at=clock_timestamp()
                 WHERE id=$1 AND revoked_at IS NOT NULL
                   AND metadata->>'revocation_status'='revoking'",
                &[&credential_id],
            )
            .await
            .map(|_| ())
            .map_err(query_unavailable)
    }
}

#[async_trait]
impl McpConnectionAdministration for PostgresMcpConnections {
    async fn list_connections(
        &self,
        auth: &AuthContext,
    ) -> Result<McpConnections, McpConnectionError> {
        self.ensure_auth_current(auth).await?;
        let client = self.pool.get().await.map_err(unavailable)?;
        let rows = client
            .query(
                "SELECT uc.server_id,uc.scope,uc.connected_at
                   FROM public.mcp_user_credentials uc
                   JOIN public.credentials c ON c.id=uc.credential_id
                  WHERE uc.user_id=$1 AND c.kind='mcp_user_token'
                    AND c.provider=uc.server_id AND c.key_id=$1 AND c.revoked_at IS NULL
                  ORDER BY uc.server_id",
                &[&auth.actor().as_str()],
            )
            .await
            .map_err(query_unavailable)?;
        let connections = rows
            .iter()
            .map(|row| {
                Ok(McpConnection {
                    server_id: row.try_get("server_id").map_err(|_| corrupt("server_id"))?,
                    scope: row.try_get("scope").map_err(|_| corrupt("scope"))?,
                    connected_at: row
                        .try_get("connected_at")
                        .map_err(|_| corrupt("connected_at"))?,
                })
            })
            .collect::<Result<Vec<_>, McpConnectionError>>()?;
        let reviewed = client
            .query_opt(
                "SELECT url,vendor,provenance,transport
                   FROM public.mcp_servers WHERE id=$1",
                &[&GOOGLE_DRIVE_SERVER_ID],
            )
            .await
            .map_err(query_unavailable)?;
        let available_server_ids = if let Some(row) = reviewed {
            let endpoint: String = row.try_get("url").map_err(|_| corrupt("server_endpoint"))?;
            let vendor: String = row.try_get("vendor").map_err(|_| corrupt("vendor"))?;
            let provenance: String = row
                .try_get("provenance")
                .map_err(|_| corrupt("provenance"))?;
            let transport: Option<String> =
                row.try_get("transport").map_err(|_| corrupt("transport"))?;
            if endpoint != GOOGLE_DRIVE_API_BASE
                || vendor != GOOGLE_DRIVE_VENDOR
                || provenance != GOOGLE_DRIVE_PROVENANCE
                || transport.as_deref() != Some(GOOGLE_DRIVE_TRANSPORT)
            {
                return Err(corrupt("reviewed_server_identity"));
            }
            vec![GOOGLE_DRIVE_SERVER_ID.to_owned()]
        } else {
            Vec::new()
        };
        Ok(McpConnections {
            available_server_ids,
            connections,
            redirect_uri: self.callback_uri.clone(),
        })
    }

    async fn begin_oauth(
        &self,
        auth: &AuthContext,
        server_id: &str,
        return_to: McpOAuthReturnTo,
    ) -> Result<McpOAuthAuthorization, McpConnectionError> {
        self.ensure_auth_current(auth).await?;
        let callback_uri = self
            .callback_uri
            .as_deref()
            .ok_or(McpConnectionError::Conflict {
                resource: "mcp_oauth_public_callback",
            })?;
        let material = self.load_server_client(server_id).await?;
        let state = random_base64::<32>()?;
        let verifier = random_base64::<48>()?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let (authorization_url, issuer, issuer_required, requested_scope) = match material.transport
        {
            VendorTransportKind::Mcp => {
                let plan = self
                    .oauth
                    .authorization_plan(
                        &material.endpoint,
                        material.client.expose(),
                        callback_uri,
                        &state,
                        &challenge,
                    )
                    .await
                    .map_err(map_oauth_vendor)?;
                (
                    plan.authorization_url().to_string(),
                    plan.issuer().to_owned(),
                    plan.requires_callback_issuer(),
                    plan.requested_scope().to_owned(),
                )
            }
            VendorTransportKind::GoogleDriveRest => {
                let plan = self
                    .drive_oauth
                    .as_ref()
                    .ok_or(McpConnectionError::Conflict {
                        resource: "google_drive_oauth_runtime",
                    })?
                    .authorization_plan(material.client.expose(), callback_uri, &state, &challenge)
                    .map_err(map_drive_oauth_vendor)?;
                (
                    plan.authorization_url().to_string(),
                    plan.issuer().to_owned(),
                    false,
                    GOOGLE_DRIVE_READONLY_SCOPE.to_owned(),
                )
            }
        };
        let now = database_now(&self.pool).await?;
        let auth_generation =
            i64::try_from(auth.auth_generation().get()).map_err(|_| corrupt("auth_generation"))?;
        let attempt = StoredAttempt {
            version: ATTEMPT_VERSION,
            deployment_id: self.deployment.as_str().to_owned(),
            tenant_id: self.tenant.as_str().to_owned(),
            actor_id: auth.actor().as_str().to_owned(),
            auth_generation,
            server_id: server_id.to_owned(),
            client_credential_id: material.credential_id,
            resource: material.endpoint,
            transport: material.transport.as_str().to_owned(),
            code_verifier: verifier,
            redirect_uri: callback_uri.to_owned(),
            issuer,
            issuer_required,
            requested_scope,
            return_to,
            expires_at_unix_seconds: (now + ATTEMPT_TTL).unix_timestamp(),
        };
        self.insert_attempt(&state, &attempt).await?;
        Ok(McpOAuthAuthorization { authorization_url })
    }

    async fn disconnect(
        &self,
        auth: &AuthContext,
        server_id: &str,
    ) -> Result<McpConnectionDisconnected, McpConnectionError> {
        self.ensure_auth_current(auth).await?;
        validate_server_id(server_id)?;
        let candidate = self.disconnect_candidate(auth.actor(), server_id).await?;
        self.tombstone_connection(auth, server_id, candidate.credential_id)
            .await?;
        let vendor_revocation = match (candidate.client, candidate.refresh) {
            (Some(client), Some(refresh)) => {
                let material = ServerClientMaterial {
                    credential_id: candidate.client_credential_id.unwrap_or(Uuid::nil()),
                    endpoint: candidate.endpoint,
                    client,
                    transport: candidate.transport,
                };
                if self
                    .try_vendor_revoke(
                        auth.actor().as_str(),
                        server_id,
                        &material,
                        OldRefresh {
                            credential_id: candidate.credential_id,
                            refresh,
                        },
                    )
                    .await
                {
                    McpVendorRevocationStatus::Revoked
                } else {
                    McpVendorRevocationStatus::Pending
                }
            }
            _ => McpVendorRevocationStatus::Pending,
        };
        Ok(McpConnectionDisconnected {
            server_id: server_id.to_owned(),
            vendor_revocation,
        })
    }

    async fn register_oauth_client(
        &self,
        auth: &AuthContext,
        server_id: &str,
        registration: &McpOAuthClientRegistration,
    ) -> Result<McpOAuthClientRegistered, McpConnectionError> {
        self.ensure_auth_current(auth).await?;
        if !auth.has_role(Role::Admin) {
            return Err(McpConnectionError::NotVisible);
        }
        validate_server_id(server_id)?;
        let client = self.pool.get().await.map_err(unavailable)?;
        let server = client
            .query_opt(
                "SELECT url,vendor,provenance,coalesce(transport,'mcp') AS transport
                   FROM public.mcp_servers WHERE id=$1",
                &[&server_id],
            )
            .await
            .map_err(query_unavailable)?
            .ok_or(McpConnectionError::NotVisible)?;
        let endpoint: String = server
            .try_get("url")
            .map_err(|_| corrupt("server_endpoint"))?;
        let vendor: String = server.try_get("vendor").map_err(|_| corrupt("vendor"))?;
        let provenance: String = server
            .try_get("provenance")
            .map_err(|_| corrupt("provenance"))?;
        let transport = VendorTransportKind::parse(
            &server
                .try_get::<_, String>("transport")
                .map_err(|_| corrupt("transport"))?,
        )
        .map_err(|_| corrupt("transport"))?;
        drop(client);
        let encoded = encoded_registration(registration)?;
        match transport {
            VendorTransportKind::Mcp => {
                self.oauth
                    .discover(&endpoint, &encoded)
                    .await
                    .map_err(map_oauth_vendor)?;
            }
            VendorTransportKind::GoogleDriveRest => {
                if server_id != GOOGLE_DRIVE_SERVER_ID
                    || endpoint != GOOGLE_DRIVE_API_BASE
                    || vendor != GOOGLE_DRIVE_VENDOR
                    || provenance != GOOGLE_DRIVE_PROVENANCE
                {
                    return Err(corrupt("transport_identity"));
                }
                self.drive_oauth
                    .as_ref()
                    .ok_or(McpConnectionError::Conflict {
                        resource: "google_drive_oauth_runtime",
                    })?
                    .validate_registration(registration)
                    .map_err(map_drive_oauth_vendor)?;
            }
        }
        let credential_id = Uuid::now_v7();
        let secret = SecretBytes::new(encoded.to_vec());
        let encrypted = self
            .vault
            .seal(
                &credential_id,
                SecretKind::McpOauthClient,
                SecretPrincipal::Deployment,
                SecretPrincipal::Service(ServiceId::new(server_id)),
                &secret,
            )
            .map_err(|_| corrupt("oauth_client"))?;
        let generation =
            i64::try_from(auth.auth_generation().get()).map_err(|_| corrupt("auth_generation"))?;
        let mut client = self.pool.get().await.map_err(unavailable)?;
        let transaction = client.transaction().await.map_err(query_unavailable)?;
        let current: bool = transaction
            .query_one(
                "SELECT EXISTS(
                    SELECT 1 FROM public.users u
                     WHERE u.id=$1 AND coalesce(u.auth_generation,0)=$2
                       AND EXISTS(SELECT 1 FROM public.user_roles ur
                                   WHERE ur.user_id=u.id AND ur.role='admin')
                       AND NOT EXISTS(SELECT 1 FROM public.revoked_access ra
                                       WHERE ra.email=lower(u.email)))",
                &[&auth.actor().as_str(), &generation],
            )
            .await
            .map_err(query_unavailable)?
            .try_get(0)
            .map_err(|_| corrupt("admin_scope"))?;
        if !current {
            return Err(McpConnectionError::NotVisible);
        }
        let server = transaction
            .query_opt(
                "SELECT s.url,s.vendor,s.provenance,coalesce(s.transport,'mcp') AS transport,
                        s.credential_id,coalesce(s.credential_generation,0)
                        AS credential_generation,s.catalog_generation,
                        c.kind AS old_client_kind,c.provider AS old_client_provider,
                        c.revoked_at AS old_client_revoked_at
                   FROM public.mcp_servers s
                   LEFT JOIN public.credentials c ON c.id=s.credential_id
                  WHERE s.id=$1 FOR UPDATE OF s",
                &[&server_id],
            )
            .await
            .map_err(query_unavailable)?
            .ok_or(McpConnectionError::NotVisible)?;
        let locked_endpoint: String = server
            .try_get("url")
            .map_err(|_| corrupt("server_endpoint"))?;
        if locked_endpoint != endpoint {
            return Err(McpConnectionError::Conflict {
                resource: "mcp_server",
            });
        }
        let locked_transport = VendorTransportKind::parse(
            &server
                .try_get::<_, String>("transport")
                .map_err(|_| corrupt("transport"))?,
        )
        .map_err(|_| corrupt("transport"))?;
        let locked_vendor: String = server.try_get("vendor").map_err(|_| corrupt("vendor"))?;
        let locked_provenance: String = server
            .try_get("provenance")
            .map_err(|_| corrupt("provenance"))?;
        if locked_transport != transport
            || locked_vendor != vendor
            || locked_provenance != provenance
        {
            return Err(McpConnectionError::Conflict {
                resource: "mcp_server",
            });
        }
        let old_client_id: Option<Uuid> = server
            .try_get("credential_id")
            .map_err(|_| corrupt("oauth_client_id"))?;
        if old_client_id.is_some() {
            let old_kind: Option<crate::db::types::CredentialKind> = server
                .try_get("old_client_kind")
                .map_err(|_| corrupt("old_oauth_client_kind"))?;
            let old_provider: Option<String> = server
                .try_get("old_client_provider")
                .map_err(|_| corrupt("old_oauth_client_provider"))?;
            let _: Option<OffsetDateTime> = server
                .try_get("old_client_revoked_at")
                .map_err(|_| corrupt("old_oauth_client_revoked_at"))?;
            if !matches!(
                old_kind,
                Some(
                    crate::db::types::CredentialKind::Mcp
                        | crate::db::types::CredentialKind::McpOauthClient
                )
            ) || old_provider.as_deref() != Some(server_id)
            {
                return Err(corrupt("old_oauth_client_binding"));
            }
        }
        let old_generation: i64 = server
            .try_get("credential_generation")
            .map_err(|_| corrupt("credential_generation"))?;
        let catalog_generation: Option<i64> = server
            .try_get("catalog_generation")
            .map_err(|_| corrupt("catalog_generation"))?;
        if catalog_generation == Some(i64::MAX) {
            return Err(corrupt("catalog_generation"));
        }
        let new_generation = old_generation
            .checked_add(1)
            .ok_or_else(|| corrupt("credential_generation"))?;
        let now: OffsetDateTime = transaction
            .query_one("SELECT clock_timestamp()", &[])
            .await
            .map_err(query_unavailable)?
            .try_get(0)
            .map_err(|_| corrupt("clock"))?;
        let metadata = serde_json::json!({
            "clientId":registration.client_id(),
            "issuer":registration.issuer(),
            "tokenEndpointAuthMethod":match registration.auth_method() {
                McpOAuthClientAuthMethod::ClientSecretBasic => "client_secret_basic",
                McpOAuthClientAuthMethod::ClientSecretPost => "client_secret_post",
            },
            "resourceMetadataUrl":registration.resource_metadata_url(),
            "credentialGeneration":new_generation
        });
        transaction
            .execute(
                "INSERT INTO public.credentials(
                   id,kind,provider,encrypted_value,key_id,metadata,created_at,updated_at
                 ) VALUES($1,'mcp_oauth_client',$2,$3,'oauth-client',$4,$5,$5)",
                &[&credential_id, &server_id, &encrypted, &metadata, &now],
            )
            .await
            .map_err(query_unavailable)?;

        // Tag every pre-existing grant with the old generation before advancing the server. It is
        // therefore invisible immediately and refresh can only suspend, never auto-revive, it.
        transaction
            .execute(
                "UPDATE public.plugin_grants SET
                   credential_generation=coalesce(credential_generation,$2),updated_at=$3
                  WHERE kind='mcp' AND split_part(ref,'/',1)=$1",
                &[&server_id, &old_generation, &now],
            )
            .await
            .map_err(query_unavailable)?;
        transaction
            .execute(
                "UPDATE public.mcp_servers SET credential_id=$2,credential_generation=$3,
                   catalog_generation=CASE WHEN catalog_generation IS NULL THEN NULL
                                           ELSE catalog_generation+1 END,
                   last_error='credential_changed_requires_regrant',updated_at=$4
                  WHERE id=$1",
                &[&server_id, &credential_id, &new_generation, &now],
            )
            .await
            .map_err(query_unavailable)?;
        if let Some(old_client_id) = old_client_id {
            transaction
                .execute(
                    "UPDATE public.credentials SET revoked_at=$2,updated_at=$2,
                       metadata=coalesce(metadata,'{}'::jsonb)||
                                jsonb_build_object('revocation_reason','oauth_client_rotated')
                     WHERE id=$1 AND revoked_at IS NULL",
                    &[&old_client_id, &now],
                )
                .await
                .map_err(query_unavailable)?;
        }

        let corrupt_connection: bool = transaction
            .query_one(
                "SELECT EXISTS(
                    SELECT 1 FROM public.mcp_user_credentials uc
                    LEFT JOIN public.credentials c ON c.id=uc.credential_id
                     WHERE uc.server_id=$1 AND
                       (c.id IS NULL OR c.kind<>'mcp_user_token' OR c.provider<>uc.server_id
                        OR c.key_id<>uc.user_id OR c.revoked_at IS NOT NULL))",
                &[&server_id],
            )
            .await
            .map_err(query_unavailable)?
            .try_get(0)
            .map_err(|_| corrupt("user_credential_binding"))?;
        if corrupt_connection {
            return Err(corrupt("user_credential_binding"));
        }
        let disconnected = transaction
            .query(
                "UPDATE public.credentials c SET revoked_at=$2,updated_at=$2,
                   metadata=coalesce(c.metadata,'{}'::jsonb)||
                            jsonb_build_object('revocation_status','pending',
                                               'revocation_reason','oauth_client_rotated')
                  FROM public.mcp_user_credentials uc
                 WHERE uc.server_id=$1 AND uc.credential_id=c.id
                   AND c.kind='mcp_user_token' AND c.provider=uc.server_id
                   AND c.key_id=uc.user_id AND c.revoked_at IS NULL
                 RETURNING c.key_id",
                &[&server_id, &now],
            )
            .await
            .map_err(query_unavailable)?;
        transaction
            .execute(
                "DELETE FROM public.mcp_user_credentials WHERE server_id=$1",
                &[&server_id],
            )
            .await
            .map_err(query_unavailable)?;
        for row in disconnected {
            let owner: String = row
                .try_get("key_id")
                .map_err(|_| corrupt("credential_owner"))?;
            self.append_connection_audit(
                &transaction,
                &ActorId::new(owner),
                server_id,
                "mcp.account_disconnected",
                Some((AuditLabel::new("oauth_client_rotated"), false)),
            )
            .await?;
        }
        let (id, created_at) = next_event_coordinates(&transaction)
            .await
            .map_err(query_unavailable)?;
        let event = AuditEvent {
            id,
            actor: Some(auth.actor().clone()),
            event_type: AuditEventType::parse("mcp.oauth_client_registered")
                .ok_or_else(|| corrupt("audit_event"))?,
            target_kind: AuditLabel::new("mcp_server"),
            target_id: Some(AuditIdentifier::new(server_id).map_err(|_| corrupt("server_id"))?),
            payload: AuditPayload::empty(),
            created_at,
        };
        append_event_in_transaction(&transaction, &event, self.checkpoint_key.expose())
            .await
            .map_err(query_unavailable)?;
        transaction.commit().await.map_err(|error| {
            tracing::error!(error = %error, "MCP OAuth client registration commit 结果未知");
            McpConnectionError::CommitUnknown
        })?;
        Ok(McpOAuthClientRegistered::success())
    }

    async fn add_curated_server(
        &self,
        auth: &AuthContext,
        key: &str,
    ) -> Result<McpServerMutation, McpConnectionError> {
        self.ensure_auth_current(auth).await?;
        if !auth.has_role(Role::Admin) {
            return Err(McpConnectionError::NotVisible);
        }
        if key != GOOGLE_DRIVE_SERVER_ID {
            return Err(McpConnectionError::InvalidInput {
                field: "catalogue_key",
            });
        }
        let generation =
            i64::try_from(auth.auth_generation().get()).map_err(|_| corrupt("auth_generation"))?;
        let mut client = self.pool.get().await.map_err(unavailable)?;
        let transaction = client.transaction().await.map_err(query_unavailable)?;
        let current: bool = transaction
            .query_one(
                "SELECT EXISTS(
                    SELECT 1 FROM public.users u
                     WHERE u.id=$1 AND coalesce(u.auth_generation,0)=$2
                       AND EXISTS(SELECT 1 FROM public.user_roles ur
                                   WHERE ur.user_id=u.id AND ur.role='admin')
                       AND NOT EXISTS(SELECT 1 FROM public.revoked_access ra
                                       WHERE ra.email=lower(u.email)))",
                &[&auth.actor().as_str(), &generation],
            )
            .await
            .map_err(query_unavailable)?
            .try_get(0)
            .map_err(|_| corrupt("admin_scope"))?;
        if !current {
            return Err(McpConnectionError::NotVisible);
        }
        let existing = transaction
            .query_opt(
                "SELECT url,vendor,provenance,transport
                   FROM public.mcp_servers WHERE id=$1 FOR UPDATE",
                &[&key],
            )
            .await
            .map_err(query_unavailable)?;
        if let Some(row) = existing {
            let endpoint: String = row.try_get("url").map_err(|_| corrupt("server_endpoint"))?;
            let vendor: String = row.try_get("vendor").map_err(|_| corrupt("vendor"))?;
            let provenance: String = row
                .try_get("provenance")
                .map_err(|_| corrupt("provenance"))?;
            let transport: Option<String> =
                row.try_get("transport").map_err(|_| corrupt("transport"))?;
            if endpoint != GOOGLE_DRIVE_API_BASE
                || vendor != GOOGLE_DRIVE_VENDOR
                || provenance != GOOGLE_DRIVE_PROVENANCE
                || transport
                    .as_deref()
                    .is_some_and(|value| value != GOOGLE_DRIVE_TRANSPORT)
            {
                return Err(McpConnectionError::Conflict {
                    resource: "mcp_server_identity",
                });
            }
            transaction
                .execute(
                    "UPDATE public.mcp_servers SET title='Google Drive',vendor='Google',
                       url=$2,provenance='first-party',transport='google_drive_rest',
                       added_by=$3,updated_at=clock_timestamp() WHERE id=$1",
                    &[&key, &GOOGLE_DRIVE_API_BASE, &auth.actor().as_str()],
                )
                .await
                .map_err(query_unavailable)?;
        } else {
            transaction
                .execute(
                    "INSERT INTO public.mcp_servers(
                       id,title,vendor,url,provenance,credential_id,tools_refreshed_at,last_error,
                       added_by,created_at,updated_at,catalog_generation,catalog_hash,
                       catalog_transport_fingerprint,credential_generation,transport
                     ) VALUES($1,'Google Drive','Google',$2,'first-party',NULL,NULL,NULL,$3,
                              clock_timestamp(),clock_timestamp(),NULL,NULL,NULL,0,
                              'google_drive_rest')",
                    &[&key, &GOOGLE_DRIVE_API_BASE, &auth.actor().as_str()],
                )
                .await
                .map_err(query_unavailable)?;
        }
        let (id, created_at) = next_event_coordinates(&transaction)
            .await
            .map_err(query_unavailable)?;
        let event = AuditEvent {
            id,
            actor: Some(auth.actor().clone()),
            event_type: AuditEventType::parse("configuration.changed")
                .ok_or_else(|| corrupt("audit_event"))?,
            target_kind: AuditLabel::new("mcp_server"),
            target_id: Some(AuditIdentifier::new(key).map_err(|_| corrupt("server_id"))?),
            payload: AuditPayload::empty(),
            created_at,
        };
        append_event_in_transaction(&transaction, &event, self.checkpoint_key.expose())
            .await
            .map_err(query_unavailable)?;
        transaction.commit().await.map_err(|error| {
            tracing::error!(error = %error, "curated MCP server commit 结果未知");
            McpConnectionError::CommitUnknown
        })?;
        drop(client);
        let refresh = self
            .catalog
            .refresh(GOOGLE_DRIVE_SERVER_ID, None)
            .await
            .map_err(map_catalog_failure)?;
        mutation_receipt(GOOGLE_DRIVE_SERVER_ID, &refresh)
    }

    async fn refresh_server(
        &self,
        auth: &AuthContext,
        server_id: &str,
    ) -> Result<McpServerMutation, McpConnectionError> {
        self.ensure_auth_current(auth).await?;
        if !auth.has_role(Role::Admin) {
            return Err(McpConnectionError::NotVisible);
        }
        validate_server_id(server_id)?;
        let client = self.pool.get().await.map_err(unavailable)?;
        let row = client
            .query_opt(
                "SELECT url,vendor,provenance,coalesce(transport,'mcp') AS transport
                   FROM public.mcp_servers WHERE id=$1",
                &[&server_id],
            )
            .await
            .map_err(query_unavailable)?
            .ok_or(McpConnectionError::NotVisible)?;
        let transport = VendorTransportKind::parse(
            &row.try_get::<_, String>("transport")
                .map_err(|_| corrupt("transport"))?,
        )
        .map_err(|_| corrupt("transport"))?;
        let endpoint: String = row.try_get("url").map_err(|_| corrupt("server_endpoint"))?;
        let vendor: String = row.try_get("vendor").map_err(|_| corrupt("vendor"))?;
        let provenance: String = row
            .try_get("provenance")
            .map_err(|_| corrupt("provenance"))?;
        if transport != VendorTransportKind::GoogleDriveRest
            || server_id != GOOGLE_DRIVE_SERVER_ID
            || endpoint != GOOGLE_DRIVE_API_BASE
            || vendor != GOOGLE_DRIVE_VENDOR
            || provenance != GOOGLE_DRIVE_PROVENANCE
        {
            return Err(McpConnectionError::Conflict {
                resource: "actor_catalog_credential",
            });
        }
        drop(client);
        let refresh = self
            .catalog
            .refresh(server_id, None)
            .await
            .map_err(map_catalog_failure)?;
        mutation_receipt(server_id, &refresh)
    }
}

#[async_trait]
impl McpOAuthCallback for PostgresMcpConnections {
    async fn complete(&self, input: McpOAuthCallbackInput) -> McpOAuthCallbackOutcome {
        let redirect_to = match self.complete_inner(&input).await {
            Ok(success) => success,
            Err(error) => {
                tracing::warn!(code = %error, "MCP OAuth callback 被拒");
                self.failure_redirect()
            }
        };
        McpOAuthCallbackOutcome { redirect_to }
    }
}

impl PostgresMcpConnections {
    async fn disconnect_candidate(
        &self,
        actor: &ActorId,
        server_id: &str,
    ) -> Result<DisconnectCandidate, McpConnectionError> {
        let client = self.pool.get().await.map_err(unavailable)?;
        let row = client
            .query_opt(
                "SELECT s.url,s.vendor,s.provenance,coalesce(s.transport,'mcp') AS transport,
                        s.credential_id AS client_credential_id,
                        dc.kind AS client_kind,dc.provider AS client_provider,
                        dc.encrypted_value AS client_value,dc.revoked_at AS client_revoked_at,
                        uc.credential_id,c.kind,c.provider,c.key_id,c.encrypted_value,c.revoked_at
                   FROM public.mcp_user_credentials uc
                   JOIN public.mcp_servers s ON s.id=uc.server_id
                   JOIN public.credentials c ON c.id=uc.credential_id
                   LEFT JOIN public.credentials dc ON dc.id=s.credential_id
                  WHERE uc.server_id=$1 AND uc.user_id=$2",
                &[&server_id, &actor.as_str()],
            )
            .await
            .map_err(query_unavailable)?
            .ok_or(McpConnectionError::NotVisible)?;
        let credential_id: Uuid = row
            .try_get("credential_id")
            .map_err(|_| corrupt("credential_id"))?;
        let kind: crate::db::types::CredentialKind = row
            .try_get("kind")
            .map_err(|_| corrupt("credential_kind"))?;
        let provider: String = row
            .try_get("provider")
            .map_err(|_| corrupt("credential_provider"))?;
        let key_id: String = row
            .try_get("key_id")
            .map_err(|_| corrupt("credential_owner"))?;
        let encrypted: String = row
            .try_get("encrypted_value")
            .map_err(|_| corrupt("credential_value"))?;
        let revoked: Option<OffsetDateTime> = row
            .try_get("revoked_at")
            .map_err(|_| corrupt("credential_revoked_at"))?;
        if kind != crate::db::types::CredentialKind::McpUserToken
            || provider != server_id
            || key_id != actor.as_str()
            || revoked.is_some()
        {
            return Err(McpConnectionError::NotVisible);
        }
        let endpoint: String = row.try_get("url").map_err(|_| corrupt("server_endpoint"))?;
        let vendor: String = row.try_get("vendor").map_err(|_| corrupt("vendor"))?;
        let provenance: String = row
            .try_get("provenance")
            .map_err(|_| corrupt("provenance"))?;
        let transport = VendorTransportKind::parse(
            &row.try_get::<_, String>("transport")
                .map_err(|_| corrupt("transport"))?,
        )
        .map_err(|_| corrupt("transport"))?;
        if transport == VendorTransportKind::GoogleDriveRest
            && (server_id != GOOGLE_DRIVE_SERVER_ID
                || endpoint != GOOGLE_DRIVE_API_BASE
                || vendor != GOOGLE_DRIVE_VENDOR
                || provenance != GOOGLE_DRIVE_PROVENANCE)
        {
            return Err(corrupt("transport_identity"));
        }
        let refresh = self
            .vault
            .open(
                &credential_id,
                SecretKind::McpUserToken,
                SecretPrincipal::Actor(actor.clone()),
                SecretPrincipal::Service(ServiceId::new(server_id)),
                &encrypted,
            )
            .ok()
            .map(|value| value.into_secret());
        let client_credential_id: Option<Uuid> = row
            .try_get("client_credential_id")
            .map_err(|_| corrupt("oauth_client_id"))?;
        let client_secret = match client_credential_id {
            Some(id)
                if row
                    .try_get::<_, Option<crate::db::types::CredentialKind>>("client_kind")
                    .ok()
                    .flatten()
                    == Some(crate::db::types::CredentialKind::McpOauthClient)
                    && row
                        .try_get::<_, Option<String>>("client_provider")
                        .ok()
                        .flatten()
                        .as_deref()
                        == Some(server_id)
                    && row
                        .try_get::<_, Option<OffsetDateTime>>("client_revoked_at")
                        .ok()
                        .flatten()
                        .is_none() =>
            {
                row.try_get::<_, Option<String>>("client_value")
                    .ok()
                    .flatten()
                    .and_then(|encrypted| {
                        self.vault
                            .open(
                                &id,
                                SecretKind::McpOauthClient,
                                SecretPrincipal::Deployment,
                                SecretPrincipal::Service(ServiceId::new(server_id)),
                                &encrypted,
                            )
                            .ok()
                            .map(|value| value.into_secret())
                    })
            }
            _ => None,
        };
        Ok(DisconnectCandidate {
            credential_id,
            endpoint,
            client_credential_id,
            client: client_secret,
            refresh,
            transport,
        })
    }

    async fn tombstone_connection(
        &self,
        auth: &AuthContext,
        server_id: &str,
        credential_id: Uuid,
    ) -> Result<(), McpConnectionError> {
        let generation =
            i64::try_from(auth.auth_generation().get()).map_err(|_| corrupt("auth_generation"))?;
        let mut client = self.pool.get().await.map_err(unavailable)?;
        let transaction = client.transaction().await.map_err(query_unavailable)?;
        let now: OffsetDateTime = transaction
            .query_one("SELECT clock_timestamp()", &[])
            .await
            .map_err(query_unavailable)?
            .try_get(0)
            .map_err(|_| corrupt("clock"))?;
        let updated = transaction
            .execute(
                "UPDATE public.credentials c SET revoked_at=$5,updated_at=$5,
                   metadata=coalesce(c.metadata,'{}'::jsonb)||
                            jsonb_build_object('revocation_status','pending',
                                               'revocation_reason','user_disconnect')
                 WHERE c.id=$1 AND c.kind='mcp_user_token' AND c.provider=$2
                   AND c.key_id=$3 AND c.revoked_at IS NULL
                   AND EXISTS(SELECT 1 FROM public.users u WHERE u.id=$3
                               AND coalesce(u.auth_generation,0)=$4
                               AND EXISTS(SELECT 1 FROM public.user_roles ur WHERE ur.user_id=u.id)
                               AND NOT EXISTS(SELECT 1 FROM public.revoked_access ra
                                               WHERE ra.email=lower(u.email)))",
                &[
                    &credential_id,
                    &server_id,
                    &auth.actor().as_str(),
                    &generation,
                    &now,
                ],
            )
            .await
            .map_err(query_unavailable)?;
        if updated != 1 {
            return Err(McpConnectionError::NotVisible);
        }
        let deleted = transaction
            .execute(
                "DELETE FROM public.mcp_user_credentials
                  WHERE server_id=$1 AND user_id=$2 AND credential_id=$3",
                &[&server_id, &auth.actor().as_str(), &credential_id],
            )
            .await
            .map_err(query_unavailable)?;
        if deleted != 1 {
            return Err(McpConnectionError::NotVisible);
        }
        self.append_connection_audit(
            &transaction,
            auth.actor(),
            server_id,
            "mcp.account_disconnected",
            Some((AuditLabel::new("user_disconnect"), false)),
        )
        .await?;
        transaction.commit().await.map_err(|error| {
            tracing::error!(error = %error, "MCP disconnect local tombstone commit 结果未知");
            McpConnectionError::CommitUnknown
        })
    }
}

impl core::fmt::Debug for PostgresMcpConnections {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PostgresMcpConnections")
            .field("deployment", &self.deployment)
            .field("tenant", &self.tenant)
            .field("state_key", &"<redacted>")
            .field("checkpoint_key", &"<redacted>")
            .field("callback_uri", &"<configured>")
            .finish_non_exhaustive()
    }
}

struct ServerClientMaterial {
    credential_id: Uuid,
    endpoint: String,
    client: SecretBytes,
    transport: VendorTransportKind,
}

struct OldRefresh {
    credential_id: Uuid,
    refresh: SecretBytes,
}

struct DisconnectCandidate {
    credential_id: Uuid,
    endpoint: String,
    client_credential_id: Option<Uuid>,
    client: Option<SecretBytes>,
    refresh: Option<SecretBytes>,
    transport: VendorTransportKind,
}

struct PendingRevocation {
    credential_id: Uuid,
    server_id: String,
    actor_id: String,
    encrypted_value: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredAttempt {
    version: u8,
    deployment_id: String,
    tenant_id: String,
    actor_id: String,
    auth_generation: i64,
    server_id: String,
    client_credential_id: Uuid,
    resource: String,
    transport: String,
    code_verifier: String,
    redirect_uri: String,
    issuer: String,
    issuer_required: bool,
    requested_scope: String,
    return_to: McpOAuthReturnTo,
    expires_at_unix_seconds: i64,
}

impl Drop for StoredAttempt {
    fn drop(&mut self) {
        self.code_verifier.zeroize();
    }
}

fn callback_uri(
    public_url: &str,
    scheme_policy: SchemePolicy,
) -> Result<String, McpConnectionError> {
    let url = Url::parse(public_url).map_err(|_| McpConnectionError::InvalidInput {
        field: "public_url",
    })?;
    let allowed = match scheme_policy {
        SchemePolicy::HttpsOnly => url.scheme() == "https",
        SchemePolicy::HttpOrHttps => {
            url.scheme() == "https"
                || (url.scheme() == "http"
                    && match url.host() {
                        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
                        Some(url::Host::Ipv4(address)) => address.is_loopback(),
                        Some(url::Host::Ipv6(address)) => address.is_loopback(),
                        None => false,
                    })
        }
    };
    if !allowed
        || url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(McpConnectionError::InvalidInput {
            field: "public_url",
        });
    }
    Ok(format!(
        "{}{}",
        public_url.trim_end_matches('/'),
        CALLBACK_PATH
    ))
}

fn validate_app_origin(value: &str) -> Result<String, McpConnectionError> {
    let url =
        Url::parse(value).map_err(|_| McpConnectionError::InvalidInput { field: "app_url" })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(McpConnectionError::InvalidInput { field: "app_url" });
    }
    Ok(value.to_owned())
}

fn random_base64<const N: usize>() -> Result<String, McpConnectionError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|_| McpConnectionError::Unavailable)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn encoded_registration(
    registration: &McpOAuthClientRegistration,
) -> Result<Zeroizing<Vec<u8>>, McpConnectionError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Wire<'a> {
        client_id: &'a str,
        client_secret: &'a str,
        issuer: &'a str,
        token_endpoint_auth_method: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        resource_metadata_url: Option<&'a str>,
    }
    let auth_method = match registration.auth_method() {
        McpOAuthClientAuthMethod::ClientSecretBasic => "client_secret_basic",
        McpOAuthClientAuthMethod::ClientSecretPost => "client_secret_post",
    };
    serde_json::to_vec(&Wire {
        client_id: registration.client_id(),
        client_secret: registration.expose_client_secret(),
        issuer: registration.issuer(),
        token_endpoint_auth_method: auth_method,
        resource_metadata_url: registration.resource_metadata_url(),
    })
    .map(Zeroizing::new)
    .map_err(|_| corrupt("oauth_client"))
}

fn valid_server_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.contains("__")
        && !value.contains('/')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn validate_server_id(value: &str) -> Result<(), McpConnectionError> {
    if valid_server_id(value) {
        Ok(())
    } else {
        Err(McpConnectionError::InvalidInput { field: "server_id" })
    }
}

fn valid_pkce(value: &str) -> bool {
    (43..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
}

fn attempt_is_expired(expires_at_unix_seconds: i64, now: OffsetDateTime) -> bool {
    now.unix_timestamp() >= expires_at_unix_seconds
}

async fn database_now(pool: &Pool) -> Result<OffsetDateTime, McpConnectionError> {
    pool.get()
        .await
        .map_err(unavailable)?
        .query_one("SELECT clock_timestamp()", &[])
        .await
        .map_err(query_unavailable)?
        .try_get(0)
        .map_err(|_| corrupt("clock"))
}

fn map_oauth_vendor(error: McpOAuthError) -> McpConnectionError {
    match error {
        McpOAuthError::Unavailable
        | McpOAuthError::AuthRequired
        | McpOAuthError::InsufficientScope
        | McpOAuthError::InvalidMetadata
        | McpOAuthError::InvalidTokenResponse => McpConnectionError::VendorFailure,
        McpOAuthError::InvalidClient => corrupt("oauth_client"),
    }
}

fn map_drive_oauth_vendor(error: GoogleDriveOAuthError) -> McpConnectionError {
    match error {
        GoogleDriveOAuthError::Unavailable
        | GoogleDriveOAuthError::AuthRequired
        | GoogleDriveOAuthError::InvalidResponse => McpConnectionError::VendorFailure,
        GoogleDriveOAuthError::InvalidClient => corrupt("oauth_client"),
    }
}

fn map_catalog_failure(error: McpCatalogError) -> McpConnectionError {
    match error {
        McpCatalogError::NotVisible => McpConnectionError::NotVisible,
        McpCatalogError::Unavailable => McpConnectionError::Unavailable,
        McpCatalogError::Corrupt { field } => McpConnectionError::Corrupt { field },
    }
}

fn mutation_receipt(
    server_id: &str,
    refresh: &McpCatalogRefresh,
) -> Result<McpServerMutation, McpConnectionError> {
    Ok(McpServerMutation {
        server_id: server_id.to_owned(),
        catalog_generation: refresh.generation.get(),
        tool_count: u32::try_from(refresh.tools.len()).map_err(|_| corrupt("tool_count"))?,
        suspended_grants: u32::try_from(refresh.suspended_grants)
            .map_err(|_| corrupt("suspended_grants"))?,
    })
}

fn unavailable(error: deadpool_postgres::PoolError) -> McpConnectionError {
    tracing::error!(error = %error, "MCP connections 获取 PostgreSQL 连接失败");
    McpConnectionError::Unavailable
}

fn query_unavailable(error: impl core::fmt::Display) -> McpConnectionError {
    tracing::error!(error = %error, "MCP connections PostgreSQL 操作失败");
    McpConnectionError::Unavailable
}

const fn corrupt(field: &'static str) -> McpConnectionError {
    McpConnectionError::Corrupt { field }
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use core::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

async fn supervise_revocations(
    connections: Arc<PostgresMcpConnections>,
    mut stop: watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(REVOCATION_RETRY_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return;
                }
            }
            _ = interval.tick() => {
                match connections.reconcile_pending_revocations().await {
                    Ok(sweep) if sweep.attempted > 0 => tracing::info!(
                        attempted = sweep.attempted,
                        revoked = sweep.revoked,
                        pending = sweep.pending,
                        "MCP vendor revocation reconciliation sweep 完成"
                    ),
                    Ok(_) => {}
                    Err(error) => tracing::warn!(code = %error,
                        "MCP vendor revocation reconciliation sweep 失败"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_uri_is_exact_configured_path_and_never_comes_from_a_host_header() {
        assert_eq!(
            callback_uri("https://api.example.test///", SchemePolicy::HttpsOnly).unwrap(),
            "https://api.example.test/api/plugins/oauth/callback"
        );
        assert!(callback_uri("http://api.example.test", SchemePolicy::HttpsOnly).is_err());
        assert_eq!(
            callback_uri("http://127.0.0.1:43123", SchemePolicy::HttpOrHttps).unwrap(),
            "http://127.0.0.1:43123/api/plugins/oauth/callback"
        );
        assert!(
            callback_uri("http://public.example", SchemePolicy::HttpOrHttps).is_err(),
            "HTTP exception is loopback-only"
        );
    }

    #[test]
    fn app_redirect_origin_rejects_query_fragment_and_userinfo() {
        assert_eq!(
            validate_app_origin("https://app.example.test/").unwrap(),
            "https://app.example.test/"
        );
        for invalid in [
            "https://user@app.example.test",
            "https://app.example.test?returnTo=https://evil.test",
            "https://app.example.test/#fragment",
        ] {
            assert!(validate_app_origin(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn oauth_attempt_expiry_is_inclusive_at_the_database_clock_boundary() {
        let before = OffsetDateTime::from_unix_timestamp(99).unwrap();
        let exact = OffsetDateTime::from_unix_timestamp(100).unwrap();
        assert!(!attempt_is_expired(100, before));
        assert!(attempt_is_expired(100, exact));
    }
}
