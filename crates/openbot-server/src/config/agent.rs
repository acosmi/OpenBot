//! Managed built-in Agent provider/budget environment parsing（v3 §7.2 / §7.3）。

use std::time::Duration;

use crate::config::address::DeploymentAddress;
use crate::config::env::{self, EnvMap};
use crate::config::error::{ConfigProblem, Expectation};
use crate::config::secret::Secret;

/// Provider selection；三家取值逐字来自固定上游 agent-langgraph。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedProviderKind {
    /// OpenAI-compatible。
    OpenAi,
    /// Anthropic。
    Anthropic,
    /// Google Generative AI。
    Google,
}

/// Provider config；secret Debug 由 `Secret` 自动脱敏。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedProviderConfig {
    /// Provider。
    pub provider: ManagedProviderKind,
    /// Model sent verbatim after TrimString-style outer whitespace removal。
    pub model: String,
    /// Selected provider API key。
    pub api_key: Secret,
    /// Provider base URL。
    pub base_url: DeploymentAddress,
    /// OpenAI only；exact `BOT_RESPONSES_API=true`。
    pub use_responses_api: bool,
    /// New exact numeric CIDR allowlist for private/self-hosted provider endpoints。
    pub egress_allow_cidrs: Vec<String>,
    /// New explicit HTTP override；still requires CIDR/global destination policy。
    pub allow_http: bool,
}

/// Agent runtime budgets independent of whether a key is currently configured。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentBudgets {
    /// `None` = AGENT_STALL_TIMEOUT_MS unset/0。
    pub stall_timeout: Option<Duration>,
    /// `None` = OPENBOT_RUN_DEADLINE_MS=0；unset defaults 30min。
    pub run_deadline: Option<Duration>,
}

/// Package-declared built-in Bots 的 OpenAI transport/fallback config。
///
/// Model 与 credential key id 不来自环境；它们分别只来自已验证的 `model.yaml`。环境 key
/// 只是 PostgreSQL 中不存在 active matching model credential 时的上游兼容 fallback。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageOpenAiProviderConfig {
    /// OpenAI-compatible base URL。
    pub base_url: DeploymentAddress,
    /// Optional environment fallback；stored active credential 始终优先且每 run 重读。
    pub environment_api_key: Option<Secret>,
    /// Exact numeric CIDR allowlist for private/self-hosted endpoints。
    pub egress_allow_cidrs: Vec<String>,
    /// Explicit HTTP override；仍需 CIDR/global destination policy。
    pub allow_http: bool,
}

