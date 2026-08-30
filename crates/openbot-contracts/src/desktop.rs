//! Desktop structured-event 的 native/WASM 共用 closed wire（v4 §13.2–§13.4 / R146–R147）。
//!
//! 本模块只定义跨 IPC 的数据形状，不调用 Tauri、不做 I/O，也不承载 window authority。
//! renderer 可以选择封闭的 [`SubscriptionRequest`] 与 durable cursor，但 wire 中没有 actor、
//! tenant、role、auth generation 或内部 broker label。

use serde::{Deserialize, Serialize};

use crate::command::{AppEvent, SubscriptionRequest};
use crate::ids::ThreadId;

/// Tauri invoke 中打开 structured stream 的固定命令名。
pub const DESKTOP_STRUCTURED_OPEN_COMMAND: &str = "openbot_structured_events_open";

/// Tauri invoke 中关闭本 window 某条 structured stream 的固定命令名。
pub const DESKTOP_STRUCTURED_CLOSE_COMMAND: &str = "openbot_structured_events_close";

/// JavaScript 可逐整数精确表示的 subscription counter 排他上界（`2^53 - 1`）。
///
/// host 只铸造小于本值的 ID；到界后 fail-closed，不能让 JSON Number round-trip 改写身份。
pub const DESKTOP_STRUCTURED_SUBSCRIPTION_ID_EXCLUSIVE_LIMIT: u64 = 9_007_199_254_740_991;

/// 每帧携带的封闭 stream 家族。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopStructuredStreamKind {
    /// 进程 heartbeat / presence。
    Health,
    /// 单条 native thread 的 durable replay/live stream。
    ThreadEvents,
    /// 当前 actor 的 channel roster invalidation stream。
    ChannelActivity,
    /// 当前 actor 的 approval invalidation stream。
    ToolApprovalActivity,
}

impl DesktopStructuredStreamKind {
    /// 从封闭订阅请求得到唯一 stream 家族。
    #[must_use]
    pub const fn from_request(request: &SubscriptionRequest) -> Self {
        match request {
            SubscriptionRequest::Health => Self::Health,
            SubscriptionRequest::ThreadEvents { .. } => Self::ThreadEvents,
            SubscriptionRequest::ChannelActivity => Self::ChannelActivity,
            SubscriptionRequest::ToolApprovalActivity => Self::ToolApprovalActivity,
        }
    }

    /// 检查 application event 是否属于本 stream；thread event 还必须命中 exact thread。
    #[must_use]
    pub fn accepts_event(self, event: &AppEvent, expected_thread: Option<&ThreadId>) -> bool {
        match (self, event) {
            (Self::Health, AppEvent::Heartbeat { .. })
            | (Self::ThreadEvents, AppEvent::ThreadStreamError { .. })
            | (Self::ChannelActivity, AppEvent::ChannelActivity(_))
            | (Self::ChannelActivity, AppEvent::ChannelStreamError { .. })
            | (Self::ToolApprovalActivity, AppEvent::ToolApprovalActivity(_))
            | (Self::ToolApprovalActivity, AppEvent::ToolApprovalStreamError { .. }) => true,
            (Self::ThreadEvents, AppEvent::ThreadRunEvent(event)) => {
                expected_thread == Some(&event.thread_id)
            }
            (Self::Health, _)
            | (Self::ThreadEvents, _)
            | (Self::ChannelActivity, _)
            | (Self::ToolApprovalActivity, _) => false,
        }
    }
}

/// 已知 sequence range 永远不会到达的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopStructuredGapCause {
    /// newer latest-value frame 取代旧帧。
    Superseded,
    /// non-sheddable frame 未能送达。
    Dropped,
}

/// 可序列化的闭区间 sequence gap。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopStructuredSequenceGap {
    /// 第一个缺失 sequence（含）。
    pub from_sequence: u64,
    /// 最后一个缺失 sequence（含）。
    pub through_sequence: u64,
    /// 稳定低基数原因。
    pub cause: DesktopStructuredGapCause,
}

/// structured subscription 的封闭 terminal 原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopStructuredTerminalReason {
    /// critical/coalescable queue pressure 导致显式断开。
    QueueOverflow,
    /// 整个 Desktop transport 正在关闭。
    Shutdown,
    /// 权威 application stream 已结束。
    UpstreamEnded,
    /// host 关闭了这一条 subscription。
    SubscriptionClosed,
}

/// 只在 queue-overflow terminal 上携带的投递等级。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopStructuredDeliveryClass {
    /// 不可静默丢弃。
    Critical,
    /// 只有存在无损 combiner 时才可合并。
    Coalescable,
    /// latest-value presence/progress。
    LatestValue,
    /// Screen 禁止进入本通道。
    Screen,
}

/// 送达一个 Tauri IPC callback 的 sequence-checked frame。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DesktopStructuredEventFrame {
    /// 一条经过 ACL 与 closed-stream 校验的 application event。
    Event {
        /// host 铸造的 subscription identity；renderer 不能选择。
        subscription_id: u64,
        /// 封闭 stream 家族。
        stream: DesktopStructuredStreamKind,
        /// 本 subscription 的 delivery sequence。
        sequence: u64,
        /// 紧邻本帧之前的已知缺失区间。
        skipped: Option<DesktopStructuredSequenceGap>,
        /// typed application event；不重新引入 scope/auth 字段。
        event: AppEvent,
    },
    /// 显式 terminal frame；不伪造 event。
    Terminal {
        /// host 铸造的 subscription identity。
        subscription_id: u64,
        /// 封闭 stream 家族。
        stream: DesktopStructuredStreamKind,
        /// 本 subscription 的最终 delivery sequence。
        sequence: u64,
        /// terminal 之前的已知缺失区间。
        skipped: Option<DesktopStructuredSequenceGap>,
        /// 稳定 terminal 原因。
        reason: DesktopStructuredTerminalReason,
        /// 只在 queue overflow 时存在。
        #[serde(skip_serializing_if = "Option::is_none")]
        overflow_class: Option<DesktopStructuredDeliveryClass>,
    },
}

