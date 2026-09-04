//! Digest-before-spawn engine lifecycle with one-shot stdin boot and OS peer credentials.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
#[cfg(unix)]
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use openbot_contracts::engine::{
    ENGINE_PROTOCOL_VERSION, ENGINE_RELEASE_EPOCH, MAX_ENGINE_CONTROL_FRAME_BYTES,
};
use openbot_contracts::ids::{ComputerGeneration, ComputerId, TabId};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::browser::{BrowserInput, CdpInputPlan, CdpInputPlanError};
use crate::control::AuthorizedHumanInput;

use super::frame::{EngineFrame, EngineFrameError, EngineFrameReader, read_frame_hello};
use super::protocol::{
    BootCapability, BootToken, EngineCommandWire, EngineEventWire, EngineInputKindWire,
    EngineOperationId, EngineProtocolError, encode_command,
};
use super::scope::EngineRole;

#[cfg(windows)]
use openbot_windows_sandbox::{
    NamedPipeConnection, NamedPipeListener, RestrictedChild, SpawnPolicy,
};
#[cfg(any(unix, windows))]
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
#[cfg(unix)]
use tokio::net::UnixListener;
#[cfg(unix)]
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
#[cfg(unix)]
use tokio::process::{Child, Command};

#[cfg(unix)]
type EngineChild = Child;
#[cfg(windows)]
type EngineChild = RestrictedChild;
#[cfg(unix)]
type EnginePipeListener = UnixListener;
#[cfg(windows)]
type EnginePipeListener = NamedPipeListener;
#[cfg(unix)]
type EnginePipeConnection = tokio::net::UnixStream;
#[cfg(windows)]
type EnginePipeConnection = NamedPipeConnection;
#[cfg(unix)]
type EnginePipeReadHalf = OwnedReadHalf;
#[cfg(unix)]
type EnginePipeWriteHalf = OwnedWriteHalf;
#[cfg(windows)]
type EnginePipeReadHalf = tokio::io::ReadHalf<NamedPipeConnection>;
#[cfg(windows)]
type EnginePipeWriteHalf = tokio::io::WriteHalf<NamedPipeConnection>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(windows)]
const WINDOWS_JOB_MAX_PROCESSES: u32 = 32;
#[cfg(windows)]
const WINDOWS_JOB_MAX_MEMORY_BYTES: usize = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Copy)]
enum EngineLaunchBoundary {
    PlatformDefault,
    #[cfg(target_os = "linux")]
    Runsc,
}

/// Positive in-container proof that the P1 probe is running under the authority-created runsc
/// bundle, not silently launching an unconfined Linux Engine on the host.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
pub struct RunscAttestation {
    _private: (),
}

#[cfg(target_os = "linux")]
impl RunscAttestation {
    /// Require the exact P1 host/rootfs and runsc marker before direct Engine spawn is enabled.
    pub fn detect() -> Result<Self, EngineProcessError> {
        if std::env::consts::ARCH != "x86_64"
            || !Path::new("/proc/gvisor/kernel_is_gvisor").is_file()
        {
            return Err(EngineProcessError::SandboxUnavailable);
        }
        let os_release = fs::read_to_string("/etc/os-release")?;
        let fields = os_release
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key, value.trim_matches('"')))
            .collect::<BTreeMap<_, _>>();
        if fields.get("ID") != Some(&"ubuntu") || fields.get("VERSION_ID") != Some(&"24.04") {
            return Err(EngineProcessError::SandboxUnavailable);
        }
        Ok(Self { _private: () })
    }
}

/// Expected SHA-256 of the release-owned sidecar manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineBundleDigest([u8; 32]);

impl EngineBundleDigest {
    /// Parse one exact lowercase/uppercase SHA-256 string from signed release metadata.
    pub fn from_hex(value: &str) -> Result<Self, EngineProcessError> {
        if value.len() != 64 {
            return Err(EngineProcessError::BundleDigest);
        }
        let mut bytes = [0_u8; 32];
        let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(EngineProcessError::BundleDigest);
        }
        for (index, pair) in pairs.iter().enumerate() {
            bytes[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
        }
        Ok(Self(bytes))
    }
}

/// A bundle whose signed manifest and all named executable/ASAR/fuse files were hash-verified.
#[derive(Clone, Debug)]
pub struct EngineBundle {
    root: PathBuf,
    executable: PathBuf,
    manifest_sha256: [u8; 32],
}

impl EngineBundle {
    /// Load and verify a bundle against the release-owned manifest digest before any spawn.
    pub fn open(
        root: impl Into<PathBuf>,
        expected: EngineBundleDigest,
    ) -> Result<Self, EngineProcessError> {
        let root = root.into();
        let manifest_path = root.join("manifest.json");
        let manifest_bytes = fs::read(&manifest_path).map_err(EngineProcessError::Io)?;
        let manifest_sha256: [u8; 32] = Sha256::digest(&manifest_bytes).into();
        if manifest_sha256 != expected.0 {
            return Err(EngineProcessError::BundleDigest);
        }
        let manifest: BundleManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|_| EngineProcessError::BundleShape)?;
        if manifest.schema != "openbot-engine-bundle"
            || manifest.schema_version != 1
            || manifest.platform != expected_platform()
            || manifest.arch != std::env::consts::ARCH
            || manifest.electron_version != "43.3.0"
            || manifest.release_epoch != ENGINE_RELEASE_EPOCH
            || manifest.protocol_version != u64::from(ENGINE_PROTOCOL_VERSION)
            || manifest.product_name != "Acosmi Engine Fixture"
            || manifest.bundle_id != "com.acosmi.engine.fixture"
            || manifest.fuse_wire != "000011001"
            || EngineBundleDigest::from_hex(&manifest.asar_header_sha256).is_err()
        {
            return Err(EngineProcessError::BundleShape);
        }
        let required_files = [
            manifest.executable.as_str(),
            manifest.fuse_file.as_str(),
            manifest.app_asar.as_str(),
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
        let manifest_files = manifest
            .files
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        if manifest_files != required_files {
            return Err(EngineProcessError::BundleShape);
        }
        for (relative, expected) in &manifest.files {
            let path = safe_join(&root, relative)?;
            if sha256_file(&path)? != *expected {
                return Err(EngineProcessError::BundleDigest);
            }
        }
        let executable = safe_join(&root, &manifest.executable)?;
        if !executable.is_file()
            || !safe_join(&root, &manifest.fuse_file)?.is_file()
            || !safe_join(&root, &manifest.app_asar)?.is_file()
        {
            return Err(EngineProcessError::BundleShape);
        }
        Ok(Self {
            root,
            executable,
            manifest_sha256,
        })
    }

