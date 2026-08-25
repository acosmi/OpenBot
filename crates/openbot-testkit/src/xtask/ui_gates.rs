//! Deterministic local gates for the GUI first source.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use flate2::{Compression, GzBuilder};
use regex::Regex;
use walkdir::WalkDir;

const UI_DIR: &str = "crates/openbot-ui";
const WASM_GZIP_LIMIT: usize = 3_670_016;
const CSS_LIMIT: usize = 96 * 1024;
const FONT_LIMIT: usize = 800 * 1024;

pub(crate) fn i18n_check(root: &Path) -> Result<()> {
    let locales = root.join(UI_DIR).join("locales");
    let en = read_locale(&locales.join("en.json"))?;
    let zh = read_locale(&locales.join("zh-CN.json"))?;

    let en_keys = en.keys().cloned().collect::<BTreeSet<_>>();
    let zh_keys = zh.keys().cloned().collect::<BTreeSet<_>>();
    if en_keys != zh_keys {
        bail!(
            "i18n key drift: en_only={:?}, zh-CN_only={:?}",
            en_keys.difference(&zh_keys).collect::<Vec<_>>(),
            zh_keys.difference(&en_keys).collect::<Vec<_>>()
        );
    }

    for key in &en_keys {
        let en_placeholders = placeholder_names(en.get(key).expect("key set was checked"))
            .with_context(|| format!("en.json key `{key}` has malformed interpolation"))?;
        let zh_placeholders = placeholder_names(zh.get(key).expect("key set was checked"))
            .with_context(|| format!("zh-CN.json key `{key}` has malformed interpolation"))?;
        if en_placeholders != zh_placeholders {
            bail!(
                "i18n placeholder drift at `{key}`: en={en_placeholders:?}, zh-CN={zh_placeholders:?}"
            );
        }
    }

    println!(
        "i18n-check: ok ({} leaf keys; en/zh-CN key and placeholder sets exact)",
        en_keys.len()
    );
    Ok(())
}

pub(crate) fn design_lint(root: &Path) -> Result<()> {
    let ui = root.join(UI_DIR);
    let sources = rust_sources(&ui.join("src"))?;
    let literal_or_arbitrary = Regex::new(r"(bg|text|border)-\[#|\[[0-9]+px\]")?;
    let semantic_surface = Regex::new(r"(bg|border)-(danger|caution|success|info)\b")?;
    let shadow = Regex::new(r"shadow-([A-Za-z0-9_-]+)")?;

    let mut violations = Vec::new();
    for (path, source) in &sources {
        for (index, line) in source.lines().enumerate() {
            let line_no = index + 1;
            if line.contains("dark:") {
                violations.push(format!("{}:{line_no}: forbidden `dark:`", path.display()));
            }
            if literal_or_arbitrary.is_match(line) {
                violations.push(format!(
                    "{}:{line_no}: literal color/arbitrary px class",
                    path.display()
                ));
            }
            if semantic_surface.is_match(line) {
                violations.push(format!(
                    "{}:{line_no}: semantic status color used as surface/border",
                    path.display()
                ));
            }
            for capture in shadow.captures_iter(line) {
                let value = capture.get(1).expect("capture exists").as_str();
                if !matches!(value, "popover" | "dialog" | "none") {
                    violations.push(format!(
                        "{}:{line_no}: unapproved shadow `{value}`",
                        path.display()
                    ));
                }
            }
            if line.contains("/_design") {
                violations.push(format!(
                    "{}:{line_no}: production source contains design-gallery route",
                    path.display()
                ));
            }
        }
    }
    if !violations.is_empty() {
        bail!("design-lint violations:\n{}", violations.join("\n"));
    }

    let icon_count = check_icons(&ui, &sources)?;
    println!(
        "design-lint: ok ({} Rust files; {icon_count} manifest/SVG icons; reverse rules clean)",
        sources.len()
    );
    Ok(())
}

pub(crate) fn css_check(root: &Path, args: &[String]) -> Result<()> {
    let ui = root.join(UI_DIR);
    let css_path = parse_path_option(root, args, "--css")?
        .map_or_else(|| unique_file_with_extension(&ui.join("dist"), "css"), Ok)?;
    let css = fs::read_to_string(&css_path)
        .with_context(|| format!("read compiled CSS {}", css_path.display()))?;
    let sources = rust_sources(&ui.join("src"))?;
    let source_classes = source_class_literals(&sources)?;
    let compiled_classes = compiled_css_classes(&css)?;
    let missing = source_classes
        .difference(&compiled_classes)
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "compiled CSS misses {} Rust class literals: {:?} (css={})",
            missing.len(),
            missing,
            css_path.display()
        );
    }
    println!(
        "css-check: ok ({} source class literals found in {})",
        source_classes.len(),
        css_path.display()
    );
    Ok(())
}

