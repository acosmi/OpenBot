//! Release-attested PostgreSQL sidecar bundle and single-instance start lock.
//!
//! This module deliberately stops before process launch. A future supervisor may spawn only a
//! [`VerifiedPostgresBundle`] while holding [`PostgresStartLock`], but still needs an OS key-store
//! secret, `initdb`/ready/shutdown state machine, and the Batch79 database attestation. Keeping
//! those boundaries separate prevents a PATH-resolved development PostgreSQL from becoming a
//! production fallback.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Component, Path, PathBuf};
#[cfg(feature = "postgres-supervisor")]
use std::process::Stdio;
use std::sync::Arc;
#[cfg(feature = "postgres-supervisor")]
use std::time::Duration;

#[cfg(feature = "postgres-supervisor")]
use openbot_contracts::desktop::DESKTOP_LOCAL_POSTGRES_ADMIN_USER;
use openbot_contracts::engine::ENGINE_RELEASE_EPOCH;
#[cfg(feature = "postgres-key-store")]
use openbot_domain::vault::SecretBytes;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
#[cfg(feature = "postgres-supervisor")]
use tokio::io::AsyncWriteExt as _;
#[cfg(feature = "postgres-supervisor")]
use tokio::process::{Child, Command};
#[cfg(feature = "postgres-supervisor")]
use tokio_postgres::NoTls;

#[cfg(feature = "postgres-key-store")]
use crate::os_secret_store::OsSecretStore;

/// Exact PGDG source release used to build the first PostgreSQL sidecar epoch.
pub const POSTGRES_VERSION: &str = "17.11";

/// SHA-256 of the official PGDG `postgresql-17.11.tar.gz` source archive.
pub const POSTGRES_SOURCE_SHA256: &str =
    "5367f6fb2ec97efe1eb2e0c7926bb33438e51b0bd3a9733b88498056a7dc9a7e";

const MANIFEST_FILE: &str = "manifest.json";
const MANIFEST_SCHEMA: &str = "openbot-postgres-sidecar-bundle";
const MANIFEST_SCHEMA_VERSION: u64 = 1;
const MANIFEST_MAX_BYTES: u64 = 1024 * 1024;
const BUNDLE_MAX_FILES: usize = 8192;
const BUNDLE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const LOCK_HEADER: &str = "openbot-postgres-start-lock-v1";
const LOCK_NONCE_BYTES: usize = 16;
#[cfg(feature = "postgres-supervisor")]
const VERSION_DEADLINE: Duration = Duration::from_secs(5);
#[cfg(feature = "postgres-supervisor")]
const INITDB_DEADLINE: Duration = Duration::from_secs(30);
#[cfg(feature = "postgres-supervisor")]
const READY_DEADLINE: Duration = Duration::from_secs(10);
#[cfg(feature = "postgres-supervisor")]
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);

/// Stable bundle/lock failures without filesystem paths, manifest payloads, or secret bytes.
#[derive(Debug, thiserror::Error)]
pub enum PostgresSidecarError {
    /// Filesystem or CSPRNG operation failed.
    #[error("postgres_sidecar_io")]
    Io(#[source] io::Error),
    /// The signed core's expected manifest SHA-256 was malformed or did not match.
    #[error("postgres_sidecar_manifest_digest")]
    ManifestDigest,
    /// Manifest fields, paths, inventory, platform, or permissions were outside the closed shape.
    #[error("postgres_sidecar_bundle_shape")]
    BundleShape,
    /// A named bundle file did not match its manifest SHA-256.
    #[error("postgres_sidecar_file_digest")]
    FileDigest,
    /// Another process or a crash residue already owns the instance start lock.
    #[error("postgres_sidecar_start_lock_held")]
    StartLockHeld,
    /// A caller-supplied release signing identity was not a reviewed closed value.
    #[error("postgres_sidecar_signing_identity_invalid")]
    SigningIdentityInvalid,
    /// Instance data directory was non-private, partial, symlinked, or not the exact direct child.
    #[cfg(feature = "postgres-supervisor")]
    #[error("postgres_sidecar_data_directory_invalid")]
    DataDirectoryInvalid,
    /// A verified program did not report the pinned PostgreSQL 17.11 version.
    #[cfg(feature = "postgres-supervisor")]
    #[error("postgres_sidecar_version_mismatch")]
    VersionMismatch,
    /// `initdb` failed, timed out, or could not receive the password through stdin.
    #[cfg(feature = "postgres-supervisor")]
    #[error("postgres_sidecar_initdb_failed")]
    InitdbFailed,
    /// The verified `postgres` process could not be spawned.
    #[cfg(feature = "postgres-supervisor")]
    #[error("postgres_sidecar_spawn_failed")]
    SpawnFailed,
    /// PostgreSQL exited before SCRAM readiness.
    #[cfg(feature = "postgres-supervisor")]
    #[error("postgres_sidecar_exited_before_ready")]
    ExitedBeforeReady,
    /// SCRAM-authenticated loopback readiness did not pass before the deadline.
    #[cfg(feature = "postgres-supervisor")]
    #[error("postgres_sidecar_ready_timeout")]
    ReadyTimeout,
    /// Graceful shutdown failed or exceeded its deadline; the start lock remains stale.
    #[cfg(feature = "postgres-supervisor")]
    #[error("postgres_sidecar_shutdown_failed")]
    ShutdownFailed,
    /// OS key-store secret acquisition failed before process launch.
    #[cfg(feature = "postgres-supervisor")]
    #[error(transparent)]
    Secret(#[from] PostgresSecretStoreError),
}

impl From<io::Error> for PostgresSidecarError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Expected SHA-256 of the release-owned PostgreSQL sidecar manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostgresBundleDigest([u8; 32]);

impl PostgresBundleDigest {
    /// Parse one exact hexadecimal SHA-256 from signed outer release metadata.
    pub fn from_hex(value: &str) -> Result<Self, PostgresSidecarError> {
        parse_sha256(value)
            .map(Self)
            .ok_or(PostgresSidecarError::ManifestDigest)
    }

    fn to_hex(self) -> String {
        encode_hex(&self.0)
    }
}

/// Host assertion that this exact code-signing identity was approved for the external release.
#[derive(Clone, PartialEq, Eq)]
pub struct ReviewedPostgresSigningIdentity(Arc<str>);

impl ReviewedPostgresSigningIdentity {
    /// Wrap a reviewed, printable signing identity that contains no prohibited source mark.
    pub fn from_reviewed_release(value: impl Into<String>) -> Result<Self, PostgresSidecarError> {
        let value = value.into();
        let lower = value.to_ascii_lowercase();
        if value.is_empty()
            || value.len() > 256
            || !value
                .bytes()
                .all(|byte| byte == b' ' || byte.is_ascii_graphic())
            || ["openbot", "copilotkit", "codex", "openai", "grok", "xai"]
                .iter()
                .any(|mark| lower.contains(mark))
        {
            return Err(PostgresSidecarError::SigningIdentityInvalid);
        }
        Ok(Self(Arc::from(value)))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for ReviewedPostgresSigningIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ReviewedPostgresSigningIdentity(<reviewed>)")
    }
}

/// A complete PostgreSQL runtime tree verified before any binary can be spawned.
pub struct VerifiedPostgresBundle {
    root: PathBuf,
    postgres: PathBuf,
    initdb: PathBuf,
    pg_ctl: PathBuf,
    manifest_sha256: [u8; 32],
}

impl core::fmt::Debug for VerifiedPostgresBundle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("VerifiedPostgresBundle")
            .field("root", &"<redacted>")
            .field("manifest_sha256", &encode_hex(&self.manifest_sha256))
            .finish_non_exhaustive()
    }
}

impl VerifiedPostgresBundle {
    /// Verify manifest authenticity, closed release fields, every bundle file, and no extras.
    ///
    /// The manifest digest must come from signed outer release metadata. The manifest's
    /// `signing_identity` is then compared to the host's separately reviewed identity; this method
    /// records that identity but does not pretend to perform platform code-signature verification.
    pub fn open(
        root: impl Into<PathBuf>,
        expected_manifest: PostgresBundleDigest,
        expected_signing_identity: &ReviewedPostgresSigningIdentity,
    ) -> Result<Self, PostgresSidecarError> {
        let root = root.into();
        let root_metadata = fs::symlink_metadata(&root)?;
        if !root_metadata.file_type().is_dir() || root_metadata.file_type().is_symlink() {
            return Err(PostgresSidecarError::BundleShape);
        }
        let manifest_path = root.join(MANIFEST_FILE);
        let manifest_metadata = fs::symlink_metadata(&manifest_path)?;
        if !manifest_metadata.file_type().is_file()
            || manifest_metadata.file_type().is_symlink()
            || manifest_metadata.len() > MANIFEST_MAX_BYTES
        {
            return Err(PostgresSidecarError::BundleShape);
        }
        let manifest_bytes = fs::read(&manifest_path)?;
        let manifest_sha256: [u8; 32] = Sha256::digest(&manifest_bytes).into();
        if manifest_sha256 != expected_manifest.0 {
            return Err(PostgresSidecarError::ManifestDigest);
        }
        let manifest: PostgresBundleManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|_| PostgresSidecarError::BundleShape)?;
        validate_manifest(&manifest, expected_signing_identity)?;

        let actual_files = inventory_files(&root)?;
        let manifest_files = manifest.files.keys().cloned().collect::<BTreeSet<_>>();
        if actual_files != manifest_files {
            return Err(PostgresSidecarError::BundleShape);
        }
        for (relative, expected) in &manifest.files {
            if !valid_lower_sha256(expected) {
                return Err(PostgresSidecarError::BundleShape);
            }
            let expected = parse_sha256(expected).ok_or(PostgresSidecarError::BundleShape)?;
            if sha256_file(&safe_join(&root, relative)?)? != expected {
                return Err(PostgresSidecarError::FileDigest);
            }
        }

