//! Real Electron P1 conformance. Explicitly ignored by default because it needs a built host bundle.

#![cfg(any(target_os = "macos", target_os = "windows"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use openbot_computer::browser::{BrowserInput, ModifierMask, MouseButton};
use openbot_computer::control::{ControlError, ControlService, HumanInputTicket};
use openbot_computer::engine::{
    ComponentRenderScope, ComputerSecurityScope, DesktopWindowSessionId, EngineBundle,
    EngineBundleDigest, EngineFrame, EngineLaunchConfig, EngineProcess, EngineProcessError,
    EngineRole, EngineSandboxFidelity, WorkspaceScope,
};
use openbot_contracts::auth::{AuthContext, AuthGeneration, Role};
use openbot_contracts::ids::{
    ActorId, BotId, ChannelId, ComputerGeneration, ComputerId, CredentialPrincipalId, DeploymentId,
    DocumentGeneration, TabId, TenantId,
};
use sha2::{Digest as _, Sha256};
use time::{Duration, OffsetDateTime, macros::datetime};

const INPUT_TIME: OffsetDateTime = datetime!(2026-09-04 12:00 UTC);
const ENGINE_INPUT_FIXTURE: &str =
    include_str!("../../../fixtures/computer/engine-input-wire-v2.json");

#[test]
fn engine_input_fixture_locks_protocol_and_unfinished_platform_boundaries() {
    let fixture = serde_json::from_str::<serde_json::Value>(ENGINE_INPUT_FIXTURE)
        .expect("engine input fixture JSON");
    assert_eq!(fixture["schema"], "openbot-engine-input-wire-v2");
    assert_eq!(fixture["protocol"]["version"], 2);
    assert_eq!(fixture["protocol"]["releaseEpoch"], 2);
    assert_eq!(
        fixture["protocol"]["generatedModuleSha256"],
        "ef213bb4d8f9f66b0854ef4feb9a7718de5bae139348320ed3fe8f6641b9bdf6"
    );
    assert_eq!(
        fixture["liveMatrix"]["inputAckAndFrameChannelsIndependent"],
        true
    );
    assert_eq!(
        fixture["liveMatrix"]["conformanceCaptureFramePerAcceptedInput"],
        true
    );
    assert_eq!(fixture["liveMatrix"]["crossScopeReceiptFrames"], 0);
    assert_eq!(fixture["liveMatrix"]["expiredReceiptFrames"], 0);
    assert_eq!(fixture["liveMatrix"]["ordinarySecretFrames"], 0);
    assert_eq!(fixture["evidenceBoundary"]["macosArm64Actual"], true);
    for unfinished in [
        "windowsRuntime",
        "linuxRunscRuntime",
        "serverOrDesktopComputerAssembly",
        "secretTypedEffect",
        "screenHub",
        "pageStartScreencast",
    ] {
        assert_eq!(fixture["evidenceBoundary"][unfinished], false);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires `cargo xtask engine bundle` and host permission to run the real confined Electron"]
async fn browser_role_start_frame_stop_has_no_debug_listener_or_orphan() {
    run_role(EngineRole::BrowserComputer(ComputerSecurityScope::new(
        TenantId::new("tenant-browser"),
        BotId::new("bot-browser"),
        CredentialPrincipalId::new("principal-browser"),
        WorkspaceScope::Channel(ChannelId::new("channel-browser")),
    )))
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires `cargo xtask engine bundle` and host permission to run the real confined Electron"]
async fn component_role_start_frame_stop_has_no_debug_listener_or_orphan() {
    run_role(EngineRole::SandboxedComponent(ComponentRenderScope::new(
        TenantId::new("tenant-component"),
        ActorId::new("actor-component"),
        DesktopWindowSessionId::new("window-component").expect("window session"),
    )))
    .await;
}

async fn run_role(role: EngineRole) {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let bundle_root = workspace.join(format!(
        "target/engine/bundle/electron-43.3.0/{}",
        bundle_platform()
    ));
    let manifest = bundle_root.join("manifest.json");
    let digest = format!(
        "{:x}",
        Sha256::digest(fs::read(&manifest).expect("bundle manifest"))
    );
    let bundle = EngineBundle::open(
        &bundle_root,
        EngineBundleDigest::from_hex(&digest).expect("manifest digest"),
    )
    .expect("verified bundle");

    let tag = match &role {
        EngineRole::BrowserComputer(_) => "browser",
        EngineRole::SandboxedComponent(_) => "component",
    };
    let temp = TestDirectories::new(tag);
    let computer_id = ComputerId::new(format!("computer-{tag}"));
    let generation = ComputerGeneration::new(1);
    let mut process = EngineProcess::launch(EngineLaunchConfig::new(
        bundle,
        role,
        computer_id.clone(),
        generation,
        &temp.profile,
        &temp.temp,
    ))
    .await
    .expect("launch + peer credential + ready");
    assert_eq!(process.sandbox_fidelity(), expected_fidelity());
    let pid = process.pid();
    assert!(process.main_creation_time().is_finite());
    assert!(process.main_creation_time() > 0.0);
    assert_no_tcp_listener(pid);

    let tab = TabId::new(format!("tab-{tag}"));
    let started = process
        .start_session(tab.clone())
        .await
        .expect("start + frame");
    assert_eq!((started.frame.width(), started.frame.height()), (1280, 800));
    assert!(started.frame.bytes().starts_with(&[0xff, 0xd8, 0xff]));
    assert!(started.frame.bytes().ends_with(&[0xff, 0xd9]));
    assert!(started.renderer_pid > 0);
    assert!(started.renderer_creation_time.is_finite());
    assert!(started.renderer_creation_time > 0.0);
    assert!(started.renderer_sandboxed, "OS-level ProcessMetric sandbox");
    assert_eq!(
        started.origin,
        if tag == "browser" {
            "acosmi-engine://session"
        } else {
            "component://session"
        }
    );
    let descendants = descendant_pids(pid);
    assert!(
        descendants.contains(&started.renderer_pid),
        "renderer is not a descendant of the authenticated main process"
    );
    for process in std::iter::once(pid).chain(descendants.iter().copied()) {
        assert_no_tcp_listener(process);
    }

    let auth = AuthContext::for_test(
        DeploymentId::new("deployment-input"),
        TenantId::new("tenant-input"),
        ActorId::new("actor-input"),
        [Role::User],
        AuthGeneration::new(9),
        false,
    );
    let mut control = ControlService::new(computer_id, tab.clone(), generation, INPUT_TIME);
    control
        .take(&auth, INPUT_TIME + Duration::minutes(5), INPUT_TIME)
        .expect("take control");
    let ticket = control
        .issue_human_input_ticket(INPUT_TIME)
        .expect("human input ticket");
    run_live_input_matrix(&mut process, &mut control, &auth, &ticket, started.frame).await;
    control.release(INPUT_TIME).expect("release control");
    assert!(matches!(
        control.authorize_human_input_receipt(&auth, &ticket, INPUT_TIME),
        Err(ControlError::TakeControlFirst)
    ));

    process.stop_session(&tab).await.expect("stop");
    process.shutdown().await.expect("shutdown in five seconds");
    assert_process_gone(pid);
    for child in descendants {
        assert_process_gone(child);
    }
    for lock in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
        assert!(
            !temp.profile.join(lock).exists(),
            "profile lock `{lock}` remained after shutdown"
        );
    }
}

async fn run_live_input_matrix(
    process: &mut EngineProcess,
    control: &mut ControlService,
    auth: &AuthContext,
    ticket: &HumanInputTicket,
    baseline: EngineFrame,
) {
    let none = ModifierMask::new(0).expect("modifiers");
    let mut sequence = baseline.sequence();
    let baseline_hash = frame_hash(&baseline);

    let mut wrong_scope = ControlService::new(
        ComputerId::new("other-computer"),
        ticket.tab_id().clone(),
        ticket.computer_generation(),
        INPUT_TIME,
    );
    wrong_scope
        .take(auth, INPUT_TIME + Duration::minutes(5), INPUT_TIME)
        .expect("wrong-scope lease");
    let wrong_ticket = wrong_scope
        .issue_human_input_ticket(INPUT_TIME)
        .expect("wrong-scope ticket");
    let wrong_receipt = wrong_scope
        .authorize_human_input_receipt(auth, &wrong_ticket, INPUT_TIME)
        .expect("wrong-scope receipt");
    assert!(matches!(
        process
            .apply_human_input(
                wrong_receipt,
                &BrowserInput::insert_text("must-not-cross-scope"),
                INPUT_TIME,
            )
            .await,
        Err(EngineProcessError::InputAuthority)
    ));

    let expired_receipt = control
        .authorize_human_input_receipt(auth, ticket, INPUT_TIME)
        .expect("receipt before authority-clock expiry");
    assert!(matches!(
        process
            .apply_human_input(
                expired_receipt,
                &BrowserInput::insert_text("must-not-dispatch"),
                ticket.expires_at(),
            )
            .await,
        Err(EngineProcessError::InputAuthority)
    ));

    let hover = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::mouse_move(80.0, 70.0, MouseButton::Left, none).expect("hover"),
        &mut sequence,
    )
    .await;
    assert_ne!(
        frame_hash(&hover),
        baseline_hash,
        "mouseMoved must change :hover"
    );

    let button_hover = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::mouse_move(80.0, 168.0, MouseButton::Left, none).expect("button hover"),
        &mut sequence,
    )
    .await;
    let pressed = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::mouse_down(80.0, 168.0, MouseButton::Left, None, none).expect("press"),
        &mut sequence,
    )
    .await;
    assert_ne!(
        frame_hash(&pressed),
        frame_hash(&button_hover),
        "mousePressed must change :active"
    );
    let released = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::mouse_up(80.0, 168.0, MouseButton::Left, None, none).expect("release"),
        &mut sequence,
    )
    .await;
    assert_ne!(
        frame_hash(&released),
        frame_hash(&pressed),
        "mouseReleased must clear :active"
    );

    let _ = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::mouse_move(80.0, 256.0, MouseButton::Left, none).expect("input hover"),
        &mut sequence,
    )
    .await;
    let _ = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::mouse_down(80.0, 256.0, MouseButton::Left, None, none).expect("input down"),
        &mut sequence,
    )
    .await;
    let focused = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::mouse_up(80.0, 256.0, MouseButton::Left, None, none).expect("input up"),
        &mut sequence,
    )
    .await;
    let typed = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::key_down("a", "KeyA", Some("a".to_owned()), none).expect("key down"),
        &mut sequence,
    )
    .await;
    assert_ne!(
        frame_hash(&typed),
        frame_hash(&focused),
        "keyDown text must alter input"
    );
    let _ = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::key_up("a", "KeyA", none).expect("key up"),
        &mut sequence,
    )
    .await;
    let erased = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::key_down("Backspace", "Backspace", None, none).expect("raw key down"),
        &mut sequence,
    )
    .await;
    assert_ne!(
        frame_hash(&erased),
        frame_hash(&typed),
        "rawKeyDown Backspace must alter input"
    );
    let _ = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::key_up("Backspace", "Backspace", none).expect("raw key up"),
        &mut sequence,
    )
    .await;
    let _ = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::key_down("F1", "F1", None, none).expect("unknown multi-unit key"),
        &mut sequence,
    )
    .await;
    let _ = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::key_up("F1", "F1", none).expect("unknown multi-unit key up"),
        &mut sequence,
    )
    .await;

    control
        .request_secret(Some("OTP"), "field-1", DocumentGeneration::new(1))
        .expect("secret request");
    let secret = BrowserInput::secret_insert(
        control.pending_secret().expect("pending secret"),
        "never-on-ordinary-wire",
    )
    .expect("secret input");
    let receipt = control
        .authorize_human_input_receipt(auth, ticket, INPUT_TIME)
        .expect("secret receipt");
    assert!(matches!(
        process
            .apply_human_input(receipt, &secret, INPUT_TIME)
            .await,
        Err(EngineProcessError::InputPlan(_))
    ));

    let inserted = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::insert_text("A中"),
        &mut sequence,
    )
    .await;
    assert_ne!(
        frame_hash(&inserted),
        frame_hash(&erased),
        "insertText must alter input"
    );

    let scroll_hover = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::mouse_move(450.0, 100.0, MouseButton::Left, none).expect("scroll hover"),
        &mut sequence,
    )
    .await;
    let scrolled = apply(
        process,
        control,
        auth,
        ticket,
        BrowserInput::wheel(450.0, 100.0, 0.0, 300.0, none).expect("wheel"),
        &mut sequence,
    )
    .await;
    assert_ne!(
        frame_hash(&scrolled),
        frame_hash(&scroll_hover),
        "mouseWheel must scroll"
    );
}

