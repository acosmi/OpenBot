//! Exception-only v4 overlay validation (R124).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

const OVERLAY_RELPATH: &str = "parity/overlay/v4.yaml";
const TOP_LEVEL_KEYS: [&str; 5] = [
    "schema",
    "schema_version",
    "baseline",
    "generated_by",
    "entries",
];
const ENTRY_KEYS: [&str; 6] = [
    "id",
    "disposition",
    "scope",
    "defect",
    "replacement",
    "notes",
];
const DISPOSITIONS: [&str; 4] = ["carry", "revalidate", "split", "superseded"];

#[derive(Debug, Serialize)]
pub(crate) struct OverlayReport {
    pub(crate) file: String,
    pub(crate) baseline: String,
    pub(crate) explicit_entries: usize,
    pub(crate) diff_required_revalidations: usize,
    pub(crate) disposition_counts: BTreeMap<String, usize>,
}

impl OverlayReport {
    pub(crate) fn empty(total_entries: usize) -> Self {
        let mut disposition_counts = empty_counts();
        disposition_counts.insert("carry".to_owned(), total_entries);
        Self {
            file: OVERLAY_RELPATH.to_owned(),
            baseline: "v4".to_owned(),
            explicit_entries: 0,
            diff_required_revalidations: 0,
            disposition_counts,
        }
    }
}

pub(crate) fn validate(
    root: &Path,
    parity_test_ids: &BTreeSet<String>,
    done_targets: &BTreeMap<String, String>,
    total_entries: usize,
    violations: &mut Vec<String>,
) -> OverlayReport {
    let path = root.join(OVERLAY_RELPATH);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            violations.push(format!(
                "{OVERLAY_RELPATH}：R124 exception-only overlay 缺失或不可读：{error}"
            ));
            return OverlayReport::empty(total_entries);
        }
    };
    let document: serde_yaml::Value = match serde_yaml::from_str(&text) {
        Ok(document) => document,
        Err(error) => {
            violations.push(format!("{OVERLAY_RELPATH}：YAML 解析失败：{error}"));
            return OverlayReport::empty(total_entries);
        }
    };
    let mut report = validate_document(&document, parity_test_ids, total_entries, violations);
    report.diff_required_revalidations =
        validate_diff_revalidation(root, done_targets, &document, violations);
    report
}

