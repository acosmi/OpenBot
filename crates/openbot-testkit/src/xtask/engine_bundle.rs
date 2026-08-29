//! Rust-only Electron bundle assembly: ASAR, rebrand, fuses, integrity and manifest (R117).

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const MANIFEST_SCHEMA: &str = "openbot-engine-bundle";
const MANIFEST_VERSION: u64 = 1;
const PRODUCT_NAME: &str = "Acosmi Engine Fixture";
const PRODUCT_SLUG: &str = "AcosmiEngine";
const BUNDLE_ID: &str = "com.acosmi.engine.fixture";
const ASAR_BLOCK_SIZE: usize = 4 * 1024 * 1024;
const FUSE_SENTINEL: &[u8] = b"dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX";
const FUSE_VERSION: u8 = 1;
const FUSE_WIRE: &[u8; 9] = b"000011001";
const SHIM_FILES: [&str; 3] = ["generated/protocol.mjs", "main.mjs", "package.json"];

#[derive(Debug, Serialize, Deserialize)]
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

struct BundleLayout {
    root: PathBuf,
    executable: PathBuf,
    fuse_file: PathBuf,
    app_asar: PathBuf,
    app_bundle: Option<PathBuf>,
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct WindowsAsarIntegrity {
    file: String,
    alg: String,
    value: String,
}

pub(crate) fn bundle(root: &Path) -> Result<()> {
    crate::engine_protocol::generate(root, true)?;
    crate::electron_shim::run(root, &[])?;

    let pins = crate::engine::pins(root)?;
    let electron = crate::engine::electron(&pins)?;
    let platform = crate::engine::current_platform()?;
    let source = crate::engine::install_dir(root, electron, platform)?;
    if !source.is_dir() {
        bail!("engine bundle: raw engine is missing; run `cargo xtask engine fetch`");
    }
    let version = crate::engine::string(electron, "version")?;
    let release_epoch = crate::engine::positive_u64(electron, "release_epoch")?;
    let protocol_version = crate::engine::positive_u64(electron, "protocol_version")?;
    if release_epoch != 1 || protocol_version != 1 {
        bail!("engine bundle: engine pins release/protocol version drift from v1 contracts");
    }
    let parent = root.join(format!("target/engine/bundle/electron-{version}"));
    fs::create_dir_all(&parent)?;
    let destination = parent.join(platform);
    let staging = parent.join(format!(".{platform}-staging-{}", std::process::id()));
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .with_context(|| format!("remove stale {}", staging.display()))?;
    }
    fs::create_dir_all(&staging)?;
    copy_tree(&source, &staging)?;

    let layout = rebrand(&staging, platform)?;
    let shim = root.join("crates/openbot-desktop/engine-shim");
    let asar = pack_asar(&shim, &layout.app_asar)?;
    remove_default_app(&layout)?;
    write_asar_integrity(&layout, &asar.header_sha256, platform)?;
    let fuse_count = write_fuses(&layout.fuse_file)?;
    if platform.starts_with("macos-") {
        codesign_adhoc(layout.app_bundle.as_ref().expect("macOS app bundle"))?;
    }

    let manifest = manifest(
        &layout,
        platform,
        version,
        release_epoch,
        protocol_version,
        &asar.header_sha256,
    )?;
    let manifest_path = layout.root.join("manifest.json");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    verify_layout(&layout.root, &manifest)?;

    if destination.exists() {
        fs::remove_dir_all(&destination)
            .with_context(|| format!("replace {}", destination.display()))?;
    }
    fs::rename(&staging, &destination)
        .with_context(|| format!("publish {}", destination.display()))?;
    println!(
        "engine bundle: ok ({}; app.asar={} bytes; header_sha256={}; fuse_sentinels={fuse_count}; release_epoch=1)",
        destination.display(),
        fs::metadata(destination.join(relative(&layout.root, &layout.app_asar)))?.len(),
        asar.header_sha256
    );
    Ok(())
}