    /// Verified bundle root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Verified main executable.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Manifest digest checked before spawn.
    #[must_use]
    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleManifest {
    schema: String,
    schema_version: u64,
    platform: String,
    arch: String,
    electron_version: String,
    release_epoch: u64,
    protocol_version: u64,
    product_name: String,
    bundle_id: String,
    executable: String,
    fuse_file: String,
    app_asar: String,
    asar_header_sha256: String,
    fuse_wire: String,
    files: BTreeMap<String, String>,
}

/// Platform sandbox fidelity surfaced to readiness/diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineSandboxFidelity {
    /// OS sandbox profile applied; P1 macOS implementation.
    Enforced,
    /// Partial platform constraints only. It must be visible but may run browser/component roles.
    Degraded,
    /// No constraints could be applied; engine launch is refused.
    Unavailable,
}

/// Complete authority-owned launch input.
pub struct EngineLaunchConfig {
    bundle: EngineBundle,
    role: EngineRole,
    computer_id: ComputerId,
    generation: ComputerGeneration,
    profile_dir: PathBuf,
    temp_dir: PathBuf,
    #[cfg(target_os = "linux")]
    runsc_virtual_display: bool,
}

impl EngineLaunchConfig {
    /// Construct launch state. Paths are canonicalized/created before the sandbox profile is minted.
    #[must_use]
    pub fn new(
        bundle: EngineBundle,
        role: EngineRole,
        computer_id: ComputerId,
        generation: ComputerGeneration,
        profile_dir: impl Into<PathBuf>,
        temp_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            bundle,
            role,
            computer_id,
            generation,
            profile_dir: profile_dir.into(),
            temp_dir: temp_dir.into(),
            #[cfg(target_os = "linux")]
            runsc_virtual_display: false,
        }
    }

    /// Bind the P1 runsc probe to its fixed in-container Xvfb display without accepting a free
    /// DISPLAY string or inheriting host environment.
    #[cfg(target_os = "linux")]
    #[must_use]
    pub fn with_runsc_probe_display(mut self) -> Self {
        self.runsc_virtual_display = true;
        self
    }
}

/// One role session's actual renderer/frame evidence.
pub struct StartedSession {
    /// Rust-validated JPEG frame.
    pub frame: EngineFrame,
    /// OS renderer PID reported by Electron and used only for conformance diagnostics.
    pub renderer_pid: u32,
    /// OS process creation time paired with PID to prevent PID-reuse ambiguity.
    pub renderer_creation_time: f64,
    /// Electron ProcessMetric OS-level sandbox result; required true on macOS/Windows.
    pub renderer_sandboxed: bool,
    /// Internal opaque origin; never a caller-provided URL.
    pub origin: String,
}

#[cfg(any(unix, windows))]
/// Live engine process. Drop kills the child; normal shutdown additionally proves bounded cleanup.
pub struct EngineProcess {
    child: EngineChild,
    child_pid: u32,
    main_creation_time: f64,
    control_reader: BufReader<EnginePipeReadHalf>,
    control_writer: EnginePipeWriteHalf,
    frame_reader: BufReader<EnginePipeReadHalf>,
    _frame_writer: EnginePipeWriteHalf,
    role: EngineRole,
    computer_id: ComputerId,
    generation: ComputerGeneration,
    _runtime: RuntimeDirectory,
    operation_sequence: AtomicU64,
    active_tab: Option<TabId>,
    frame_decoder: Option<EngineFrameReader>,
    fidelity: EngineSandboxFidelity,
}

#[cfg(not(any(unix, windows)))]
/// Placeholder type for unsupported targets; partial launch is refused.
pub struct EngineProcess;

#[cfg(any(unix, windows))]
impl EngineProcess {
    /// Spawn, authenticate both UDS connections, and complete hello/ready.
    pub async fn launch(config: EngineLaunchConfig) -> Result<Self, EngineProcessError> {
        Self::launch_with(config, EngineLaunchBoundary::PlatformDefault).await
    }

    /// P1 probe-only Linux launch after the surrounding runsc bundle has been positively attested.
    #[cfg(target_os = "linux")]
    pub async fn launch_inside_runsc(
        config: EngineLaunchConfig,
        _attestation: RunscAttestation,
    ) -> Result<Self, EngineProcessError> {
        if !config.runsc_virtual_display {
            return Err(EngineProcessError::SandboxUnavailable);
        }
        Self::launch_with(config, EngineLaunchBoundary::Runsc).await
    }

