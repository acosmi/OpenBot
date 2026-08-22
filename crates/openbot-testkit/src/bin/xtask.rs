//! `xtask` —— 仓库闸门驱动器。
//!
//! 落点是 `openbot-testkit` 的 bin target 而不是第 11 个 crate：v3 §5.1 只允许四个理由建
//! crate（独立安全边界 / 独立发布单元 / 明显不同的 feature graph / 可单独复用的纯协议），
//! xtask 一个都不满足；而 §5.1 的 crate 表里 `openbot-testkit` 的职责原文就写着 "xtask"。
//! `required-features = ["xtask"]` 让它对 `cargo build --workspace` 完全透明。
//!
//! 子命令：
//!
//! - `parity-check` —— 校验 `parity/*.yaml`，强制统一 schema v1 的 8 条规则。
//! - `ci`           —— 按 v3 §16.3 的固定顺序跑本机可执行的那一段闸门。
//!
//! 用法（`.cargo/config.toml` 已配 alias）：
//!
//! ```text
//! cargo xtask parity-check
//! cargo xtask parity-check --json
//! cargo xtask ci
//! ```

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;

// `test-inventory` 的实现落在 `crates/openbot-testkit/src/xtask/test_inventory.rs`。
// 用 `#[path]` 而不是把它挪进 `src/bin/xtask/`：模块源码归属 crate 的 `src/xtask/`（与它
// 服务的 xtask 同名目录），而 `required-features = ["xtask"]` 的隔离靠 bin target 本身 ——
// 挂在 lib 上会把五个 oxc crate 拖进不开 feature 的 `cargo build --workspace`。
#[path = "../xtask/test_inventory.rs"]
mod test_inventory;

// ---------------------------------------------------------------------------
// 契约常量
// ---------------------------------------------------------------------------

/// v3 §5.1 的十个 crate，同时也是 parity ledger `owner` 字段的封闭取值域（规则 3）。
const OWNERS: [&str; 10] = [
    "openbot-contracts",
    "openbot-domain",
    "openbot-application",
    "openbot-infra",
    "openbot-agent",
    "openbot-computer",
    "openbot-server",
    "openbot-ui",
    "openbot-desktop",
    "openbot-testkit",
];

/// CLAUDE.md §4〈parity 与新增必须分开标注〉：三值封闭域，中文原样（规则 2）。
///
/// 把新增写成"当前行为"是 v2 审计里最重的一类错误（v3 §28.1 R1），所以这里不接受
/// 大小写变体、英文译名或第四个值。
const LABELS: [&str; 3] = ["parity", "新增", "替代"];

/// 统一 schema v1 的 `status` 封闭域。规则 4 依赖它 —— 如果 `status` 可以是任意字符串，
/// "status=done 当且仅当 done_evidence 非空"这条双向约束在 `status: blocked` 上会静默为真。
const STATUSES: [&str; 3] = ["todo", "in_progress", "done"];

/// 顶层键封闭集。主控裁决："不得自行增删顶层键"。
const TOP_LEVEL_KEYS: [&str; 6] = [
    "schema",
    "schema_version",
    "upstream_commit",
    "generated_by",
    "recount",
    "entries",
];

/// entry 的必填键（规则 1）。
const ENTRY_REQUIRED_KEYS: [&str; 9] = [
    "id",
    "upstream",
    "label",
    "target",
    "owner",
    "test_id",
    "migration_rule",
    "status",
    "evidence",
];

/// entry 的可选键（规则 1 的两个豁免）。
const ENTRY_OPTIONAL_KEYS: [&str; 2] = ["notes", "done_evidence"];

/// `recount` 条目的键集合（规则 8）。
const RECOUNT_KEYS: [&str; 3] = ["command", "cwd", "expect"];

/// `recount[].cwd` 的封闭域：`upstream` = 上游只读克隆，`repo` = 本仓根。
const RECOUNT_CWD: [&str; 2] = ["upstream", "repo"];

/// v3 §1.2 固定源码基线里的 CopilotKit/OpenBot commit。
/// 只用于**告警**，不是 8 条硬规则之一。
const EXPECTED_UPSTREAM_COMMIT: &str = "891df72f1827454d8b353d108fe5dd2313b7e30d";

/// v3 §19.3 + 设计系统文档 §11 列出的 ledger 名。只用于**告警**，不是硬规则。
const KNOWN_SCHEMAS: [&str; 9] = [
    "api",
    "routes",
    "tables",
    "env",
    "events",
    "components",
    "browser-operations",
    "ui",
    "tests",
];

/// `migration_rule` 的已知前缀（允许 `preserve: 说明` 这种英文冒号后缀）。
/// 只用于**告警**，不是硬规则。
const MIGRATION_RULE_PREFIXES: [&str; 4] = ["preserve", "rename", "remove", "n/a"];