        let postgres = verified_program(&root, &manifest.programs.postgres)?;
        let initdb = verified_program(&root, &manifest.programs.initdb)?;
        let pg_ctl = verified_program(&root, &manifest.programs.pg_ctl)?;
        Ok(Self {
            root,
            postgres,
            initdb,
            pg_ctl,
            manifest_sha256,
        })
    }

    /// Verified bundle root, used only for child library/share lookup.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Verified PostgreSQL server executable.
    #[must_use]
    pub fn postgres(&self) -> &Path {
        &self.postgres
    }

    /// Verified `initdb` executable.
    #[must_use]
    pub fn initdb(&self) -> &Path {
        &self.initdb
    }

    /// Verified `pg_ctl` executable.
    #[must_use]
    pub fn pg_ctl(&self) -> &Path {
        &self.pg_ctl
    }

    /// Manifest digest that was authenticated before file inventory verification.
    #[must_use]
    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }
}

/// Exclusive per-instance startup ownership; crash residue intentionally fails closed.
pub struct PostgresStartLock {
    path: PathBuf,
    bytes: Vec<u8>,
    instance_id: Arc<str>,
    remove_on_drop: bool,
    _file: File,
}

impl core::fmt::Debug for PostgresStartLock {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PostgresStartLock(<redacted>)")
    }
}

impl PostgresStartLock {
    /// Atomically acquire the direct app-data lock for one instance and verified bundle digest.
    ///
    /// Existing files are never auto-recovered by PID guess. A future supervisor may add
    /// platform-authenticated process identity recovery; until then crash residue requires an
    /// explicit repair flow and therefore cannot cause two PostgreSQL processes to start.
    pub fn acquire(
        app_data_root: &Path,
        instance_id: &str,
        bundle_digest: PostgresBundleDigest,
    ) -> Result<Self, PostgresSidecarError> {
        if !app_data_root.is_absolute() || !valid_instance_id(instance_id) {
            return Err(PostgresSidecarError::BundleShape);
        }
        let root_metadata = fs::symlink_metadata(app_data_root)?;
        if !root_metadata.file_type().is_dir() || root_metadata.file_type().is_symlink() {
            return Err(PostgresSidecarError::BundleShape);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if root_metadata.permissions().mode() & 0o077 != 0 {
                return Err(PostgresSidecarError::BundleShape);
            }
        }
        let path = app_data_root.join(format!(".postgresql-17-{instance_id}.start-lock-v1"));
        let mut nonce = [0_u8; LOCK_NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|error| {
            PostgresSidecarError::Io(io::Error::other(format!("getrandom: {error}")))
        })?;
        let bytes = format!(
            "{LOCK_HEADER}\npid={}\ninstance={instance_id}\nmanifest={}\nnonce={}\n",
            std::process::id(),
            bundle_digest.to_hex(),
            encode_hex(&nonce)
        )
        .into_bytes();

        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(PostgresSidecarError::StartLockHeld);
            }
            Err(error) => return Err(error.into()),
        };
        let written = file.write_all(&bytes).and_then(|()| file.sync_all());
        if let Err(error) = written {
            let _ = fs::remove_file(&path);
            return Err(error.into());
        }
        sync_directory(app_data_root)?;
        Ok(Self {
            path,
            bytes,
            instance_id: Arc::from(instance_id),
            remove_on_drop: true,
            _file: file,
        })
    }

    #[cfg(feature = "postgres-supervisor")]
    fn preserve_on_drop(&mut self) {
        self.remove_on_drop = false;
    }
}

#[cfg(feature = "postgres-key-store")]
impl PostgresStartLock {
    /// Load or create the only PostgreSQL SCRAM secret while this instance lock is live.
    ///
    /// A newly generated value is written to the OS store and immediately read back; a missing or
    /// different read-back fails reconciliation rather than using an in-memory value that future
    /// restarts cannot recover.
    pub fn load_or_create_scram_secret<S: PostgresSecretStore + ?Sized>(
        &self,
        store: &S,
        service: &ReviewedPostgresKeyStoreService,
    ) -> Result<PostgresScramSecret, PostgresSecretStoreError> {
        let account = format!("postgresql-17-{}", self.instance_id);
        if let Some(stored) = store.read(service.as_str(), &account)? {
            return PostgresScramSecret::from_stored(stored);
        }
        let generated = PostgresScramSecret::generate()?;
        store.write(service.as_str(), &account, generated.expose())?;
        let persisted = store
            .read(service.as_str(), &account)?
            .ok_or(PostgresSecretStoreError::ReconciliationRequired)
            .and_then(PostgresScramSecret::from_stored)?;
        if !generated.0.ct_eq(&persisted.0) {
            return Err(PostgresSecretStoreError::ReconciliationRequired);
        }
        Ok(persisted)
    }
}

impl Drop for PostgresStartLock {
    fn drop(&mut self) {
        if self.remove_on_drop && lock_file_matches(&self.path, &self.bytes) {
            let _ = fs::remove_file(&self.path);
            if let Some(parent) = self.path.parent() {
                let _ = sync_directory(parent);
            }
        }
    }
}

/// Stable OS key-store failures; no service/account/secret or platform prose is retained.
#[cfg(feature = "postgres-key-store")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PostgresSecretStoreError {
    /// Reviewed external key-store service identity was malformed or prohibited.
    #[error("postgres_key_store_identity_invalid")]
    IdentityInvalid,
    /// OS key store or CSPRNG was unavailable.
    #[error("postgres_key_store_unavailable")]
    Unavailable,
    /// Existing bytes were not the exact closed PostgreSQL SCRAM secret shape.
    #[error("postgres_key_store_secret_corrupt")]
    Corrupt,
    /// Write succeeded ambiguously or immediate read-back did not equal the generated value.
    #[error("postgres_key_store_reconciliation_required")]
    ReconciliationRequired,
}

/// Host assertion that this external key-store service identifier passed product review.
#[cfg(feature = "postgres-key-store")]
#[derive(Clone, PartialEq, Eq)]
pub struct ReviewedPostgresKeyStoreService(Arc<str>);

#[cfg(feature = "postgres-key-store")]
impl ReviewedPostgresKeyStoreService {
    /// Wrap a reviewed ASCII service identifier with no prohibited source mark.
    pub fn from_reviewed_release(
        value: impl Into<String>,
    ) -> Result<Self, PostgresSecretStoreError> {
        let value = value.into();
        let lower = value.to_ascii_lowercase();
        if value.len() < 3
            || value.len() > 128
            || value != lower
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':')
            })
            || ["openbot", "copilotkit", "codex", "openai", "grok", "xai"]
                .iter()
                .any(|mark| lower.contains(mark))
        {
            return Err(PostgresSecretStoreError::IdentityInvalid);
        }
        Ok(Self(Arc::from(value)))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(feature = "postgres-key-store")]
impl core::fmt::Debug for ReviewedPostgresKeyStoreService {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ReviewedPostgresKeyStoreService(<reviewed>)")
    }
}

/// Exact 256-bit random PostgreSQL password encoded as 64 lowercase hexadecimal bytes.
#[cfg(feature = "postgres-key-store")]
pub struct PostgresScramSecret(SecretBytes);

#[cfg(feature = "postgres-key-store")]
impl PostgresScramSecret {
    fn generate() -> Result<Self, PostgresSecretStoreError> {
        let mut raw = vec![0_u8; 32];
        getrandom::fill(&mut raw).map_err(|_| PostgresSecretStoreError::Unavailable)?;
        let raw = SecretBytes::new(raw);
        let encoded = encode_hex(raw.expose()).into_bytes();
        drop(raw);
        Self::from_stored(PostgresStoredSecret::from_owned_bytes(encoded))
    }

    fn from_stored(stored: PostgresStoredSecret) -> Result<Self, PostgresSecretStoreError> {
        if stored.0.len() != 64
            || !stored
                .0
                .expose()
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(PostgresSecretStoreError::Corrupt);
        }
        Ok(Self(stored.0))
    }

    /// Explicitly expose the password only to `initdb`/PostgreSQL process framing.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        self.0.expose()
    }
}

/// Owned key-store read result that zeroizes unless consumed by the closed SCRAM validator.
#[cfg(feature = "postgres-key-store")]
pub struct PostgresStoredSecret(SecretBytes);

#[cfg(feature = "postgres-key-store")]
impl PostgresStoredSecret {
    /// Transfer a platform adapter's unique plaintext allocation into zeroizing ownership.
    #[must_use]
    pub fn from_owned_bytes(bytes: Vec<u8>) -> Self {
        Self(SecretBytes::new(bytes))
    }

    fn from_secret_bytes(bytes: SecretBytes) -> Self {
        Self(bytes)
    }
}

#[cfg(feature = "postgres-key-store")]
impl core::fmt::Debug for PostgresStoredSecret {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PostgresStoredSecret([REDACTED])")
    }
}

#[cfg(feature = "postgres-key-store")]
impl core::fmt::Debug for PostgresScramSecret {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PostgresScramSecret([REDACTED])")
    }
}

/// Minimal OS secret-store port. Implementations must return owned bytes directly into
/// [`SecretBytes`] ownership and never log service/account/secret values. Platform key-store APIs
/// are blocking; the future supervisor must call this port off the Tauri UI thread.
#[cfg(feature = "postgres-key-store")]
pub trait PostgresSecretStore: Send + Sync {
    /// Read one exact service/account entry; unknown is `None`.
    fn read(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<PostgresStoredSecret>, PostgresSecretStoreError>;

    /// Create or replace one exact service/account entry.
    fn write(
        &self,
        service: &str,
        account: &str,
        secret: &[u8],
    ) -> Result<(), PostgresSecretStoreError>;
}

impl<T: OsSecretStore + ?Sized> PostgresSecretStore for T {
    fn read(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<PostgresStoredSecret>, PostgresSecretStoreError> {
        OsSecretStore::read(self, service, account)
            .map(|secret| secret.map(PostgresStoredSecret::from_secret_bytes))
            .map_err(|_| PostgresSecretStoreError::Unavailable)
    }

    fn write(
        &self,
        service: &str,
        account: &str,
        secret: &[u8],
    ) -> Result<(), PostgresSecretStoreError> {
        OsSecretStore::write(self, service, account, secret)
            .map_err(|_| PostgresSecretStoreError::Unavailable)
    }
}

/// Backwards-compatible PostgreSQL-facing name for the shared macOS generic-secret adapter.
#[cfg(all(feature = "postgres-key-store", target_os = "macos"))]
pub type MacOsKeychainPostgresSecretStore = crate::os_secret_store::MacOsKeychainSecretStore;

/// Backwards-compatible PostgreSQL-facing name for the shared Windows generic-secret adapter.
#[cfg(all(feature = "postgres-key-store", target_os = "windows"))]
pub type WindowsCredentialPostgresSecretStore =
    crate::os_secret_store::WindowsCredentialSecretStore;

/// Whether this supervisor initialized a new cluster or opened an existing PostgreSQL 17 cluster.
#[cfg(feature = "postgres-supervisor")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostgresSidecarOrigin {
    /// The exact instance directory was empty and `initdb` completed this start.
    Fresh,
    /// A private instance directory already contained exact `PG_VERSION=17`.
    Existing,
}

/// Borrowed SCRAM connection material for the later Batch79/bootstrap assembly.
#[cfg(feature = "postgres-supervisor")]
pub struct PostgresSidecarConnection<'a> {
    port: u16,
    secret: &'a PostgresScramSecret,
}