    async fn launch_with(
        config: EngineLaunchConfig,
        boundary: EngineLaunchBoundary,
    ) -> Result<Self, EngineProcessError> {
        fs::create_dir_all(&config.profile_dir).map_err(EngineProcessError::Io)?;
        fs::create_dir_all(&config.temp_dir).map_err(EngineProcessError::Io)?;
        let profile_dir = config
            .profile_dir
            .canonicalize()
            .map_err(EngineProcessError::Io)?;
        let temp_dir = config
            .temp_dir
            .canonicalize()
            .map_err(EngineProcessError::Io)?;
        let bundle_root = config
            .bundle
            .root
            .canonicalize()
            .map_err(EngineProcessError::Io)?;

        let token = BootToken::random()?;
        let runtime = RuntimeDirectory::create(&token)?;
        let control_listener = bind_listener(&runtime.control_pipe)?;
        let frame_listener = bind_listener(&runtime.frame_pipe)?;
        let boot = BootCapability::new(
            &runtime.control_pipe,
            &runtime.frame_pipe,
            &token,
            &config.role,
            &config.computer_id,
            config.generation,
        )?;

        let (mut child, fidelity) = spawn_engine(
            &config.bundle.executable,
            &bundle_root,
            &profile_dir,
            &temp_dir,
            &runtime,
            boundary,
            #[cfg(target_os = "linux")]
            config.runsc_virtual_display,
        )?;
        let child_pid = child_pid(&child)?;
        write_boot(&mut child, &boot.line()?).await?;

        let (control, frame) = tokio::time::timeout(CONNECT_TIMEOUT, async {
            tokio::try_join!(
                accept_listener(control_listener),
                accept_listener(frame_listener)
            )
        })
        .await
        .map_err(|_| EngineProcessError::ConnectTimeout)??;
        verify_peer(&control, &mut child, child_pid)?;
        verify_peer(&frame, &mut child, child_pid)?;
        let (control_read, control_writer) = split_connection(control);
        let (frame_read, frame_writer) = split_connection(frame);
        let mut control_reader = BufReader::new(control_read);
        let mut frame_reader = BufReader::new(frame_read);
        tokio::time::timeout(CONNECT_TIMEOUT, read_frame_hello(&mut frame_reader, &token))
            .await
            .map_err(|_| EngineProcessError::FrameHelloTimeout)??;
        let hello = tokio::time::timeout(CONNECT_TIMEOUT, read_event(&mut control_reader))
            .await
            .map_err(|_| EngineProcessError::HelloTimeout)??;
        match hello {
            EngineEventWire::Hello { token: echoed } if echoed == token.hex() => {}
            _ => return Err(EngineProcessError::HelloAuthentication),
        }
        let ready = tokio::time::timeout(CONNECT_TIMEOUT, read_event(&mut control_reader))
            .await
            .map_err(|_| EngineProcessError::ReadyTimeout)??;
        let main_creation_time = match ready {
            EngineEventWire::Ready {
                main_pid,
                main_creation_time,
                protocol_version,
            } => {
                if main_pid != child_pid {
                    return Err(EngineProcessError::ReadyPid);
                }
                verify_main_creation_time(&child, main_creation_time)?;
                if protocol_version != ENGINE_PROTOCOL_VERSION {
                    return Err(EngineProcessError::ReadyProtocol);
                }
                main_creation_time
            }
            EngineEventWire::Error { code, .. } => return Err(reported(code)),
            _ => return Err(EngineProcessError::ReadyIdentity),
        };

        Ok(Self {
            child,
            child_pid,
            main_creation_time,
            control_reader,
            control_writer,
            frame_reader,
            _frame_writer: frame_writer,
            role: config.role,
            computer_id: config.computer_id,
            generation: config.generation,
            _runtime: runtime,
            operation_sequence: AtomicU64::new(0),
            active_tab: None,
            frame_decoder: None,
            fidelity,
        })
    }

    /// Main process PID authenticated by UDS peer credentials.
    #[must_use]
    pub const fn pid(&self) -> u32 {
        self.child_pid
    }

    /// OS process creation time paired with the peer-authenticated main PID.
    #[must_use]
    pub const fn main_creation_time(&self) -> f64 {
        self.main_creation_time
    }

    /// Platform confinement fidelity.
    #[must_use]
    pub const fn sandbox_fidelity(&self) -> EngineSandboxFidelity {
        self.fidelity
    }

    /// Start the one allowed P1 session, concurrently validating control outcome and binary frame.
    pub async fn start_session(
        &mut self,
        tab_id: TabId,
    ) -> Result<StartedSession, EngineProcessError> {
        let operation = self.next_operation()?;
        let command =
            EngineCommandWire::start(&operation, &self.computer_id, self.generation, &tab_id);
        self.control_writer
            .write_all(&encode_command(&command)?)
            .await?;
        let mut decoder = EngineFrameReader::new(
            self.role.kind(),
            self.computer_id.clone(),
            self.generation,
            tab_id.clone(),
        );
        let (event, frame) = read_operation_frame(
            &mut self.control_reader,
            &mut self.frame_reader,
            &mut decoder,
            &operation,
        )
        .await?;
        match event {
            EngineEventWire::Started {
                operation_id,
                tab_id: echoed_tab,
                renderer_pid,
                renderer_creation_time,
                renderer_sandboxed,
                node_exposed,
                origin,
            } if operation_id == operation.as_str()
                && echoed_tab == tab_id.as_str()
                && renderer_pid != 0
                && renderer_identity_matches(&self.child, renderer_pid, renderer_creation_time)
                && renderer_sandbox_signal_matches(renderer_sandboxed)
                && !node_exposed
                && internal_origin_matches(self.role.kind(), &origin) =>
            {
                self.active_tab = Some(tab_id);
                self.frame_decoder = Some(decoder);
                Ok(StartedSession {
                    frame,
                    renderer_pid,
                    renderer_creation_time,
                    renderer_sandboxed,
                    origin,
                })
            }
            _ => Err(EngineProcessError::StartedIdentity),
        }
    }