async fn apply(
    process: &mut EngineProcess,
    control: &mut ControlService,
    auth: &AuthContext,
    ticket: &HumanInputTicket,
    input: BrowserInput,
    sequence: &mut u64,
) -> EngineFrame {
    let receipt = control
        .authorize_human_input_receipt(auth, ticket, INPUT_TIME)
        .expect("fresh input authority");
    process
        .apply_human_input(receipt, &input, INPUT_TIME)
        .await
        .expect("authenticated live CDP input");
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), process.next_frame())
        .await
        .expect("next input frame deadline")
        .expect("next input frame");
    assert_eq!(
        frame.sequence(),
        sequence.checked_add(1).expect("frame sequence overflow"),
        "each accepted input emits exactly one next frame"
    );
    *sequence = frame.sequence();
    frame
}

fn frame_hash(frame: &EngineFrame) -> [u8; 32] {
    Sha256::digest(frame.bytes()).into()
}

#[cfg(target_os = "macos")]
fn assert_no_tcp_listener(pid: u32) {
    let output = Command::new("/usr/sbin/lsof")
        .args(["-nP", "-a", "-p", &pid.to_string(), "-iTCP", "-sTCP:LISTEN"])
        .output()
        .expect("lsof");
    assert!(
        output.stdout.is_empty(),
        "engine opened a TCP listener: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[cfg(target_os = "windows")]
fn assert_no_tcp_listener(pid: u32) {
    let script = format!(
        "$items = @(Get-NetTCPConnection -State Listen -ErrorAction Stop | Where-Object {{ $_.OwningProcess -eq {pid} }}); if ($items.Count -ne 0) {{ $items | ConvertTo-Json -Compress; exit 7 }}"
    );
    let output = powershell(&script);
    assert!(
        output.status.success(),
        "engine opened a TCP listener or the probe failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "macos")]
fn assert_process_gone(pid: u32) {
    let output = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "pid="])
        .output()
        .expect("ps");
    assert!(
        String::from_utf8_lossy(&output.stdout).trim().is_empty(),
        "engine PID {pid} remained after shutdown"
    );
}

#[cfg(target_os = "windows")]
fn assert_process_gone(pid: u32) {
    let script = format!("if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 7 }}");
    for _ in 0..50 {
        if powershell(&script).status.success() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("engine PID {pid} remained after shutdown");
}

#[cfg(target_os = "macos")]
fn descendant_pids(root: u32) -> Vec<u32> {
    let output = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid="])
        .output()
        .expect("ps process tree");
    assert!(output.status.success());
    let rows = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((
                fields.next()?.parse::<u32>().ok()?,
                fields.next()?.parse::<u32>().ok()?,
            ))
        })
        .collect::<Vec<_>>();
    let mut pending = vec![root];
    let mut descendants = Vec::new();
    while let Some(parent) = pending.pop() {
        for (pid, ppid) in &rows {
            if *ppid == parent && !descendants.contains(pid) {
                descendants.push(*pid);
                pending.push(*pid);
            }
        }
    }
    descendants
}

#[cfg(target_os = "windows")]
fn descendant_pids(root: u32) -> Vec<u32> {
    let output = powershell(
        "Get-CimInstance Win32_Process -ErrorAction Stop | ForEach-Object { '{0} {1}' -f $_.ProcessId,$_.ParentProcessId }",
    );
    assert!(
        output.status.success(),
        "Win32 process-tree probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((
                fields.next()?.parse::<u32>().ok()?,
                fields.next()?.parse::<u32>().ok()?,
            ))
        })
        .collect::<Vec<_>>();
    descendants_from_rows(root, &rows)
}

