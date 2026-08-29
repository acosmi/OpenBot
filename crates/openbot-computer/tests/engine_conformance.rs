//! Real Electron P1 conformance. Explicitly ignored by default because it needs a built host bundle.

#![cfg(any(target_os = "macos", target_os = "windows"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use openbot_computer::engine::{
    ComponentRenderScope, ComputerSecurityScope, DesktopWindowSessionId, EngineBundle,
    EngineBundleDigest, EngineLaunchConfig, EngineProcess, EngineRole, EngineSandboxFidelity,
    WorkspaceScope,
};
use openbot_contracts::ids::{
    ActorId, BotId, ChannelId, ComputerGeneration, ComputerId, CredentialPrincipalId, TabId,
    TenantId,
};
use sha2::{Digest as _, Sha256};

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
        computer_id,
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