pub(crate) fn verify_if_required(root: &Path) -> Result<()> {
    if !root.join("crates/openbot-desktop/engine-shim").is_dir() {
        return Ok(());
    }
    let pins = crate::engine::pins(root)?;
    let electron = crate::engine::electron(&pins)?;
    let platform = crate::engine::current_platform()?;
    let version = crate::engine::string(electron, "version")?;
    let bundle = root.join(format!(
        "target/engine/bundle/electron-{version}/{platform}"
    ));
    let manifest_path = bundle.join("manifest.json");
    let manifest: BundleManifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .with_context(|| format!("read {}; run engine bundle", manifest_path.display()))?,
    )
    .context("parse engine bundle manifest")?;
    verify_layout(&bundle, &manifest)?;
    println!(
        "engine bundle verify: ok ({platform}; release_epoch={}; protocol={}; asar_header={})",
        manifest.release_epoch, manifest.protocol_version, manifest.asar_header_sha256
    );
    Ok(())
}

fn rebrand(root: &Path, platform: &str) -> Result<BundleLayout> {
    if platform.starts_with("macos-") {
        return rebrand_macos(root);
    }
    if platform.starts_with("linux-") {
        let old = root.join("electron");
        let executable = root.join("acosmi-engine-fixture");
        fs::rename(&old, &executable)?;
        return Ok(BundleLayout {
            root: root.to_path_buf(),
            executable,
            fuse_file: root.join("acosmi-engine-fixture"),
            app_asar: root.join("resources/app.asar"),
            app_bundle: None,
        });
    }
    if platform == "windows-x64" {
        let old = root.join("electron.exe");
        let executable = root.join("acosmi-engine-fixture.exe");
        fs::rename(&old, &executable)?;
        return Ok(BundleLayout {
            root: root.to_path_buf(),
            executable: executable.clone(),
            fuse_file: executable,
            app_asar: root.join("resources/app.asar"),
            app_bundle: None,
        });
    }
    bail!("engine bundle: unsupported platform `{platform}`")
}

fn rebrand_macos(root: &Path) -> Result<BundleLayout> {
    let old_app = root.join("Electron.app");
    let app = root.join(format!("{PRODUCT_SLUG}.app"));
    let old_executable = old_app.join("Contents/MacOS/Electron");
    let new_executable = old_app.join(format!("Contents/MacOS/{PRODUCT_SLUG}"));
    fs::rename(&old_executable, &new_executable)?;
    edit_plist(
        &old_app.join("Contents/Info.plist"),
        PRODUCT_NAME,
        PRODUCT_SLUG,
        BUNDLE_ID,
        true,
    )?;

    // Electron documents helper renaming as optional. P1 keeps the framework/helper internals at
    // their upstream names because their launch stubs are part of the signed external engine, while
    // the containing app, main executable and bundle identity are rebranded. Final helper display
    // branding belongs to G8 after the reviewed external identity exists.
    let icon = old_app.join("Contents/Resources/electron.icns");
    if icon.exists() {
        fs::remove_file(icon)?;
    }
    fs::rename(&old_app, &app)?;
    Ok(BundleLayout {
        root: root.to_path_buf(),
        executable: app.join(format!("Contents/MacOS/{PRODUCT_SLUG}")),
        fuse_file: app
            .join("Contents/Frameworks/Electron Framework.framework/Versions/A/Electron Framework"),
        app_asar: app.join("Contents/Resources/app.asar"),
        app_bundle: Some(app),
    })
}

fn edit_plist(
    path: &Path,
    display_name: &str,
    executable: &str,
    identifier: &str,
    main: bool,
) -> Result<()> {
    let mut value =
        plist::Value::from_file(path).with_context(|| format!("read plist {}", path.display()))?;
    let dictionary = value
        .as_dictionary_mut()
        .ok_or_else(|| anyhow!("{} is not a plist dictionary", path.display()))?;
    dictionary.insert(
        "CFBundleDisplayName".to_owned(),
        plist::Value::String(display_name.to_owned()),
    );
    dictionary.insert(
        "CFBundleName".to_owned(),
        plist::Value::String(executable.to_owned()),
    );
    dictionary.insert(
        "CFBundleExecutable".to_owned(),
        plist::Value::String(executable.to_owned()),
    );
    dictionary.insert(
        "CFBundleIdentifier".to_owned(),
        plist::Value::String(identifier.to_owned()),
    );
    dictionary.remove("CFBundleIconFile");
    dictionary.remove("CFBundleIconName");
    if main {
        dictionary.remove("ElectronAsarIntegrity");
    }
    value
        .to_file_xml(path)
        .with_context(|| format!("write plist {}", path.display()))
}