#[cfg(feature = "postgres-supervisor")]
impl<'a> PostgresSidecarConnection<'a> {
    /// Exact numeric loopback host; hostnames and remote addresses are unrepresentable.
    #[must_use]
    pub const fn host(&self) -> &'static str {
        "127.0.0.1"
    }

    /// PostgreSQL port selected before the verified process was spawned.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Fixed local administrative role created by `initdb`.
    #[must_use]
    pub const fn user(&self) -> &'static str {
        DESKTOP_LOCAL_POSTGRES_ADMIN_USER
    }

    /// Explicitly expose the password only to the later database-pool constructor.
    #[must_use]
    pub fn expose_password(&self) -> &'a [u8] {
        self.secret.expose()
    }
}

#[cfg(feature = "postgres-supervisor")]
impl core::fmt::Debug for PostgresSidecarConnection<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PostgresSidecarConnection")
            .field("host", &self.host())
            .field("port", &self.port)
            .field("user", &DESKTOP_LOCAL_POSTGRES_ADMIN_USER)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// A SCRAM-ready PostgreSQL child that still owns its bundle, secret, and exclusive start lock.
#[cfg(feature = "postgres-supervisor")]
pub struct RunningPostgresSidecar {
    child: Option<Child>,
    lock: Option<PostgresStartLock>,
    secret: PostgresScramSecret,
    bundle: VerifiedPostgresBundle,
    data_dir: PathBuf,
    port: u16,
    origin: PostgresSidecarOrigin,
}

#[cfg(feature = "postgres-supervisor")]
impl core::fmt::Debug for RunningPostgresSidecar {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RunningPostgresSidecar")
            .field("pid", &self.child.as_ref().and_then(Child::id))
            .field("port", &self.port)
            .field("origin", &self.origin)
            .field("data_dir", &"<redacted>")
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

#[cfg(feature = "postgres-supervisor")]
impl RunningPostgresSidecar {
    /// Fresh/existing result proved before process start.
    #[must_use]
    pub const fn origin(&self) -> PostgresSidecarOrigin {
        self.origin
    }

    /// SCRAM connection input for the later application/bootstrap assembly.
    #[must_use]
    pub const fn connection(&self) -> PostgresSidecarConnection<'_> {
        PostgresSidecarConnection {
            port: self.port,
            secret: &self.secret,
        }
    }

    /// Instance data directory already bound by Batch79/B82 path checks.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Stop through the verified `pg_ctl`, then wait for the exact owned child before releasing
    /// the start lock. Failure preserves a stale lock and kills the child best-effort.
    pub async fn shutdown(mut self) -> Result<(), PostgresSidecarError> {
        let status = run_pg_ctl_stop(&self.bundle, &self.data_dir).await;
        if !matches!(status, Ok(status) if status.success()) {
            self.preserve_lock();
            if let Some(child) = self.child.as_mut() {
                let _ = terminate_child(child).await;
            }
            self.child.take();
            return Err(PostgresSidecarError::ShutdownFailed);
        }
        let Some(mut child) = self.child.take() else {
            self.preserve_lock();
            return Err(PostgresSidecarError::ShutdownFailed);
        };
        let waited = tokio::time::timeout(SHUTDOWN_DEADLINE, child.wait()).await;
        match waited {
            Ok(Ok(status)) if status.success() => {
                drop(self.lock.take());
                Ok(())
            }
            _ => {
                self.preserve_lock();
                let _ = terminate_child(&mut child).await;
                Err(PostgresSidecarError::ShutdownFailed)
            }
        }
    }

    fn preserve_lock(&mut self) {
        if let Some(lock) = self.lock.as_mut() {
            lock.preserve_on_drop();
        }
    }
}

#[cfg(feature = "postgres-supervisor")]
impl Drop for RunningPostgresSidecar {
    fn drop(&mut self) {
        if self.child.is_some() {
            self.preserve_lock();
        }
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
    }
}

/// Verified PostgreSQL process lifecycle owner. It ends at SCRAM readiness; schema/package/window
/// assembly remains a separate step and cannot run before this succeeds.
#[cfg(feature = "postgres-supervisor")]
#[derive(Clone, Copy, Debug, Default)]
pub struct PostgresSidecarSupervisor;

#[cfg(feature = "postgres-supervisor")]
impl PostgresSidecarSupervisor {
    /// Start one exact instance from a verified release bundle.
    pub async fn start<S: PostgresSecretStore + ?Sized>(
        bundle: VerifiedPostgresBundle,
        app_data_root: &Path,
        instance_id: &str,
        data_dir: &Path,
        store: &S,
        service: &ReviewedPostgresKeyStoreService,
    ) -> Result<RunningPostgresSidecar, PostgresSidecarError> {
        validate_supervisor_paths(app_data_root, instance_id, data_dir)?;
        let mut lock = PostgresStartLock::acquire(
            app_data_root,
            instance_id,
            PostgresBundleDigest(bundle.manifest_sha256),
        )?;
        verify_program_versions(&bundle).await?;
        let secret = lock.load_or_create_scram_secret(store, service)?;
        let origin = data_directory_origin(data_dir)?;
        if origin == PostgresSidecarOrigin::Fresh {
            run_initdb(&bundle, data_dir, &secret).await?;
        }
        let port = reserve_loopback_port()?;
        write_runtime_configuration(data_dir, port)?;
        let mut child = spawn_postgres(&bundle, data_dir)?;
        if let Err(error) = wait_until_ready(&mut child, port, &secret).await {
            if terminate_child(&mut child).await.is_err() {
                lock.preserve_on_drop();
            }
            return Err(error);
        }
        Ok(RunningPostgresSidecar {
            child: Some(child),
            lock: Some(lock),
            secret,
            bundle,
            data_dir: data_dir.to_owned(),
            port,
            origin,
        })
    }
}

#[cfg(feature = "postgres-supervisor")]
fn validate_supervisor_paths(
    app_data_root: &Path,
    instance_id: &str,
    data_dir: &Path,
) -> Result<(), PostgresSidecarError> {
    if !app_data_root.is_absolute()
        || !valid_instance_id(instance_id)
        || data_dir != app_data_root.join(format!("postgresql-17-{instance_id}"))
    {
        return Err(PostgresSidecarError::DataDirectoryInvalid);
    }
    let root = fs::symlink_metadata(app_data_root)?;
    let data = fs::symlink_metadata(data_dir)?;
    if !root.file_type().is_dir()
        || root.file_type().is_symlink()
        || !data.file_type().is_dir()
        || data.file_type().is_symlink()
    {
        return Err(PostgresSidecarError::DataDirectoryInvalid);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if root.permissions().mode() & 0o077 != 0 || data.permissions().mode() & 0o077 != 0 {
            return Err(PostgresSidecarError::DataDirectoryInvalid);
        }
    }
    let root = fs::canonicalize(app_data_root)?;
    let data = fs::canonicalize(data_dir)?;
    if data.parent() != Some(root.as_path()) {
        return Err(PostgresSidecarError::DataDirectoryInvalid);
    }
    Ok(())
}

