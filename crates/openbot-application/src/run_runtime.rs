//! Built-in/remote Agent 共用的 durable run 写入边界（v3 §4.3 / §7.2）。
//!
//! 本模块的 lease/claim 类型不实现 serde，不能从 HTTP、renderer 或 remote Agent 字节铸造。
//! PostgreSQL adapter 仍逐次验证 owner + fencing + expiry；类型私有字段不是授权替代品。

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use openbot_contracts::ids::{ActorId, BotId, RunId, ThreadId};
use openbot_domain::thread::FencingToken;

use crate::chunk::{SEMANTIC_CHUNK_MAX_BYTES, SemanticChunkAccumulator};

/// Run runtime 的稳定内部错误域；不得携带数据库/provider 原文。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RunRuntimeError {
    /// PostgreSQL/runtime 依赖不可用。
    #[error("run_runtime_unavailable")]
    Unavailable,
    /// Durable 行结构损坏。
    #[error("run_runtime_corrupt field={field}")]
    Corrupt {
        /// 静态字段名。
        field: &'static str,
    },
    /// 输入不满足内部 typed boundary。
    #[error("run_runtime_input_invalid field={field}")]
    InvalidInput {
        /// 静态字段名。
        field: &'static str,
    },
    /// owner/fencing/expiry 已失效；旧 writer 不能提交。
    #[error("run_runtime_stale_lease")]
    StaleLease,
    /// expected sequence 已被不同内容占用，或 terminal 目标冲突。
    #[error("run_runtime_request_conflict")]
    Conflict,
    /// Commit 结果未知；调用方必须以相同 expected sequence 重试核对。
    #[error("run_runtime_commit_unknown")]
    CommitUnknown,
}

/// Terminal/recovery 可落库的封闭稳定错误码。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunFailureCode {
    /// G4 Agent runtime 尚不可用或永久拒绝 dispatch。
    AgentRuntimeUnavailable,
    /// 已被 runtime 接受的 run 失去 lease，外部 effect 状态未知。
    RuntimeLeaseExpired,
    /// 内部 outbox payload 与 durable run 不一致。
    DispatchPayloadCorrupt,
    /// Dispatch 经过有界重试仍不能被 runtime 接受。
    DispatchDeadLetter,
    /// Provider key/auth rejected。
    ProviderAuthentication,
    /// Provider rate limit exhausted for this run。
    ProviderRateLimited,
    /// Provider transport/5xx unavailable。
    ProviderUnavailable,
    /// Provider SSE/schema/sequence invalid。
    ProviderInvalidResponse,
    /// Provider real body read gap exceeded watchdog。
    ProviderStreamStalled,
    /// Provider reported failed/incomplete generation。
    ProviderGenerationFailed,
    /// Built-in Agent tool sampling step cap reached。
    ToolStepLimit,
    /// G4 tool loop not yet available for this requested call。
    ToolLoopUnavailable,
    /// Tool approval/policy denial。
    ToolDenied,
    /// Absolute run deadline elapsed。
    RunDeadlineExceeded,
    /// Run journal commit result unknown after provider/tool effect。
    JournalCommitUnknown,
}

impl RunFailureCode {
    /// PostgreSQL/semantic event 的稳定字面量。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AgentRuntimeUnavailable => "agent_runtime_unavailable",
            Self::RuntimeLeaseExpired => "runtime_lease_expired",
            Self::DispatchPayloadCorrupt => "dispatch_payload_corrupt",
            Self::DispatchDeadLetter => "dispatch_dead_letter",
            Self::ProviderAuthentication => "provider_authentication",
            Self::ProviderRateLimited => "provider_rate_limited",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::ProviderInvalidResponse => "provider_invalid_response",
            Self::ProviderStreamStalled => "agent_stream_stalled",
            Self::ProviderGenerationFailed => "provider_generation_failed",
            Self::ToolStepLimit => "tool_step_limit",
            Self::ToolLoopUnavailable => "tool_loop_unavailable",
            Self::ToolDenied => "tool_denied",
            Self::RunDeadlineExceeded => "run_deadline_exceeded",
            Self::JournalCommitUnknown => "journal_commit_unknown",
        }
    }
}