fn write_asar_integrity(layout: &BundleLayout, hash: &str, platform: &str) -> Result<()> {
    if platform.starts_with("macos-") {
        let plist_path = layout
            .app_bundle
            .as_ref()
            .expect("macOS app")
            .join("Contents/Info.plist");
        let mut value = plist::Value::from_file(&plist_path)?;
        let dictionary = value
            .as_dictionary_mut()
            .ok_or_else(|| anyhow!("main Info.plist is not a dictionary"))?;
        let mut integrity = plist::Dictionary::new();
        let mut payload = plist::Dictionary::new();
        payload.insert(
            "algorithm".to_owned(),
            plist::Value::String("SHA256".to_owned()),
        );
        payload.insert("hash".to_owned(), plist::Value::String(hash.to_owned()));
        integrity.insert(
            "Resources/app.asar".to_owned(),
            plist::Value::Dictionary(payload),
        );
        dictionary.insert(
            "ElectronAsarIntegrity".to_owned(),
            plist::Value::Dictionary(integrity),
        );
        value.to_file_xml(plist_path)?;
        return Ok(());
    }
    if platform.starts_with("linux-") {
        return Ok(());
    }
    if platform == "windows-x64" {
        #[cfg(windows)]
        {
            let payload = windows_integrity_payload(hash)?;
            openbot_windows_sandbox::replace_pe_resource(
                &layout.executable,
                "Integrity",
                "ElectronAsar",
                &payload,
            )?;
            let actual = openbot_windows_sandbox::read_pe_resource(
                &layout.executable,
                "Integrity",
                "ElectronAsar",
            )?;
            if actual != payload {
                bail!("Windows ElectronAsar resource differs immediately after write");
            }
            return Ok(());
        }
        #[cfg(not(windows))]
        bail!("Windows PE resources may only be assembled on a Windows host");
    }
    bail!("ASAR integrity metadata is not implemented for `{platform}`")
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn windows_integrity_payload(hash: &str) -> Result<Vec<u8>> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Windows ElectronAsar header hash is not SHA-256 hex");
    }
    Ok(serde_json::to_vec(&[WindowsAsarIntegrity {
        file: r"resources\app.asar".to_owned(),
        alg: "sha256".to_owned(),
        value: hash.to_ascii_lowercase(),
    }])?)
}

fn remove_default_app(layout: &BundleLayout) -> Result<()> {
    let resources = layout
        .app_asar
        .parent()
        .ok_or_else(|| anyhow!("app.asar has no resources parent"))?;
    let default_app = resources.join("default_app.asar");
    if default_app.exists() {
        fs::remove_file(default_app)?;
    }
    let unpacked_app = resources.join("app");
    if unpacked_app.exists() {
        fs::remove_dir_all(unpacked_app)?;
    }
    Ok(())
}

struct PackedAsar {
    header_sha256: String,
}