/// 8 条规则的原文，违规时原样打印，避免"规则 5"这种只有编号没有内容的报错。
const RULES: [&str; 8] = [
    "规则 1：除 notes / done_evidence 外每个键都必须存在且非空字符串（顶层键集合固定，不得自行增删）",
    "规则 2：label 只能是 parity / 新增 / 替代 三个值之一",
    "规则 3：owner 必须是 v3 §5.1 十个 crate 之一",
    "规则 4：status=done 当且仅当 done_evidence 存在且非空（status 本身限 todo / in_progress / done）",
    "规则 5：id 在文件内唯一；test_id 在全部 ledger 内唯一",
    "规则 6：test_id 匹配 ^T-[A-Z]+-[0-9]{4}$",
    "规则 7：upstream 字段禁止裸行号（禁止以 :<数字> 结尾）",
    "规则 8：recount 至少一条，且每条 command 非空",
];

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let subcommand = args.first().map(String::as_str);

    let result = match subcommand {
        Some("parity-check") => cmd_parity_check(&args[1..]),
        Some("test-inventory") => cmd_test_inventory(&args[1..]),
        Some("ci") => cmd_ci(),
        Some("help") | Some("--help") | Some("-h") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => {
            eprintln!("xtask: 未知子命令 `{other}`");
            print_usage();
            Err(anyhow!("未知子命令"))
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("\nxtask 失败：{err:#}");
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    println!(
        "\
xtask —— OpenBot 仓库闸门驱动器

用法：
  cargo xtask parity-check [--json]   校验 parity/*.yaml（统一 schema v1 的 8 条规则）
  cargo xtask test-inventory --upstream <上游干净克隆路径> [--dry-run]
                                      用 oxc_parser 做 AST 级 test inventory（v3 §24 G0 / G8），
                                      产出 parity/tests.yaml 与 fixtures/tests/upstream-ast-inventory.json
  cargo xtask ci                      按 v3 §16.3 顺序跑本机可执行的闸门段
  cargo xtask help                    打印本帮助
"
    );
}

// ---------------------------------------------------------------------------
// test-inventory
// ---------------------------------------------------------------------------

/// v3 §19.3 的 `parity/tests.yaml` 与 §24 G0 的"上游基线测试原始结果归档"。
/// 实现在 [`test_inventory`]；这里只负责定位仓根并透传参数。
fn cmd_test_inventory(args: &[String]) -> Result<()> {
    let root = workspace_root()?;
    test_inventory::run(args, &root)
}

// ---------------------------------------------------------------------------
// 仓库根定位
// ---------------------------------------------------------------------------

/// 从当前目录向上找带 `[workspace]` 的 `Cargo.toml`；找不到时退回编译期的
/// `CARGO_MANIFEST_DIR/../..`（`crates/openbot-testkit` 的祖父目录 = 仓根）。
///
/// 不写死绝对路径：`cargo xtask` 可能在任意子目录里被调用。
fn workspace_root() -> Result<PathBuf> {
    let start = std::env::current_dir().context("读取当前工作目录失败")?;
    for dir in start.ancestors() {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file() {
            let text = std::fs::read_to_string(&manifest)
                .with_context(|| format!("读取 {} 失败", manifest.display()))?;
            if text.contains("[workspace]") {
                return Ok(dir.to_path_buf());
            }
        }
    }

    let fallback = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("无法从 CARGO_MANIFEST_DIR 推出仓库根"))?;
    if fallback.join("Cargo.toml").is_file() {
        return Ok(fallback);
    }
    bail!(
        "从 {} 向上没有找到含 [workspace] 的 Cargo.toml，且编译期兜底路径 {} 也不成立",
        start.display(),
        fallback.display()
    )
}

// ---------------------------------------------------------------------------
// parity-check
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct LedgerReport {
    file: String,
    schema: String,
    entries: usize,
    /// status -> 计数。BTreeMap 保证输出顺序稳定，diff 才有意义。
    status_counts: BTreeMap<String, usize>,
    recount_commands: usize,
}

#[derive(Serialize)]
struct ParityReport {
    ledgers: Vec<LedgerReport>,
    total_entries: usize,
    total_status_counts: BTreeMap<String, usize>,
    violations: Vec<String>,
    warnings: Vec<String>,
}

fn cmd_parity_check(args: &[String]) -> Result<()> {
    let json = args.iter().any(|a| a == "--json");
    for a in args {
        if a != "--json" {
            bail!("parity-check: 未知参数 `{a}`（只接受 --json）");
        }
    }

    let root = workspace_root()?;
    let parity_dir = root.join("parity");

    let mut files: Vec<PathBuf> = Vec::new();
    if parity_dir.is_dir() {
        for entry in walkdir::WalkDir::new(&parity_dir)
            .max_depth(1)
            .sort_by_file_name()
        {
            let entry = entry.with_context(|| format!("遍历 {} 失败", parity_dir.display()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.into_path();
            // 只认 .yaml；.yml 会被告警点名（见下），不静默接受也不静默忽略。
            if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                files.push(path);
            }
        }
    }

    // 空目录必须优雅返回，不 panic：Phase 0 里 parity/ 由别的 agent 填，
    // 骨架 PR 自己跑 `cargo xtask ci` 时它就是空的。
    if files.is_empty() {
        let reason = if parity_dir.is_dir() {
            format!("{} 存在但没有 *.yaml", parity_dir.display())
        } else {
            format!("{} 不存在", parity_dir.display())
        };
        if json {
            let report = ParityReport {
                ledgers: Vec::new(),
                total_entries: 0,
                total_status_counts: BTreeMap::new(),
                violations: Vec::new(),
                warnings: vec![format!("0 ledger：{reason}")],
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!("parity-check: 0 ledger（{reason}）—— 无可校验项，通过。");
        }
        return Ok(());
    }

    let mut violations: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut ledgers: Vec<LedgerReport> = Vec::new();
    // 规则 5 的跨文件那一半：test_id -> 首次出现的 "文件#id"。
    let mut global_test_ids: BTreeMap<String, String> = BTreeMap::new();

    // 顺带把 .yml 点名，避免"文件名写错所以没被校验"这种静默漏检。
    if parity_dir.is_dir() {
        for entry in walkdir::WalkDir::new(&parity_dir)
            .max_depth(1)
            .sort_by_file_name()
        {
            let entry = entry.with_context(|| format!("遍历 {} 失败", parity_dir.display()))?;
            if entry.file_type().is_file()
                && entry.path().extension().and_then(|e| e.to_str()) == Some("yml")
            {
                warnings.push(format!(
                    "{}：扩展名是 .yml，parity-check 只扫 .yaml，这个文件没有被校验",
                    rel(&root, entry.path())
                ));
            }
        }
    }

    for path in &files {
        let display = rel(&root, path);
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取 {} 失败", path.display()))?;
        let doc: serde_yaml::Value = match serde_yaml::from_str(&text) {
            Ok(v) => v,
            Err(err) => {
                violations.push(format!("{display}：YAML 解析失败：{err}"));
                continue;
            }
        };

        if let Some(report) = check_ledger(
            &display,
            &doc,
            &mut violations,
            &mut warnings,
            &mut global_test_ids,
        ) {
            ledgers.push(report);
        }
    }

    let total_entries = ledgers.iter().map(|l| l.entries).sum();
    let mut total_status_counts: BTreeMap<String, usize> = BTreeMap::new();
    for ledger in &ledgers {
        for (status, count) in &ledger.status_counts {
            *total_status_counts.entry(status.clone()).or_insert(0) += count;
        }
    }

    let report = ParityReport {
        ledgers,
        total_entries,
        total_status_counts,
        violations,
        warnings,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_parity_report(&report);
    }

    if report.violations.is_empty() {
        Ok(())
    } else {
        bail!("parity-check: {} 条违反", report.violations.len())
    }
}

fn print_parity_report(report: &ParityReport) {
    println!("parity-check: {} ledger", report.ledgers.len());
    for ledger in &report.ledgers {
        let dist = if ledger.status_counts.is_empty() {
            "（无 entry）".to_string()
        } else {
            ledger
                .status_counts
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!(
            "  {:<32} schema={:<20} entries={:<5} recount={:<3} status: {}",
            ledger.file, ledger.schema, ledger.entries, ledger.recount_commands, dist
        );
    }
    let total_dist = if report.total_status_counts.is_empty() {
        "（无 entry）".to_string()
    } else {
        report
            .total_status_counts
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    println!(
        "  合计: entries={} status: {}",
        report.total_entries, total_dist
    );

    if !report.warnings.is_empty() {
        println!("\n告警（不影响退出码）：");
        for w in &report.warnings {
            println!("  ! {w}");
        }
    }

    if report.violations.is_empty() {
        println!("\nparity-check: 通过（0 违反）");
    } else {
        println!("\n违反的规则原文：");
        for rule in RULES {
            println!("  {rule}");
        }
        println!("\n违反明细（{} 条）：", report.violations.len());
        for v in &report.violations {
            println!("  x {v}");
        }
    }
}

/// 校验单个 ledger。返回 `None` 表示文件结构坏到无法统计（违反已记入 `violations`）。
fn check_ledger(
    file: &str,
    doc: &serde_yaml::Value,
    violations: &mut Vec<String>,
    warnings: &mut Vec<String>,
    global_test_ids: &mut BTreeMap<String, String>,
) -> Option<LedgerReport> {
    let map = match doc.as_mapping() {
        Some(m) => m,
        None => {
            violations.push(format!("{file} [规则 1]：顶层不是 mapping"));
            return None;
        }
    };

    // --- 规则 1：顶层键封闭集，缺一不可、多一不可 -------------------------
    let present: BTreeSet<String> = map
        .keys()
        .filter_map(|k| k.as_str().map(str::to_string))
        .collect();
    for required in TOP_LEVEL_KEYS {
        if !present.contains(required) {
            violations.push(format!("{file} [规则 1]：缺顶层键 `{required}`"));
        }
    }
    for got in &present {
        if !TOP_LEVEL_KEYS.contains(&got.as_str()) {
            violations.push(format!(
                "{file} [规则 1]：出现未定义的顶层键 `{got}`（统一 schema v1 的顶层键固定为 {TOP_LEVEL_KEYS:?}，不得自行增删）"
            ));
        }
    }

    let schema_name = nonempty_str(map, "schema").unwrap_or_default();
    if schema_name.is_empty() && present.contains("schema") {
        violations.push(format!("{file} [规则 1]：`schema` 不是非空字符串"));
    }
    if !schema_name.is_empty() && !KNOWN_SCHEMAS.contains(&schema_name.as_str()) {
        warnings.push(format!(
            "{file}：schema=`{schema_name}` 不在 v3 §19.3 + 设计系统文档 §11 的 ledger 名单 {KNOWN_SCHEMAS:?} 里"
        ));
    }

    // 1.98.0 的 clippy::collapsible_match 要求这里用 match guard 而不是嵌套 if。
    match map.get(serde_yaml::Value::from("schema_version")) {
        Some(v) if v.as_u64() != Some(1) => {
            violations.push(format!(
                "{file} [规则 1]：`schema_version` 必须是整数 1，实得 {v:?}"
            ));
        }
        // 缺键上面已按规则 1 记过；合法值无需处理。
        _ => {}
    }

    let upstream_commit = nonempty_str(map, "upstream_commit").unwrap_or_default();
    if upstream_commit.is_empty() && present.contains("upstream_commit") {
        violations.push(format!("{file} [规则 1]：`upstream_commit` 不是非空字符串"));
    }
    if !upstream_commit.is_empty() && upstream_commit != EXPECTED_UPSTREAM_COMMIT {
        warnings.push(format!(
            "{file}：upstream_commit=`{upstream_commit}` 不等于 v3 §1.2 固定基线 `{EXPECTED_UPSTREAM_COMMIT}`"
        ));
    }

    if nonempty_str(map, "generated_by").is_none() && present.contains("generated_by") {
        violations.push(format!("{file} [规则 1]：`generated_by` 不是非空字符串"));
    }

    // --- 规则 8：recount ---------------------------------------------------
    let recount_commands = check_recount(file, map, violations, warnings);

    // --- entries -----------------------------------------------------------
    let entries = match map.get(serde_yaml::Value::from("entries")) {
        Some(serde_yaml::Value::Sequence(seq)) => seq.clone(),
        Some(other) => {
            violations.push(format!(
                "{file} [规则 1]：`entries` 必须是序列，实得 {}",
                type_name(other)
            ));
            return None;
        }
        None => return None,
    };

    let mut status_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut local_ids: BTreeMap<String, ()> = BTreeMap::new();
    let test_id_re = regex::Regex::new(r"^T-[A-Z]+-[0-9]{4}$").expect("test_id 正则是常量");
    let bare_line_re = regex::Regex::new(r":[0-9]+$").expect("裸行号正则是常量");

    for (index, entry) in entries.iter().enumerate() {
        let emap = match entry.as_mapping() {
            Some(m) => m,
            None => {
                violations.push(format!("{file} entry#{index} [规则 1]：entry 不是 mapping"));
                continue;
            }
        };

        // entry 的 id 先取出来做定位标签；取不到就用序号。
        let id = nonempty_str(emap, "id");
        let tag = match &id {
            Some(v) => format!("entry `{v}`"),
            None => format!("entry#{index}（id 缺失或为空）"),
        };

        // 规则 1：必填键非空 + 键集合封闭
        for key in ENTRY_REQUIRED_KEYS {
            if nonempty_str(emap, key).is_none() {
                violations.push(format!(
                    "{file} {tag} [规则 1]：`{key}` 缺失或不是非空字符串"
                ));
            }
        }
        for key in emap.keys() {
            let Some(k) = key.as_str() else {
                violations.push(format!("{file} {tag} [规则 1]：entry 里出现非字符串键"));
                continue;
            };
            if !ENTRY_REQUIRED_KEYS.contains(&k) && !ENTRY_OPTIONAL_KEYS.contains(&k) {
                violations.push(format!(
                    "{file} {tag} [规则 1]：出现未定义的 entry 键 `{k}`（允许集合 = {ENTRY_REQUIRED_KEYS:?} + {ENTRY_OPTIONAL_KEYS:?}）"
                ));
            }
        }

        // 规则 2：label
        if let Some(label) = nonempty_str(emap, "label")
            && !LABELS.contains(&label.as_str())
        {
            violations.push(format!(
                "{file} {tag} [规则 2]：label=`{label}` 不在 {LABELS:?} 内"
            ));
        }

        // 规则 3：owner
        if let Some(owner) = nonempty_str(emap, "owner")
            && !OWNERS.contains(&owner.as_str())
        {
            violations.push(format!(
                "{file} {tag} [规则 3]：owner=`{owner}` 不是 v3 §5.1 十个 crate 之一"
            ));
        }

        // 规则 4：status ⟺ done_evidence
        let status = nonempty_str(emap, "status");
        let done_evidence = nonempty_str(emap, "done_evidence");
        let has_done_key = emap.contains_key(serde_yaml::Value::from("done_evidence"));
        if let Some(s) = &status {
            if !STATUSES.contains(&s.as_str()) {
                violations.push(format!(
                    "{file} {tag} [规则 4]：status=`{s}` 不在 {STATUSES:?} 内（status 域不封闭则规则 4 的双向约束形同虚设）"
                ));
            }
            if s == "done" && done_evidence.is_none() {
                violations.push(format!(
                    "{file} {tag} [规则 4]：status=done 但 done_evidence 缺失或为空"
                ));
            }
            if s != "done" && has_done_key {
                violations.push(format!(
                    "{file} {tag} [规则 4]：status=`{s}` 却带了 done_evidence 键（非 done 时该键必须不出现）"
                ));
            }
            *status_counts.entry(s.clone()).or_insert(0) += 1;
        } else {
            *status_counts.entry("<缺失>".to_string()).or_insert(0) += 1;
        }

        // 规则 5：id 文件内唯一
        if let Some(v) = &id
            && local_ids.insert(v.clone(), ()).is_some()
        {
            violations.push(format!("{file} {tag} [规则 5]：id `{v}` 在本文件内重复"));
        }

        // 规则 5 + 6：test_id
        if let Some(test_id) = nonempty_str(emap, "test_id") {
            if !test_id_re.is_match(&test_id) {
                violations.push(format!(
                    "{file} {tag} [规则 6]：test_id=`{test_id}` 不匹配 ^T-[A-Z]+-[0-9]{{4}}$"
                ));
            }
            let owner_tag = format!(
                "{file}#{}",
                id.clone().unwrap_or_else(|| format!("#{index}"))
            );
            if let Some(first) = global_test_ids.insert(test_id.clone(), owner_tag.clone()) {
                violations.push(format!(
                    "{file} {tag} [规则 5]：test_id `{test_id}` 与 {first} 重复（test_id 必须全仓唯一）"
                ));
                // 保留首次出现者，让后续重复都指向同一个"首犯"。
                global_test_ids.insert(test_id, first);
            }
        }

        // 规则 7：upstream 禁裸行号
        if let Some(upstream) = nonempty_str(emap, "upstream")
            && bare_line_re.is_match(&upstream)
        {
            violations.push(format!(
                "{file} {tag} [规则 7]：upstream=`{upstream}` 以裸行号结尾；位置引用只用符号名（`path::symbol`）"
            ));
        }

        // 以下两条是告警，不在 8 条硬规则里，但 schema 正文写明了。
        if let Some(target) = nonempty_str(emap, "target")
            && (target.contains("TBD") || target.contains("未定"))
        {
            warnings.push(format!(
                "{file} {tag}：target=`{target}` 含 TBD/未定，schema 正文要求 target 是确定落点"
            ));
        }
        if let Some(rule) = nonempty_str(emap, "migration_rule") {
            let head = rule.split(':').next().unwrap_or("").trim();
            if !MIGRATION_RULE_PREFIXES.contains(&head) {
                warnings.push(format!(
                    "{file} {tag}：migration_rule=`{rule}` 的前缀不在 {MIGRATION_RULE_PREFIXES:?} 内"
                ));
            }
        }
    }

    Some(LedgerReport {
        file: file.to_string(),
        schema: if schema_name.is_empty() {
            "<缺失>".to_string()
        } else {
            schema_name
        },
        entries: entries.len(),
        status_counts,
        recount_commands,
    })
}

/// 规则 8。返回 recount 条数（用于报表）。
fn check_recount(
    file: &str,
    map: &serde_yaml::Mapping,
    violations: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> usize {
    let items = match map.get(serde_yaml::Value::from("recount")) {
        Some(serde_yaml::Value::Sequence(seq)) => seq.clone(),
        Some(other) => {
            violations.push(format!(
                "{file} [规则 8]：`recount` 必须是序列，实得 {}",
                type_name(other)
            ));
            return 0;
        }
        None => return 0,
    };

    if items.is_empty() {
        violations.push(format!("{file} [规则 8]：`recount` 至少要有一条"));
        return 0;
    }

    for (index, item) in items.iter().enumerate() {
        let imap = match item.as_mapping() {
            Some(m) => m,
            None => {
                violations.push(format!("{file} recount#{index} [规则 8]：不是 mapping"));
                continue;
            }
        };
        if nonempty_str(imap, "command").is_none() {
            violations.push(format!(
                "{file} recount#{index} [规则 8]：`command` 缺失或为空"
            ));
        }
        match nonempty_str(imap, "cwd") {
            Some(cwd) if RECOUNT_CWD.contains(&cwd.as_str()) => {}
            Some(cwd) => violations.push(format!(
                "{file} recount#{index} [规则 8]：cwd=`{cwd}` 不在 {RECOUNT_CWD:?} 内"
            )),
            None => violations.push(format!("{file} recount#{index} [规则 8]：`cwd` 缺失或为空")),
        }
        if !imap.contains_key(serde_yaml::Value::from("expect")) {
            violations.push(format!(
                "{file} recount#{index} [规则 8]：缺 `expect`（复算命令没有期望值就无法被重跑核对）"
            ));
        }
        for key in imap.keys() {
            let Some(k) = key.as_str() else { continue };
            if !RECOUNT_KEYS.contains(&k) {
                warnings.push(format!(
                    "{file} recount#{index}：出现未定义的键 `{k}`（允许集合 = {RECOUNT_KEYS:?}）"
                ));
            }
        }
    }

    items.len()
}

// ---------------------------------------------------------------------------
// ci
// ---------------------------------------------------------------------------

fn cmd_ci() -> Result<()> {
    let root = workspace_root()?;

    // 顺序 = v3 §16.3〈Supply chain〉固定清单的前三条，加上 v3 §19.3 的 parity 闸门。
    //
    // 后面几条（cargo deny / cargo audit / cargo vet / OSV / secret scan /
    // license-NOTICE-provenance / SBOM / 可复现构建 / 签名校验）刻意**不**在这里跑：
    // 它们各自需要一份本仓此刻还不存在的基线（`supply-chain/` 目录、SBOM 工具链、
    // Electron shim 产物、签名密钥），塞进来只会得到一条恒红的闸门 —— 恒红等于没有闸门。
    // 它们在 .github/workflows/ci.yml 里以独立 job 编排，并在那里写明解除条件。
    let steps: [(&str, &str, &[&str]); 3] = [
        (
            "cargo fmt --check",
            "cargo",
            &["fmt", "--all", "--", "--check"],
        ),
        (
            "cargo clippy --all-targets --all-features -D warnings",
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ],
        ),
        (
            "cargo test --locked",
            "cargo",
            &["test", "--workspace", "--all-features", "--locked"],
        ),
    ];

    for (index, (label, program, args)) in steps.iter().enumerate() {
        let step_no = index + 1;
        println!("\n=== xtask ci [{step_no}/4] {label} ===");
        let status = Command::new(program)
            .args(*args)
            .current_dir(&root)
            .status()
            .with_context(|| format!("启动 `{program} {}` 失败", args.join(" ")))?;
        if !status.success() {
            bail!(
                "第 {step_no} 步失败：{label}（退出码 {}）",
                status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "被信号终止".to_string())
            );
        }
    }

    println!("\n=== xtask ci [4/4] parity-check ===");
    // 进程内调用而不是再 spawn 一次 cargo：避免把刚跑完的 4 分钟编译再等一遍，
    // 也让 parity 违规的退出码来源唯一。
    cmd_parity_check(&[]).context("第 4 步失败：parity-check")?;

    println!("\nxtask ci: 4/4 全绿");
    Ok(())
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

/// 取一个必须是"非空字符串"的字段。空串、非字符串、缺键一律返回 `None`。
fn nonempty_str(map: &serde_yaml::Mapping, key: &str) -> Option<String> {
    map.get(serde_yaml::Value::from(key))
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn type_name(value: &serde_yaml::Value) -> &'static str {
    match value {
        serde_yaml::Value::Null => "null",
        serde_yaml::Value::Bool(_) => "bool",
        serde_yaml::Value::Number(_) => "number",
        serde_yaml::Value::String(_) => "string",
        serde_yaml::Value::Sequence(_) => "sequence",
        serde_yaml::Value::Mapping(_) => "mapping",
        serde_yaml::Value::Tagged(_) => "tagged",
    }
}

/// 相对仓根的路径，且统一成 `/` 分隔 —— Windows 与 Linux 上的报错文本必须逐字相同，
/// 否则 CI 日志和本机日志没法直接对比。
fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

// ---------------------------------------------------------------------------
// 自检：8 条规则各自的正向 + 负向对照
// ---------------------------------------------------------------------------
//
// 为什么必须成对写：一个"从不报错"的校验器和一个"没有违规"的仓库，输出完全一样。
// 只测"坏输入报红"证明不了它在好输入上不误报；只测"好输入放行"在"规则压根没实现"的
// 世界里同样成立。两边都钉住，这份闸门才有判别力。
#[cfg(test)]
mod tests {
    use super::*;

    /// 一条各字段都合法的 entry。各用例用 `replace` 只改**一个**字段来隔离被测规则。
    const VALID_ENTRY: &str = "  - id: get-thread\n    upstream: server/src/routes/thread.ts::getThread\n    label: parity\n    target: openbot-server::routes::thread::get\n    owner: openbot-server\n    test_id: T-API-0001\n    migration_rule: preserve\n    status: todo\n    evidence: rg -n getThread server/src/routes/thread.ts\n";

    /// 合法的 recount 段。刻意不含反斜杠：被测的是规则，不是转义。
    const VALID_RECOUNT: &str =
        "recount:\n  - command: rg -c app server/src/index.ts\n    cwd: upstream\n    expect: 95\n";

    fn doc(entries: &str) -> String {
        format!(
            "schema: api\nschema_version: 1\nupstream_commit: {EXPECTED_UPSTREAM_COMMIT}\ngenerated_by: manual\n{VALID_RECOUNT}entries:\n{entries}"
        )
    }

    /// 跑一遍校验，返回 (违规, 告警)。
    fn run(yaml: &str) -> (Vec<String>, Vec<String>) {
        let mut violations = Vec::new();
        let mut warnings = Vec::new();
        let mut global = BTreeMap::new();
        let parsed: serde_yaml::Value =
            serde_yaml::from_str(yaml).expect("测试用例 YAML 必须可解析");
        check_ledger(
            "parity/test.yaml",
            &parsed,
            &mut violations,
            &mut warnings,
            &mut global,
        );
        (violations, warnings)
    }

    /// 断言恰好命中某条规则，且没有别的规则被顺带点亮
    /// —— 否则"测规则 7"的用例可能其实是被规则 1 判红的，等于没测。
    fn assert_only_rule(violations: &[String], rule_tag: &str) {
        assert!(
            !violations.is_empty(),
            "期望命中 {rule_tag}，实际零违规 —— 规则没有生效"
        );
        for v in violations {
            assert!(
                v.contains(rule_tag),
                "期望只命中 {rule_tag}，却出现了别的违规：{v}"
            );
        }
    }

    // --- 全局正向对照 ------------------------------------------------------

    #[test]
    fn valid_ledger_has_zero_violations() {
        let (violations, warnings) = run(&doc(VALID_ENTRY));
        assert!(
            violations.is_empty(),
            "合法 ledger 不该有违规：{violations:?}"
        );
        assert!(warnings.is_empty(), "合法 ledger 不该有告警：{warnings:?}");
    }

    // --- 规则 1 ------------------------------------------------------------

    #[test]
    fn rule1_missing_required_entry_key() {
        let entry = VALID_ENTRY.replace(
            "    evidence: rg -n getThread server/src/routes/thread.ts\n",
            "",
        );
        assert_only_rule(&run(&doc(&entry)).0, "[规则 1]");
    }

    #[test]
    fn rule1_empty_required_entry_value() {
        // 空串与"只有空白"都必须判红：`target: "   "` 在 YAML 里是完全合法的字符串。
        let entry = VALID_ENTRY.replace(
            "    target: openbot-server::routes::thread::get\n",
            "    target: \"   \"\n",
        );
        assert_only_rule(&run(&doc(&entry)).0, "[规则 1]");
    }

    #[test]
    fn rule1_unknown_top_level_key_is_rejected() {
        let yaml = format!("{}extra_key: 随手加的\n", doc(VALID_ENTRY));
        assert_only_rule(&run(&yaml).0, "[规则 1]");
    }

    #[test]
    fn rule1_unknown_entry_key_is_rejected() {
        let entry = format!("{VALID_ENTRY}    owner_note: 随手加的\n");
        assert_only_rule(&run(&doc(&entry)).0, "[规则 1]");
    }

    #[test]
    fn rule1_missing_top_level_key() {
        let yaml = doc(VALID_ENTRY).replace("generated_by: manual\n", "");
        assert_only_rule(&run(&yaml).0, "[规则 1]");
    }

    #[test]
    fn rule1_wrong_schema_version() {
        let yaml = doc(VALID_ENTRY).replace("schema_version: 1\n", "schema_version: 2\n");
        assert_only_rule(&run(&yaml).0, "[规则 1]");
    }

    // --- 规则 2 ------------------------------------------------------------

    #[test]
    fn rule2_rejects_label_outside_three_values() {
        for bad in ["Parity", "new", "新增功能", "替换"] {
            let entry = VALID_ENTRY.replace("    label: parity\n", &format!("    label: {bad}\n"));
            assert_only_rule(&run(&doc(&entry)).0, "[规则 2]");
        }
    }

    #[test]
    fn rule2_accepts_all_three_values() {
        for good in LABELS {
            let entry = VALID_ENTRY.replace("    label: parity\n", &format!("    label: {good}\n"));
            let (violations, _) = run(&doc(&entry));
            assert!(
                violations.is_empty(),
                "label={good} 应当合法：{violations:?}"
            );
        }
    }

    // --- 规则 3 ------------------------------------------------------------

    #[test]
    fn rule3_rejects_owner_outside_ten_crates() {
        // `openbot-core` 不在 v3 §5.1 的十个 crate 里；建第 11 个 crate 要过四条理由的评审。
        let entry = VALID_ENTRY.replace("    owner: openbot-server\n", "    owner: openbot-core\n");
        assert_only_rule(&run(&doc(&entry)).0, "[规则 3]");
    }

    #[test]
    fn rule3_accepts_all_ten_crates() {
        for owner in OWNERS {
            let entry = VALID_ENTRY.replace(
                "    owner: openbot-server\n",
                &format!("    owner: {owner}\n"),
            );
            let (violations, _) = run(&doc(&entry));
            assert!(
                violations.is_empty(),
                "owner={owner} 应当合法：{violations:?}"
            );
        }
    }

    // --- 规则 4 ------------------------------------------------------------

    #[test]
    fn rule4_done_without_done_evidence() {
        let entry = VALID_ENTRY.replace("    status: todo\n", "    status: done\n");
        assert_only_rule(&run(&doc(&entry)).0, "[规则 4]");
    }

    #[test]
    fn rule4_done_with_empty_done_evidence() {
        let entry = format!(
            "{}    done_evidence: \"\"\n",
            VALID_ENTRY.replace("    status: todo\n", "    status: done\n")
        );
        assert_only_rule(&run(&doc(&entry)).0, "[规则 4]");
    }

    #[test]
    fn rule4_done_with_evidence_is_accepted() {
        let entry = format!(
            "{}    done_evidence: cargo test -p openbot-server thread_get\n",
            VALID_ENTRY.replace("    status: todo\n", "    status: done\n")
        );
        let (violations, _) = run(&doc(&entry));
        assert!(violations.is_empty(), "done + 证据应当合法：{violations:?}");
    }

    #[test]
    fn rule4_non_done_must_not_carry_done_evidence() {
        // 双向约束的另一半：todo 却带着 done_evidence，说明有人把 status 改回去却没删证据，
        // 下一次误读就会当成"已完成"。
        let entry = format!("{VALID_ENTRY}    done_evidence: 早先跑过\n");
        assert_only_rule(&run(&doc(&entry)).0, "[规则 4]");
    }

    #[test]
    fn rule4_rejects_status_outside_three_values() {
        let entry = VALID_ENTRY.replace("    status: todo\n", "    status: blocked\n");
        assert_only_rule(&run(&doc(&entry)).0, "[规则 4]");
    }

    // --- 规则 5 ------------------------------------------------------------

    #[test]
    fn rule5_duplicate_id_within_file() {
        let entries = format!(
            "{VALID_ENTRY}{}",
            VALID_ENTRY.replace("    test_id: T-API-0001\n", "    test_id: T-API-0002\n")
        );
        assert_only_rule(&run(&doc(&entries)).0, "[规则 5]");
    }

    #[test]
    fn rule5_duplicate_test_id_across_ledgers() {
        // 跨文件那一半：同一个 global map 连续喂两个文件。
        let mut violations = Vec::new();
        let mut warnings = Vec::new();
        let mut global = BTreeMap::new();
        for file in ["parity/api.yaml", "parity/routes.yaml"] {
            let parsed: serde_yaml::Value =
                serde_yaml::from_str(&doc(VALID_ENTRY)).expect("测试用例 YAML 必须可解析");
            check_ledger(file, &parsed, &mut violations, &mut warnings, &mut global);
        }
        assert_only_rule(&violations, "[规则 5]");
        assert!(
            violations[0].contains("parity/api.yaml"),
            "重复报告应指回首次出现的文件：{violations:?}"
        );
    }

    #[test]
    fn rule5_distinct_test_ids_across_ledgers_are_fine() {
        let mut violations = Vec::new();
        let mut warnings = Vec::new();
        let mut global = BTreeMap::new();
        for (file, test_id) in [
            ("parity/api.yaml", "T-API-0001"),
            ("parity/routes.yaml", "T-ROUTE-0001"),
        ] {
            let entry = VALID_ENTRY.replace(
                "    test_id: T-API-0001\n",
                &format!("    test_id: {test_id}\n"),
            );
            let parsed: serde_yaml::Value =
                serde_yaml::from_str(&doc(&entry)).expect("测试用例 YAML 必须可解析");
            check_ledger(file, &parsed, &mut violations, &mut warnings, &mut global);
        }
        assert!(
            violations.is_empty(),
            "不同 test_id 不该冲突：{violations:?}"
        );
    }

    // --- 规则 6 ------------------------------------------------------------

    #[test]
    fn rule6_rejects_malformed_test_id() {
        for bad in [
            "T-api-0001",  // 小写
            "T-API-001",   // 三位数字
            "T-API-00001", // 五位数字
            "T_API_0001",  // 下划线
            "API-0001",    // 缺前缀
            "T-API-0001x", // 尾巴
        ] {
            let entry = VALID_ENTRY.replace(
                "    test_id: T-API-0001\n",
                &format!("    test_id: {bad}\n"),
            );
            assert_only_rule(&run(&doc(&entry)).0, "[规则 6]");
        }
    }

    #[test]
    fn rule6_accepts_wellformed_test_id() {
        for good in ["T-API-0001", "T-BROWSEROPS-9999", "T-UI-0042"] {
            let entry = VALID_ENTRY.replace(
                "    test_id: T-API-0001\n",
                &format!("    test_id: {good}\n"),
            );
            let (violations, _) = run(&doc(&entry));
            assert!(
                violations.is_empty(),
                "test_id={good} 应当合法：{violations:?}"
            );
        }
    }

    // --- 规则 7 ------------------------------------------------------------

    #[test]
    fn rule7_rejects_bare_line_number_in_upstream() {
        let entry = VALID_ENTRY.replace(
            "    upstream: server/src/routes/thread.ts::getThread\n",
            "    upstream: server/src/routes/thread.ts:142\n",
        );
        assert_only_rule(&run(&doc(&entry)).0, "[规则 7]");
    }

    #[test]
    fn rule7_accepts_symbol_reference_and_dash() {
        // 正向对照含三种合法形态：`::符号名`、纯路径、无上游对应物的 `-`。
        // 顺带钉住"`::getThread` 里的双冒号不能被误判成行号"。
        for good in [
            "server/src/routes/thread.ts::getThread",
            "server/src/routes/thread.ts",
            "-",
            "app/routes/_index.tsx::Component",
        ] {
            let entry = VALID_ENTRY.replace(
                "    upstream: server/src/routes/thread.ts::getThread\n",
                &format!("    upstream: \"{good}\"\n"),
            );
            let (violations, _) = run(&doc(&entry));
            assert!(
                violations.is_empty(),
                "upstream={good} 应当合法：{violations:?}"
            );
        }
    }

    // --- 规则 8 ------------------------------------------------------------

    #[test]
    fn rule8_rejects_empty_recount_list() {
        let yaml = doc(VALID_ENTRY).replace(VALID_RECOUNT, "recount: []\n");
        assert_only_rule(&run(&yaml).0, "[规则 8]");
    }

    #[test]
    fn rule8_rejects_empty_command() {
        let yaml = doc(VALID_ENTRY).replace(
            "  - command: rg -c app server/src/index.ts\n",
            "  - command: \"\"\n",
        );
        assert_only_rule(&run(&yaml).0, "[规则 8]");
    }

    #[test]
    fn rule8_rejects_unknown_cwd() {
        let yaml = doc(VALID_ENTRY).replace("    cwd: upstream\n", "    cwd: /tmp\n");
        assert_only_rule(&run(&yaml).0, "[规则 8]");
    }

    #[test]
    fn rule8_rejects_missing_expect() {
        let yaml = doc(VALID_ENTRY).replace("    expect: 95\n", "");
        assert_only_rule(&run(&yaml).0, "[规则 8]");
    }

    #[test]
    fn rule8_accepts_string_expect_and_both_cwds() {
        for cwd in RECOUNT_CWD {
            let yaml = doc(VALID_ENTRY)
                .replace("    cwd: upstream\n", &format!("    cwd: {cwd}\n"))
                .replace("    expect: 95\n", "    expect: \"95 = 91 + 4\"\n");
            let (violations, _) = run(&yaml);
            assert!(
                violations.is_empty(),
                "cwd={cwd} + 字符串 expect 应当合法：{violations:?}"
            );
        }
    }

    // --- 告警（不影响退出码，但必须真的会响）-------------------------------

    #[test]
    fn warns_on_upstream_commit_drift() {
        let yaml = doc(VALID_ENTRY).replace(
            EXPECTED_UPSTREAM_COMMIT,
            "0000000000000000000000000000000000000000",
        );
        let (violations, warnings) = run(&yaml);
        assert!(
            violations.is_empty(),
            "commit 漂移是告警不是违规：{violations:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("upstream_commit")),
            "commit 漂移必须出告警：{warnings:?}"
        );
    }

    #[test]
    fn warns_on_tbd_target() {
        let entry = VALID_ENTRY.replace(
            "    target: openbot-server::routes::thread::get\n",
            "    target: TBD\n",
        );
        let (violations, warnings) = run(&doc(&entry));
        assert!(violations.is_empty(), "TBD 是告警不是违规：{violations:?}");
        assert!(
            warnings.iter().any(|w| w.contains("TBD")),
            "target=TBD 必须出告警：{warnings:?}"
        );
    }

    #[test]
    fn warns_on_unknown_migration_rule_prefix() {
        let entry = VALID_ENTRY.replace(
            "    migration_rule: preserve\n",
            "    migration_rule: 保留\n",
        );
        let (violations, warnings) = run(&doc(&entry));
        assert!(
            violations.is_empty(),
            "migration_rule 前缀是告警不是违规：{violations:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("migration_rule")),
            "未知 migration_rule 前缀必须出告警：{warnings:?}"
        );
    }

    #[test]
    fn accepts_migration_rule_with_colon_suffix() {
        let entry = VALID_ENTRY.replace(
            "    migration_rule: preserve\n",
            "    migration_rule: \"rename: OPENBOT_ 前缀统一\"\n",
        );
        let (violations, warnings) = run(&doc(&entry));
        assert!(violations.is_empty(), "带冒号后缀应当合法：{violations:?}");
        assert!(warnings.is_empty(), "带冒号后缀不该告警：{warnings:?}");
    }

    // --- 常量自洽 ----------------------------------------------------------

    #[test]
    fn owners_match_workspace_members() {
        // OWNERS 与 workspace members 是同一份事实的两个副本；这条把它们钉在一起，
        // 免得有人加了 crate 却忘了 ledger 的 owner 域（或反过来）。
        let manifest = include_str!("../../../../Cargo.toml");
        for owner in OWNERS {
            assert!(
                manifest.contains(&format!("\"crates/{owner}\"")),
                "OWNERS 里的 {owner} 不在 workspace members 里"
            );
        }
        let member_count = manifest
            .lines()
            .filter(|l| l.trim_start().starts_with("\"crates/openbot-"))
            .count();
        assert_eq!(
            member_count,
            OWNERS.len(),
            "workspace members 数量与 OWNERS 不等 —— v3 §5.1 是十个 crate"
        );
    }
}