/// Run terminal 目标；terminal 属性与错误码要求不能由自由布尔值/字符串自报。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunTerminal {
    /// 正常完成。
    Completed,
    /// 确定失败。
    Failed(RunFailureCode),
    /// 已确认所有子任务停止后的取消。
    Cancelled,
    /// 可能已有外部 effect，必须人工 reconciliation。
    ReconciliationRequired(RunFailureCode),
}

impl RunTerminal {
    /// Runs/event 共用的封闭状态字面量。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed(_) => "failed",
            Self::Cancelled => "cancelled",
            Self::ReconciliationRequired(_) => "reconciliation_required",
        }
    }

    /// 失败/reconciliation 的稳定码。
    #[must_use]
    pub const fn error_code(self) -> Option<RunFailureCode> {
        match self {
            Self::Failed(code) | Self::ReconciliationRequired(code) => Some(code),
            Self::Completed | Self::Cancelled => None,
        }
    }
}

/// PostgreSQL 已验证的一次 run writer lease；不实现 serde。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunExecutionLease {
    run_id: RunId,
    thread_id: ThreadId,
    bot_id: BotId,
    actor_id: ActorId,
    fencing: FencingToken,
    next_event_sequence: u64,
}

impl RunExecutionLease {
    /// Adapter 从同一事务读出的 durable facts 构造。
    pub fn new(
        run_id: RunId,
        thread_id: ThreadId,
        bot_id: BotId,
        actor_id: ActorId,
        fencing: FencingToken,
        next_event_sequence: u64,
    ) -> Result<Self, RunRuntimeError> {
        if [
            run_id.as_str(),
            thread_id.as_str(),
            bot_id.as_str(),
            actor_id.as_str(),
        ]
        .into_iter()
        .any(|value| value.is_empty() || value.as_bytes().contains(&0))
            || i64::try_from(next_event_sequence).is_err()
        {
            return Err(RunRuntimeError::Corrupt {
                field: "run_execution_lease",
            });
        }
        Ok(Self {
            run_id,
            thread_id,
            bot_id,
            actor_id,
            fencing,
            next_event_sequence,
        })
    }

    /// Run id。
    #[must_use]
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Thread id。
    #[must_use]
    pub const fn thread_id(&self) -> &ThreadId {
        &self.thread_id
    }

    /// Bot id。
    #[must_use]
    pub const fn bot_id(&self) -> &BotId {
        &self.bot_id
    }

    /// Authoritative actor。
    #[must_use]
    pub const fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    /// 当前 fencing token。
    #[must_use]
    pub const fn fencing(&self) -> FencingToken {
        self.fencing
    }

    /// Dispatch 时权威 next run-local event sequence。
    #[must_use]
    pub const fn next_event_sequence(&self) -> u64 {
        self.next_event_sequence
    }
}

/// 一个已被 PostgreSQL 原子 claim 的 replay-safe dispatch。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimedRunDispatch {
    outbox_id: String,
    attempt: u32,
    lease: RunExecutionLease,
}

impl ClaimedRunDispatch {
    /// Adapter 构造；空 ID/零 attempt 是 durable corruption。
    pub fn new(
        outbox_id: String,
        attempt: u32,
        lease: RunExecutionLease,
    ) -> Result<Self, RunRuntimeError> {
        if outbox_id.is_empty() || outbox_id.as_bytes().contains(&0) {
            return Err(RunRuntimeError::Corrupt { field: "outbox_id" });
        }
        if attempt == 0 {
            return Err(RunRuntimeError::Corrupt {
                field: "attempt_count",
            });
        }
        Ok(Self {
            outbox_id,
            attempt,
            lease,
        })
    }

    /// Outbox id。
    #[must_use]
    pub fn outbox_id(&self) -> &str {
        &self.outbox_id
    }

