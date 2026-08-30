//! Desktop Local current-OS-user + app-instance authority source.
//!
//! The current OS user is represented by the caller-provided *per-user application data root*, not
//! by `USER`/`USERNAME` environment variables. A non-secret random instance ID is elected once in
//! that root with a no-clobber hard-link transaction. The resulting deployment/tenant plus the
//! single local actor form the identity described by v4 §6.1.

#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use deadpool_postgres::Pool;
use openbot_application::tenant::package::{TenantPackageAudienceContext, TenantPackageError};
use openbot_contracts::auth::{AuthContext, AuthContextBuilder, AuthGeneration, Role};
use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};

use super::initialize_canonical_principal;
use crate::db::InfraError;

const FILE_NAME: &str = "desktop-instance-v1";
const FILE_HEADER: &str = "openbot-desktop-instance-v1";
const INSTANCE_PREFIX: &str = "instance=";
const INSTANCE_BYTES: usize = 32;
const INSTANCE_HEX_LEN: usize = INSTANCE_BYTES * 2;
const FILE_MAX_BYTES: u64 = 128;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Desktop Local's only actor inside its per-user, per-instance deployment.
pub const DESKTOP_LOCAL_ACTOR_ID: &str = "desktop-local-user";

/// Canonical non-routable profile email for the per-instance local principal.
pub const DESKTOP_LOCAL_EMAIL: &str = "desktop-local@localhost.invalid";

/// Canonical profile name; it does not claim to be the OS account's display name.
pub const DESKTOP_LOCAL_NAME: &str = "Desktop Local User";

/// Stable failure classes; paths, OS usernames and file contents never enter the error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DesktopLocalAuthorityError {
    /// The caller did not provide an absolute per-user app-data root.
    #[error("desktop_local_app_data_root_invalid")]
    InvalidAppDataRoot,
    /// Filesystem or CSPRNG operation was unavailable.
    #[error("desktop_local_identity_unavailable")]
    Unavailable,
    /// Existing identity bytes, type, version or encoding were invalid.
    #[error("desktop_local_identity_corrupt")]
    Corrupt,
    /// On Unix the existing identity file is visible to group/other principals.
    #[error("desktop_local_identity_permissions_insecure")]
    InsecurePermissions,
}

/// Host assertion that this absolute directory came from the current OS user's app-data API.
///
/// This type does not infer identity from environment variables. A future Tauri setup obtains its
/// path from `AppHandle::path().app_data_dir()` and makes that trust decision at the host edge.
#[derive(Clone, PartialEq, Eq)]
pub struct CurrentOsUserAppDataRoot(PathBuf);

impl core::fmt::Debug for CurrentOsUserAppDataRoot {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("CurrentOsUserAppDataRoot(<redacted>)")
    }
}

impl CurrentOsUserAppDataRoot {
    /// Wrap an absolute path already resolved by the host's current-user app-data API.
    pub fn from_current_os_user_app_data(
        path: impl Into<PathBuf>,
    ) -> Result<Self, DesktopLocalAuthorityError> {
        let path = path.into();
        if !path.is_absolute() || path.as_os_str().is_empty() {
            return Err(DesktopLocalAuthorityError::InvalidAppDataRoot);
        }
        Ok(Self(path))
    }

    /// Borrow the asserted app-data root.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Stable Desktop Local authority derived from one elected app-instance identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopLocalAuthority {
    instance_id: Arc<str>,
    auth: AuthContext,
}

impl DesktopLocalAuthority {
    /// Lowercase 256-bit app-instance identifier. It is an identifier, not a credential.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Borrow the single-user admin authority.
    #[must_use]
    pub const fn auth_context(&self) -> &AuthContext {
        &self.auth
    }

    /// Consume the record for Desktop window/session assembly.
    #[must_use]
    pub fn into_auth_context(self) -> AuthContext {
        self.auth
    }

