//! Action policy 的跨 transport DTO；不含编译后的 CEL 或内部内容版本。

use serde::{Deserialize, Serialize};

/// Policy 执行档位的 wire 值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionPolicyMode {
    /// 真正拦截拒绝。
    #[default]
    Enforce,
    /// 记录拒绝但继续执行。
    DryRun,
}

/// 管理员读写的原始 policy 文档。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionPolicyDocument {
    /// enforce / dry-run；不得缺省。
    pub mode: ActionPolicyMode,
    /// deny 表达式，保持顺序与原文。
    #[serde(default)]
    pub deny: Vec<String>,
    /// allow 表达式，保持顺序与原文；空表 = default-deny。
    #[serde(default)]
    pub allow: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_document_wire_values_are_exact_and_closed() {
        let policy = ActionPolicyDocument {
            mode: ActionPolicyMode::DryRun,
            deny: vec!["false".to_owned()],
            allow: vec!["true".to_owned()],
        };
        let json = serde_json::to_string(&policy).unwrap();
        assert_eq!(
            json,
            r#"{"mode":"dry-run","deny":["false"],"allow":["true"]}"#
        );
        assert_eq!(
            serde_json::from_str::<ActionPolicyDocument>(&json).unwrap(),
            policy
        );
        assert!(serde_json::from_str::<ActionPolicyDocument>(r#"{"mode":"advisory"}"#).is_err());
        assert!(serde_json::from_str::<ActionPolicyDocument>(r#"{"deny":[]}"#).is_err());
    }
}