pub(crate) fn bundle_budget(root: &Path, args: &[String]) -> Result<()> {
    let ui = root.join(UI_DIR);
    let dist = parse_path_option(root, args, "--dist")?.unwrap_or_else(|| ui.join("dist"));
    let wasm = unique_file_with_extension(&dist, "wasm")?;
    let css = unique_file_with_extension(&dist, "css")?;
    let wasm_bytes = fs::read(&wasm).with_context(|| format!("read {}", wasm.display()))?;
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::best());
    encoder.write_all(&wasm_bytes)?;
    let wasm_gzip = encoder.finish()?.len();
    let css_bytes = file_len(&css)?;
    let fonts = [
        ui.join("assets/fonts/InterVariable.woff2"),
        ui.join("assets/fonts/InterVariable-Italic.woff2"),
    ];
    let font_bytes = fonts
        .iter()
        .map(|path| file_len(path))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .sum::<usize>();

    let mut failures = Vec::new();
    if wasm_gzip > WASM_GZIP_LIMIT {
        failures.push(format!(
            "wasm gzip {wasm_gzip} > {WASM_GZIP_LIMIT} bytes ({})",
            wasm.display()
        ));
    }
    if css_bytes > CSS_LIMIT {
        failures.push(format!(
            "css {css_bytes} > {CSS_LIMIT} bytes ({})",
            css.display()
        ));
    }
    if font_bytes > FONT_LIMIT {
        failures.push(format!("fonts {font_bytes} > {FONT_LIMIT} bytes"));
    }
    if !failures.is_empty() {
        bail!("bundle budget exceeded:\n{}", failures.join("\n"));
    }
    println!(
        "bundle-budget: ok (wasm gzip={wasm_gzip}/{WASM_GZIP_LIMIT}; css={css_bytes}/{CSS_LIMIT}; fonts={font_bytes}/{FONT_LIMIT})"
    );
    Ok(())
}

fn read_locale(path: &Path) -> Result<BTreeMap<String, String>> {
    let source = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&source).with_context(|| format!("parse {}", path.display()))?;
    let mut leaves = BTreeMap::new();
    collect_locale_leaves("", &value, &mut leaves)?;
    Ok(leaves)
}

fn collect_locale_leaves(
    prefix: &str,
    value: &serde_json::Value,
    leaves: &mut BTreeMap<String, String>,
) -> Result<()> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if key.is_empty() {
                    bail!("locale key segment is empty at `{prefix}`");
                }
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                collect_locale_leaves(&path, value, leaves)?;
            }
        }
        serde_json::Value::String(text) if !prefix.is_empty() => {
            if leaves.insert(prefix.to_owned(), text.clone()).is_some() {
                bail!("duplicate locale leaf `{prefix}`");
            }
        }
        _ => bail!("locale leaf `{prefix}` must be a string"),
    }
    Ok(())
}

fn placeholder_names(value: &str) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    let mut rest = value;
    loop {
        let Some(open) = rest.find('{') else {
            if rest.contains('}') {
                bail!("unmatched closing brace");
            }
            break;
        };
        if rest[..open].contains('}') || !rest[open..].starts_with("{{") {
            bail!("single/unmatched brace; leptos_i18n requires `{{{{ name }}}}`");
        }
        let after_open = &rest[open + 2..];
        let close = after_open
            .find("}}")
            .ok_or_else(|| anyhow!("unclosed interpolation"))?;
        let inner = after_open[..close].trim();
        let name = inner.split(',').next().unwrap_or("").trim();
        if name.is_empty()
            || !name.chars().enumerate().all(|(index, character)| {
                character == '_'
                    || character.is_ascii_alphanumeric()
                        && (index > 0 || !character.is_ascii_digit())
            })
        {
            bail!("invalid interpolation name `{name}`");
        }
        names.insert(name.to_owned());
        rest = &after_open[close + 2..];
    }
    Ok(names)
}

fn rust_sources(dir: &Path) -> Result<Vec<(PathBuf, String)>> {
    let mut sources = Vec::new();
    for entry in WalkDir::new(dir).sort_by_file_name() {
        let entry = entry.with_context(|| format!("walk {}", dir.display()))?;
        let path = entry.path();
        if entry.file_type().is_file()
            && path.extension().and_then(|value| value.to_str()) == Some("rs")
        {
            sources.push((
                path.to_path_buf(),
                fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?,
            ));
        }
    }
    Ok(sources)
}

