//! Tool invocation 的跨 transport DTO；授权结论、metadata 与 capability 不在这里。

use core::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{BotId, RunId, ToolCallId};

/// Agent 交给唯一 application 入口的一次工具调用。
///
/// `call_id` / `call_seq` 由 Rust Agent gateway 铸造；`run_id` / `bot_id` 仍须由 application
/// 的权威 scope resolver 复核。参数只是一份不可信 JSON 对象，不能携带 actor、policy、effect、
/// approval 或 target；这些字段刻意不存在，renderer/model 因而没有自报它们的入口。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolInvocation {
    /// Rust 铸造的本次调用 ID。
    pub call_id: ToolCallId,
    /// 声称所属 run；application 必须与权威 scope 逐字比对。
    pub run_id: RunId,
    /// 声称代表的 Bot；application 必须与权威 scope 逐字比对。
    pub bot_id: BotId,
    /// run 内严格递增的调用序号；由 Rust Agent gateway 铸造。
    pub call_seq: u64,
    /// catalog key。application 只拿它查权威 metadata，不采信任何外部 effect 声明。
    pub tool_name: String,
    /// 不可信参数；application 会要求顶层为 object 并走 schema/size/canonical hash。
    pub arguments: Value,
}

impl fmt::Debug for ToolInvocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolInvocation")
            .field("call_id", &self.call_id)
            .field("run_id", &self.run_id)
            .field("bot_id", &self.bot_id)
            .field("call_seq", &self.call_seq)
            .field("tool_name", &self.tool_name)
            .field("arguments", &"<redacted>")
            .finish()
    }
}

/// 已持久化 outcome 的线上提交状态。`unknown` 不可能成为成功 reply，而是 AppError 的
/// reconciliation 分支。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCommitState {
    /// 副作用已提交。
    Committed,
    /// 确知未提交。
    NotCommitted,
}

/// 给 Agent/model 的脱敏工具结果。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolResult {
    /// 对应调用。
    pub call_id: ToolCallId,
    /// 已经过 control plane 脱敏、再由 application 按 metadata 上限裁剪的 UTF-8 文本。
    pub content: String,
    /// 稳定错误码；`None` 表示工具成功。
    pub error_code: Option<String>,
    /// outcome 的确定提交状态。
    pub commit_state: ToolCommitState,
    /// 实际交给模型的 UTF-8 字节数。
    pub visible_bytes: u32,
    /// 完整脱敏输出是否更长。
    pub truncated: bool,
}

impl fmt::Debug for ToolResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolResult")
            .field("call_id", &self.call_id)
            .field("content", &"<redacted>")
            .field("error_code", &self.error_code)
            .field("commit_state", &self.commit_state)
            .field("visible_bytes", &self.visible_bytes)
            .field("truncated", &self.truncated)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn invocation_wire_is_closed_and_debug_redacts_arguments() {
        let invocation = ToolInvocation {
            call_id: ToolCallId::new("call-1"),
            run_id: RunId::new("run-1"),
            bot_id: BotId::new("bot-1"),
            call_seq: 3,
            tool_name: "computer.click".to_owned(),
            arguments: json!({"secret":"SENTINEL-DO-NOT-LOG"}),
        };
        let rendered = format!("{invocation:?}");
        assert!(!rendered.contains("SENTINEL-DO-NOT-LOG"));
        assert!(rendered.contains("<redacted>"));

        let wire = serde_json::to_string(&invocation).unwrap();
        assert!(wire.contains(r#""callId":"call-1""#));
        assert!(wire.contains(r#""arguments":{"secret":"SENTINEL-DO-NOT-LOG"}"#));
        assert_eq!(
            serde_json::from_str::<ToolInvocation>(&wire).unwrap(),
            invocation
        );
        assert!(
            serde_json::from_str::<ToolInvocation>(
                r#"{"callId":"c","runId":"r","botId":"b","callSeq":0,"toolName":"t","arguments":{},"actor":"admin"}"#,
            )
            .is_err(),
            "外部自报 actor 必须被 deny_unknown_fields 拒绝",
        );
    }

    #[test]
    fn tool_result_debug_never_prints_model_visible_content() {
        let result = ToolResult {
            call_id: ToolCallId::new("call-1"),
            content: "SENTINEL-MODEL-OUTPUT".to_owned(),
            error_code: None,
            commit_state: ToolCommitState::Committed,
            visible_bytes: 21,
            truncated: false,
        };
        let rendered = format!("{result:?}");
        assert!(!rendered.contains("SENTINEL-MODEL-OUTPUT"));
        assert!(rendered.contains("<redacted>"));
    }
}