#[derive(Serialize, Deserialize)]
struct AsarDirectory {
    files: BTreeMap<String, AsarEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum AsarEntry {
    Directory(AsarDirectory),
    File(AsarFile),
}

#[derive(Serialize, Deserialize)]
struct AsarFile {
    offset: String,
    size: usize,
    integrity: AsarIntegrity,
}

#[derive(Serialize, Deserialize)]
struct AsarIntegrity {
    algorithm: String,
    hash: String,
    #[serde(rename = "blockSize")]
    block_size: usize,
    blocks: Vec<String>,
}

fn pack_asar(source: &Path, destination: &Path) -> Result<PackedAsar> {
    let mut root = AsarDirectory {
        files: BTreeMap::new(),
    };
    let mut payloads = Vec::new();
    let mut offset = 0_u64;
    for relative in SHIM_FILES {
        let bytes = fs::read(source.join(relative))?;
        let record = AsarFile {
            offset: offset.to_string(),
            size: bytes.len(),
            integrity: integrity(&bytes),
        };
        insert_asar(&mut root, &relative.split('/').collect::<Vec<_>>(), record)?;
        offset = offset
            .checked_add(u64::try_from(bytes.len())?)
            .ok_or_else(|| anyhow!("ASAR offset overflow"))?;
        payloads.push(bytes);
    }
    let json = serde_json::to_vec(&root)?;
    let header = pickle_string(&json)?;
    let size = pickle_u32(u32::try_from(header.len())?);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = File::create(destination)?;
    output.write_all(&size)?;
    output.write_all(&header)?;
    for payload in payloads {
        output.write_all(&payload)?;
    }
    output.sync_all()?;
    Ok(PackedAsar {
        header_sha256: format!("{:x}", Sha256::digest(&json)),
    })
}

fn insert_asar(directory: &mut AsarDirectory, path: &[&str], file: AsarFile) -> Result<()> {
    let (name, rest) = path
        .split_first()
        .ok_or_else(|| anyhow!("empty ASAR path"))?;
    if name.is_empty() || matches!(*name, "." | "..") || name.contains(['/', '\\']) {
        bail!("invalid ASAR entry `{name}`");
    }
    if rest.is_empty() {
        if directory
            .files
            .insert((*name).to_owned(), AsarEntry::File(file))
            .is_some()
        {
            bail!("duplicate ASAR entry `{name}`");
        }
        return Ok(());
    }
    let entry = directory
        .files
        .entry((*name).to_owned())
        .or_insert_with(|| {
            AsarEntry::Directory(AsarDirectory {
                files: BTreeMap::new(),
            })
        });
    let AsarEntry::Directory(child) = entry else {
        bail!("ASAR path collides with file `{name}`")
    };
    insert_asar(child, rest, file)
}

fn integrity(bytes: &[u8]) -> AsarIntegrity {
    let blocks = if bytes.is_empty() {
        vec![format!("{:x}", Sha256::digest([]))]
    } else {
        bytes
            .chunks(ASAR_BLOCK_SIZE)
            .map(|block| format!("{:x}", Sha256::digest(block)))
            .collect()
    };
    AsarIntegrity {
        algorithm: "SHA256".to_owned(),
        hash: format!("{:x}", Sha256::digest(bytes)),
        block_size: ASAR_BLOCK_SIZE,
        blocks,
    }
}

fn pickle_u32(value: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8);
    bytes.extend_from_slice(&4_u32.to_le_bytes());
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes
}

fn pickle_string(value: &[u8]) -> Result<Vec<u8>> {
    let aligned = value.len().div_ceil(4) * 4;
    let payload = 4_usize
        .checked_add(aligned)
        .ok_or_else(|| anyhow!("ASAR header overflow"))?;
    let mut bytes = Vec::with_capacity(4 + payload);
    bytes.extend_from_slice(&u32::try_from(payload)?.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(value.len())?.to_le_bytes());
    bytes.extend_from_slice(value);
    bytes.resize(4 + payload, 0);
    Ok(bytes)
}

fn write_fuses(path: &Path) -> Result<usize> {
    let positions = find_fuse_sentinels(path)?;
    if positions.is_empty() || positions.len() > 2 {
        bail!("fuse sentinel count {} is outside 1..=2", positions.len());
    }
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    for position in &positions {
        file.seek(SeekFrom::Start(
            *position + u64::try_from(FUSE_SENTINEL.len())?,
        ))?;
        let mut header = [0_u8; 2];
        file.read_exact(&mut header)?;
        if header != [FUSE_VERSION, FUSE_WIRE.len() as u8] {
            bail!("unexpected fuse schema version/length: {header:?}");
        }
        let mut current = [0_u8; 9];
        file.read_exact(&mut current)?;
        if current
            .iter()
            .any(|state| !matches!(*state, b'0' | b'1' | b'r'))
        {
            bail!("invalid fuse state bytes: {current:?}");
        }
        if current.contains(&b'r') {
            bail!("Electron removed a fuse that P1 requires explicitly");
        }
        file.seek(SeekFrom::Start(
            *position + u64::try_from(FUSE_SENTINEL.len() + 2)?,
        ))?;
        file.write_all(FUSE_WIRE)?;
    }
    file.sync_all()?;
    Ok(positions.len())
}