    /// Apply one freshly authorized ordinary input through protocol-v2 and require an exact
    /// operation-bound acknowledgement. Frames remain on the independent [`Self::next_frame`]
    /// channel. `SecretInsert` is rejected before any control-pipe write.
    pub async fn apply_human_input(
        &mut self,
        authorization: AuthorizedHumanInput,
        input: &BrowserInput,
        now: time::OffsetDateTime,
    ) -> Result<(), EngineProcessError> {
        if authorization.computer_id() != &self.computer_id
            || authorization.computer_generation() != self.generation
            || self.active_tab.as_ref() != Some(authorization.tab_id())
            || now >= authorization.expires_at()
        {
            return Err(EngineProcessError::InputAuthority);
        }
        let plan = CdpInputPlan::try_from(input).map_err(EngineProcessError::InputPlan)?;
        let expected_kind = EngineInputKindWire::from_plan(&plan);
        let operation = self.next_operation()?;
        let command = EngineCommandWire::input(
            &operation,
            &self.computer_id,
            self.generation,
            authorization.tab_id(),
            &plan,
        );
        let bytes = encode_command(&command)?;
        self.control_writer.write_all(&bytes).await?;
        let event = tokio::time::timeout(COMMAND_TIMEOUT, read_event(&mut self.control_reader))
            .await
            .map_err(|_| EngineProcessError::CommandTimeout)??;
        match event {
            EngineEventWire::InputApplied {
                operation_id,
                tab_id,
                input_kind,
            } if operation_id == operation.as_str()
                && tab_id == authorization.tab_id().as_str()
                && input_kind == expected_kind =>
            {
                Ok(())
            }
            EngineEventWire::Error { operation_id, code } => {
                Err(reported_for(&operation, operation_id, code))
            }
            _ => Err(EngineProcessError::InputAppliedIdentity),
        }
    }

    /// Read the next independently framed image for the active session. Input acknowledgements and
    /// Screen delivery intentionally remain separate channels so Page.startScreencast can replace
    /// the current conformance capture without changing the authority API.
    pub async fn next_frame(&mut self) -> Result<EngineFrame, EngineProcessError> {
        let decoder = self
            .frame_decoder
            .as_mut()
            .ok_or(EngineProcessError::InputAuthority)?;
        decoder
            .read(&mut self.frame_reader)
            .await
            .map_err(Into::into)
    }

    /// Stop one exact tab and require an operation-bound acknowledgement.
    pub async fn stop_session(&mut self, tab_id: &TabId) -> Result<(), EngineProcessError> {
        let operation = self.next_operation()?;
        let command =
            EngineCommandWire::stop(&operation, &self.computer_id, self.generation, tab_id);
        self.control_writer
            .write_all(&encode_command(&command)?)
            .await?;
        let event = tokio::time::timeout(COMMAND_TIMEOUT, read_event(&mut self.control_reader))
            .await
            .map_err(|_| EngineProcessError::CommandTimeout)??;
        match event {
            EngineEventWire::Stopped { operation_id } if operation_id == operation.as_str() => {
                self.active_tab = None;
                self.frame_decoder = None;
                Ok(())
            }
            EngineEventWire::Error { operation_id, code } => {
                Err(reported_for(&operation, operation_id, code))
            }
            _ => Err(EngineProcessError::StoppedIdentity),
        }
    }

    /// Graceful bounded shutdown. Timeout kills the process and returns a hard failure.
    pub async fn shutdown(mut self) -> Result<(), EngineProcessError> {
        let operation = self.next_operation()?;
        self.control_writer
            .write_all(&encode_command(&EngineCommandWire::shutdown(&operation))?)
            .await?;
        let event = tokio::time::timeout(COMMAND_TIMEOUT, read_event(&mut self.control_reader))
            .await
            .map_err(|_| EngineProcessError::CommandTimeout)??;
        match event {
            EngineEventWire::ShutdownComplete { operation_id }
                if operation_id == operation.as_str() => {}
            EngineEventWire::Error { operation_id, code } => {
                return Err(reported_for(&operation, operation_id, code));
            }
            _ => return Err(EngineProcessError::ShutdownIdentity),
        }
        let status = match tokio::time::timeout(SHUTDOWN_TIMEOUT, wait_child(&mut self.child)).await
        {
            Ok(status) => status?,
            Err(_) => {
                kill_child(&mut self.child).await?;
                return Err(EngineProcessError::ShutdownTimeout);
            }
        };
        if !status.success() {
            return Err(EngineProcessError::Exited);
        }
        Ok(())
    }

    fn next_operation(&self) -> Result<EngineOperationId, EngineProcessError> {
        let previous = self
            .operation_sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| EngineProcessError::OperationExhausted)?;
        EngineOperationId::new(format!("op-{}", previous + 1)).map_err(Into::into)
    }
}

#[cfg(any(unix, windows))]
async fn read_operation_frame(
    control_reader: &mut BufReader<EnginePipeReadHalf>,
    frame_reader: &mut BufReader<EnginePipeReadHalf>,
    decoder: &mut EngineFrameReader,
    operation: &EngineOperationId,
) -> Result<(EngineEventWire, EngineFrame), EngineProcessError> {
    tokio::time::timeout(COMMAND_TIMEOUT, async {
        let event = read_event(control_reader);
        let frame = decoder.read(frame_reader);
        tokio::pin!(event);
        tokio::pin!(frame);
        tokio::select! {
            event = &mut event => {
                let event = event?;
                if let EngineEventWire::Error { operation_id, code } = &event {
                    return Err(reported_for(operation, operation_id.clone(), code.clone()));
                }
                let frame = frame.await?;
                Ok((event, frame))
            }
            frame = &mut frame => {
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(frame_error) => {
                        if let Ok(Ok(EngineEventWire::Error { operation_id, code })) =
                            tokio::time::timeout(Duration::from_secs(1), &mut event).await
                        {
                            return Err(reported_for(operation, operation_id, code));
                        }
                        return Err(frame_error.into());
                    }
                };
                let event = event.await?;
                Ok((event, frame))
            }
        }
    })
    .await
    .map_err(|_| EngineProcessError::CommandTimeout)?
}