fn validate_document(
    document: &serde_yaml::Value,
    parity_test_ids: &BTreeSet<String>,
    total_entries: usize,
    violations: &mut Vec<String>,
) -> OverlayReport {
    let Some(map) = document.as_mapping() else {
        violations.push(format!("{OVERLAY_RELPATH}：顶层必须是 mapping"));
        return OverlayReport::empty(total_entries);
    };

    let present = map
        .keys()
        .filter_map(serde_yaml::Value::as_str)
        .collect::<BTreeSet<_>>();
    for key in TOP_LEVEL_KEYS {
        if !present.contains(key) {
            violations.push(format!("{OVERLAY_RELPATH}：缺顶层键 `{key}`"));
        }
    }
    for key in &present {
        if !TOP_LEVEL_KEYS.contains(key) {
            violations.push(format!("{OVERLAY_RELPATH}：出现未定义的顶层键 `{key}`"));
        }
    }

    if string(map, "schema") != Some("parity-overlay") {
        violations.push(format!(
            "{OVERLAY_RELPATH}：schema 必须逐字等于 `parity-overlay`"
        ));
    }
    if map
        .get(serde_yaml::Value::from("schema_version"))
        .and_then(serde_yaml::Value::as_u64)
        != Some(1)
    {
        violations.push(format!("{OVERLAY_RELPATH}：schema_version 必须是整数 1"));
    }
    let baseline = string(map, "baseline").unwrap_or_default().to_owned();
    if baseline != "v4" {
        violations.push(format!("{OVERLAY_RELPATH}：baseline 必须逐字等于 `v4`"));
    }
    if string(map, "generated_by").is_none() {
        violations.push(format!("{OVERLAY_RELPATH}：generated_by 必须是非空字符串"));
    }

    let Some(entries) = map
        .get(serde_yaml::Value::from("entries"))
        .and_then(serde_yaml::Value::as_sequence)
    else {
        violations.push(format!("{OVERLAY_RELPATH}：entries 必须是序列"));
        return OverlayReport::empty(total_entries);
    };

    let mut seen = BTreeSet::new();
    let mut explicit_counts = empty_counts();
    let mut shapes = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let Some(entry) = entry.as_mapping() else {
            violations.push(format!("{OVERLAY_RELPATH} entry#{index}：必须是 mapping"));
            continue;
        };
        for key in entry.keys() {
            let Some(key) = key.as_str() else {
                violations.push(format!("{OVERLAY_RELPATH} entry#{index}：键必须是字符串"));
                continue;
            };
            if !ENTRY_KEYS.contains(&key) {
                violations.push(format!(
                    "{OVERLAY_RELPATH} entry#{index}：出现未定义的键 `{key}`"
                ));
            }
        }

        let Some(id) = string(entry, "id") else {
            violations.push(format!(
                "{OVERLAY_RELPATH} entry#{index}：id 必须是非空字符串"
            ));
            continue;
        };
        if !seen.insert(id.to_owned()) {
            violations.push(format!("{OVERLAY_RELPATH}：重复 id `{id}`"));
            continue;
        }
        if !parity_test_ids.contains(id) {
            violations.push(format!(
                "{OVERLAY_RELPATH}：id `{id}` 不存在于 parity ledger 的 test_id 集合"
            ));
        }

        let Some(disposition) = string(entry, "disposition") else {
            violations.push(format!(
                "{OVERLAY_RELPATH} `{id}`：disposition 必须是非空字符串"
            ));
            continue;
        };
        if !DISPOSITIONS.contains(&disposition) {
            violations.push(format!(
                "{OVERLAY_RELPATH} `{id}`：disposition=`{disposition}` 不在 {DISPOSITIONS:?} 内"
            ));
            continue;
        }
        *explicit_counts.entry(disposition.to_owned()).or_insert(0) += 1;
        if disposition == "carry" {
            violations.push(format!(
                "{OVERLAY_RELPATH} `{id}`：carry 必须隐含，exception-only overlay 禁止显式 carry 行"
            ));
        }

        let scope = string(entry, "scope");
        let replacement = string(entry, "replacement");
        let defect = entry
            .get(serde_yaml::Value::from("defect"))
            .and_then(serde_yaml::Value::as_bool);
        if entry.contains_key(serde_yaml::Value::from("defect")) && defect != Some(true) {
            violations.push(format!(
                "{OVERLAY_RELPATH} `{id}`：defect 只允许布尔值 true；无缺陷时应省略该键"
            ));
        }
        match disposition {
            "revalidate" => {
                if scope.is_some() || replacement.is_some() {
                    violations.push(format!(
                        "{OVERLAY_RELPATH} `{id}`：revalidate 不允许 scope/replacement"
                    ));
                }
            }
            "split" => {
                if !matches!(scope, Some("web" | "desktop")) {
                    violations.push(format!(
                        "{OVERLAY_RELPATH} `{id}`：split 必须带 scope=web|desktop"
                    ));
                }
                if defect.is_some() || replacement.is_some() {
                    violations.push(format!(
                        "{OVERLAY_RELPATH} `{id}`：split 不允许 defect/replacement"
                    ));
                }
            }
            "superseded" => {
                if replacement.is_none() {
                    violations.push(format!(
                        "{OVERLAY_RELPATH} `{id}`：superseded 必须带非空 replacement"
                    ));
                }
                if scope.is_some() || defect.is_some() {
                    violations.push(format!(
                        "{OVERLAY_RELPATH} `{id}`：superseded 不允许 scope/defect"
                    ));
                }
            }
            "carry" => {}
            _ => unreachable!("disposition domain checked above"),
        }
        shapes.insert(
            id.to_owned(),
            (disposition.to_owned(), scope.map(str::to_owned), defect),
        );
    }

    require_initial(
        &shapes,
        "T-BROP-0046",
        "revalidate",
        None,
        Some(true),
        violations,
    );
    require_initial(
        &shapes,
        "T-CMP-0015",
        "split",
        Some("web"),
        None,
        violations,
    );
    require_initial(
        &shapes,
        "T-CMP-0018",
        "split",
        Some("web"),
        None,
        violations,
    );

    if seen.len() > total_entries {
        violations.push(format!(
            "{OVERLAY_RELPATH}：显式条目 {} 多于 parity 总条目 {total_entries}",
            seen.len()
        ));
    }
    let mut disposition_counts = explicit_counts;
    let explicit_non_carry = DISPOSITIONS[1..]
        .iter()
        .map(|key| disposition_counts.get(*key).copied().unwrap_or_default())
        .sum::<usize>();
    disposition_counts.insert(
        "carry".to_owned(),
        total_entries.saturating_sub(explicit_non_carry),
    );

    OverlayReport {
        file: OVERLAY_RELPATH.to_owned(),
        baseline,
        explicit_entries: seen.len(),
        diff_required_revalidations: 0,
        disposition_counts,
    }
}