fn find_fuse_sentinels(path: &Path) -> Result<Vec<u64>> {
    let mut file = File::open(path)?;
    let mut positions = Vec::new();
    let mut absolute = 0_u64;
    let mut carry = Vec::new();
    let mut buffer = vec![0_u8; 4 * 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let mut chunk = carry;
        chunk.extend_from_slice(&buffer[..read]);
        for index in find_subslice(&chunk, FUSE_SENTINEL) {
            let base = absolute.saturating_sub(u64::try_from(chunk.len() - read)?);
            let position = base + u64::try_from(index)?;
            if positions.last().copied() != Some(position) {
                positions.push(position);
            }
        }
        let overlap = FUSE_SENTINEL.len() - 1;
        carry = chunk[chunk.len().saturating_sub(overlap)..].to_vec();
        absolute += u64::try_from(read)?;
    }
    Ok(positions)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return Vec::new();
    }
    haystack
        .windows(needle.len())
        .enumerate()
        .filter_map(|(index, window)| (window == needle).then_some(index))
        .collect()
}

fn codesign_adhoc(app: &Path) -> Result<()> {
    let status = Command::new("/usr/bin/codesign")
        .args([
            "--sign",
            "-",
            "--force",
            "--deep",
            "--preserve-metadata=entitlements,requirements,flags,runtime",
        ])
        .arg(app)
        .status()?;
    if !status.success() {
        bail!("ad-hoc codesign failed with {status}");
    }
    let verify = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app)
        .status()?;
    if !verify.success() {
        bail!("ad-hoc codesign verification failed with {verify}");
    }
    Ok(())
}

fn manifest(
    layout: &BundleLayout,
    platform: &str,
    electron_version: &str,
    release_epoch: u64,
    protocol_version: u64,
    asar_header_sha256: &str,
) -> Result<BundleManifest> {
    let mut files = BTreeMap::new();
    for path in [&layout.executable, &layout.fuse_file, &layout.app_asar] {
        files.insert(relative(&layout.root, path), sha256(path)?);
    }
    Ok(BundleManifest {
        schema: MANIFEST_SCHEMA.to_owned(),
        schema_version: MANIFEST_VERSION,
        platform: platform.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        electron_version: electron_version.to_owned(),
        release_epoch,
        protocol_version,
        product_name: PRODUCT_NAME.to_owned(),
        bundle_id: BUNDLE_ID.to_owned(),
        executable: relative(&layout.root, &layout.executable),
        fuse_file: relative(&layout.root, &layout.fuse_file),
        app_asar: relative(&layout.root, &layout.app_asar),
        asar_header_sha256: asar_header_sha256.to_owned(),
        fuse_wire: String::from_utf8(FUSE_WIRE.to_vec()).expect("ASCII fuse wire"),
        files,
    })
}

fn verify_layout(root: &Path, manifest: &BundleManifest) -> Result<()> {
    if manifest.schema != MANIFEST_SCHEMA
        || manifest.schema_version != MANIFEST_VERSION
        || manifest.platform != crate::engine::current_platform()?
        || manifest.release_epoch != 1
        || manifest.protocol_version != 1
        || manifest.product_name != PRODUCT_NAME
        || manifest.bundle_id != BUNDLE_ID
        || manifest.fuse_wire.as_bytes() != FUSE_WIRE
    {
        bail!("engine bundle manifest fixed fields drift: {manifest:?}");
    }
    for (path, expected) in &manifest.files {
        let path = root.join(path);
        let actual = sha256(&path)?;
        if &actual != expected {
            bail!("bundle digest mismatch for {}", path.display());
        }
    }
    let fuse_path = root.join(&manifest.fuse_file);
    verify_fuses(&fuse_path)?;
    let asar_path = root.join(&manifest.app_asar);
    let header = verify_asar(&asar_path)?;
    if header != manifest.asar_header_sha256 {
        bail!("ASAR header hash differs from manifest");
    }
    let resources = asar_path.parent().expect("resources parent");
    if resources.join("default_app.asar").exists() || resources.join("app").exists() {
        bail!("OnlyLoadAppFromAsar invariant violated by fallback app source");
    }
    if manifest.platform.starts_with("macos-") {
        verify_macos(root, manifest)?;
    }
    if manifest.platform == "windows-x64" {
        verify_windows(root, manifest)?;
    }
    Ok(())
}