/// Parse budgets and optional managed provider; appends all problems, never reads process env。
pub fn parse_agent_config(
    env_map: &EnvMap,
    problems: &mut Vec<ConfigProblem>,
) -> (
    AgentBudgets,
    PackageOpenAiProviderConfig,
    Option<ManagedProviderConfig>,
) {
    let stall_timeout = parse_milliseconds(env_map, "AGENT_STALL_TIMEOUT_MS", None, problems);
    let run_deadline = parse_milliseconds(
        env_map,
        "OPENBOT_RUN_DEADLINE_MS",
        Some(1_800_000),
        problems,
    );
    let budgets = AgentBudgets {
        stall_timeout,
        run_deadline,
    };
    let package_openai = parse_package_openai(env_map, problems);
    let managed_requested = [
        "BOT_PROVIDER",
        "BOT_MODEL",
        "BOT_RESPONSES_API",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "GOOGLE_API_KEY",
        "GOOGLE_GENERATIVE_AI_BASE_URL",
    ]
    .into_iter()
    .any(|name| env::optional(env_map, name).is_some());
    if !managed_requested {
        return (budgets, package_openai, None);
    }
    let provider = match env::optional(env_map, "BOT_PROVIDER").unwrap_or("openai") {
        "openai" => ManagedProviderKind::OpenAi,
        "anthropic" => ManagedProviderKind::Anthropic,
        "google" => ManagedProviderKind::Google,
        _ => {
            problems.push(ConfigProblem::Malformed {
                variable: "BOT_PROVIDER",
                expectation: Expectation::ProviderName,
            });
            ManagedProviderKind::OpenAi
        }
    };
    let default_model = match provider {
        ManagedProviderKind::OpenAi => "gpt-5.5",
        ManagedProviderKind::Anthropic => "claude-sonnet-4-5",
        ManagedProviderKind::Google => "gemini-2.5-flash",
    };
    let model = env::optional(env_map, "BOT_MODEL")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_model)
        .to_owned();
    let (key_name, base_name, default_base) = match provider {
        ManagedProviderKind::OpenAi => ("OPENAI_API_KEY", "OPENAI_BASE_URL", None),
        ManagedProviderKind::Anthropic => (
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_BASE_URL",
            Some("https://api.anthropic.com"),
        ),
        ManagedProviderKind::Google => (
            "GOOGLE_API_KEY",
            "GOOGLE_GENERATIVE_AI_BASE_URL",
            Some("https://generativelanguage.googleapis.com"),
        ),
    };
    let key = if provider == ManagedProviderKind::OpenAi {
        package_openai.environment_api_key.clone()
    } else {
        env::optional(env_map, key_name)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Secret::new)
    };
    if key.is_none() {
        problems.push(ConfigProblem::Malformed {
            variable: key_name,
            expectation: Expectation::NonEmptySecret,
        });
    }
    let base_url = if provider == ManagedProviderKind::OpenAi {
        package_openai.base_url.clone()
    } else {
        parse_provider_base(
            env_map,
            base_name,
            default_base.expect("non-OpenAI default base"),
            problems,
        )
    };
    (
        budgets,
        package_openai.clone(),
        key.map(|api_key| ManagedProviderConfig {
            provider,
            model,
            api_key,
            base_url,
            use_responses_api: provider == ManagedProviderKind::OpenAi
                && env::optional(env_map, "BOT_RESPONSES_API") == Some("true"),
            egress_allow_cidrs: package_openai.egress_allow_cidrs.clone(),
            allow_http: package_openai.allow_http,
        }),
    )
}

fn parse_package_openai(
    env_map: &EnvMap,
    problems: &mut Vec<ConfigProblem>,
) -> PackageOpenAiProviderConfig {
    PackageOpenAiProviderConfig {
        base_url: parse_provider_base(
            env_map,
            "OPENAI_BASE_URL",
            "https://api.openai.com/v1",
            problems,
        ),
        environment_api_key: env::optional(env_map, "OPENAI_API_KEY")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Secret::new),
        egress_allow_cidrs: env::comma_separated(env_map, "OPENBOT_PROVIDER_EGRESS_ALLOW_CIDRS"),
        allow_http: env::optional(env_map, "OPENBOT_PROVIDER_ALLOW_HTTP") == Some("true"),
    }
}

fn parse_provider_base(
    env_map: &EnvMap,
    variable: &'static str,
    default: &'static str,
    problems: &mut Vec<ConfigProblem>,
) -> DeploymentAddress {
    let raw = env::optional(env_map, variable)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default);
    match DeploymentAddress::parse(raw) {
        Ok(value) => value,
        Err(_) => {
            problems.push(ConfigProblem::Malformed {
                variable,
                expectation: Expectation::AbsoluteHttpUrl,
            });
            DeploymentAddress::parse(default).expect("static provider URL")
        }
    }
}