fn validate_diff_revalidation(
    root: &Path,
    done_targets: &BTreeMap<String, String>,
    document: &serde_yaml::Value,
    violations: &mut Vec<String>,
) -> usize {
    let prefixes = match changed_target_prefixes(root) {
        Ok(prefixes) => prefixes,
        Err(error) => {
            violations.push(format!(
                "{OVERLAY_RELPATH}：无法计算 git diff target 前缀：{error}"
            ));
            return 0;
        }
    };
    let dispositions = document
        .as_mapping()
        .and_then(|map| map.get(serde_yaml::Value::from("entries")))
        .and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.as_mapping()?;
            Some((
                string(entry, "id")?.to_owned(),
                string(entry, "disposition")?.to_owned(),
            ))
        })
        .collect::<BTreeMap<_, _>>();

    let required = done_targets
        .iter()
        .filter(|(_, target)| prefixes.iter().any(|prefix| target.contains(prefix)))
        .collect::<Vec<_>>();
    for (test_id, target) in &required {
        if dispositions.get(*test_id).map(String::as_str) != Some("revalidate") {
            violations.push(format!(
                "{OVERLAY_RELPATH}：git diff 命中 done target `{target}`，必须为 `{test_id}` 添加 disposition=revalidate 并重跑证据"
            ));
        }
    }
    required.len()
}

fn changed_target_prefixes(root: &Path) -> anyhow::Result<BTreeSet<String>> {
    let reference = ["origin/main", "main"]
        .into_iter()
        .find(|candidate| {
            Command::new("git")
                .args(["rev-parse", "--verify", &format!("{candidate}^{{commit}}")])
                .current_dir(root)
                .output()
                .is_ok_and(|output| output.status.success())
        })
        .ok_or_else(|| anyhow::anyhow!("origin/main 与 main 都不可解析"))?;
    let merge_base = Command::new("git")
        .args(["merge-base", "HEAD", reference])
        .current_dir(root)
        .output()?;
    if !merge_base.status.success() {
        return Err(anyhow::anyhow!(
            "git merge-base HEAD {reference} failed: {}",
            String::from_utf8_lossy(&merge_base.stderr).trim()
        ));
    }
    let base = String::from_utf8(merge_base.stdout)?.trim().to_owned();
    let output = Command::new("git")
        .args([
            "diff",
            "--name-only",
            "--diff-filter=ACMRT",
            &base,
            "--",
            "crates",
        ])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8(output.stdout)?
        .lines()
        .filter_map(path_target_prefix)
        .collect())
}

