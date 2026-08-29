//! Mechanical tier-1 inventory for the immutable `grok-bot/` reference tree (R116).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use sha2::{Digest as _, Sha256};

const REFERENCE_DIR: &str = "grok-bot";
const OUTPUT_RELPATH: &str = "inventory/grok/files.yaml";
const EXPECTED_GIT_TREE: &str = "86f5a85f560f721677fa7e587a67ac0ffc036cb5";

#[derive(Serialize)]
struct Inventory {
    schema: &'static str,
    schema_version: u8,
    source_tree: &'static str,
    tree_sha256: String,
    generated_by: &'static str,
    maturity_rules: Vec<&'static str>,
    files: Vec<InventoryFile>,
}

#[derive(Serialize)]
struct InventoryFile {
    path: String,
    family: String,
    nonempty_loc: usize,
    maturity: &'static str,
}

pub(crate) fn run(root: &Path, args: &[String]) -> Result<()> {
    let check = match args {
        [] => false,
        [arg] if arg == "--check" => true,
        _ => bail!("usage: cargo xtask grok-inventory [--check]"),
    };

    ensure_reference_tree(root)?;
    let (rendered, summary) = render(root)?;
    let output = root.join(OUTPUT_RELPATH);
    if check {
        let current = fs::read(&output)
            .with_context(|| format!("read {} (run without --check first)", output.display()))?;
        if current != rendered.as_bytes() {
            let first = first_differing_line(&current, rendered.as_bytes());
            bail!(
                "grok-inventory: {OUTPUT_RELPATH} is stale (first differing line {first}); run `cargo xtask grok-inventory`"
            );
        }
        println!(
            "grok-inventory --check: ok (files={}; families={}; production={} partial={} generated={} artifact-only={}; tree_sha256={})",
            summary.files,
            summary.families,
            summary.maturity["production"],
            summary.maturity["partial"],
            summary.maturity["generated"],
            summary.maturity["artifact-only"],
            summary.tree_sha256,
        );
        return Ok(());
    }

    let parent = output
        .parent()
        .ok_or_else(|| anyhow!("inventory output has no parent"))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create inventory directory {}", parent.display()))?;
    let temporary = parent.join("files.yaml.tmp");
    fs::write(&temporary, rendered.as_bytes())
        .with_context(|| format!("write {}", temporary.display()))?;
    if output.exists() {
        fs::remove_file(&output).with_context(|| format!("replace {}", output.display()))?;
    }
    fs::rename(&temporary, &output).with_context(|| format!("publish {}", output.display()))?;
    println!(
        "grok-inventory: wrote {OUTPUT_RELPATH} (files={}; families={}; tree_sha256={})",
        summary.files, summary.families, summary.tree_sha256
    );
    Ok(())
}

struct Summary {
    files: usize,
    families: usize,
    maturity: BTreeMap<&'static str, usize>,
    tree_sha256: String,
}

fn render(root: &Path) -> Result<(String, Summary)> {
    let reference = root.join(REFERENCE_DIR);
    if !reference.is_dir() {
        bail!("grok-inventory: {} is missing", reference.display());
    }

    let mut paths = walkdir::WalkDir::new(&reference)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(entry) if entry.file_type().is_file() => Some(Ok(entry.into_path())),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort_by_key(|path| relative(&reference, path));

    let mut digest = Sha256::new();
    let mut files = Vec::with_capacity(paths.len());
    let mut families = BTreeSet::new();
    let mut maturity = [
        ("production", 0_usize),
        ("partial", 0),
        ("generated", 0),
        ("artifact-only", 0),
    ]
    .into_iter()
    .collect::<BTreeMap<_, _>>();

    for path in paths {
        let relative = relative(&reference, &path);
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        digest.update((relative.len() as u64).to_le_bytes());
        digest.update(relative.as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);

        let text = String::from_utf8_lossy(&bytes);
        let nonempty_loc = text.lines().filter(|line| !line.trim().is_empty()).count();
        if nonempty_loc == 0 {
            bail!(
                "grok-inventory: `{relative}` has zero non-empty LOC; every inventory row must carry a non-zero mechanical count"
            );
        }
        let family = family(&relative);
        let file_maturity = classify_maturity(&relative, &text);
        families.insert(family.clone());
        *maturity
            .get_mut(file_maturity)
            .expect("maturity classifier returns a closed value") += 1;
        files.push(InventoryFile {
            path: relative,
            family,
            nonempty_loc,
            maturity: file_maturity,
        });
    }

    let tree_sha256 = format!("{:x}", digest.finalize());
    let summary = Summary {
        files: files.len(),
        families: families.len(),
        maturity,
        tree_sha256: tree_sha256.clone(),
    };
    let inventory = Inventory {
        schema: "grok-files",
        schema_version: 1,
        source_tree: EXPECTED_GIT_TREE,
        tree_sha256,
        generated_by: "cargo xtask grok-inventory",
        maturity_rules: vec![
            "generated: path/name carries a generated or minified marker",
            "artifact-only: file is outside source/ and frontend/src/ or has a binary asset extension",
            "partial: recovered frontend path or deterministic partial/placeholder marker in text",
            "production: remaining source/ or frontend/src/ file",
        ],
        files,
    };
    let body = serde_yaml::to_string(&inventory).context("serialize grok inventory")?;
    let rendered = format!(
        "# Generated mechanically by `cargo xtask grok-inventory`; do not hand edit.\n\
         # This tier-1 inventory is evidence only and is not a parity/product denominator (R115/R116).\n{body}"
    );
    Ok((rendered, summary))
}