    /// Atomically provision/repair the canonical PostgreSQL user and sole admin role.
    pub async fn provision_postgres(&self, pool: &Pool) -> Result<(), InfraError> {
        initialize_canonical_principal(
            pool,
            DESKTOP_LOCAL_ACTOR_ID,
            DESKTOP_LOCAL_EMAIL,
            DESKTOP_LOCAL_NAME,
        )
        .await
    }

    /// Build the exact single-user audience context used by Tenant Package synchronization.
    pub fn tenant_package_audience_context(
        &self,
    ) -> Result<TenantPackageAudienceContext, TenantPackageError> {
        TenantPackageAudienceContext::single_user(self.auth.actor().clone())
    }
}

/// Persistent authority source for one current-user application data root.
#[derive(Clone)]
pub struct DesktopLocalAuthorityStore {
    inner: Arc<StoreInner>,
}

struct StoreInner {
    root: CurrentOsUserAppDataRoot,
    mutation: Mutex<()>,
}

impl DesktopLocalAuthorityStore {
    /// Bind to one host-asserted current-user app-data root.
    #[must_use]
    pub fn new(root: CurrentOsUserAppDataRoot) -> Self {
        Self {
            inner: Arc::new(StoreInner {
                root,
                mutation: Mutex::new(()),
            }),
        }
    }

    /// Load the existing identity or atomically elect one fully-written CSPRNG candidate.
    pub fn load_or_create(&self) -> Result<DesktopLocalAuthority, DesktopLocalAuthorityError> {
        let _guard = self
            .inner
            .mutation
            .lock()
            .map_err(|_| DesktopLocalAuthorityError::Unavailable)?;
        prepare_root(self.inner.root.as_path())?;
        let path = self.inner.root.as_path().join(FILE_NAME);
        let instance = match read_instance(&path)? {
            Some(instance) => {
                cleanup_matching_temporary(self.inner.root.as_path(), instance)?;
                instance
            }
            None => elect_instance(self.inner.root.as_path(), &path)?,
        };
        Ok(authority(instance))
    }
}

fn prepare_root(root: &Path) -> Result<(), DesktopLocalAuthorityError> {
    match fs::symlink_metadata(root) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(DesktopLocalAuthorityError::InvalidAppDataRoot);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(root).map_err(|_| DesktopLocalAuthorityError::Unavailable)?;
        }
        Err(_) => return Err(DesktopLocalAuthorityError::Unavailable),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))
            .map_err(|_| DesktopLocalAuthorityError::Unavailable)?;
    }
    Ok(())
}

fn read_instance(path: &Path) -> Result<Option<[u8; INSTANCE_BYTES]>, DesktopLocalAuthorityError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(DesktopLocalAuthorityError::Unavailable),
    };
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > FILE_MAX_BYTES
    {
        return Err(DesktopLocalAuthorityError::Corrupt);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(DesktopLocalAuthorityError::InsecurePermissions);
        }
    }
    let raw = fs::read_to_string(path).map_err(|_| DesktopLocalAuthorityError::Corrupt)?;
    parse(&raw).map(Some)
}