#[cfg(target_os = "windows")]
fn powershell(script: &str) -> std::process::Output {
    let system_root = std::env::var_os("SystemRoot").expect("SystemRoot");
    let executable =
        PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    Command::new(executable)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .output()
        .expect("PowerShell host probe")
}

#[cfg(target_os = "windows")]
fn descendants_from_rows(root: u32, rows: &[(u32, u32)]) -> Vec<u32> {
    let mut pending = vec![root];
    let mut descendants = Vec::new();
    while let Some(parent) = pending.pop() {
        for (pid, ppid) in rows {
            if *ppid == parent && !descendants.contains(pid) {
                descendants.push(*pid);
                pending.push(*pid);
            }
        }
    }
    descendants
}

fn bundle_platform() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos-arm64"
    }
    #[cfg(target_os = "windows")]
    {
        "windows-x64"
    }
}

fn expected_fidelity() -> EngineSandboxFidelity {
    #[cfg(target_os = "macos")]
    {
        EngineSandboxFidelity::Enforced
    }
    #[cfg(target_os = "windows")]
    {
        EngineSandboxFidelity::Degraded
    }
}

struct TestDirectories {
    root: PathBuf,
    profile: PathBuf,
    temp: PathBuf,
}

impl TestDirectories {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "openbot-engine-conformance-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let profile = root.join("profile");
        let temp = root.join("temp");
        fs::create_dir_all(&profile).expect("profile");
        fs::create_dir_all(&temp).expect("temp");
        Self {
            root,
            profile,
            temp,
        }
    }
}

impl Drop for TestDirectories {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