fn ensure_reference_tree(root: &Path) -> Result<()> {
    for args in [
        ["diff", "--quiet", "--", REFERENCE_DIR].as_slice(),
        ["diff", "--cached", "--quiet", "--", REFERENCE_DIR].as_slice(),
    ] {
        let status = Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .context("start git diff for grok-bot reference tree")?;
        if !status.success() {
            bail!(
                "grok-inventory: `{REFERENCE_DIR}/` differs from HEAD; R116 forbids changing the pinned reference tree"
            );
        }
    }
    let output = Command::new("git")
        .args(["rev-parse", "HEAD:grok-bot"])
        .current_dir(root)
        .output()
        .context("start git rev-parse HEAD:grok-bot")?;
    if !output.status.success() {
        bail!(
            "grok-inventory: git rev-parse failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let actual = String::from_utf8(output.stdout)
        .context("git tree hash is not UTF-8")?
        .trim()
        .to_owned();
    if actual != EXPECTED_GIT_TREE {
        bail!("grok-inventory: HEAD:grok-bot tree `{actual}` != pinned `{EXPECTED_GIT_TREE}`");
    }
    Ok(())
}

fn family(path: &str) -> String {
    let segments = path.split('/').collect::<Vec<_>>();
    let take = match segments.as_slice() {
        ["source", "packages", _, ..] => 3,
        ["source", _, ..] => 2,
        ["frontend", "src", "recovered", "features", _, ..] => 5,
        ["frontend", "src", "recovered", _, ..] => 4,
        ["frontend", "src", _, ..] => 3,
        _ => 1,
    };
    segments[..take.min(segments.len())].join("/")
}

fn classify_maturity(path: &str, text: &str) -> &'static str {
    let lower_path = path.to_ascii_lowercase();
    let file_name = lower_path.rsplit('/').next().unwrap_or(&lower_path);
    if lower_path.contains("/generated/")
        || lower_path.contains("/dist/")
        || file_name.contains(".generated.")
        || file_name.contains(".gen.")
        || file_name.ends_with(".min.js")
        || file_name.ends_with(".min.css")
    {
        return "generated";
    }
    if !lower_path.starts_with("source/") && !lower_path.starts_with("frontend/src/")
        || has_binary_asset_extension(file_name)
    {
        return "artifact-only";
    }
    let lower_text = text.to_ascii_lowercase();
    if lower_path.starts_with("frontend/src/recovered/")
        || lower_text.contains("partial recovery")
        || lower_text.contains("partial reconstruction")
        || lower_text.contains("placeholder")
    {
        return "partial";
    }
    "production"
}

fn has_binary_asset_extension(file_name: &str) -> bool {
    [
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".icns", ".woff", ".woff2", ".ttf",
        ".otf", ".zip", ".dmg", ".exe", ".pdf",
    ]
    .iter()
    .any(|extension| file_name.ends_with(extension))
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("walked path remains under reference root")
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn first_differing_line(left: &[u8], right: &[u8]) -> usize {
    let left = String::from_utf8_lossy(left);
    let right = String::from_utf8_lossy(right);
    left.lines()
        .zip(right.lines())
        .position(|(left, right)| left != right)
        .map_or_else(
            || left.lines().count().min(right.lines().count()) + 1,
            |index| index + 1,
        )
}

#[cfg(test)]
mod tests {
    use super::{classify_maturity, family};

    #[test]
    fn family_is_derived_only_from_stable_path_segments() {
        assert_eq!(
            family("source/packages/agent/state.ts"),
            "source/packages/agent"
        );
        assert_eq!(
            family("frontend/src/recovered/features/chat/index.ts"),
            "frontend/src/recovered/features/chat"
        );
        assert_eq!(family("docs/README.md"), "docs");
    }

    #[test]
    fn maturity_rules_are_closed_and_generated_wins_over_partial() {
        assert_eq!(
            classify_maturity("source/packages/proto/generated/a.ts", "placeholder"),
            "generated"
        );
        assert_eq!(
            classify_maturity("frontend/src/recovered/features/a.ts", "complete"),
            "partial"
        );
        assert_eq!(
            classify_maturity("source/host/a.ts", "partial recovery"),
            "partial"
        );
        assert_eq!(classify_maturity("source/host/a.ts", "ready"), "production");
        assert_eq!(
            classify_maturity("docs/assets/a.png", "bytes"),
            "artifact-only"
        );
    }
}