#[cfg(feature = "postgres-supervisor")]
fn data_directory_origin(data_dir: &Path) -> Result<PostgresSidecarOrigin, PostgresSidecarError> {
    let version = data_dir.join("PG_VERSION");
    match fs::symlink_metadata(&version) {
        Ok(metadata) => {
            if !metadata.file_type().is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > 16
                || fs::read_to_string(version)
                    .map_err(PostgresSidecarError::Io)?
                    .trim()
                    != "17"
            {
                return Err(PostgresSidecarError::DataDirectoryInvalid);
            }
            Ok(PostgresSidecarOrigin::Existing)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if fs::read_dir(data_dir)?.next().is_some() {
                return Err(PostgresSidecarError::DataDirectoryInvalid);
            }
            Ok(PostgresSidecarOrigin::Fresh)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(feature = "postgres-supervisor")]
async fn verify_program_versions(
    bundle: &VerifiedPostgresBundle,
) -> Result<(), PostgresSidecarError> {
    for (program, label) in [
        (bundle.postgres(), "postgres"),
        (bundle.initdb(), "initdb"),
        (bundle.pg_ctl(), "pg_ctl"),
    ] {
        let mut command = clean_command(program);
        command.arg("--version").kill_on_drop(true);
        let output = tokio::time::timeout(VERSION_DEADLINE, command.output())
            .await
            .map_err(|_| PostgresSidecarError::VersionMismatch)?
            .map_err(|_| PostgresSidecarError::VersionMismatch)?;
        if !output.status.success() || output.stdout.len() + output.stderr.len() > 4096 {
            return Err(PostgresSidecarError::VersionMismatch);
        }
        let bytes = match (output.stdout.is_empty(), output.stderr.is_empty()) {
            (false, true) => output.stdout,
            (true, false) => output.stderr,
            _ => return Err(PostgresSidecarError::VersionMismatch),
        };
        let line = std::str::from_utf8(&bytes)
            .map_err(|_| PostgresSidecarError::VersionMismatch)?
            .trim();
        let prefix = format!("{label} (PostgreSQL) {POSTGRES_VERSION}");
        let suffix = line
            .strip_prefix(&prefix)
            .ok_or(PostgresSidecarError::VersionMismatch)?;
        if !suffix.is_empty()
            && !(suffix.starts_with(" (")
                && suffix.ends_with(')')
                && suffix
                    .bytes()
                    .all(|byte| byte == b' ' || byte.is_ascii_graphic()))
        {
            return Err(PostgresSidecarError::VersionMismatch);
        }
    }
    Ok(())
}

#[cfg(feature = "postgres-supervisor")]
async fn run_initdb(
    bundle: &VerifiedPostgresBundle,
    data_dir: &Path,
    secret: &PostgresScramSecret,
) -> Result<(), PostgresSidecarError> {
    let mut command = clean_command(bundle.initdb());
    command
        .arg("--pgdata")
        .arg(data_dir)
        .arg(format!("--username={DESKTOP_LOCAL_POSTGRES_ADMIN_USER}"))
        .args([
            "--pwprompt",
            "--auth-host=scram-sha-256",
            "--auth-local=reject",
            "--encoding=UTF8",
            "--no-locale",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|_| PostgresSidecarError::InitdbFailed)?;
    let completed = tokio::time::timeout(INITDB_DEADLINE, async {
        let mut stdin = child
            .stdin
            .take()
            .ok_or(PostgresSidecarError::InitdbFailed)?;
        for _ in 0..2 {
            stdin
                .write_all(secret.expose())
                .await
                .map_err(|_| PostgresSidecarError::InitdbFailed)?;
            stdin
                .write_all(b"\n")
                .await
                .map_err(|_| PostgresSidecarError::InitdbFailed)?;
        }
        stdin
            .shutdown()
            .await
            .map_err(|_| PostgresSidecarError::InitdbFailed)?;
        drop(stdin);
        child
            .wait()
            .await
            .map_err(|_| PostgresSidecarError::InitdbFailed)
    })
    .await;
    match completed {
        Ok(Ok(status)) if status.success() => Ok(()),
        _ => {
            let _ = terminate_child(&mut child).await;
            Err(PostgresSidecarError::InitdbFailed)
        }
    }
}

#[cfg(feature = "postgres-supervisor")]
fn write_runtime_configuration(data_dir: &Path, port: u16) -> Result<(), PostgresSidecarError> {
    let settings = format!(
        "# managed by signed Desktop core\nlisten_addresses = '127.0.0.1'\nport = {port}\nssl = off\npassword_encryption = 'scram-sha-256'\nmax_connections = 32\nlogging_collector = off\n"
    );
    #[cfg(unix)]
    let settings =
        settings + "unix_socket_directories = ''\ndynamic_shared_memory_type = 'posix'\n";
    let hba = if cfg!(unix) {
        "# managed by signed Desktop core\nlocal all all reject\nhost all all 127.0.0.1/32 scram-sha-256\nhost all all ::1/128 scram-sha-256\n"
    } else {
        "# managed by signed Desktop core\nhost all all 127.0.0.1/32 scram-sha-256\nhost all all ::1/128 scram-sha-256\n"
    };
    write_private_file(&data_dir.join("postgresql.auto.conf"), settings.as_bytes())?;
    write_private_file(&data_dir.join("pg_hba.conf"), hba.as_bytes())?;
    sync_directory(data_dir)
}

#[cfg(feature = "postgres-supervisor")]
fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), PostgresSidecarError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.file_type().is_file() || metadata.file_type().is_symlink())
    {
        return Err(PostgresSidecarError::DataDirectoryInvalid);
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(feature = "postgres-supervisor")]
fn spawn_postgres(
    bundle: &VerifiedPostgresBundle,
    data_dir: &Path,
) -> Result<Child, PostgresSidecarError> {
    let mut command = clean_command(bundle.postgres());
    command
        .arg("-D")
        .arg(data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| PostgresSidecarError::SpawnFailed)
}

#[cfg(feature = "postgres-supervisor")]
async fn wait_until_ready(
    child: &mut Child,
    port: u16,
    secret: &PostgresScramSecret,
) -> Result<(), PostgresSidecarError> {
    let deadline = tokio::time::Instant::now() + READY_DEADLINE;
    loop {
        if child
            .try_wait()
            .map_err(|_| PostgresSidecarError::ExitedBeforeReady)?
            .is_some()
        {
            return Err(PostgresSidecarError::ExitedBeforeReady);
        }
        if postgres_ready(port, secret).await {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(PostgresSidecarError::ReadyTimeout);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(feature = "postgres-supervisor")]
async fn postgres_ready(port: u16, secret: &PostgresScramSecret) -> bool {
    let mut config = tokio_postgres::Config::new();
    config
        .host("127.0.0.1")
        .port(port)
        .user(DESKTOP_LOCAL_POSTGRES_ADMIN_USER)
        .password(secret.expose())
        .dbname("postgres")
        .application_name("desktop-postgres-ready")
        .connect_timeout(Duration::from_secs(1));
    let connected = tokio::time::timeout(Duration::from_secs(2), config.connect(NoTls)).await;
    let Ok(Ok((client, connection))) = connected else {
        return false;
    };
    let driver = tokio::spawn(connection);
    let ready = tokio::time::timeout(
        Duration::from_secs(1),
        client.query_one("SELECT 1::int4", &[]),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .and_then(|row| row.try_get::<_, i32>(0).ok())
        == Some(1);
    drop(client);
    driver.abort();
    ready
}

#[cfg(feature = "postgres-supervisor")]
async fn run_pg_ctl_stop(
    bundle: &VerifiedPostgresBundle,
    data_dir: &Path,
) -> Result<std::process::ExitStatus, PostgresSidecarError> {
    let mut command = clean_command(bundle.pg_ctl());
    command
        .arg("-D")
        .arg(data_dir)
        .args(["-m", "fast", "-w", "-t", "5", "stop"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    tokio::time::timeout(SHUTDOWN_DEADLINE + Duration::from_secs(1), command.status())
        .await
        .map_err(|_| PostgresSidecarError::ShutdownFailed)?
        .map_err(|_| PostgresSidecarError::ShutdownFailed)
}

#[cfg(feature = "postgres-supervisor")]
async fn terminate_child(child: &mut Child) -> Result<(), PostgresSidecarError> {
    if child
        .try_wait()
        .map_err(|_| PostgresSidecarError::ShutdownFailed)?
        .is_some()
    {
        return Ok(());
    }
    child
        .start_kill()
        .map_err(|_| PostgresSidecarError::ShutdownFailed)?;
    tokio::time::timeout(SHUTDOWN_DEADLINE, child.wait())
        .await
        .map_err(|_| PostgresSidecarError::ShutdownFailed)?
        .map_err(|_| PostgresSidecarError::ShutdownFailed)?;
    Ok(())
}

#[cfg(feature = "postgres-supervisor")]
fn reserve_loopback_port() -> Result<u16, PostgresSidecarError> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

#[cfg(feature = "postgres-supervisor")]
fn clean_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.env_clear().env("LC_ALL", "C").env("LANG", "C");
    if let Some(root) = program.parent().and_then(Path::parent) {
        command.current_dir(root);
    }
    command
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PostgresBundleManifest {
    schema: String,
    schema_version: u64,
    platform: String,
    arch: String,
    postgresql_version: String,
    source_archive_sha256: String,
    release_epoch: u64,
    minimum_compatible_core: String,
    signing_identity: String,
    programs: PostgresPrograms,
    files: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PostgresPrograms {
    postgres: String,
    initdb: String,
    pg_ctl: String,
}

fn validate_manifest(
    manifest: &PostgresBundleManifest,
    signing_identity: &ReviewedPostgresSigningIdentity,
) -> Result<(), PostgresSidecarError> {
    if expected_platform() == "unsupported"
        || !matches!(std::env::consts::ARCH, "aarch64" | "x86_64")
        || manifest.schema != MANIFEST_SCHEMA
        || manifest.schema_version != MANIFEST_SCHEMA_VERSION
        || manifest.platform != expected_platform()
        || manifest.arch != std::env::consts::ARCH
        || manifest.postgresql_version != POSTGRES_VERSION
        || manifest.source_archive_sha256 != POSTGRES_SOURCE_SHA256
        || manifest.release_epoch != ENGINE_RELEASE_EPOCH
        || manifest.minimum_compatible_core != env!("CARGO_PKG_VERSION")
        || manifest.signing_identity != signing_identity.as_str()
        || manifest.files.len() < 3
        || manifest.files.len() > BUNDLE_MAX_FILES
    {
        return Err(PostgresSidecarError::BundleShape);
    }
    let required = expected_program_paths();
    if manifest.programs.postgres != required[0]
        || manifest.programs.initdb != required[1]
        || manifest.programs.pg_ctl != required[2]
        || required
            .iter()
            .any(|program| !manifest.files.contains_key(*program))
    {
        return Err(PostgresSidecarError::BundleShape);
    }
    Ok(())
}

fn expected_program_paths() -> [&'static str; 3] {
    if cfg!(target_os = "windows") {
        ["bin/postgres.exe", "bin/initdb.exe", "bin/pg_ctl.exe"]
    } else {
        ["bin/postgres", "bin/initdb", "bin/pg_ctl"]
    }
}

fn expected_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unsupported"
    }
}

fn verified_program(root: &Path, relative: &str) -> Result<PathBuf, PostgresSidecarError> {
    let path = safe_join(root, relative)?;
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(PostgresSidecarError::BundleShape);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(PostgresSidecarError::BundleShape);
        }
    }
    Ok(path)
}

fn inventory_files(root: &Path) -> Result<BTreeSet<String>, PostgresSidecarError> {
    let mut pending = vec![root.to_owned()];
    let mut files = BTreeSet::new();
    let mut total_bytes = 0_u64;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(PostgresSidecarError::BundleShape);
            }
            if metadata.file_type().is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.file_type().is_file() {
                return Err(PostgresSidecarError::BundleShape);
            }
            let relative = normalized_relative(root, &path)?;
            if relative == MANIFEST_FILE {
                continue;
            }
            total_bytes = total_bytes
                .checked_add(metadata.len())
                .ok_or(PostgresSidecarError::BundleShape)?;
            if total_bytes > BUNDLE_MAX_BYTES
                || !files.insert(relative)
                || files.len() > BUNDLE_MAX_FILES
            {
                return Err(PostgresSidecarError::BundleShape);
            }
        }
    }
    Ok(files)
}

