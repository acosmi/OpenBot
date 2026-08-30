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
use std::sync::Arc;

use openbot_contracts::engine::ENGINE_RELEASE_EPOCH;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

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
            _file: file,
        })
    }
}

impl Drop for PostgresStartLock {
    fn drop(&mut self) {
        if lock_file_matches(&self.path, &self.bytes) {
            let _ = fs::remove_file(&self.path);
            if let Some(parent) = self.path.parent() {
                let _ = sync_directory(parent);
            }
        }
    }
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
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::{Value, json};

    use super::*;

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
        let mut files = BTreeMap::new();
        for relative in [
            expected_program_paths()[0],
            expected_program_paths()[1],
            expected_program_paths()[2],
            "share/timezonesets/Default",
        ] {
            files.insert(
                relative.to_owned(),
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
        (root, PostgresBundleDigest(digest))
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
}