#[cfg(not(any(unix, windows)))]
impl EngineProcess {
    /// Unsupported targets never fall back to an unconfined Engine.
    pub async fn launch(_config: EngineLaunchConfig) -> Result<Self, EngineProcessError> {
        Err(EngineProcessError::UnsupportedPlatform)
    }
}

#[cfg(unix)]
fn bind_listener(path: &Path) -> Result<EnginePipeListener, EngineProcessError> {
    let listener = UnixListener::bind(path)?;
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

#[cfg(windows)]
fn bind_listener(path: &Path) -> Result<EnginePipeListener, EngineProcessError> {
    NamedPipeListener::bind(path).map_err(|_| EngineProcessError::SandboxUnavailable)
}

#[cfg(unix)]
async fn accept_listener(
    listener: EnginePipeListener,
) -> Result<EnginePipeConnection, EngineProcessError> {
    listener
        .accept()
        .await
        .map(|(stream, _)| stream)
        .map_err(Into::into)
}

#[cfg(windows)]
async fn accept_listener(
    listener: EnginePipeListener,
) -> Result<EnginePipeConnection, EngineProcessError> {
    listener
        .accept()
        .await
        .map_err(|_| EngineProcessError::SandboxUnavailable)
}

#[cfg(unix)]
fn split_connection(connection: EnginePipeConnection) -> (EnginePipeReadHalf, EnginePipeWriteHalf) {
    connection.into_split()
}

#[cfg(windows)]
fn split_connection(connection: EnginePipeConnection) -> (EnginePipeReadHalf, EnginePipeWriteHalf) {
    tokio::io::split(connection)
}

#[cfg(unix)]
fn verify_peer(
    stream: &EnginePipeConnection,
    child: &mut EngineChild,
    child_pid: u32,
) -> Result<(), EngineProcessError> {
    let credential = stream.peer_cred()?;
    if credential.pid().and_then(|pid| u32::try_from(pid).ok()) != Some(child_pid) {
        return Err(EngineProcessError::PeerCredential);
    }
    if child.try_wait()?.is_some() {
        return Err(EngineProcessError::Exited);
    }
    // Holding the live Child and observing it still running prevents PID reuse between spawn and
    // credential verification; the OS cannot reuse a PID while that exact child remains alive.
    Ok(())
}

#[cfg(windows)]
fn verify_peer(
    stream: &EnginePipeConnection,
    child: &mut EngineChild,
    child_pid: u32,
) -> Result<(), EngineProcessError> {
    if child.identity().pid() != child_pid {
        return Err(EngineProcessError::SpawnIdentity);
    }
    stream
        .verify_peer(child.identity())
        .map_err(|_| EngineProcessError::PeerCredential)?;
    if child
        .try_wait()
        .map_err(|_| EngineProcessError::SandboxUnavailable)?
        .is_some()
    {
        return Err(EngineProcessError::Exited);
    }
    Ok(())
}

async fn read_event<R>(reader: &mut R) -> Result<EngineEventWire, EngineProcessError>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    let read = reader.read_until(b'\n', &mut line).await?;
    if read == 0 || line.last() != Some(&b'\n') || line.len() > MAX_ENGINE_CONTROL_FRAME_BYTES {
        return Err(EngineProcessError::ControlFrame);
    }
    line.pop();
    serde_json::from_slice(&line).map_err(|_| EngineProcessError::ControlFrame)
}

#[cfg(unix)]
fn spawn_engine(
    executable: &Path,
    bundle_root: &Path,
    profile_dir: &Path,
    temp_dir: &Path,
    runtime: &RuntimeDirectory,
    boundary: EngineLaunchBoundary,
    #[cfg(target_os = "linux")] runsc_virtual_display: bool,
) -> Result<(EngineChild, EngineSandboxFidelity), EngineProcessError> {
    let (mut command, fidelity) = launch_command(
        executable,
        bundle_root,
        profile_dir,
        temp_dir,
        runtime,
        boundary,
        #[cfg(target_os = "linux")]
        runsc_virtual_display,
    )?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
        .spawn()
        .map(|child| (child, fidelity))
        .map_err(EngineProcessError::Io)
}

#[cfg(windows)]
fn spawn_engine(
    executable: &Path,
    bundle_root: &Path,
    profile_dir: &Path,
    temp_dir: &Path,
    _runtime: &RuntimeDirectory,
    boundary: EngineLaunchBoundary,
) -> Result<(EngineChild, EngineSandboxFidelity), EngineProcessError> {
    if !matches!(boundary, EngineLaunchBoundary::PlatformDefault) {
        return Err(EngineProcessError::SandboxUnavailable);
    }
    let policy = SpawnPolicy::new(
        executable,
        engine_args(profile_dir, temp_dir),
        bundle_root,
        profile_dir,
        temp_dir,
        WINDOWS_JOB_MAX_PROCESSES,
        WINDOWS_JOB_MAX_MEMORY_BYTES,
    )
    .map_err(|_| EngineProcessError::SandboxUnavailable)?;
    openbot_windows_sandbox::spawn_restricted(&policy)
        .map(|child| (child, EngineSandboxFidelity::Degraded))
        .map_err(|_| EngineProcessError::SandboxUnavailable)
}

#[cfg(unix)]
fn child_pid(child: &EngineChild) -> Result<u32, EngineProcessError> {
    child.id().ok_or(EngineProcessError::SpawnIdentity)
}

#[cfg(windows)]
fn child_pid(child: &EngineChild) -> Result<u32, EngineProcessError> {
    let pid = child.identity().pid();
    (pid != 0)
        .then_some(pid)
        .ok_or(EngineProcessError::SpawnIdentity)
}

#[cfg(unix)]
async fn write_boot(child: &mut EngineChild, line: &[u8]) -> Result<(), EngineProcessError> {
    let mut stdin = child.stdin.take().ok_or(EngineProcessError::BootPipe)?;
    stdin.write_all(line).await?;
    stdin.shutdown().await?;
    Ok(())
}