fn normalized_relative(root: &Path, path: &Path) -> Result<String, PostgresSidecarError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| PostgresSidecarError::BundleShape)?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(PostgresSidecarError::BundleShape);
        };
        parts.push(part.to_str().ok_or(PostgresSidecarError::BundleShape)?);
    }
    if parts.is_empty() {
        return Err(PostgresSidecarError::BundleShape);
    }
    Ok(parts.join("/"))
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, PostgresSidecarError> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(PostgresSidecarError::BundleShape);
    }
    Ok(root.join(relative))
}

fn sha256_file(path: &Path) -> Result<[u8; 32], PostgresSidecarError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > BUNDLE_MAX_BYTES
    {
        return Err(PostgresSidecarError::BundleShape);
    }
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finalize().into())
}

fn lock_file_matches(path: &Path, expected: &[u8]) -> bool {
    let Ok(expected_len) = u64::try_from(expected.len()) else {
        return false;
    };
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != expected_len
    {
        return false;
    }
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut actual = Vec::with_capacity(expected.len());
    if std::io::Read::by_ref(&mut file)
        .take(expected_len.saturating_add(1))
        .read_to_end(&mut actual)
        .is_err()
    {
        return false;
    }
    actual == expected
}

fn parse_sha256(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut decoded = [0_u8; 32];
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return None;
    }
    for (index, pair) in pairs.iter().enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Some(decoded)
}