impl DesktopStructuredEventFrame {
    /// host-minted subscription identity。
    #[must_use]
    pub const fn subscription_id(&self) -> u64 {
        match self {
            Self::Event {
                subscription_id, ..
            }
            | Self::Terminal {
                subscription_id, ..
            } => *subscription_id,
        }
    }

    /// 本帧所属的封闭 stream 家族。
    #[must_use]
    pub const fn stream(&self) -> DesktopStructuredStreamKind {
        match self {
            Self::Event { stream, .. } | Self::Terminal { stream, .. } => *stream,
        }
    }

    /// 本 subscription 的 delivery sequence。
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        match self {
            Self::Event { sequence, .. } | Self::Terminal { sequence, .. } => *sequence,
        }
    }

    /// 紧邻本帧之前的已知缺失区间。
    #[must_use]
    pub const fn skipped(&self) -> Option<DesktopStructuredSequenceGap> {
        match self {
            Self::Event { skipped, .. } | Self::Terminal { skipped, .. } => *skipped,
        }
    }

    /// typed application event；terminal frame 返回 `None`。
    #[must_use]
    pub const fn event(&self) -> Option<&AppEvent> {
        match self {
            Self::Event { event, .. } => Some(event),
            Self::Terminal { .. } => None,
        }
    }

    /// terminal 原因；event frame 返回 `None`。
    #[must_use]
    pub const fn terminal_reason(&self) -> Option<DesktopStructuredTerminalReason> {
        match self {
            Self::Event { .. } => None,
            Self::Terminal { reason, .. } => Some(*reason),
        }
    }
}

/// actual open command 立即返回的 host receipt。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopStructuredSubscriptionOpened {
    /// host-minted identity；只可用于关闭同一个 native window 的本 subscription。
    pub subscription_id: u64,
}

/// actual close command 的封闭参数。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopStructuredSubscriptionCloseRequest {
    /// 由 open receipt 得到的 host-minted identity。
    pub subscription_id: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_names_and_receipts_are_closed() {
        assert_eq!(
            DESKTOP_STRUCTURED_OPEN_COMMAND,
            "openbot_structured_events_open"
        );
        assert_eq!(
            DESKTOP_STRUCTURED_CLOSE_COMMAND,
            "openbot_structured_events_close"
        );
        assert_eq!(
            DESKTOP_STRUCTURED_SUBSCRIPTION_ID_EXCLUSIVE_LIMIT,
            (1_u64 << 53) - 1
        );
        let opened = DesktopStructuredSubscriptionOpened {
            subscription_id: 41,
        };
        assert_eq!(
            serde_json::to_string(&opened).unwrap(),
            r#"{"subscriptionId":41}"#
        );
        assert!(
            serde_json::from_str::<DesktopStructuredSubscriptionOpened>(
                r#"{"subscriptionId":41,"actor":"forged"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn stream_family_rejects_wrong_events_and_wrong_thread() {
        let expected = ThreadId::new("thread-a");
        let other = ThreadId::new("thread-b");
        let event = AppEvent::ThreadRunEvent(crate::command::ThreadRunEvent {
            thread_id: other,
            run_id: crate::ids::RunId::new("run-1"),
            event_sequence: 1,
            event_type: crate::command::ThreadRunEventKind::Started,
            payload: serde_json::json!({}),
            terminal: false,
            created_at: time::OffsetDateTime::UNIX_EPOCH,
        });
        assert!(!DesktopStructuredStreamKind::ThreadEvents.accepts_event(&event, Some(&expected)));
        assert!(!DesktopStructuredStreamKind::Health.accepts_event(&event, None));
        assert!(
            DesktopStructuredStreamKind::Health
                .accepts_event(&AppEvent::Heartbeat { seq: 1 }, None)
        );
    }

    #[test]
    fn non_overflow_terminal_omits_overflow_class_and_authority() {
        let frame = DesktopStructuredEventFrame::Terminal {
            subscription_id: 7,
            stream: DesktopStructuredStreamKind::Health,
            sequence: 3,
            skipped: None,
            reason: DesktopStructuredTerminalReason::UpstreamEnded,
            overflow_class: None,
        };
        let json = serde_json::to_value(frame).unwrap();
        assert_eq!(json["subscriptionId"], 7);
        assert_eq!(json["reason"], "upstream_ended");
        assert!(json.get("overflowClass").is_none());
        for forbidden in ["window", "actor", "tenant", "authGeneration"] {
            assert!(
                json.get(forbidden).is_none(),
                "authority field leaked: {forbidden}"
            );
        }
    }

    #[test]
    fn frame_json_text_preserves_u64_beyond_javascript_number_precision() {
        let frame = DesktopStructuredEventFrame::Event {
            subscription_id: 7,
            stream: DesktopStructuredStreamKind::Health,
            sequence: u64::MAX,
            skipped: None,
            event: AppEvent::Heartbeat { seq: u64::MAX },
        };
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.matches("18446744073709551615").count() >= 2);
        assert_eq!(
            serde_json::from_str::<DesktopStructuredEventFrame>(&json).unwrap(),
            frame
        );
    }
}