    /// 当前 claim attempt；旧 attempt 不能 ack 新 claim。
    #[must_use]
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Run lease facts。
    #[must_use]
    pub const fn lease(&self) -> &RunExecutionLease {
        &self.lease
    }
}

/// 一次 durable event write 的 receipt。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunWriteReceipt {
    /// Run-local sequence。
    pub run_event_sequence: u64,
    /// Thread-global reconnect cursor。
    pub thread_event_sequence: u64,
    /// Terminal materialize assistant message 时的 sequence。
    pub message_sequence: Option<u64>,
    /// 相同 expected sequence + 相同 payload 的精确 replay。
    pub replayed: bool,
}

/// Normalized provider semantic channel；raw vendor event 不进入 journal。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunSemanticChannel {
    /// User-visible assistant text；terminal 时物化到 messages。
    Text,
    /// Provider reasoning；durable/replayable，但绝不拼进 assistant text。
    Reasoning,
}

impl RunSemanticChannel {
    /// Journal payload 的稳定字面量。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Reasoning => "reasoning",
        }
    }
}

/// Relay 对 dispatch consumer 的封闭结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunDispatchDecision {
    /// 已按 `(run_id,fencing)` 幂等接受；relay 才可 ack outbox。
    Accepted,
    /// 本机有界队列暂满；outbox 回 pending 并按 attempt 退避。
    RetryableBusy,
    /// 永久拒绝，run 以确定失败收口。
    Rejected(RunFailureCode),
}

/// Built-in/remote Agent 的 in-process dispatch 边界。
#[async_trait]
pub trait RunDispatchConsumer: Send + Sync {
    /// 预留/幂等 enqueue；不得在 durable outbox ack 前启动 provider/tool effect。
    async fn dispatch(&self, lease: RunExecutionLease) -> RunDispatchDecision;

    /// Outbox 已 durable ack 后启动该 reservation；必须幂等，队列关闭须显式失败。
    async fn activate(&self, lease: &RunExecutionLease) -> Result<(), RunFailureCode>;

    /// 撤销尚未取得 durable outbox ack 的 enqueue；必须幂等并传播 cancel。
    async fn revoke(&self, lease: &RunExecutionLease);
}

/// G4 Agent 尚未接线时的 production fail-closed consumer；它明确失败，不执行、不伪造回复。
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRunDispatchConsumer;

#[async_trait]
impl RunDispatchConsumer for NoRunDispatchConsumer {
    async fn dispatch(&self, _lease: RunExecutionLease) -> RunDispatchDecision {
        RunDispatchDecision::Rejected(RunFailureCode::AgentRuntimeUnavailable)
    }

    async fn activate(&self, _lease: &RunExecutionLease) -> Result<(), RunFailureCode> {
        Ok(())
    }

    async fn revoke(&self, _lease: &RunExecutionLease) {}
}

/// Run journal/outbox/lease 的唯一 application port。
#[async_trait]
pub trait RunRuntime: Send + Sync {
    /// Claim 当前 worker 可合法接管的一条 `agent_run_dispatch`。
    async fn claim_dispatch(&self) -> Result<Option<ClaimedRunDispatch>, RunRuntimeError>;

    /// Consumer 已幂等接收后确认 outbox delivered。
    async fn acknowledge_dispatch(
        &self,
        claim: &ClaimedRunDispatch,
    ) -> Result<RunExecutionLease, RunRuntimeError>;

    /// 暂时无法 enqueue；回 pending，退避由 adapter 按 attempt 封闭计算。
    async fn retry_dispatch(&self, claim: &ClaimedRunDispatch) -> Result<(), RunRuntimeError>;

    /// 永久拒绝；同事务 terminalize run + deliver/dead-letter outbox。
    async fn reject_dispatch(
        &self,
        claim: &ClaimedRunDispatch,
        code: RunFailureCode,
    ) -> Result<RunWriteReceipt, RunRuntimeError>;

    /// 续租；只有 owner/fencing/status 都仍匹配时成功。
    async fn renew_lease(&self, lease: &RunExecutionLease) -> Result<(), RunRuntimeError>;