fn valid_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn valid_instance_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), PostgresSidecarError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), PostgresSidecarError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(feature = "postgres-key-store")]
    use std::sync::Mutex;
    #[cfg(feature = "postgres-key-store")]
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(all(feature = "desktop-vault", unix))]
    use openbot_application::tenant::package::{
        LoadedTenantPackage, TenantPackageFiles, validate_tenant_package,
    };
    #[cfg(all(feature = "desktop-vault", unix))]
    use openbot_domain::vault::{SecretKind, SecretPrincipal};
    #[cfg(all(feature = "desktop-vault", unix))]
    use openbot_infra::auth::single_user::desktop_local::{
        CurrentOsUserAppDataRoot, DESKTOP_LOCAL_ACTOR_ID, DesktopLocalAuthorityStore,
    };
    #[cfg(all(feature = "desktop-vault", unix))]
    use openbot_infra::db::desktop_local::DesktopLocalDatabaseOrigin;
    #[cfg(all(feature = "desktop-vault", unix))]
    use openbot_infra::db::initialization::DatabaseOrigin;
    use serde_json::{Value, json};
    #[cfg(all(feature = "desktop-vault", unix))]
    use uuid::Uuid;

    use super::*;
    #[cfg(all(feature = "desktop-vault", unix))]
    use crate::desktop_local_bootstrap::{DesktopLocalCompositionError, bootstrap_running_sidecar};
    #[cfg(all(feature = "desktop-vault", unix))]
    use crate::desktop_vault::ReviewedDesktopVaultKeyStoreService;
    #[cfg(all(feature = "desktop-vault", unix))]
    use crate::os_secret_store::{OsSecretStore, OsSecretStoreError};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "openbot-postgres-bundle-{name}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn signing_identity() -> ReviewedPostgresSigningIdentity {
        ReviewedPostgresSigningIdentity::from_reviewed_release(
            "Developer ID Application: Example Product (ABCDE12345)",
        )
        .unwrap()
    }

    fn materialize_bundle(name: &str) -> (PathBuf, PostgresBundleDigest) {
        let root = root(name);
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir(root.join("share")).unwrap();
        for (relative, bytes) in [
            (expected_program_paths()[0], b"postgres".as_slice()),
            (expected_program_paths()[1], b"initdb".as_slice()),
            (expected_program_paths()[2], b"pg_ctl".as_slice()),
            ("share/timezonesets/Default", b"timezone".as_slice()),
        ] {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, bytes).unwrap();
            #[cfg(unix)]
            if relative.starts_with("bin/") {
                use std::os::unix::fs::PermissionsExt as _;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let digest = write_manifest(&root);
        (root, digest)
    }

    fn write_manifest(root: &Path) -> PostgresBundleDigest {
        let mut files = BTreeMap::new();
        for relative in inventory_files(root).unwrap() {
            files.insert(
                relative.clone(),
                encode_hex(&sha256_file(&root.join(relative)).unwrap()),
            );
        }
        let manifest = json!({
            "schema": MANIFEST_SCHEMA,
            "schema_version": MANIFEST_SCHEMA_VERSION,
            "platform": expected_platform(),
            "arch": std::env::consts::ARCH,
            "postgresql_version": POSTGRES_VERSION,
            "source_archive_sha256": POSTGRES_SOURCE_SHA256,
            "release_epoch": ENGINE_RELEASE_EPOCH,
            "minimum_compatible_core": env!("CARGO_PKG_VERSION"),
            "signing_identity": signing_identity().as_str(),
            "programs": {
                "postgres": expected_program_paths()[0],
                "initdb": expected_program_paths()[1],
                "pg_ctl": expected_program_paths()[2],
            },
            "files": files,
        });
        let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        fs::write(root.join(MANIFEST_FILE), &bytes).unwrap();
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        PostgresBundleDigest(digest)
    }

    fn rewrite_manifest(root: &Path, mutate: impl FnOnce(&mut Value)) -> PostgresBundleDigest {
        let mut value: Value =
            serde_json::from_slice(&fs::read(root.join(MANIFEST_FILE)).unwrap()).unwrap();
        mutate(&mut value);
        let bytes = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(root.join(MANIFEST_FILE), &bytes).unwrap();
        PostgresBundleDigest(Sha256::digest(&bytes).into())
    }

    #[test]
    fn complete_manifest_tree_opens_and_debug_redacts_paths() {
        let (root, digest) = materialize_bundle("valid");
        let bundle = VerifiedPostgresBundle::open(&root, digest, &signing_identity()).unwrap();
        assert_eq!(bundle.root(), root);
        assert!(bundle.postgres().ends_with(expected_program_paths()[0]));
        assert!(bundle.initdb().ends_with(expected_program_paths()[1]));
        assert!(bundle.pg_ctl().ends_with(expected_program_paths()[2]));
        assert!(!format!("{bundle:?}").contains(root.to_str().unwrap()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn digest_tree_inventory_and_fixed_fields_each_fail_closed() {
        let (root, digest) = materialize_bundle("negative");
        assert!(matches!(
            VerifiedPostgresBundle::open(&root, PostgresBundleDigest([0; 32]), &signing_identity()),
            Err(PostgresSidecarError::ManifestDigest)
        ));
        fs::write(root.join(expected_program_paths()[0]), b"tampered").unwrap();
        assert!(matches!(
            VerifiedPostgresBundle::open(&root, digest, &signing_identity()),
            Err(PostgresSidecarError::FileDigest)
        ));
        fs::remove_dir_all(root).unwrap();

        let (root, digest) = materialize_bundle("extra");
        fs::write(root.join("unregistered.dll"), b"extra").unwrap();
        assert!(matches!(
            VerifiedPostgresBundle::open(&root, digest, &signing_identity()),
            Err(PostgresSidecarError::BundleShape)
        ));
        fs::remove_dir_all(root).unwrap();

        let (root, _) = materialize_bundle("shape");
        let digest = rewrite_manifest(&root, |manifest| {
            manifest["postgresql_version"] = json!("17.10");
        });
        assert!(matches!(
            VerifiedPostgresBundle::open(&root, digest, &signing_identity()),
            Err(PostgresSidecarError::BundleShape)
        ));
        fs::remove_dir_all(root).unwrap();

        let (root, _) = materialize_bundle("uppercase-hash");
        let digest = rewrite_manifest(&root, |manifest| {
            let key = expected_program_paths()[0];
            let uppercase = manifest["files"][key]
                .as_str()
                .unwrap()
                .to_ascii_uppercase();
            manifest["files"][key] = json!(uppercase);
        });
        assert!(matches!(
            VerifiedPostgresBundle::open(&root, digest, &signing_identity()),
            Err(PostgresSidecarError::BundleShape)
        ));
        fs::remove_dir_all(root).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let (root, digest) = materialize_bundle("symlink");
            symlink(root.join("share"), root.join("linked-share")).unwrap();
            assert!(matches!(
                VerifiedPostgresBundle::open(&root, digest, &signing_identity()),
                Err(PostgresSidecarError::BundleShape)
            ));
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn path_and_signing_identity_domains_are_closed() {
        assert!(matches!(
            safe_join(Path::new("/tmp/root"), "../escape"),
            Err(PostgresSidecarError::BundleShape)
        ));
        for rejected in [
            "",
            "Developer ID Application: OpenBot Fixture",
            "Developer ID Application: line\nbreak",
        ] {
            assert!(ReviewedPostgresSigningIdentity::from_reviewed_release(rejected).is_err());
        }
        assert!(ReviewedPostgresSigningIdentity::from_reviewed_release("Example Corp").is_ok());
        let pins = include_str!("../../../tools/postgres-pins.toml");
        assert!(pins.contains(&format!("version = \"{POSTGRES_VERSION}\"")));
        assert!(pins.contains(&format!("sha256 = \"{POSTGRES_SOURCE_SHA256}\"")));
    }

    #[test]
    fn start_lock_is_exclusive_private_and_never_removes_a_replacement() {
        let lock_root = root("lock");
        fs::create_dir(&lock_root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&lock_root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let instance = "a".repeat(64);
        let digest = PostgresBundleDigest([0x42; 32]);
        let first = PostgresStartLock::acquire(&lock_root, &instance, digest).unwrap();
        let lock_path = first.path.clone();
        assert!(matches!(
            PostgresStartLock::acquire(&lock_root, &instance, digest),
            Err(PostgresSidecarError::StartLockHeld)
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&lock_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(first);
        assert!(!lock_path.exists());

        let replacement_guard = PostgresStartLock::acquire(&lock_root, &instance, digest).unwrap();
        let replacement_path = replacement_guard.path.clone();
        fs::remove_file(&replacement_path).unwrap();
        fs::write(&replacement_path, b"replacement").unwrap();
        drop(replacement_guard);
        assert_eq!(fs::read(&replacement_path).unwrap(), b"replacement");
        fs::remove_dir_all(lock_root).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let wide = root("wide-lock-root");
            fs::create_dir(&wide).unwrap();
            fs::set_permissions(&wide, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(matches!(
                PostgresStartLock::acquire(&wide, &instance, digest),
                Err(PostgresSidecarError::BundleShape)
            ));
            fs::remove_dir_all(wide).unwrap();
        }
    }

    #[cfg(feature = "postgres-supervisor")]
    #[test]
    fn supervisor_paths_cluster_state_configuration_and_stale_lock_are_closed() {
        let app_root = root("supervisor-paths");
        let instance = "c".repeat(64);
        let data_dir = app_root.join(format!("postgresql-17-{instance}"));
        fs::create_dir_all(&data_dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&app_root, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o700)).unwrap();
        }
        validate_supervisor_paths(&app_root, &instance, &data_dir).unwrap();
        assert_eq!(
            data_directory_origin(&data_dir).unwrap(),
            PostgresSidecarOrigin::Fresh
        );
        fs::write(data_dir.join("partial"), b"partial").unwrap();
        assert!(matches!(
            data_directory_origin(&data_dir),
            Err(PostgresSidecarError::DataDirectoryInvalid)
        ));
        fs::remove_file(data_dir.join("partial")).unwrap();
        fs::write(data_dir.join("PG_VERSION"), b"16\n").unwrap();
        assert!(matches!(
            data_directory_origin(&data_dir),
            Err(PostgresSidecarError::DataDirectoryInvalid)
        ));
        fs::write(data_dir.join("PG_VERSION"), b"17\n").unwrap();
        assert_eq!(
            data_directory_origin(&data_dir).unwrap(),
            PostgresSidecarOrigin::Existing
        );
        write_runtime_configuration(&data_dir, 55482).unwrap();
        let settings = fs::read_to_string(data_dir.join("postgresql.auto.conf")).unwrap();
        let hba = fs::read_to_string(data_dir.join("pg_hba.conf")).unwrap();
        assert!(settings.contains("listen_addresses = '127.0.0.1'"));
        assert!(settings.contains("port = 55482"));
        assert!(settings.contains("password_encryption = 'scram-sha-256'"));
        assert!(hba.contains("127.0.0.1/32 scram-sha-256"));

        let mut stale =
            PostgresStartLock::acquire(&app_root, &instance, PostgresBundleDigest([0x66; 32]))
                .unwrap();
        let stale_path = stale.path.clone();
        stale.preserve_on_drop();
        drop(stale);
        assert!(stale_path.is_file());
        let source = include_str!("postgres_sidecar.rs");
        let production = source.split("\nmod tests {").next().unwrap();
        for forbidden in [
            ["PG", "PASSWORD"].concat(),
            ["--pw", "file"].concat(),
            ["std::env::", "var"].concat(),
        ] {
            assert!(
                !production.contains(&forbidden),
                "secret fallback appeared: {forbidden}"
            );
        }
        assert!(production.contains(".stdin(Stdio::piped())"));
        fs::remove_file(stale_path).unwrap();
        fs::remove_dir_all(app_root).unwrap();
    }

    #[cfg(feature = "postgres-key-store")]
    struct MemorySecretStore {
        value: Mutex<Option<Vec<u8>>>,
        writes: AtomicUsize,
        replace_on_write: bool,
    }

    #[cfg(feature = "postgres-key-store")]
    impl MemorySecretStore {
        fn empty() -> Self {
            Self {
                value: Mutex::new(None),
                writes: AtomicUsize::new(0),
                replace_on_write: false,
            }
        }

        fn with_value(value: Vec<u8>) -> Self {
            Self {
                value: Mutex::new(Some(value)),
                writes: AtomicUsize::new(0),
                replace_on_write: false,
            }
        }

        fn racing() -> Self {
            Self {
                value: Mutex::new(None),
                writes: AtomicUsize::new(0),
                replace_on_write: true,
            }
        }
    }

    #[cfg(feature = "postgres-key-store")]
    impl PostgresSecretStore for MemorySecretStore {
        fn read(
            &self,
            _service: &str,
            _account: &str,
        ) -> Result<Option<PostgresStoredSecret>, PostgresSecretStoreError> {
            Ok(self
                .value
                .lock()
                .unwrap()
                .clone()
                .map(PostgresStoredSecret::from_owned_bytes))
        }

        fn write(
            &self,
            _service: &str,
            _account: &str,
            secret: &[u8],
        ) -> Result<(), PostgresSecretStoreError> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            *self.value.lock().unwrap() = Some(if self.replace_on_write {
                let mut replacement = secret.to_vec();
                replacement[0] = if replacement[0] == b'0' { b'1' } else { b'0' };
                replacement
            } else {
                secret.to_vec()
            });
            Ok(())
        }
    }

    #[cfg(all(feature = "desktop-vault", unix))]
    struct MemoryVaultStore {
        value: Mutex<Option<Vec<u8>>>,
        writes: AtomicUsize,
    }

    #[cfg(all(feature = "desktop-vault", unix))]
    impl MemoryVaultStore {
        fn empty() -> Self {
            Self {
                value: Mutex::new(None),
                writes: AtomicUsize::new(0),
            }
        }
    }

    #[cfg(all(feature = "desktop-vault", unix))]
    impl OsSecretStore for MemoryVaultStore {
        fn read(
            &self,
            _service: &str,
            _account: &str,
        ) -> Result<Option<SecretBytes>, OsSecretStoreError> {
            Ok(self.value.lock().unwrap().clone().map(SecretBytes::new))
        }

        fn write(
            &self,
            _service: &str,
            _account: &str,
            secret: &[u8],
        ) -> Result<(), OsSecretStoreError> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            *self.value.lock().unwrap() = Some(secret.to_vec());
            Ok(())
        }
    }

    #[cfg(feature = "postgres-supervisor")]
    fn supervisor_test_paths(name: &str) -> (PathBuf, String, PathBuf) {
        let app_root = root(name);
        let instance = "e".repeat(64);
        let data_dir = app_root.join(format!("postgresql-17-{instance}"));
        fs::create_dir_all(&data_dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&app_root, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o700)).unwrap();
        }
        (app_root, instance, data_dir)
    }

    #[cfg(feature = "postgres-supervisor")]
    #[tokio::test]
    async fn version_failure_precedes_secret_store_and_releases_unstarted_lock() {
        let (bundle_root, digest) = materialize_bundle("bad-version-process");
        let bundle =
            VerifiedPostgresBundle::open(&bundle_root, digest, &signing_identity()).unwrap();
        let (app_root, instance, data_dir) = supervisor_test_paths("bad-version-app");
        let store = MemorySecretStore::empty();
        let service = ReviewedPostgresKeyStoreService::from_reviewed_release(
            "com.example.product.postgresql.bad-version",
        )
        .unwrap();
        assert!(matches!(
            PostgresSidecarSupervisor::start(
                bundle, &app_root, &instance, &data_dir, &store, &service,
            )
            .await,
            Err(PostgresSidecarError::VersionMismatch)
        ));
        assert_eq!(store.writes.load(Ordering::Relaxed), 0);
        assert!(
            !fs::read_dir(&app_root)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains("start-lock"))
        );
        fs::remove_dir_all(app_root).unwrap();
        fs::remove_dir_all(bundle_root).unwrap();
    }

    #[cfg(all(feature = "postgres-supervisor", unix))]
    fn materialize_failing_initdb_bundle() -> (PathBuf, PostgresBundleDigest) {
        let root = root("failing-initdb");
        fs::create_dir_all(root.join("bin")).unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        for (relative, label) in [
            (expected_program_paths()[0], "postgres"),
            (expected_program_paths()[1], "initdb"),
            (expected_program_paths()[2], "pg_ctl"),
        ] {
            let script = format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"{label} (PostgreSQL) {POSTGRES_VERSION}\"; exit 0; fi\nexit 17\n"
            );
            let path = root.join(relative);
            fs::write(&path, script).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let digest = write_manifest(&root);
        (root, digest)
    }

    #[cfg(all(feature = "postgres-supervisor", unix))]
    #[tokio::test]
    async fn initdb_failure_keeps_persisted_secret_but_starts_no_process_and_releases_lock() {
        let (bundle_root, digest) = materialize_failing_initdb_bundle();
        let bundle =
            VerifiedPostgresBundle::open(&bundle_root, digest, &signing_identity()).unwrap();
        let (app_root, instance, data_dir) = supervisor_test_paths("failing-initdb-app");
        let store = MemorySecretStore::empty();
        let service = ReviewedPostgresKeyStoreService::from_reviewed_release(
            "com.example.product.postgresql.failing-initdb",
        )
        .unwrap();
        assert!(matches!(
            PostgresSidecarSupervisor::start(
                bundle, &app_root, &instance, &data_dir, &store, &service,
            )
            .await,
            Err(PostgresSidecarError::InitdbFailed)
        ));
        assert_eq!(store.writes.load(Ordering::Relaxed), 1);
        assert!(store.value.lock().unwrap().is_some());
        assert!(fs::read_dir(&data_dir).unwrap().next().is_none());
        assert!(
            !fs::read_dir(&app_root)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains("start-lock"))
        );
        fs::remove_dir_all(app_root).unwrap();
        fs::remove_dir_all(bundle_root).unwrap();
    }

    #[cfg(all(feature = "postgres-supervisor", unix))]
    fn materialize_exiting_postgres_bundle() -> (PathBuf, PostgresBundleDigest) {
        let root = root("exiting-postgres");
        fs::create_dir_all(root.join("bin")).unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        let postgres = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"postgres (PostgreSQL) {POSTGRES_VERSION}\"; exit 0; fi\nexit 17\n"
        );
        let initdb = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"initdb (PostgreSQL) {POSTGRES_VERSION}\"; exit 0; fi\ndata=\nwhile [ $# -gt 0 ]; do if [ \"$1\" = \"--pgdata\" ]; then data=$2; shift 2; else shift; fi; done\nIFS= read -r first\nIFS= read -r second\n[ -n \"$data\" ] && [ \"$first\" = \"$second\" ] || exit 18\nprintf '17\\n' > \"$data/PG_VERSION\"\nexit 0\n"
        );
        let pg_ctl = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"pg_ctl (PostgreSQL) {POSTGRES_VERSION}\"; exit 0; fi\nexit 17\n"
        );
        for (relative, script) in [
            (expected_program_paths()[0], postgres),
            (expected_program_paths()[1], initdb),
            (expected_program_paths()[2], pg_ctl),
        ] {
            let path = root.join(relative);
            fs::write(&path, script).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let digest = write_manifest(&root);
        (root, digest)
    }

    #[cfg(all(feature = "postgres-supervisor", unix))]
    #[tokio::test]
    async fn child_exit_before_ready_is_explicit_and_releases_confirmed_dead_lock() {
        let (bundle_root, digest) = materialize_exiting_postgres_bundle();
        let bundle =
            VerifiedPostgresBundle::open(&bundle_root, digest, &signing_identity()).unwrap();
        let (app_root, instance, data_dir) = supervisor_test_paths("exiting-postgres-app");
        let store = MemorySecretStore::empty();
        let service = ReviewedPostgresKeyStoreService::from_reviewed_release(
            "com.example.product.postgresql.exiting",
        )
        .unwrap();
        assert!(matches!(
            PostgresSidecarSupervisor::start(
                bundle, &app_root, &instance, &data_dir, &store, &service,
            )
            .await,
            Err(PostgresSidecarError::ExitedBeforeReady)
        ));
        assert_eq!(store.writes.load(Ordering::Relaxed), 1);
        assert!(
            !fs::read_dir(&app_root)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains("start-lock"))
        );
        fs::remove_dir_all(app_root).unwrap();
        fs::remove_dir_all(bundle_root).unwrap();
    }

    #[cfg(all(feature = "postgres-supervisor", unix))]
    fn materialize_host_postgres_bundle(bin_dir: &Path) -> (PathBuf, PostgresBundleDigest) {
        let root = root("host-postgres");
        fs::create_dir_all(root.join("bin")).unwrap();
        for relative in expected_program_paths() {
            let name = Path::new(relative).file_name().unwrap();
            fs::copy(bin_dir.join(name), root.join(relative)).unwrap();
        }
        let digest = write_manifest(&root);
        (root, digest)
    }

    #[cfg(all(feature = "desktop-vault", unix))]
    fn loaded_desktop_package(tenant_id: &str) -> LoadedTenantPackage {
        let files = TenantPackageFiles {
            brand: format!("tenant: {{ id: {tenant_id}, product_name: Desktop Local }}"),
            agents: "agents: [{ id: desktop-assistant, name: Assistant, title: Local Assistant, role_description: Help locally., type: built-in, system_prompt: Answer carefully. }]".to_owned(),
            channels: "channels: [{ id: desktop-home, name: Home, description: Local home., permitted_agents: [desktop-assistant], allowed_groups: [all] }]".to_owned(),
            model: "model: { provider: openai, credential_secret_ref: openai-key, default_model: gpt-4.1 }".to_owned(),
            knowledge: "sources: []".to_owned(),
        };
        LoadedTenantPackage::new(
            validate_tenant_package(files).unwrap(),
            "/desktop-local/package".to_owned(),
            "d".repeat(64),
        )
        .unwrap()
    }

    #[cfg(all(feature = "desktop-vault", unix))]
    async fn fixed_application_database_exists(running: &RunningPostgresSidecar) -> bool {
        let connection = running.connection();
        let mut config = tokio_postgres::Config::new();
        config
            .host(connection.host())
            .port(connection.port())
            .user(connection.user())
            .password(connection.expose_password())
            .dbname("postgres");
        let (client, driver) = config.connect(NoTls).await.unwrap();
        let driver = tokio::spawn(driver);
        let exists = client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_database WHERE datname='openbot')",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        drop(client);
        tokio::time::timeout(Duration::from_secs(2), driver)
            .await
            .expect("probe driver close deadline")
            .expect("probe driver task join")
            .expect("probe driver close");
        exists
    }

    #[cfg(all(feature = "desktop-vault", unix))]
    #[tokio::test]
    #[ignore = "需要本机PostgreSQL 17.11 binaries；设置OPENBOT_TEST_POSTGRES_BIN_DIR后运行"]
    async fn verified_sidecar_composes_batch79_before_clean_shutdown() {
        let bin_dir = PathBuf::from(std::env::var_os("OPENBOT_TEST_POSTGRES_BIN_DIR").unwrap());
        let (bundle_root, digest) = materialize_host_postgres_bundle(&bin_dir);
        let signing = signing_identity();
        let service = ReviewedPostgresKeyStoreService::from_reviewed_release(
            "com.example.product.postgresql.bootstrap-composition-test",
        )
        .unwrap();
        let secret_store = MemorySecretStore::empty();
        let vault_store = MemoryVaultStore::empty();
        let vault_service = ReviewedDesktopVaultKeyStoreService::from_reviewed_release(
            "com.example.product.desktop-vault.composition-test",
        )
        .unwrap();
        let app_root = root("bootstrap-composition-app");
        let authority_store = DesktopLocalAuthorityStore::new(
            CurrentOsUserAppDataRoot::from_current_os_user_app_data(&app_root).unwrap(),
        );
        let installation = authority_store.load_or_create_installation().unwrap();
        let instance = installation.authority().instance_id().to_owned();
        let data_dir = installation.sidecar_data_dir().to_owned();
        let correct_package =
            loaded_desktop_package(installation.authority().auth_context().tenant().as_str());
        let wrong_package = loaded_desktop_package(&format!("desktop-local-{}", "f".repeat(64)));

        let bundle = VerifiedPostgresBundle::open(&bundle_root, digest, &signing).unwrap();
        let running = PostgresSidecarSupervisor::start(
            bundle,
            &app_root,
            &instance,
            &data_dir,
            &secret_store,
            &service,
        )
        .await
        .unwrap();
        assert!(matches!(
            bootstrap_running_sidecar(installation.clone(), running, &wrong_package).await,
            Err(DesktopLocalCompositionError::PackageScope(_))
        ));
        assert!(
            !fs::read_dir(&app_root)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains("start-lock"))
        );

        let bundle = VerifiedPostgresBundle::open(&bundle_root, digest, &signing).unwrap();
        let running = PostgresSidecarSupervisor::start(
            bundle,
            &app_root,
            &instance,
            &data_dir,
            &secret_store,
            &service,
        )
        .await
        .unwrap();
        assert!(
            !fixed_application_database_exists(&running).await,
            "tenant mismatch must precede CREATE DATABASE"
        );
        running.shutdown().await.unwrap();

        let bundle = VerifiedPostgresBundle::open(&bundle_root, digest, &signing).unwrap();
        let running = PostgresSidecarSupervisor::start(
            bundle,
            &app_root,
            &instance,
            &data_dir,
            &secret_store,
            &service,
        )
        .await
        .unwrap();
        let data_plane = bootstrap_running_sidecar(installation.clone(), running, &correct_package)
            .await
            .unwrap();
        assert_eq!(
            data_plane.sidecar_origin(),
            Some(PostgresSidecarOrigin::Existing)
        );
        assert_eq!(
            data_plane.database_origin(),
            DesktopLocalDatabaseOrigin::Created
        );
        assert_eq!(
            data_plane.bootstrap_report().database_origin,
            DatabaseOrigin::Fresh
        );
        assert_eq!(data_plane.bootstrap_report().package.memberships_granted, 1);
        assert!(
            data_plane
                .bootstrap_report()
                .package
                .single_user_groups_ignored
        );
        assert_eq!(
            data_plane.auth_context().tenant(),
            installation.authority().auth_context().tenant()
        );
        let client = data_plane.pool().get().await.unwrap();
        let row = client
            .query_one(
                "SELECT current_database(), count(*)::bigint, \
                        EXISTS(SELECT 1 FROM public.user_roles WHERE user_id=$1 AND role='admin'), \
                        EXISTS(SELECT 1 FROM public.channel_memberships WHERE user_id=$1 AND channel_id='desktop-home') \
                 FROM public.users WHERE id=$1 GROUP BY current_database()",
                &[&DESKTOP_LOCAL_ACTOR_ID],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, String>(0), "openbot");
        assert_eq!(row.get::<_, i64>(1), 1);
        assert!(row.get::<_, bool>(2));
        assert!(row.get::<_, bool>(3));
        drop(client);
        let first_material = data_plane
            .load_application_key_material(&vault_store, &vault_service)
            .unwrap();
        assert_eq!(vault_store.writes.load(Ordering::Relaxed), 1);
        let credential_id = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        let credential_plaintext = SecretBytes::new(b"composition-vault-canary".to_vec());
        let sealed = first_material
            .credential_vault()
            .seal(
                &credential_id,
                SecretKind::Model,
                SecretPrincipal::Deployment,
                SecretPrincipal::Deployment,
                &credential_plaintext,
            )
            .unwrap();
        assert!(!format!("{first_material:?}").contains("composition-vault-canary"));
        drop(first_material);
        data_plane.shutdown().await.unwrap();

        let bundle = VerifiedPostgresBundle::open(&bundle_root, digest, &signing).unwrap();
        let running = PostgresSidecarSupervisor::start(
            bundle,
            &app_root,
            &instance,
            &data_dir,
            &secret_store,
            &service,
        )
        .await
        .unwrap();
        let restarted = bootstrap_running_sidecar(installation, running, &correct_package)
            .await
            .unwrap();
        assert_eq!(
            restarted.database_origin(),
            DesktopLocalDatabaseOrigin::Existing
        );
        assert_eq!(
            restarted.bootstrap_report().database_origin,
            DatabaseOrigin::RustManaged
        );
        assert_eq!(restarted.bootstrap_report().package.memberships_granted, 0);
        let restarted_material = restarted
            .load_application_key_material(&vault_store, &vault_service)
            .unwrap();
        assert_eq!(vault_store.writes.load(Ordering::Relaxed), 1);
        assert_eq!(
            restarted_material
                .credential_vault()
                .open(
                    &credential_id,
                    SecretKind::Model,
                    SecretPrincipal::Deployment,
                    SecretPrincipal::Deployment,
                    &sealed,
                )
                .unwrap()
                .into_secret()
                .expose(),
            credential_plaintext.expose()
        );
        drop(restarted_material);
        restarted.shutdown().await.unwrap();

        assert!(
            !fs::read_dir(&app_root)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains("start-lock"))
        );
        fs::remove_dir_all(app_root).unwrap();
        fs::remove_dir_all(bundle_root).unwrap();
    }

    #[cfg(all(feature = "postgres-supervisor", unix))]
    #[tokio::test]
    #[ignore = "需要本机PostgreSQL 17.11 binaries；设置OPENBOT_TEST_POSTGRES_BIN_DIR后运行"]
    async fn verified_host_fixture_fresh_ready_shutdown_and_existing_restart() {
        let bin_dir = PathBuf::from(std::env::var_os("OPENBOT_TEST_POSTGRES_BIN_DIR").unwrap());
        let (bundle_root, digest) = materialize_host_postgres_bundle(&bin_dir);
        let signing = signing_identity();
        let service = ReviewedPostgresKeyStoreService::from_reviewed_release(
            "com.example.product.postgresql.process-test",
        )
        .unwrap();
        let store = MemorySecretStore::empty();
        let app_root = root("supervisor-process");
        let instance = "d".repeat(64);
        let data_dir = app_root.join(format!("postgresql-17-{instance}"));
        fs::create_dir_all(&data_dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&app_root, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o700)).unwrap();
        }

        let bundle = VerifiedPostgresBundle::open(&bundle_root, digest, &signing).unwrap();
        let running = PostgresSidecarSupervisor::start(
            bundle, &app_root, &instance, &data_dir, &store, &service,
        )
        .await
        .unwrap();
        assert_eq!(running.origin(), PostgresSidecarOrigin::Fresh);
        let first_password = running.connection().expose_password().to_vec();
        assert!(!format!("{running:?}").contains(std::str::from_utf8(&first_password).unwrap()));
        for relative in ["postgresql.auto.conf", "pg_hba.conf", "postmaster.opts"] {
            let path = data_dir.join(relative);
            if path.is_file() {
                let bytes = fs::read(path).unwrap();
                assert!(
                    !bytes
                        .windows(first_password.len())
                        .any(|window| window == first_password),
                    "raw SCRAM secret leaked into {relative}"
                );
            }
        }
        probe_running_sidecar(&running).await;
        running.shutdown().await.unwrap();
        assert!(
            !fs::read_dir(&app_root)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains("start-lock"))
        );

        let bundle = VerifiedPostgresBundle::open(&bundle_root, digest, &signing).unwrap();
        let restarted = PostgresSidecarSupervisor::start(
            bundle, &app_root, &instance, &data_dir, &store, &service,
        )
        .await
        .unwrap();
        assert_eq!(restarted.origin(), PostgresSidecarOrigin::Existing);
        assert_eq!(restarted.connection().expose_password(), first_password);
        probe_running_sidecar(&restarted).await;
        restarted.shutdown().await.unwrap();

        let bundle = VerifiedPostgresBundle::open(&bundle_root, digest, &signing).unwrap();
        let unclean = PostgresSidecarSupervisor::start(
            bundle, &app_root, &instance, &data_dir, &store, &service,
        )
        .await
        .unwrap();
        let unclean_port = unclean.connection().port();
        drop(unclean);
        let closed_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if tokio::net::TcpStream::connect(("127.0.0.1", unclean_port))
                .await
                .is_err()
            {
                break;
            }
            assert!(tokio::time::Instant::now() < closed_deadline);
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let stale_lock = app_root.join(format!(".postgresql-17-{instance}.start-lock-v1"));
        assert!(stale_lock.is_file());
        let bundle = VerifiedPostgresBundle::open(&bundle_root, digest, &signing).unwrap();
        assert!(matches!(
            PostgresSidecarSupervisor::start(
                bundle, &app_root, &instance, &data_dir, &store, &service,
            )
            .await,
            Err(PostgresSidecarError::StartLockHeld)
        ));
        fs::remove_file(stale_lock).unwrap();
        fs::remove_dir_all(app_root).unwrap();
        fs::remove_dir_all(bundle_root).unwrap();
    }

    #[cfg(all(feature = "postgres-supervisor", unix))]
    async fn probe_running_sidecar(running: &RunningPostgresSidecar) {
        let connection = running.connection();
        let mut config = tokio_postgres::Config::new();
        config
            .host(connection.host())
            .port(connection.port())
            .user(connection.user())
            .password(connection.expose_password())
            .dbname("postgres");
        let (client, driver) = config.connect(NoTls).await.unwrap();
        let driver = tokio::spawn(driver);
        let row = client
            .query_one(
                "SELECT current_setting('server_version_num'), current_setting('data_directory'), current_setting('listen_addresses'), current_setting('password_encryption'), (SELECT bool_and(auth_method='scram-sha-256') FROM pg_hba_file_rules WHERE type LIKE 'host%' AND error IS NULL)",
                &[],
            )
            .await
            .unwrap();
        assert!(row.get::<_, String>(0).starts_with("17"));
        assert_eq!(
            fs::canonicalize(row.get::<_, String>(1)).unwrap(),
            fs::canonicalize(running.data_dir()).unwrap()
        );
        assert_eq!(row.get::<_, String>(2), "127.0.0.1");
        assert_eq!(row.get::<_, String>(3), "scram-sha-256");
        assert!(row.get::<_, bool>(4));
        drop(client);
        driver.abort();
    }

    #[cfg(feature = "postgres-key-store")]
    fn secret_lock(name: &str) -> (PathBuf, PostgresStartLock) {
        let root = root(name);
        fs::create_dir(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let lock =
            PostgresStartLock::acquire(&root, &"b".repeat(64), PostgresBundleDigest([0x55; 32]))
                .unwrap();
        (root, lock)
    }

    #[cfg(feature = "postgres-key-store")]
    #[test]
    fn secret_load_create_restart_corrupt_and_reconciliation_are_closed() {
        let service = ReviewedPostgresKeyStoreService::from_reviewed_release(
            "com.example.product.postgresql",
        )
        .unwrap();
        let (root, lock) = secret_lock("secret");
        let store = MemorySecretStore::empty();
        let first = lock.load_or_create_scram_secret(&store, &service).unwrap();
        assert_eq!(first.expose().len(), 64);
        assert!(
            first
                .expose()
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        );
        let second = lock.load_or_create_scram_secret(&store, &service).unwrap();
        assert_eq!(first.expose(), second.expose());
        assert_eq!(store.writes.load(Ordering::Relaxed), 1);
        assert!(!format!("{first:?}").contains(std::str::from_utf8(first.expose()).unwrap()));
        drop(lock);
        fs::remove_dir_all(root).unwrap();

        let (root, lock) = secret_lock("corrupt-secret");
        let corrupt = MemorySecretStore::with_value(vec![b'Z'; 64]);
        assert!(matches!(
            lock.load_or_create_scram_secret(&corrupt, &service),
            Err(PostgresSecretStoreError::Corrupt)
        ));
        assert_eq!(corrupt.writes.load(Ordering::Relaxed), 0);
        drop(lock);
        fs::remove_dir_all(root).unwrap();

        let (root, lock) = secret_lock("racing-secret");
        let racing = MemorySecretStore::racing();
        assert!(matches!(
            lock.load_or_create_scram_secret(&racing, &service),
            Err(PostgresSecretStoreError::ReconciliationRequired)
        ));
        drop(lock);
        fs::remove_dir_all(root).unwrap();

        for rejected in [
            "",
            "ab",
            "Com.Example.Product.PostgreSQL",
            "com.example.openbot.postgresql",
            "spaces invalid",
        ] {
            assert!(ReviewedPostgresKeyStoreService::from_reviewed_release(rejected).is_err());
        }
    }

    #[cfg(all(feature = "postgres-key-store", target_os = "macos"))]
    #[test]
    fn macos_private_keychain_persists_one_instance_secret_without_default_keychain_state() {
        use security_framework::os::macos::keychain::CreateOptions;

        let keychain_root = root("private-keychain");
        fs::create_dir(&keychain_root).unwrap();
        let keychain_path = keychain_root.join("postgres-test.keychain-db");
        let mut options = CreateOptions::new();
        options
            .password("test-only-private-keychain-password")
            .prompt_user(false);
        let keychain = options.create(&keychain_path).unwrap();
        let cleanup_keychain = keychain.clone();
        let store = MacOsKeychainPostgresSecretStore::from_keychain(keychain);
        let service = ReviewedPostgresKeyStoreService::from_reviewed_release(
            "com.example.product.postgresql.test",
        )
        .unwrap();
        let (lock_root, lock) = secret_lock("private-keychain-lock");
        let first = lock.load_or_create_scram_secret(&store, &service).unwrap();
        let second = lock.load_or_create_scram_secret(&store, &service).unwrap();
        assert_eq!(first.expose(), second.expose());
        let account = format!("postgresql-17-{}", lock.instance_id);
        let (_password, item) = cleanup_keychain
            .find_generic_password(service.as_str(), &account)
            .unwrap();
        item.delete();
        drop(first);
        drop(second);
        drop(lock);
        drop(store);
        fs::remove_dir_all(lock_root).unwrap();
        fs::remove_dir_all(keychain_root).unwrap();
    }
}