fn check_icons(ui: &Path, sources: &[(PathBuf, String)]) -> Result<usize> {
    let manifest_path = ui.join("design/icons.toml");
    let manifest: toml::Value = toml::from_str(
        &fs::read_to_string(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))?,
    )?;
    let entries = manifest
        .get("icon")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| anyhow!("icons.toml missing [[icon]]"))?;
    let names = entries
        .iter()
        .map(|entry| {
            entry
                .get("name")
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("icon entry missing name"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if names.len() != entries.len() {
        bail!("icons.toml contains duplicate icon names");
    }
    let expected = manifest
        .get("meta")
        .and_then(|value| value.get("expected_icon_count"))
        .and_then(toml::Value::as_integer)
        .ok_or_else(|| anyhow!("icons.toml missing meta.expected_icon_count"))?;
    if i64::try_from(names.len())? != expected {
        bail!("icon count drift: expected {expected}, got {}", names.len());
    }

    let icon_dir = ui.join("design/icons");
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(&icon_dir).with_context(|| format!("read {}", icon_dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("svg") {
            let name = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| anyhow!("non-UTF8 SVG name: {}", path.display()))?;
            actual.insert(name.to_owned());
            let svg = fs::read_to_string(&path)?;
            let lower = svg.to_ascii_lowercase();
            if !svg.contains("currentColor")
                || !svg.contains("stroke-width=\"1.75\"")
                || [
                    "<script",
                    "javascript:",
                    "<foreignobject",
                    "<image",
                    "<use",
                    "href=",
                    "url(",
                ]
                .iter()
                .any(|needle| lower.contains(needle))
            {
                bail!("unsafe or drifted SVG: {}", path.display());
            }
        }
    }
    if names != actual {
        bail!(
            "icons.toml/SVG drift: manifest_only={:?}, svg_only={:?}",
            names.difference(&actual).collect::<Vec<_>>(),
            actual.difference(&names).collect::<Vec<_>>()
        );
    }

    let variants = names
        .iter()
        .map(|name| icon_variant(name))
        .collect::<BTreeSet<_>>();
    let usage = Regex::new(r"\bIcon::([A-Z][A-Za-z0-9_]*)")?;
    for (path, source) in sources {
        for capture in usage.captures_iter(source) {
            let variant = capture.get(1).expect("capture exists").as_str();
            if variant != "ALL" && !variants.contains(variant) {
                bail!(
                    "{} uses icon outside allowlist: Icon::{variant}",
                    path.display()
                );
            }
        }
    }
    Ok(names.len())
}

fn icon_variant(name: &str) -> String {
    name.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_ascii_uppercase().to_string() + characters.as_str()
            })
        })
        .collect()
}

fn source_class_literals(sources: &[(PathBuf, String)]) -> Result<BTreeSet<String>> {
    let literal = Regex::new(r#"class\s*=\s*"([^"]*)""#)?;
    let assignment = Regex::new(r"class\s*=")?;
    let mut classes = BTreeSet::new();
    for (path, source) in sources {
        for found in assignment.find_iter(source) {
            let rest = source[found.end()..].trim_start();
            if !rest.starts_with('"') && !rest.starts_with('(') {
                bail!(
                    "{} has non-literal class assignment near byte {}",
                    path.display(),
                    found.start()
                );
            }
        }
        for capture in literal.captures_iter(source) {
            classes.extend(
                capture
                    .get(1)
                    .expect("capture exists")
                    .as_str()
                    .split_ascii_whitespace()
                    .map(str::to_owned),
            );
        }
    }
    Ok(classes)
}

fn compiled_css_classes(css: &str) -> Result<BTreeSet<String>> {
    let selector = Regex::new(r"\.((?:\\.|[A-Za-z0-9_-])+)")?;
    Ok(selector
        .captures_iter(css)
        .filter_map(|capture| capture.get(1))
        .map(|value| value.as_str().replace("\\:", ":").replace("\\/", "/"))
        .collect())
}

fn parse_path_option(root: &Path, args: &[String], option: &str) -> Result<Option<PathBuf>> {
    match args {
        [] => Ok(None),
        [flag, path] if flag == option => {
            let path = PathBuf::from(path);
            Ok(Some(if path.is_absolute() {
                path
            } else {
                root.join(path)
            }))
        }
        _ => bail!("expected no arguments or `{option} <path>`"),
    }
}

fn unique_file_with_extension(dir: &Path, extension: &str) -> Result<PathBuf> {
    if !dir.is_dir() {
        bail!("artifact directory does not exist: {}", dir.display());
    }
    let mut files = WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some(extension))
        .collect::<Vec<_>>();
    files.sort();
    match files.as_slice() {
        [file] => Ok(file.clone()),
        [] => bail!("no .{extension} artifact under {}", dir.display()),
        _ => bail!(
            "expected one .{extension} artifact under {}, got {files:?}",
            dir.display()
        ),
    }
}

fn file_len(path: &Path) -> Result<usize> {
    usize::try_from(
        fs::metadata(path)
            .with_context(|| format!("stat {}", path.display()))?
            .len(),
    )
    .context("artifact length does not fit usize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_use_real_leptos_i18n_syntax() {
        assert_eq!(
            placeholder_names("Hello {{ name }}, {{ count }}").unwrap(),
            BTreeSet::from(["count".to_owned(), "name".to_owned()])
        );
        assert!(placeholder_names("Hello {name}").is_err());
        assert!(placeholder_names("Hello {{ 9name }}").is_err());
    }

    #[test]
    fn icon_variant_mapping_matches_build_script_contract() {
        assert_eq!(icon_variant("arrow-up-right"), "ArrowUpRight");
        assert_eq!(icon_variant("x"), "X");
    }

    #[test]
    fn compiled_css_class_parser_unescapes_tailwind_selectors() {
        let classes =
            compiled_css_classes(".ob-card{display:block}.md\\:grid{display:grid}").unwrap();
        assert!(classes.contains("ob-card"));
        assert!(classes.contains("md:grid"));
    }
}