    /// 精确 expected-sequence append；commit unknown 可安全重试同一输入。
    async fn append_semantic_chunk(
        &self,
        lease: &RunExecutionLease,
        expected_sequence: u64,
        channel: RunSemanticChannel,
        chunk: &str,
    ) -> Result<RunWriteReceipt, RunRuntimeError>;

    /// 精确 expected-sequence terminal；同事务 materialize assistant text 并 notify。
    async fn finish_run(
        &self,
        lease: &RunExecutionLease,
        expected_sequence: u64,
        terminal: RunTerminal,
    ) -> Result<RunWriteReceipt, RunRuntimeError>;

    /// 对已 delivered 且 lease 过期的 running run 做一次 fencing takeover + reconciliation。
    async fn recover_one_stale_run(&self) -> Result<Option<RunWriteReceipt>, RunRuntimeError>;
}

/// 把 50ms/8KiB accumulator 真正接到 durable writer；provider adapter 后续只喂 delta。
pub struct DurableTextRun {
    runtime: Arc<dyn RunRuntime>,
    lease: RunExecutionLease,
    expected_sequence: u64,
    accumulator: SemanticChunkAccumulator,
    pending_durable: VecDeque<(RunSemanticChannel, String)>,
}

impl core::fmt::Debug for DurableTextRun {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DurableTextRun")
            .field("run_id", self.lease.run_id())
            .field("expected_sequence", &self.expected_sequence)
            .field("pending_durable", &self.pending_durable.len())
            .finish_non_exhaustive()
    }
}

impl DurableTextRun {
    /// 从已 acknowledge 的 lease 构造。
    #[must_use]
    pub fn new(runtime: Arc<dyn RunRuntime>, lease: RunExecutionLease) -> Self {
        let expected_sequence = lease.next_event_sequence();
        Self {
            runtime,
            lease,
            expected_sequence,
            accumulator: SemanticChunkAccumulator::new(),
            pending_durable: VecDeque::new(),
        }
    }

    /// 推入 provider text delta；达到 50ms/8KiB 的 chunk 立即顺序持久化。
    pub async fn push(&mut self, delta: &str, now: Instant) -> Result<(), RunRuntimeError> {
        if delta.as_bytes().contains(&0) {
            return Err(RunRuntimeError::InvalidInput { field: "delta" });
        }
        self.pending_durable.extend(
            self.accumulator
                .push(delta, now)
                .into_iter()
                .map(|chunk| (RunSemanticChannel::Text, chunk)),
        );
        self.drain_pending().await
    }

    /// 持久化 normalized reasoning delta。先冲刷此前 text，再按同一 8KiB UTF-8 边界切片，
    /// 因而 journal 的跨 channel 顺序与 provider 事件顺序一致。
    pub async fn push_reasoning(
        &mut self,
        delta: &str,
        now: Instant,
    ) -> Result<(), RunRuntimeError> {
        if delta.as_bytes().contains(&0) {
            return Err(RunRuntimeError::InvalidInput {
                field: "reasoning_delta",
            });
        }
        self.pending_durable.extend(
            self.accumulator
                .finish()
                .map(|chunk| (RunSemanticChannel::Text, chunk)),
        );
        let mut reasoning = SemanticChunkAccumulator::new();
        self.pending_durable.extend(
            reasoning
                .push(delta, now)
                .into_iter()
                .chain(reasoning.finish())
                .map(|chunk| (RunSemanticChannel::Reasoning, chunk)),
        );
        self.drain_pending().await
    }

    /// Timer 到点时持久化 pending chunk。
    pub async fn flush_due(&mut self, now: Instant) -> Result<(), RunRuntimeError> {
        self.pending_durable.extend(
            self.accumulator
                .flush_due(now)
                .map(|chunk| (RunSemanticChannel::Text, chunk)),
        );
        self.drain_pending().await
    }