#[cfg(windows)]
async fn write_boot(child: &mut EngineChild, line: &[u8]) -> Result<(), EngineProcessError> {
    use std::io::Write as _;

    let mut stdin = child.take_stdin().ok_or(EngineProcessError::BootPipe)?;
    stdin.write_all(line)?;
    stdin.flush()?;
    drop(stdin);
    Ok(())
}

#[cfg(unix)]
fn verify_main_creation_time(
    _child: &EngineChild,
    reported: f64,
) -> Result<(), EngineProcessError> {
    (reported.is_finite() && reported > 0.0)
        .then_some(())
        .ok_or(EngineProcessError::ReadyCreationTime)
}

#[cfg(windows)]
fn verify_main_creation_time(child: &EngineChild, reported: f64) -> Result<(), EngineProcessError> {
    let expected = child
        .identity()
        .creation_unix_millis()
        .map_err(|_| EngineProcessError::ReadyCreationTime)?;
    (reported.is_finite() && reported == expected)
        .then_some(())
        .ok_or(EngineProcessError::ReadyCreationTime)
}

#[cfg(unix)]
fn renderer_identity_matches(_child: &EngineChild, pid: u32, creation_time: f64) -> bool {
    pid != 0 && creation_time.is_finite() && creation_time > 0.0
}

#[cfg(windows)]
fn renderer_identity_matches(child: &EngineChild, pid: u32, creation_time: f64) -> bool {
    child.verify_job_member(pid, creation_time).is_ok()
}

#[cfg(target_os = "linux")]
fn renderer_sandbox_signal_matches(reported: bool) -> bool {
    // Electron intentionally does not expose ProcessMetric.sandboxed on Linux. The P1 runsc probe
    // must keep this false and independently prove namespaces + Seccomp/NoNewPrivs from /proc.
    !reported
}

#[cfg(not(target_os = "linux"))]
fn renderer_sandbox_signal_matches(reported: bool) -> bool {
    reported
}

#[cfg(unix)]
async fn wait_child(
    child: &mut EngineChild,
) -> Result<std::process::ExitStatus, EngineProcessError> {
    child.wait().await.map_err(Into::into)
}