fn verify_fuses(path: &Path) -> Result<()> {
    let positions = find_fuse_sentinels(path)?;
    if positions.is_empty() || positions.len() > 2 {
        bail!("bundle fuse sentinel count invalid: {}", positions.len());
    }
    let mut file = File::open(path)?;
    for position in positions {
        file.seek(SeekFrom::Start(
            position + u64::try_from(FUSE_SENTINEL.len())?,
        ))?;
        let mut bytes = [0_u8; 11];
        file.read_exact(&mut bytes)?;
        if bytes[..2] != [FUSE_VERSION, FUSE_WIRE.len() as u8] || &bytes[2..] != FUSE_WIRE {
            bail!("bundle fuse wire drift: {bytes:?}");
        }
    }
    Ok(())
}

fn verify_asar(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    if bytes.len() < 16 || u32::from_le_bytes(bytes[0..4].try_into()?) != 4 {
        bail!("ASAR size pickle is invalid");
    }
    let header_size = usize::try_from(u32::from_le_bytes(bytes[4..8].try_into()?))?;
    if header_size < 8 || 8 + header_size > bytes.len() {
        bail!("ASAR header size is invalid");
    }
    let payload_size = usize::try_from(u32::from_le_bytes(bytes[8..12].try_into()?))?;
    let json_size = usize::try_from(u32::from_le_bytes(bytes[12..16].try_into()?))?;
    if payload_size + 4 != header_size || 16 + json_size > 8 + header_size {
        bail!("ASAR header pickle is invalid");
    }
    let json = &bytes[16..16 + json_size];
    let header: AsarDirectory = serde_json::from_slice(json)?;
    let content_start = 8 + header_size;
    verify_asar_directory(&header, &bytes, content_start)?;
    Ok(format!("{:x}", Sha256::digest(json)))
}

fn verify_asar_directory(directory: &AsarDirectory, bytes: &[u8], content: usize) -> Result<()> {
    for entry in directory.files.values() {
        match entry {
            AsarEntry::Directory(child) => verify_asar_directory(child, bytes, content)?,
            AsarEntry::File(file) => {
                if file.integrity.algorithm != "SHA256"
                    || file.integrity.block_size != ASAR_BLOCK_SIZE
                {
                    bail!("ASAR integrity metadata drift");
                }
                let offset = file.offset.parse::<usize>()?;
                let end = content
                    .checked_add(offset)
                    .and_then(|start| start.checked_add(file.size))
                    .ok_or_else(|| anyhow!("ASAR file bounds overflow"))?;
                if end > bytes.len() {
                    bail!("ASAR file exceeds archive");
                }
                let payload = &bytes[content + offset..end];
                if format!("{:x}", Sha256::digest(payload)) != file.integrity.hash {
                    bail!("ASAR file hash mismatch");
                }
                let blocks = integrity(payload).blocks;
                if blocks != file.integrity.blocks {
                    bail!("ASAR block hash mismatch");
                }
            }
        }
    }
    Ok(())
}

fn verify_macos(root: &Path, manifest: &BundleManifest) -> Result<()> {
    let app = root.join(format!("{PRODUCT_SLUG}.app"));
    let plist_path = app.join("Contents/Info.plist");
    let plist = plist::Value::from_file(&plist_path)?;
    let dictionary = plist
        .as_dictionary()
        .ok_or_else(|| anyhow!("main plist is not a dictionary"))?;
    if dictionary
        .get("CFBundleIdentifier")
        .and_then(plist::Value::as_string)
        != Some(BUNDLE_ID)
        || dictionary
            .get("CFBundleExecutable")
            .and_then(plist::Value::as_string)
            != Some(PRODUCT_SLUG)
    {
        bail!("macOS rebrand plist drift");
    }
    let integrity = dictionary
        .get("ElectronAsarIntegrity")
        .and_then(plist::Value::as_dictionary)
        .and_then(|value| value.get("Resources/app.asar"))
        .and_then(plist::Value::as_dictionary)
        .ok_or_else(|| anyhow!("ElectronAsarIntegrity missing"))?;
    if integrity.get("algorithm").and_then(plist::Value::as_string) != Some("SHA256")
        || integrity.get("hash").and_then(plist::Value::as_string)
            != Some(manifest.asar_header_sha256.as_str())
    {
        bail!("ElectronAsarIntegrity drift");
    }
    let status = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app)
        .status()?;
    if !status.success() {
        bail!("bundle code signature verification failed");
    }
    Ok(())
}