    /// Pending 的最迟 flush 时刻；provider loop 应把它放入同一个 `select!`。
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.accumulator.next_deadline()
    }

    /// Flush 最后一块并写唯一 terminal；相同 expected sequence 可在 commit unknown 后重试。
    pub async fn finish(
        &mut self,
        terminal: RunTerminal,
    ) -> Result<RunWriteReceipt, RunRuntimeError> {
        self.pending_durable.extend(
            self.accumulator
                .finish()
                .map(|chunk| (RunSemanticChannel::Text, chunk)),
        );
        self.drain_pending().await?;
        let receipt = self
            .runtime
            .finish_run(&self.lease, self.expected_sequence, terminal)
            .await?;
        self.expected_sequence =
            receipt
                .run_event_sequence
                .checked_add(1)
                .ok_or(RunRuntimeError::Corrupt {
                    field: "next_event_sequence",
                })?;
        Ok(receipt)
    }

    async fn drain_pending(&mut self) -> Result<(), RunRuntimeError> {
        while let Some((channel, chunk)) = self.pending_durable.front() {
            if chunk.is_empty() || chunk.len() > SEMANTIC_CHUNK_MAX_BYTES {
                return Err(RunRuntimeError::Corrupt {
                    field: "semantic_chunk",
                });
            }
            let receipt = self
                .runtime
                .append_semantic_chunk(&self.lease, self.expected_sequence, *channel, chunk)
                .await?;
            self.expected_sequence =
                receipt
                    .run_event_sequence
                    .checked_add(1)
                    .ok_or(RunRuntimeError::Corrupt {
                        field: "next_event_sequence",
                    })?;
            self.pending_durable.pop_front();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use openbot_contracts::ids::{ActorId, BotId, RunId, ThreadId};
    use openbot_domain::thread::FencingToken;

    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Call {
        Chunk(u64, RunSemanticChannel, String),
        Finish(u64, RunTerminal),
    }

    #[derive(Default)]
    struct FakeRuntime {
        calls: Mutex<Vec<Call>>,
        fail_first_chunk: Mutex<bool>,
    }

    impl FakeRuntime {
        fn failing_once() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_first_chunk: Mutex::new(true),
            }
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().expect("fake lock").clone()
        }
    }

    #[async_trait]
    impl RunRuntime for FakeRuntime {
        async fn claim_dispatch(&self) -> Result<Option<ClaimedRunDispatch>, RunRuntimeError> {
            Err(RunRuntimeError::Unavailable)
        }

        async fn acknowledge_dispatch(
            &self,
            _claim: &ClaimedRunDispatch,
        ) -> Result<RunExecutionLease, RunRuntimeError> {
            Err(RunRuntimeError::Unavailable)
        }

        async fn retry_dispatch(&self, _claim: &ClaimedRunDispatch) -> Result<(), RunRuntimeError> {
            Err(RunRuntimeError::Unavailable)
        }

        async fn reject_dispatch(
            &self,
            _claim: &ClaimedRunDispatch,
            _code: RunFailureCode,
        ) -> Result<RunWriteReceipt, RunRuntimeError> {
            Err(RunRuntimeError::Unavailable)
        }

        async fn renew_lease(&self, _lease: &RunExecutionLease) -> Result<(), RunRuntimeError> {
            Ok(())
        }

        async fn append_semantic_chunk(
            &self,
            _lease: &RunExecutionLease,
            expected_sequence: u64,
            channel: RunSemanticChannel,
            chunk: &str,
        ) -> Result<RunWriteReceipt, RunRuntimeError> {
            self.calls.lock().expect("fake lock").push(Call::Chunk(
                expected_sequence,
                channel,
                chunk.to_owned(),
            ));
            let mut fail = self.fail_first_chunk.lock().expect("fake lock");
            if *fail {
                *fail = false;
                return Err(RunRuntimeError::CommitUnknown);
            }
            Ok(receipt(expected_sequence))
        }

        async fn finish_run(
            &self,
            _lease: &RunExecutionLease,
            expected_sequence: u64,
            terminal: RunTerminal,
        ) -> Result<RunWriteReceipt, RunRuntimeError> {
            self.calls
                .lock()
                .expect("fake lock")
                .push(Call::Finish(expected_sequence, terminal));
            Ok(receipt(expected_sequence))
        }

        async fn recover_one_stale_run(&self) -> Result<Option<RunWriteReceipt>, RunRuntimeError> {
            Ok(None)
        }
    }

    fn lease() -> RunExecutionLease {
        RunExecutionLease::new(
            RunId::new("run-1"),
            ThreadId::new("thread-1"),
            BotId::new("bot-1"),
            ActorId::new("actor-1"),
            FencingToken::new(7).unwrap(),
            1,
        )
        .unwrap()
    }

    const fn receipt(sequence: u64) -> RunWriteReceipt {
        RunWriteReceipt {
            run_event_sequence: sequence,
            thread_event_sequence: sequence + 10,
            message_sequence: None,
            replayed: false,
        }
    }

    #[tokio::test]
    async fn eight_kib_chunks_reach_the_port_in_order_before_terminal() {
        let runtime = Arc::new(FakeRuntime::default());
        let mut run = DurableTextRun::new(runtime.clone(), lease());
        let input = "a".repeat(SEMANTIC_CHUNK_MAX_BYTES + 1);
        run.push(&input, Instant::now()).await.unwrap();
        let terminal = run.finish(RunTerminal::Completed).await.unwrap();
        assert_eq!(terminal.run_event_sequence, 3);
        assert_eq!(runtime.calls().len(), 3);
        assert_eq!(
            runtime.calls(),
            [
                Call::Chunk(
                    1,
                    RunSemanticChannel::Text,
                    "a".repeat(SEMANTIC_CHUNK_MAX_BYTES)
                ),
                Call::Chunk(2, RunSemanticChannel::Text, "a".to_owned()),
                Call::Finish(3, RunTerminal::Completed),
            ]
        );
    }

    #[tokio::test]
    async fn commit_unknown_keeps_the_same_chunk_and_expected_sequence_for_reconciliation() {
        let runtime = Arc::new(FakeRuntime::failing_once());
        let mut run = DurableTextRun::new(runtime.clone(), lease());
        let start = Instant::now();
        run.push("durable", start).await.unwrap();
        assert_eq!(
            run.flush_due(start + crate::chunk::SEMANTIC_CHUNK_MAX_DELAY)
                .await,
            Err(RunRuntimeError::CommitUnknown)
        );
        run.push("", start + crate::chunk::SEMANTIC_CHUNK_MAX_DELAY)
            .await
            .unwrap();
        assert_eq!(
            runtime.calls(),
            [
                Call::Chunk(1, RunSemanticChannel::Text, "durable".to_owned()),
                Call::Chunk(1, RunSemanticChannel::Text, "durable".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn reasoning_keeps_provider_order_but_uses_a_distinct_channel() {
        let runtime = Arc::new(FakeRuntime::default());
        let start = Instant::now();
        let mut run = DurableTextRun::new(runtime.clone(), lease());
        run.push("before", start).await.unwrap();
        run.push_reasoning("internal", start + std::time::Duration::from_millis(1))
            .await
            .unwrap();
        run.push("after", start + std::time::Duration::from_millis(2))
            .await
            .unwrap();
        run.finish(RunTerminal::Completed).await.unwrap();
        assert_eq!(
            runtime.calls(),
            [
                Call::Chunk(1, RunSemanticChannel::Text, "before".to_owned()),
                Call::Chunk(2, RunSemanticChannel::Reasoning, "internal".to_owned()),
                Call::Chunk(3, RunSemanticChannel::Text, "after".to_owned()),
                Call::Finish(4, RunTerminal::Completed),
            ]
        );
    }

    #[tokio::test]
    async fn nul_delta_is_rejected_before_the_runtime_port() {
        let runtime = Arc::new(FakeRuntime::default());
        let mut run = DurableTextRun::new(runtime.clone(), lease());
        assert_eq!(
            run.push("bad\0delta", Instant::now()).await,
            Err(RunRuntimeError::InvalidInput { field: "delta" })
        );
        assert!(runtime.calls().is_empty());
    }
}
