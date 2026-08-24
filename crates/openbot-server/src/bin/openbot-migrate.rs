//! OpenBot PostgreSQL/cutover migration binary；当前交付环境语义预检子命令。

use std::process::ExitCode;

use openbot_server::config::{env_map_from_process, preflight_audit_retention};

const ACTION_REQUIRED_EXIT: u8 = 2;
const USAGE_EXIT: u8 = 64;
const SOFTWARE_EXIT: u8 = 70;

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.len() != 1 || arguments[0] != "preflight-audit-retention" {
        eprintln!(r#"{{"code":"migration_preflight_usage"}}"#);
        return ExitCode::from(USAGE_EXIT);
    }

    let report = preflight_audit_retention(&env_map_from_process());
    let rendered = match serde_json::to_string_pretty(&report) {
        Ok(rendered) => rendered,
        Err(_) => {
            eprintln!(r#"{{"code":"migration_preflight_serialization_failed"}}"#);
            return ExitCode::from(SOFTWARE_EXIT);
        }
    };
    println!("{rendered}");
    if report.requires_action() {
        ExitCode::from(ACTION_REQUIRED_EXIT)
    } else {
        ExitCode::SUCCESS
    }
}