fn verify_windows(root: &Path, manifest: &BundleManifest) -> Result<()> {
    if manifest.executable != "acosmi-engine-fixture.exe"
        || manifest.fuse_file != manifest.executable
    {
        bail!("Windows fixture executable/rebrand drift");
    }
    #[cfg(windows)]
    {
        let executable = root.join(&manifest.executable);
        let resource =
            openbot_windows_sandbox::read_pe_resource(&executable, "Integrity", "ElectronAsar")?;
        if resource != windows_integrity_payload(&manifest.asar_header_sha256)? {
            bail!("Windows ElectronAsar/Integrity resource drift");
        }
        let decoded: Vec<WindowsAsarIntegrity> = serde_json::from_slice(&resource)?;
        if decoded.len() != 1
            || decoded[0].file != r"resources\app.asar"
            || decoded[0].alg != "sha256"
            || decoded[0].value != manifest.asar_header_sha256
        {
            bail!("Windows ElectronAsar resource shape drift");
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        bail!("Windows PE resources may only be verified on a Windows host");
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(source)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_symlink() {
            copy_symlink(entry.path(), &target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
            fs::set_permissions(&target, fs::metadata(entry.path())?.permissions())?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    std::os::unix::fs::symlink(fs::read_link(source)?, target)?;
    Ok(())
}

#[cfg(windows)]
fn copy_symlink(_source: &Path, _target: &Path) -> Result<()> {
    bail!("engine bundle: unexpected symlink in Windows Electron archive")
}

fn sha256(path: &Path) -> Result<String> {
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

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("bundle path remains under root")
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::{
        FUSE_SENTINEL, FUSE_WIRE, WindowsAsarIntegrity, find_subslice, integrity, pickle_string,
        pickle_u32, windows_integrity_payload,
    };

    #[test]
    fn pickle_layout_matches_official_asar_shape() {
        let json = br#"{"files":{}}"#;
        let header = pickle_string(json).expect("header");
        let outer = pickle_u32(u32::try_from(header.len()).expect("small"));
        assert_eq!(outer.len(), 8);
        assert_eq!(u32::from_le_bytes(outer[0..4].try_into().unwrap()), 4);
        assert_eq!(
            usize::try_from(u32::from_le_bytes(outer[4..8].try_into().unwrap())).unwrap(),
            header.len()
        );
        assert_eq!(
            u32::from_le_bytes(header[4..8].try_into().unwrap()),
            json.len() as u32
        );
    }

    #[test]
    fn file_integrity_has_whole_and_four_mib_blocks() {
        let bytes = vec![7_u8; 4 * 1024 * 1024 + 1];
        let value = integrity(&bytes);
        assert_eq!(value.blocks.len(), 2);
        assert_eq!(value.hash.len(), 64);
    }

    #[test]
    fn fuse_scanner_handles_overlap_and_wire_is_strict_v1() {
        let mut bytes = vec![0_u8; 7];
        bytes.extend_from_slice(FUSE_SENTINEL);
        bytes.extend_from_slice(FUSE_SENTINEL);
        assert_eq!(
            find_subslice(&bytes, FUSE_SENTINEL),
            vec![7, 7 + FUSE_SENTINEL.len()]
        );
        assert_eq!(FUSE_WIRE, b"000011001");
    }

    #[test]
    fn windows_integrity_resource_is_the_official_closed_shape() {
        let hash = "ab".repeat(32);
        let payload = windows_integrity_payload(&hash).expect("payload");
        assert_eq!(
            serde_json::from_slice::<Vec<WindowsAsarIntegrity>>(&payload).unwrap(),
            vec![WindowsAsarIntegrity {
                file: r"resources\app.asar".to_owned(),
                alg: "sha256".to_owned(),
                value: hash,
            }]
        );
        assert!(windows_integrity_payload("not-a-hash").is_err());
    }
}