#[cfg(windows)]
async fn wait_child(
    child: &mut EngineChild,
) -> Result<std::process::ExitStatus, EngineProcessError> {
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|_| EngineProcessError::SandboxUnavailable)?
        {
            return Ok(status);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[cfg(unix)]
async fn kill_child(child: &mut EngineChild) -> Result<(), EngineProcessError> {
    child.kill().await.map_err(Into::into)
}

#[cfg(windows)]
async fn kill_child(child: &mut EngineChild) -> Result<(), EngineProcessError> {
    child
        .kill()
        .map_err(|_| EngineProcessError::SandboxUnavailable)
}

#[cfg(unix)]
fn launch_command(
    executable: &Path,
    bundle_root: &Path,
    profile_dir: &Path,
    temp_dir: &Path,
    runtime: &RuntimeDirectory,
    boundary: EngineLaunchBoundary,
    #[cfg(target_os = "linux")] runsc_virtual_display: bool,
) -> Result<(Command, EngineSandboxFidelity), EngineProcessError> {
    #[cfg(target_os = "macos")]
    {
        if !matches!(boundary, EngineLaunchBoundary::PlatformDefault) {
            return Err(EngineProcessError::SandboxUnavailable);
        }
        let profile = sandbox_profile(
            executable,
            bundle_root,
            profile_dir,
            temp_dir,
            &runtime.root,
        )?;
        fs::write(&runtime.sandbox_profile, profile)?;
        let mut command = Command::new("/usr/bin/sandbox-exec");
        command
            .arg("-f")
            .arg(&runtime.sandbox_profile)
            .arg(executable);
        append_engine_args(&mut command, profile_dir, temp_dir);
        Ok((command, EngineSandboxFidelity::Enforced))
    }
    #[cfg(target_os = "linux")]
    {
        if !matches!(boundary, EngineLaunchBoundary::Runsc) || !runsc_virtual_display {
            return Err(EngineProcessError::SandboxUnavailable);
        }
        let _ = (bundle_root, runtime);
        let mut command = Command::new(executable);
        append_engine_args(&mut command, profile_dir, temp_dir);
        command.env("DISPLAY", ":99").env_remove("XAUTHORITY");
        Ok((command, EngineSandboxFidelity::Enforced))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (
            executable,
            bundle_root,
            profile_dir,
            temp_dir,
            runtime,
            boundary,
        );
        Err(EngineProcessError::SandboxUnavailable)
    }
}

#[cfg(unix)]
fn append_engine_args(command: &mut Command, profile_dir: &Path, temp_dir: &Path) {
    command
        .args(engine_args(profile_dir, temp_dir))
        .env_remove("ELECTRON_RUN_AS_NODE")
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_EXTRA_CA_CERTS")
        .env_remove("ELECTRON_ENABLE_LOGGING")
        .env("TMPDIR", temp_dir);
}

fn engine_args(profile_dir: &Path, temp_dir: &Path) -> Vec<OsString> {
    [
        format!("--user-data-dir={}", profile_dir.display()),
        format!("--disk-cache-dir={}", temp_dir.join("cache").display()),
        "--disable-background-networking".into(),
        "--disable-component-update".into(),
        "--disable-default-apps".into(),
        "--disable-features=WebRtcAllowInputVolumeAdjustment".into(),
        "--disable-quic".into(),
        "--disable-sync".into(),
        "--no-default-browser-check".into(),
        "--no-first-run".into(),
        "--proxy-server=http=127.0.0.1:1;https=127.0.0.1:1".into(),
        "--proxy-bypass-list=<-loopback>".into(),
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

#[cfg(target_os = "macos")]
fn sandbox_profile(
    executable: &Path,
    bundle: &Path,
    profile: &Path,
    temp: &Path,
    runtime: &Path,
) -> Result<String, EngineProcessError> {
    let app = bundle.join("AcosmiEngine.app/Contents");
    let helpers = [
        app.join("Frameworks/Electron Helper.app/Contents/MacOS/Electron Helper"),
        app.join("Frameworks/Electron Helper (GPU).app/Contents/MacOS/Electron Helper (GPU)"),
        app.join("Frameworks/Electron Helper (Plugin).app/Contents/MacOS/Electron Helper (Plugin)"),
        app.join(
            "Frameworks/Electron Helper (Renderer).app/Contents/MacOS/Electron Helper (Renderer)",
        ),
        app.join(
            "Frameworks/Electron Framework.framework/Versions/A/Helpers/chrome_crashpad_handler",
        ),
    ];
    for path in std::iter::once(executable)
        .chain(std::iter::once(bundle))
        .chain(std::iter::once(profile))
        .chain(std::iter::once(temp))
        .chain(std::iter::once(runtime))
        .chain(helpers.iter().map(PathBuf::as_path))
    {
        let value = path
            .to_str()
            .ok_or(EngineProcessError::SandboxUnavailable)?;
        if value.contains(['"', '\\', '\n', '\r']) {
            return Err(EngineProcessError::SandboxUnavailable);
        }
    }
    Ok(format!(
        "(version 1)\n\
         (allow default)\n\
         (deny file-write*)\n\
         (allow file-write* (subpath \"{}\") (subpath \"{}\") (subpath \"{}\") (literal \"/dev/null\"))\n\
         (deny network-inbound)\n\
         (deny network-outbound)\n\
         (allow network-outbound (remote unix-socket))\n\
         (allow network-outbound (remote ip \"localhost:*\"))\n\
         (deny process-exec*)\n\
         (allow process-exec* (literal \"{}\"))\n\
         (allow process-exec* (with no-sandbox) (literal \"{}\") (literal \"{}\") (literal \"{}\") (literal \"{}\") (literal \"{}\"))\n",
        profile.display(),
        temp.display(),
        runtime.display(),
        executable.display(),
        helpers[0].display(),
        helpers[1].display(),
        helpers[2].display(),
        helpers[3].display(),
        helpers[4].display(),
    ))
}

#[cfg(unix)]
struct RuntimeDirectory {
    root: PathBuf,
    control_pipe: PathBuf,
    frame_pipe: PathBuf,
    #[cfg(target_os = "macos")]
    sandbox_profile: PathBuf,
}

#[cfg(unix)]
impl RuntimeDirectory {
    fn create(token: &BootToken) -> Result<Self, EngineProcessError> {
        #[cfg(target_os = "macos")]
        let base = Path::new("/private/tmp");
        #[cfg(not(target_os = "macos"))]
        let base = Path::new("/tmp");
        let root = base.join(format!("ob-eng-{}", &token.hex()[..16]));
        fs::create_dir(&root)?;
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        Ok(Self {
            control_pipe: root.join("control.sock"),
            frame_pipe: root.join("frame.sock"),
            #[cfg(target_os = "macos")]
            sandbox_profile: root.join("engine.sb"),
            root,
        })
    }
}

#[cfg(unix)]
impl Drop for RuntimeDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.control_pipe);
        let _ = fs::remove_file(&self.frame_pipe);
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(windows)]
struct RuntimeDirectory {
    control_pipe: PathBuf,
    frame_pipe: PathBuf,
}

#[cfg(windows)]
impl RuntimeDirectory {
    fn create(token: &BootToken) -> Result<Self, EngineProcessError> {
        let nonce = token.hex();
        if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(EngineProcessError::SandboxUnavailable);
        }
        Ok(Self {
            control_pipe: PathBuf::from(format!(r"\\.\pipe\ob-eng-{nonce}.control")),
            frame_pipe: PathBuf::from(format!(r"\\.\pipe\ob-eng-{nonce}.frame")),
        })
    }
}

fn internal_origin_matches(role: super::scope::EngineRoleKind, origin: &str) -> bool {
    match role {
        super::scope::EngineRoleKind::BrowserComputer => origin == "acosmi-engine://session",
        super::scope::EngineRoleKind::SandboxedComponent => origin == "component://session",
    }
}

fn reported(code: String) -> EngineProcessError {
    if code.is_empty()
        || code.len() > 64
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        EngineProcessError::ControlFrame
    } else {
        EngineProcessError::EngineReported(code)
    }
}

fn reported_for(
    operation: &EngineOperationId,
    echoed: Option<String>,
    code: String,
) -> EngineProcessError {
    if echoed
        .as_deref()
        .is_some_and(|value| value != operation.as_str())
    {
        EngineProcessError::ControlFrame
    } else {
        reported(code)
    }
}

fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, EngineProcessError> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(EngineProcessError::BundleShape);
    }
    Ok(root.join(path))
}

fn expected_platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("windows", "x86_64") => "windows-x64",
        _ => "unsupported",
    }
}

fn sha256_file(path: &Path) -> Result<String, EngineProcessError> {
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
    Ok(format!("{:x}", hash.finalize()))
}

fn hex_nibble(value: u8) -> Result<u8, EngineProcessError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(EngineProcessError::BundleDigest),
    }
}