fn elect_instance(
    root: &Path,
    final_path: &Path,
) -> Result<[u8; INSTANCE_BYTES], DesktopLocalAuthorityError> {
    let mut candidate = [0_u8; INSTANCE_BYTES];
    getrandom::fill(&mut candidate).map_err(|_| DesktopLocalAuthorityError::Unavailable)?;
    let encoded = render(candidate);
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = root.join(format!(
        ".{FILE_NAME}.tmp.{}.{}.{}",
        std::process::id(),
        sequence,
        &encode(candidate)[..16]
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|_| DesktopLocalAuthorityError::Unavailable)?;
        file.write_all(encoded.as_bytes())
            .map_err(|_| DesktopLocalAuthorityError::Unavailable)?;
        file.sync_all()
            .map_err(|_| DesktopLocalAuthorityError::Unavailable)?;
        drop(file);

        match fs::hard_link(&temporary, final_path) {
            Ok(()) => {
                sync_directory(root)?;
                remove_temporary(&temporary)?;
                sync_directory(root)?;
                Ok(candidate)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                remove_temporary(&temporary)?;
                read_instance(final_path)?.ok_or(DesktopLocalAuthorityError::Corrupt)
            }
            Err(_) => Err(DesktopLocalAuthorityError::Unavailable),
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn remove_temporary(path: &Path) -> Result<(), DesktopLocalAuthorityError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(DesktopLocalAuthorityError::Unavailable),
    }
}

fn cleanup_matching_temporary(
    root: &Path,
    instance: [u8; INSTANCE_BYTES],
) -> Result<(), DesktopLocalAuthorityError> {
    let prefix = format!(".{FILE_NAME}.tmp.");
    let mut removed = false;
    for entry in fs::read_dir(root).map_err(|_| DesktopLocalAuthorityError::Unavailable)? {
        let Ok(entry) = entry else {
            continue;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !metadata.file_type().is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > FILE_MAX_BYTES
        {
            continue;
        }
        let Ok(raw) = fs::read_to_string(entry.path()) else {
            continue;
        };
        if parse(&raw).ok() == Some(instance) {
            remove_temporary(&entry.path())?;
            removed = true;
        }
    }
    if removed {
        sync_directory(root)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), DesktopLocalAuthorityError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| DesktopLocalAuthorityError::Unavailable)
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), DesktopLocalAuthorityError> {
    Ok(())
}

fn authority(instance: [u8; INSTANCE_BYTES]) -> DesktopLocalAuthority {
    let instance_id = encode(instance);
    let scope_id = format!("desktop-local-{instance_id}");
    let auth = AuthContextBuilder::from_verified_session(
        DeploymentId::new(scope_id.clone()),
        TenantId::new(scope_id),
        ActorId::new(DESKTOP_LOCAL_ACTOR_ID),
        AuthGeneration::new(0),
        true,
    )
    .with_roles([Role::Admin, Role::User])
    .build();
    DesktopLocalAuthority {
        instance_id: Arc::from(instance_id),
        auth,
    }
}

fn render(instance: [u8; INSTANCE_BYTES]) -> String {
    format!("{FILE_HEADER}\n{INSTANCE_PREFIX}{}\n", encode(instance))
}

fn parse(raw: &str) -> Result<[u8; INSTANCE_BYTES], DesktopLocalAuthorityError> {
    let mut lines = raw.lines();
    if lines.next() != Some(FILE_HEADER) {
        return Err(DesktopLocalAuthorityError::Corrupt);
    }
    let encoded = lines
        .next()
        .and_then(|line| line.strip_prefix(INSTANCE_PREFIX))
        .ok_or(DesktopLocalAuthorityError::Corrupt)?;
    if lines.next().is_some() {
        return Err(DesktopLocalAuthorityError::Corrupt);
    }
    decode(encoded).ok_or(DesktopLocalAuthorityError::Corrupt)
}

fn encode(bytes: [u8; INSTANCE_BYTES]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(INSTANCE_HEX_LEN);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn decode(value: &str) -> Option<[u8; INSTANCE_BYTES]> {
    if value.len() != INSTANCE_HEX_LEN {
        return None;
    }
    let mut decoded = [0_u8; INSTANCE_BYTES];
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return None;
    }
    for (index, pair) in pairs.iter().enumerate() {
        decoded[index] = lower_hex(pair[0])? << 4 | lower_hex(pair[1])?;
    }
    Some(decoded)
}

const fn lower_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::thread;

    use super::*;

    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "openbot-desktop-local-authority-{name}-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn store(path: &Path) -> DesktopLocalAuthorityStore {
        DesktopLocalAuthorityStore::new(
            CurrentOsUserAppDataRoot::from_current_os_user_app_data(path).unwrap(),
        )
    }

    #[test]
    fn first_run_and_restart_produce_the_same_single_user_admin_authority() {
        let root = root("restart");
        let first = store(&root).load_or_create().unwrap();
        let bytes = fs::read(root.join(FILE_NAME)).unwrap();
        let second = store(&root).load_or_create().unwrap();
        assert_eq!(first, second);
        assert_eq!(fs::read(root.join(FILE_NAME)).unwrap(), bytes);
        assert_eq!(first.instance_id().len(), INSTANCE_HEX_LEN);
        assert!(
            first
                .instance_id()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
        let auth = first.auth_context();
        assert!(auth.is_single_user());
        assert!(auth.has_role(Role::Admin));
        assert!(auth.has_role(Role::User));
        assert_eq!(auth.auth_generation().get(), 0);
        assert_eq!(auth.actor().as_str(), DESKTOP_LOCAL_ACTOR_ID);
        assert_eq!(auth.deployment().as_str(), auth.tenant().as_str());
        assert!(auth.deployment().as_str().ends_with(first.instance_id()));
        assert_eq!(
            first
                .tenant_package_audience_context()
                .unwrap()
                .single_user_principal(),
            Some(auth.actor())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(root.join(FILE_NAME))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert_eq!(
            fs::read_dir(&root).unwrap().filter_map(Result::ok).count(),
            1,
            "no temporary identity file may remain"
        );

        let stale = root.join(format!(".{FILE_NAME}.tmp.stale"));
        fs::hard_link(root.join(FILE_NAME), &stale).unwrap();
        assert!(stale.exists());
        assert_eq!(store(&root).load_or_create().unwrap(), first);
        assert!(!stale.exists(), "matching crash residue must be reclaimed");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn independent_store_objects_elect_exactly_one_cross_thread_candidate() {
        let root = root("race");
        let workers = 32;
        let barrier = Arc::new(Barrier::new(workers));
        let mut handles = Vec::new();
        for _ in 0..workers {
            let root = root.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                store(&root)
                    .load_or_create()
                    .unwrap()
                    .instance_id()
                    .to_owned()
            }));
        }
        let identities = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert!(identities.iter().all(|identity| identity == &identities[0]));
        assert_eq!(
            fs::read_dir(&root).unwrap().filter_map(Result::ok).count(),
            1
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_symlink_and_broad_permissions_fail_closed() {
        let corrupt_root = root("corrupt");
        prepare_root(&corrupt_root).unwrap();
        fs::write(corrupt_root.join(FILE_NAME), "bad\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(
                corrupt_root.join(FILE_NAME),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        assert_eq!(
            store(&corrupt_root).load_or_create(),
            Err(DesktopLocalAuthorityError::Corrupt)
        );
        fs::remove_dir_all(corrupt_root).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt as _, symlink};

            let permission_root = root("permissions");
            let _ = store(&permission_root).load_or_create().unwrap();
            fs::set_permissions(
                permission_root.join(FILE_NAME),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap();
            assert_eq!(
                store(&permission_root).load_or_create(),
                Err(DesktopLocalAuthorityError::InsecurePermissions)
            );
            fs::remove_dir_all(permission_root).unwrap();

            let symlink_root = root("symlink");
            prepare_root(&symlink_root).unwrap();
            let target = symlink_root.join("target");
            fs::write(&target, render([7; INSTANCE_BYTES])).unwrap();
            symlink(&target, symlink_root.join(FILE_NAME)).unwrap();
            assert_eq!(
                store(&symlink_root).load_or_create(),
                Err(DesktopLocalAuthorityError::Corrupt)
            );
            fs::remove_dir_all(symlink_root).unwrap();
        }
    }

    #[test]
    fn format_is_closed_and_app_data_root_must_be_absolute() {
        assert_eq!(
            CurrentOsUserAppDataRoot::from_current_os_user_app_data("relative"),
            Err(DesktopLocalAuthorityError::InvalidAppDataRoot)
        );
        let bytes = [0xab; INSTANCE_BYTES];
        let encoded = encode(bytes);
        assert_eq!(decode(&encoded), Some(bytes));
        assert!(decode(&encoded.to_ascii_uppercase()).is_none());
        assert!(
            parse(&format!(
                "{FILE_HEADER}\n{INSTANCE_PREFIX}{encoded}\nextra=x\n"
            ))
            .is_err()
        );
        let forbidden_env_read = ["std::env", "::var"].concat();
        assert!(!include_str!("desktop_local.rs").contains(&forbidden_env_read));
    }
}