fn path_target_prefix(path: &str) -> Option<String> {
    let path = path.strip_prefix("crates/openbot-")?;
    let (crate_name, source) = path.split_once("/src/")?;
    let mut prefix = format!("openbot_{}", crate_name.replace('-', "_"));
    let source = source.strip_suffix(".rs")?;
    if source != "lib" {
        let source = source.strip_suffix("/mod").unwrap_or(source);
        if !source.is_empty() {
            prefix.push_str("::");
            prefix.push_str(&source.replace('/', "::"));
        }
    }
    Some(prefix)
}

fn require_initial(
    shapes: &BTreeMap<String, (String, Option<String>, Option<bool>)>,
    id: &str,
    disposition: &str,
    scope: Option<&str>,
    defect: Option<bool>,
    violations: &mut Vec<String>,
) {
    let expected = (disposition.to_owned(), scope.map(str::to_owned), defect);
    if shapes.get(id) != Some(&expected) {
        violations.push(format!(
            "{OVERLAY_RELPATH}：R124 初值 `{id}` 必须是 disposition={disposition}, scope={scope:?}, defect={defect:?}"
        ));
    }
}

fn string<'a>(map: &'a serde_yaml::Mapping, key: &str) -> Option<&'a str> {
    map.get(serde_yaml::Value::from(key))
        .and_then(serde_yaml::Value::as_str)
        .filter(|value| !value.is_empty())
}

fn empty_counts() -> BTreeMap<String, usize> {
    DISPOSITIONS
        .into_iter()
        .map(|key| (key.to_owned(), 0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{DISPOSITIONS, validate_document};
    use std::collections::BTreeSet;

    const VALID: &str = r#"
schema: parity-overlay
schema_version: 1
baseline: v4
generated_by: manual
entries:
  - id: T-BROP-0046
    disposition: revalidate
    defect: true
  - id: T-CMP-0015
    disposition: split
    scope: web
  - id: T-CMP-0018
    disposition: split
    scope: web
"#;

    fn test_ids() -> BTreeSet<String> {
        ["T-BROP-0046", "T-CMP-0015", "T-CMP-0018"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn initial_overlay_is_exception_only_and_counts_implicit_carry() {
        let document = serde_yaml::from_str(VALID).expect("valid yaml");
        let mut violations = Vec::new();
        let report = validate_document(&document, &test_ids(), 10, &mut violations);
        assert!(violations.is_empty(), "{violations:?}");
        assert_eq!(report.explicit_entries, 3);
        assert_eq!(report.disposition_counts["carry"], 7);
        assert_eq!(report.disposition_counts["revalidate"], 1);
        assert_eq!(report.disposition_counts["split"], 2);
        assert_eq!(report.disposition_counts["superseded"], 0);
        assert_eq!(report.disposition_counts.len(), DISPOSITIONS.len());
    }

    #[test]
    fn explicit_carry_and_missing_initial_defect_are_rejected() {
        let changed = VALID.replace("    defect: true\n", "").replacen(
            "    disposition: split\n",
            "    disposition: carry\n",
            1,
        );
        let document = serde_yaml::from_str(&changed).expect("valid yaml");
        let mut violations = Vec::new();
        validate_document(&document, &test_ids(), 10, &mut violations);
        assert!(
            violations
                .iter()
                .any(|item| item.contains("carry 必须隐含"))
        );
        assert!(violations.iter().any(|item| item.contains("T-BROP-0046")));
    }

    #[test]
    fn rust_source_path_becomes_ledger_target_prefix() {
        assert_eq!(
            super::path_target_prefix("crates/openbot-computer/src/control.rs").as_deref(),
            Some("openbot_computer::control")
        );
        assert_eq!(
            super::path_target_prefix("crates/openbot-testkit/src/xtask/engine.rs").as_deref(),
            Some("openbot_testkit::xtask::engine")
        );
    }
}