/// Stable lifecycle/bundle failures.
#[derive(Debug, thiserror::Error)]
pub enum EngineProcessError {
    /// Filesystem/socket/process operation failed.
    #[error("engine_io")]
    Io(#[from] std::io::Error),
    /// Boot/control protocol construction failed.
    #[error("engine_protocol")]
    Protocol(#[from] EngineProtocolError),
    /// Binary frame was malformed, stale, or unauthenticated.
    #[error("engine_frame")]
    Frame(#[from] EngineFrameError),
    /// Pure BrowserInput→CDP plan rejected a non-ordinary input before any pipe write.
    #[error("engine_input_plan")]
    InputPlan(#[source] CdpInputPlanError),
    /// Signed manifest digest or a named file digest did not match.
    #[error("engine_bundle_digest")]
    BundleDigest,
    /// Manifest/path/fixed release fields were malformed.
    #[error("engine_bundle_shape")]
    BundleShape,
    /// Child PID was unavailable immediately after spawn.
    #[error("engine_spawn_identity")]
    SpawnIdentity,
    /// Child stdin was not available for the one-line boot capability.
    #[error("engine_boot_pipe")]
    BootPipe,
    /// Engine failed to connect both private pipes within the deadline.
    #[error("engine_connect_timeout")]
    ConnectTimeout,
    /// Connected frame pipe did not send its token preface within the deadline.
    #[error("engine_frame_hello_timeout")]
    FrameHelloTimeout,
    /// Connected control pipe did not send its token hello within the deadline.
    #[error("engine_hello_timeout")]
    HelloTimeout,
    /// Authenticated shim did not reach Electron ready within the deadline.
    #[error("engine_ready_timeout")]
    ReadyTimeout,
    /// UDS/Named Pipe peer credential did not equal the live spawned child.
    #[error("engine_peer_credential")]
    PeerCredential,
    /// Control hello token was wrong.
    #[error("engine_hello_authentication")]
    HelloAuthentication,
    /// Ready protocol/PID did not match Rust-owned state.
    #[error("engine_ready_identity")]
    ReadyIdentity,
    /// Ready main PID differed from the peer-authenticated spawned PID.
    #[error("engine_ready_pid")]
    ReadyPid,
    /// Ready main process creation time was absent/non-finite.
    #[error("engine_ready_creation_time")]
    ReadyCreationTime,
    /// Ready protocol version differed from the Rust constant.
    #[error("engine_ready_protocol")]
    ReadyProtocol,
    /// Control frame was oversized, malformed, unknown, or truncated.
    #[error("engine_control_frame")]
    ControlFrame,
    /// Session command exceeded its deadline.
    #[error("engine_command_timeout")]
    CommandTimeout,
    /// Started event did not echo the exact operation/tab or exposed Node.
    #[error("engine_started_identity")]
    StartedIdentity,
    /// Fresh HumanLease receipt did not match this process/generation/active tab.
    #[error("engine_input_authority")]
    InputAuthority,
    /// Input acknowledgement did not echo the exact operation/tab/kind.
    #[error("engine_input_applied_identity")]
    InputAppliedIdentity,
    /// Stop acknowledgement did not match the operation.
    #[error("engine_stopped_identity")]
    StoppedIdentity,
    /// Shutdown acknowledgement did not match the operation.
    #[error("engine_shutdown_identity")]
    ShutdownIdentity,
    /// Graceful process cleanup exceeded five seconds.
    #[error("engine_shutdown_timeout")]
    ShutdownTimeout,
    /// Child exited unexpectedly or non-zero.
    #[error("engine_exited")]
    Exited,
    /// Engine returned one bounded stable error code.
    #[error("engine_reported:{0}")]
    EngineReported(String),
    /// Operation counter exhausted rather than wrapping.
    #[error("engine_operation_exhausted")]
    OperationExhausted,
    /// Required macOS/Windows/Linux process confinement could not be applied.
    #[error("engine_sandbox_unavailable")]
    SandboxUnavailable,
    /// Platform implementation is intentionally absent rather than silently unconfined.
    #[error("engine_platform_unsupported")]
    UnsupportedPlatform,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{EngineBundle, EngineBundleDigest, EngineProcessError, safe_join};

    #[test]
    fn bundle_digest_parser_and_relative_paths_fail_closed() {
        assert!(EngineBundleDigest::from_hex(&"00".repeat(32)).is_ok());
        assert!(EngineBundleDigest::from_hex("00").is_err());
        assert!(matches!(
            safe_join(std::path::Path::new("/tmp/root"), "../escape"),
            Err(EngineProcessError::BundleShape)
        ));
    }

    #[test]
    fn manifest_digest_is_checked_before_manifest_shape() {
        let root = std::env::temp_dir().join(format!(
            "openbot-engine-bundle-negative-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join("manifest.json"), b"{}").expect("manifest");
        let result = EngineBundle::open(&root, EngineBundleDigest([0; 32]));
        assert!(matches!(result, Err(EngineProcessError::BundleDigest)));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_profile_confines_main_and_releases_only_bundle_helpers_to_chromium_sandbox() {
        let profile = super::sandbox_profile(
            std::path::Path::new("/Applications/AcosmiEngine.app/Contents/MacOS/AcosmiEngine"),
            std::path::Path::new("/Applications/AcosmiEngine.app"),
            std::path::Path::new("/private/tmp/profile"),
            std::path::Path::new("/private/tmp/temp"),
            std::path::Path::new("/private/tmp/runtime"),
        )
        .expect("profile");
        for required in [
            "(deny file-write*)",
            "(deny network-inbound)",
            "(deny network-outbound)",
            "(remote unix-socket)",
            "(remote ip \"localhost:*\")",
            "(deny process-exec*)",
            "(with no-sandbox)",
            "(literal \"/Applications/AcosmiEngine.app/Contents/MacOS/AcosmiEngine\")",
            "Electron Helper (Renderer).app/Contents/MacOS/Electron Helper (Renderer)",
            "chrome_crashpad_handler",
        ] {
            assert!(profile.contains(required), "missing `{required}`");
        }
        assert_eq!(profile.matches("(with no-sandbox)").count(), 1);
        assert!(
            super::sandbox_profile(
                std::path::Path::new("/Applications/bad\"name.app/Contents/MacOS/AcosmiEngine"),
                std::path::Path::new("/Applications/bad\"name.app"),
                std::path::Path::new("/tmp/profile"),
                std::path::Path::new("/tmp/temp"),
                std::path::Path::new("/tmp/runtime"),
            )
            .is_err()
        );
    }
}