fn parse_milliseconds(
    env_map: &EnvMap,
    variable: &'static str,
    default: Option<u64>,
    problems: &mut Vec<ConfigProblem>,
) -> Option<Duration> {
    let raw = env::optional(env_map, variable);
    let value = match raw {
        None => default,
        Some(raw) => match raw.parse::<u64>() {
            Ok(value) => Some(value),
            Err(_) => {
                problems.push(ConfigProblem::Malformed {
                    variable,
                    expectation: Expectation::WholeMillisecondsOrZero,
                });
                default
            }
        },
    };
    value.and_then(|value| (value != 0).then(|| Duration::from_millis(value)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn openai_defaults_and_exact_responses_flag_match_fixed_upstream() {
        let env = BTreeMap::from([
            ("OPENAI_API_KEY".to_owned(), " key ".to_owned()),
            ("BOT_PROVIDER".to_owned(), "openai".to_owned()),
            ("BOT_RESPONSES_API".to_owned(), "TRUE".to_owned()),
        ]);
        let mut problems = Vec::new();
        let (budgets, package, provider) = parse_agent_config(&env, &mut problems);
        assert!(problems.is_empty());
        assert_eq!(budgets.stall_timeout, None);
        assert_eq!(budgets.run_deadline, Some(Duration::from_millis(1_800_000)));
        let provider = provider.unwrap();
        assert_eq!(
            package.environment_api_key.as_ref().unwrap().expose(),
            "key"
        );
        assert_eq!(provider.provider, ManagedProviderKind::OpenAi);
        assert_eq!(provider.model, "gpt-5.5");
        assert!(!provider.use_responses_api);
        assert!(provider.egress_allow_cidrs.is_empty());
        assert!(!provider.allow_http);
        assert_eq!(provider.api_key.expose(), "key");
        assert_eq!(provider.base_url.as_str(), "https://api.openai.com/v1");
    }

    #[test]
    fn zero_disables_budgets_and_bad_values_are_all_reported() {
        let env = BTreeMap::from([
            ("AGENT_STALL_TIMEOUT_MS".to_owned(), "0".to_owned()),
            ("OPENBOT_RUN_DEADLINE_MS".to_owned(), "0".to_owned()),
            ("BOT_PROVIDER".to_owned(), "OPENAI".to_owned()),
            ("OPENAI_BASE_URL".to_owned(), "file:///tmp/model".to_owned()),
        ]);
        let mut problems = Vec::new();
        let (budgets, _, provider) = parse_agent_config(&env, &mut problems);
        assert_eq!(budgets.stall_timeout, None);
        assert_eq!(budgets.run_deadline, None);
        assert!(provider.is_none());
        assert_eq!(problems.len(), 3);
    }

    #[test]
    fn package_base_url_does_not_require_an_environment_key_because_vault_is_authoritative() {
        let env = BTreeMap::from([(
            "OPENAI_BASE_URL".to_owned(),
            "https://gateway.example/v1".to_owned(),
        )]);
        let mut problems = Vec::new();
        let (_, package, managed) = parse_agent_config(&env, &mut problems);
        assert!(problems.is_empty());
        assert!(managed.is_none());
        assert!(package.environment_api_key.is_none());
        assert_eq!(package.base_url.as_str(), "https://gateway.example/v1");
    }

    #[test]
    fn private_provider_transport_overrides_are_explicit_and_http_is_exact_true_only() {
        let env = BTreeMap::from([
            (
                "OPENBOT_PROVIDER_EGRESS_ALLOW_CIDRS".to_owned(),
                "10.42.0.0/16,127.0.0.1/32".to_owned(),
            ),
            ("OPENBOT_PROVIDER_ALLOW_HTTP".to_owned(), "TRUE".to_owned()),
        ]);
        let mut problems = Vec::new();
        let (_, package, managed) = parse_agent_config(&env, &mut problems);
        assert!(problems.is_empty());
        assert!(managed.is_none());
        assert_eq!(package.egress_allow_cidrs, ["10.42.0.0/16", "127.0.0.1/32"]);
        assert!(!package.allow_http);

        let mut exact = env;
        exact.insert("OPENBOT_PROVIDER_ALLOW_HTTP".to_owned(), "true".to_owned());
        let (_, package, _) = parse_agent_config(&exact, &mut Vec::new());
        assert!(package.allow_http);
    }
}
