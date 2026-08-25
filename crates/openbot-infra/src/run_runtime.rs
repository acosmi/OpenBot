//! Native run dispatch/outbox/lease/chunk/terminal 的 PostgreSQL 原子适配器。

use core::time::Duration as StdDuration;
use std::sync::Arc;

use async_trait::async_trait;
use openbot_application::{
    ClaimedRunDispatch, RunDispatchConsumer, RunDispatchDecision, RunExecutionLease,
    RunFailureCode, RunRuntime, RunRuntimeError, RunSemanticChannel, RunTerminal, RunToolExchange,
    RunWriteReceipt, SEMANTIC_CHUNK_MAX_BYTES,
};
use openbot_contracts::ids::{ActorId, BotId, RunId, ThreadId};
use openbot_domain::audit::hash::Sha256Digest;
use openbot_domain::thread::FencingToken;
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_postgres::error::SqlState;
use tokio_postgres::{Row, Transaction};

const THREAD_EVENT_TOPIC: &str = "openbot_thread_events";
const DISPATCH_DESTINATION: &str = "agent_run_dispatch";
const DISPATCH_RETRY_CODE: &str = "runtime_busy";
const DISPATCH_RETRY_BASE: Duration = Duration::milliseconds(100);
const ASSISTANT_MESSAGE_SUFFIX: &str = ":assistant";

/// Dispatch claim 的默认有效期；等于 30s thread lease 的三分之一。
pub const DEFAULT_DISPATCH_CLAIM_DURATION: Duration = Duration::seconds(10);
/// Relay 空闲轮询；NOTIFY 不是 outbox 真源，100ms 在交互延迟与空闲 DB 压力间取有界值。
pub const DEFAULT_RUN_RELAY_POLL: StdDuration = StdDuration::from_millis(100);
const RUN_RELAY_BATCH: usize = 64;

/// Production outbox relay 生命周期句柄。
pub struct RunRelay {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl core::fmt::Debug for RunRelay {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RunRelay").finish_non_exhaustive()
    }
}

impl RunRelay {
    /// 启动 multi-replica-safe relay；具体 Agent consumer 由组装层注入。
    #[must_use]
    pub fn start(runtime: Arc<dyn RunRuntime>, consumer: Arc<dyn RunDispatchConsumer>) -> Self {
        let (stop, stop_rx) = watch::channel(false);
        let task = tokio::spawn(supervise_run_relay(runtime, consumer, stop_rx));
        Self { stop, task }
    }

    /// 停止 claim 新工作并等待当前一次原子操作收口。
    pub async fn stop(self) {
        self.stop.send_replace(true);
        let _ = self.task.await;
    }
}

/// Run runtime 的 production PostgreSQL adapter。
#[derive(Clone)]
pub struct PostgresRunRuntime {
    pool: deadpool_postgres::Pool,
    owner_id: String,
    lease_duration: Duration,
    claim_duration: Duration,
}

impl core::fmt::Debug for PostgresRunRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PostgresRunRuntime")
            .field("lease_duration", &self.lease_duration)
            .field("claim_duration", &self.claim_duration)
            .finish_non_exhaustive()
    }
}

impl PostgresRunRuntime {
    /// 用与 `PostgresThreadDirectory` 相同的 process owner 构造。
    pub fn new(
        pool: deadpool_postgres::Pool,
        owner_id: String,
        lease_duration: Duration,
        claim_duration: Duration,
    ) -> Result<Self, RunRuntimeError> {
        if owner_id.is_empty()
            || owner_id.as_bytes().contains(&0)
            || lease_duration <= Duration::ZERO
            || claim_duration <= Duration::ZERO
            || claim_duration >= lease_duration
        {
            return Err(RunRuntimeError::InvalidInput {
                field: "run_runtime_config",
            });
        }
        Ok(Self {
            pool,
            owner_id,
            lease_duration,
            claim_duration,
        })
    }

    async fn client(&self) -> Result<deadpool_postgres::Client, RunRuntimeError> {
        self.pool.get().await.map_err(|error| {
            tracing::error!(error = %error, "run runtime 获取数据库连接失败");
            RunRuntimeError::Unavailable
        })
    }
}

#[async_trait]
impl RunRuntime for PostgresRunRuntime {
    async fn claim_dispatch(&self) -> Result<Option<ClaimedRunDispatch>, RunRuntimeError> {
        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|error| unavailable("开始 dispatch claim 事务", error))?;
        let result = claim_dispatch_in_transaction(
            &transaction,
            &self.owner_id,
            self.lease_duration,
            self.claim_duration,
        )
        .await;
        finish_transaction(transaction, result).await
    }

    async fn acknowledge_dispatch(
        &self,
        claim: &ClaimedRunDispatch,
    ) -> Result<RunExecutionLease, RunRuntimeError> {
        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|error| unavailable("开始 dispatch ack 事务", error))?;
        let result = acknowledge_dispatch_in_transaction(&transaction, &self.owner_id, claim).await;
        finish_transaction(transaction, result).await
    }

    async fn retry_dispatch(&self, claim: &ClaimedRunDispatch) -> Result<(), RunRuntimeError> {
        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|error| unavailable("开始 dispatch retry 事务", error))?;
        let result =
            retry_dispatch_in_transaction(&transaction, &self.owner_id, claim, self.claim_duration)
                .await;
        finish_transaction(transaction, result).await
    }

    async fn reject_dispatch(
        &self,
        claim: &ClaimedRunDispatch,
        code: RunFailureCode,
    ) -> Result<RunWriteReceipt, RunRuntimeError> {
        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|error| unavailable("开始 dispatch reject 事务", error))?;
        let result =
            reject_dispatch_in_transaction(&transaction, &self.owner_id, claim, code).await;
        finish_transaction(transaction, result).await
    }

    async fn renew_lease(&self, lease: &RunExecutionLease) -> Result<(), RunRuntimeError> {
        let client = self.client().await?;
        let now = database_now_client(&client).await?;
        let expires_at = checked_add(now, self.lease_duration, "lease_expiry")?;
        let updated = client
            .execute(
                "UPDATE public.thread_leases l SET expires_at=$6,updated_at=$5 \
                 WHERE l.thread_id=$1 AND l.owner_id=$2 AND l.fencing_token=$3 \
                   AND l.expires_at>$5 AND EXISTS( \
                     SELECT 1 FROM public.runs r WHERE r.run_id=$4 AND r.thread_id=$1 \
                       AND r.status='running' AND r.fencing_token=$3)",
                &[
                    &lease.thread_id().as_str(),
                    &self.owner_id,
                    &lease.fencing().get(),
                    &lease.run_id().as_str(),
                    &now,
                    &expires_at,
                ],
            )
            .await
            .map_err(|error| unavailable("续租 run lease", error))?;
        if updated == 1 {
            Ok(())
        } else {
            Err(RunRuntimeError::StaleLease)
        }
    }

    async fn append_semantic_chunk(
        &self,
        lease: &RunExecutionLease,
        expected_sequence: u64,
        channel: RunSemanticChannel,
        chunk: &str,
    ) -> Result<RunWriteReceipt, RunRuntimeError> {
        validate_chunk(chunk)?;
        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|error| unavailable("开始 semantic chunk 事务", error))?;
        let result = append_chunk_in_transaction(
            &transaction,
            &self.owner_id,
            lease,
            expected_sequence,
            channel,
            chunk,
        )
        .await;
        finish_transaction(transaction, result).await
    }

    async fn append_tool_exchange(
        &self,
        lease: &RunExecutionLease,
        expected_sequence: u64,
        exchange: &RunToolExchange,
    ) -> Result<RunWriteReceipt, RunRuntimeError> {
        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|error| unavailable("开始 tool exchange 事务", error))?;
        let result = append_tool_exchange_in_transaction(
            &transaction,
            &self.owner_id,
            lease,
            expected_sequence,
            exchange,
        )
        .await;
        finish_transaction(transaction, result).await
    }

    async fn finish_run(
        &self,
        lease: &RunExecutionLease,
        expected_sequence: u64,
        terminal: RunTerminal,
    ) -> Result<RunWriteReceipt, RunRuntimeError> {
        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|error| unavailable("开始 run terminal 事务", error))?;
        let result = finish_run_in_transaction(
            &transaction,
            &self.owner_id,
            lease,
            expected_sequence,
            terminal,
            LeaseCheck::Active,
        )
        .await;
        finish_transaction(transaction, result).await
    }

    async fn recover_one_stale_run(&self) -> Result<Option<RunWriteReceipt>, RunRuntimeError> {
        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|error| unavailable("开始 stale run recovery 事务", error))?;
        let result = recover_one_in_transaction(&transaction, &self.owner_id).await;
        finish_transaction(transaction, result).await
    }
}

async fn supervise_run_relay(
    runtime: Arc<dyn RunRuntime>,
    consumer: Arc<dyn RunDispatchConsumer>,
    mut stop: watch::Receiver<bool>,
) {
    let mut poll = tokio::time::interval(DEFAULT_RUN_RELAY_POLL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return;
                }
            }
            _ = poll.tick() => {}
        }
        if *stop.borrow() {
            return;
        }

        for _ in 0..RUN_RELAY_BATCH {
            match runtime.recover_one_stale_run().await {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(error) => {
                    tracing::error!(code = %error, "stale run recovery 失败；等待下一轮 durable 重试");
                    break;
                }
            }
        }

        for _ in 0..RUN_RELAY_BATCH {
            let claim = match runtime.claim_dispatch().await {
                Ok(Some(claim)) => claim,
                Ok(None) => break,
                Err(error) => {
                    tracing::error!(code = %error, "run dispatch claim 失败；等待下一轮 durable 重试");
                    break;
                }
            };
            handle_dispatch(runtime.as_ref(), consumer.as_ref(), claim).await;
        }
    }
}

async fn handle_dispatch(
    runtime: &dyn RunRuntime,
    consumer: &dyn RunDispatchConsumer,
    claim: ClaimedRunDispatch,
) {
    match consumer.dispatch(claim.lease().clone()).await {
        RunDispatchDecision::Accepted => match runtime.acknowledge_dispatch(&claim).await {
            Ok(lease) => {
                if let Err(code) = consumer.activate(&lease).await
                    && let Err(error) = runtime
                        .finish_run(
                            &lease,
                            lease.next_event_sequence(),
                            RunTerminal::Failed(code),
                        )
                        .await
                {
                    tracing::error!(code = %error,
                        "durable dispatch ack 后 activation/terminal 均失败；等待 lease recovery");
                }
            }
            Err(error) => {
                consumer.revoke(claim.lease()).await;
                tracing::error!(code = %error,
                    "Agent reservation 未取得 durable dispatch ack；已撤销，等待 fencing/recovery");
            }
        },
        RunDispatchDecision::RetryableBusy => {
            if let Err(error) = runtime.retry_dispatch(&claim).await {
                tracing::error!(code = %error,
                    "busy dispatch 回 pending 未确认；由 claim expiry/reconciliation 接管");
            }
        }
        RunDispatchDecision::Rejected(code) => {
            if let Err(error) = runtime.reject_dispatch(&claim, code).await {
                tracing::error!(code = %error,
                    "永久拒绝 dispatch 的 terminal/outbox 事务未确认；等待精确重放");
            }
        }
    }
}

async fn claim_dispatch_in_transaction(
    transaction: &Transaction<'_>,
    owner: &str,
    lease_duration: Duration,
    claim_duration: Duration,
) -> Result<Option<ClaimedRunDispatch>, RunRuntimeError> {
    let now = database_now(transaction).await?;
    let row = transaction
        .query_opt(
            "SELECT o.outbox_id,o.aggregate_id,o.seq,o.payload,o.status,o.attempt_count, \
                    o.claimed_by,o.claim_expires_at \
             FROM public.outbox o \
             WHERE o.destination=$1 AND ( \
               (o.status='delivering' AND o.claimed_by=$2) \
               OR (o.status='pending' AND o.available_at<=$3) \
               OR (o.status='delivering' AND o.claim_expires_at<=$3) \
             ) AND ( \
               NOT EXISTS(SELECT 1 FROM public.thread_leases lx \
                          WHERE lx.thread_id=o.aggregate_id) \
               OR EXISTS(SELECT 1 FROM public.thread_leases lx \
                         WHERE lx.thread_id=o.aggregate_id \
                           AND (lx.owner_id=$2 OR lx.expires_at<=$3)) \
             ) \
             ORDER BY CASE WHEN o.status='delivering' AND o.claimed_by=$2 THEN 0 ELSE 1 END, \
                      o.available_at,o.outbox_id \
             FOR UPDATE OF o SKIP LOCKED LIMIT 1",
            &[&DISPATCH_DESTINATION, &owner, &now],
        )
        .await
        .map_err(|error| unavailable("选择 dispatch outbox", error))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let outbox_id: String = decode(&row, "outbox_id")?;
    let aggregate_id: String = decode(&row, "aggregate_id")?;
    let outbox_seq: i64 = decode(&row, "seq")?;
    let payload: Value = decode(&row, "payload")?;
    let outbox_status: String = decode(&row, "status")?;
    let old_attempt: i32 = decode(&row, "attempt_count")?;
    let claimed_by: Option<String> = decode(&row, "claimed_by")?;

    let Some(run_id) = payload.get("runId").and_then(Value::as_str) else {
        return dead_letter_dispatch(transaction, &outbox_id, now, "dispatch_run_id").await;
    };
    let Some(thread_id) = payload.get("threadId").and_then(Value::as_str) else {
        return dead_letter_dispatch(transaction, &outbox_id, now, "dispatch_thread_id").await;
    };
    let Some(event_sequence) = payload.get("eventSequence").and_then(Value::as_u64) else {
        return dead_letter_dispatch(transaction, &outbox_id, now, "dispatch_event_sequence").await;
    };
    if aggregate_id != thread_id
        || i64::try_from(event_sequence).ok() != Some(outbox_seq)
        || outbox_id != format!("{run_id}:{DISPATCH_DESTINATION}")
    {
        return dead_letter_dispatch(transaction, &outbox_id, now, "dispatch_binding").await;
    }

    let Some(mut locked) = lock_run(transaction, run_id).await? else {
        return dead_letter_dispatch(transaction, &outbox_id, now, "dispatch_run").await;
    };
    if locked.thread_id != thread_id {
        return dead_letter_dispatch(transaction, &outbox_id, now, "dispatch_thread_binding").await;
    }
    if is_terminal_status(&locked.status) {
        transaction
            .execute(
                "UPDATE public.outbox SET status='delivered',delivered_at=coalesce(delivered_at,$2), \
                 claimed_by=NULL,claim_expires_at=NULL,updated_at=$2 WHERE outbox_id=$1",
                &[&outbox_id, &now],
            )
            .await
            .map_err(|error| unavailable("收敛 terminal run 的 dispatch", error))?;
        return Ok(None);
    }
    if locked.status != "running" {
        return dead_letter_dispatch(transaction, &outbox_id, now, "dispatch_run_status").await;
    }
    if locked.lease_fencing != locked.fencing {
        return dead_letter_dispatch(transaction, &outbox_id, now, "dispatch_fencing_binding")
            .await;
    }

    let lease_expires_at = checked_add(now, lease_duration, "lease_expiry")?;
    if locked.lease_expires_at <= now {
        let Some(next) = locked.lease_fencing.checked_add(1) else {
            return dead_letter_dispatch(transaction, &outbox_id, now, "fencing_exhausted").await;
        };
        transaction
            .execute(
                "UPDATE public.thread_leases SET owner_id=$2,fencing_token=$3,acquired_at=$4, \
                 expires_at=$5,updated_at=$4 WHERE thread_id=$1",
                &[&thread_id, &owner, &next, &now, &lease_expires_at],
            )
            .await
            .map_err(|error| unavailable("接管 dispatch lease", error))?;
        transaction
            .execute(
                "UPDATE public.runs SET fencing_token=$2 WHERE run_id=$1 AND status='running'",
                &[&run_id, &next],
            )
            .await
            .map_err(|error| unavailable("重绑 dispatch run fencing", error))?;
        locked.fencing = next;
        locked.lease_fencing = next;
        locked.lease_owner = owner.to_owned();
    } else if locked.lease_owner == owner {
        transaction
            .execute(
                "UPDATE public.thread_leases SET expires_at=$3,updated_at=$2 WHERE thread_id=$1",
                &[&thread_id, &now, &lease_expires_at],
            )
            .await
            .map_err(|error| unavailable("claim 时续租", error))?;
    } else {
        return Ok(None);
    }

    let replaying_owned_claim =
        outbox_status == "delivering" && claimed_by.as_deref() == Some(owner);
    let attempt = if replaying_owned_claim {
        old_attempt
    } else {
        old_attempt.checked_add(1).ok_or(RunRuntimeError::Corrupt {
            field: "attempt_count",
        })?
    };
    if attempt <= 0 {
        return Err(RunRuntimeError::Corrupt {
            field: "attempt_count",
        });
    }
    let claim_expires_at = checked_add(now, claim_duration, "claim_expiry")?;
    transaction
        .execute(
            "UPDATE public.outbox SET status='delivering',claimed_by=$2,claim_expires_at=$3, \
             attempt_count=$4,updated_at=$5 WHERE outbox_id=$1",
            &[&outbox_id, &owner, &claim_expires_at, &attempt, &now],
        )
        .await
        .map_err(|error| unavailable("写 dispatch claim", error))?;
    let lease = lease_from_locked(&locked)?;
    let attempt = u32::try_from(attempt).map_err(|_| RunRuntimeError::Corrupt {
        field: "attempt_count",
    })?;
    ClaimedRunDispatch::new(outbox_id, attempt, lease).map(Some)
}

async fn acknowledge_dispatch_in_transaction(
    transaction: &Transaction<'_>,
    owner: &str,
    claim: &ClaimedRunDispatch,
) -> Result<RunExecutionLease, RunRuntimeError> {
    let now = database_now(transaction).await?;
    let row = transaction
        .query_opt(
            "SELECT status,claimed_by,attempt_count FROM public.outbox \
             WHERE outbox_id=$1 FOR UPDATE",
            &[&claim.outbox_id()],
        )
        .await
        .map_err(|error| unavailable("读取 dispatch ack 行", error))?
        .ok_or(RunRuntimeError::Corrupt { field: "outbox" })?;
    let status: String = decode(&row, "status")?;
    let claimed_by: Option<String> = decode(&row, "claimed_by")?;
    let attempt: i32 = decode(&row, "attempt_count")?;
    let Some(locked) = lock_run(transaction, claim.lease().run_id().as_str()).await? else {
        return Err(RunRuntimeError::StaleLease);
    };
    validate_claim_binding(&locked, claim)?;
    if status == "delivered" {
        return lease_from_locked(&locked);
    }
    if status != "delivering"
        || claimed_by.as_deref() != Some(owner)
        || u32::try_from(attempt).ok() != Some(claim.attempt())
    {
        return Err(RunRuntimeError::StaleLease);
    }
    validate_active_lease(&locked, owner, claim.lease(), now)?;
    transaction
        .execute(
            "UPDATE public.outbox SET status='delivered',delivered_at=$2,updated_at=$2, \
             claimed_by=NULL,claim_expires_at=NULL,last_error_code=NULL WHERE outbox_id=$1",
            &[&claim.outbox_id(), &now],
        )
        .await
        .map_err(|error| unavailable("确认 dispatch delivered", error))?;
    lease_from_locked(&locked)
}

async fn retry_dispatch_in_transaction(
    transaction: &Transaction<'_>,
    owner: &str,
    claim: &ClaimedRunDispatch,
    claim_duration: Duration,
) -> Result<(), RunRuntimeError> {
    let now = database_now(transaction).await?;
    let exponent = claim.attempt().saturating_sub(1).min(6);
    let factor = 1_i32
        .checked_shl(exponent)
        .ok_or(RunRuntimeError::Corrupt {
            field: "attempt_count",
        })?;
    let backoff = (DISPATCH_RETRY_BASE * factor).min(claim_duration);
    let available_at = checked_add(now, backoff, "dispatch_backoff")?;
    let attempt_i32 = i32::try_from(claim.attempt()).map_err(|_| RunRuntimeError::Corrupt {
        field: "attempt_count",
    })?;
    let updated = transaction
        .execute(
            "UPDATE public.outbox SET status='pending',available_at=$5,claimed_by=NULL, \
             claim_expires_at=NULL,last_error_code=$4,updated_at=$6 \
             WHERE outbox_id=$1 AND status='delivering' AND claimed_by=$2 AND attempt_count=$3",
            &[
                &claim.outbox_id(),
                &owner,
                &attempt_i32,
                &DISPATCH_RETRY_CODE,
                &available_at,
                &now,
            ],
        )
        .await
        .map_err(|error| unavailable("dispatch 回 pending", error))?;
    if updated == 1 {
        return Ok(());
    }
    let exact: bool = transaction
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM public.outbox WHERE outbox_id=$1 AND status='pending' \
             AND attempt_count=$2 AND last_error_code=$3)",
            &[&claim.outbox_id(), &attempt_i32, &DISPATCH_RETRY_CODE],
        )
        .await
        .map_err(|error| unavailable("核对 dispatch retry", error))?
        .try_get(0)
        .map_err(|_| RunRuntimeError::Corrupt {
            field: "retry_exact",
        })?;
    if exact {
        Ok(())
    } else {
        Err(RunRuntimeError::StaleLease)
    }
}

async fn reject_dispatch_in_transaction(
    transaction: &Transaction<'_>,
    owner: &str,
    claim: &ClaimedRunDispatch,
    code: RunFailureCode,
) -> Result<RunWriteReceipt, RunRuntimeError> {
    let expected = claim.lease().next_event_sequence();
    let row = transaction
        .query_opt(
            "SELECT status,claimed_by,attempt_count FROM public.outbox WHERE outbox_id=$1 FOR UPDATE",
            &[&claim.outbox_id()],
        )
        .await
        .map_err(|error| unavailable("读取 rejected dispatch", error))?
        .ok_or(RunRuntimeError::Corrupt { field: "outbox" })?;
    let status: String = decode(&row, "status")?;
    let claimed_by: Option<String> = decode(&row, "claimed_by")?;
    let attempt: i32 = decode(&row, "attempt_count")?;
    if status == "delivered" {
        return finish_run_in_transaction(
            transaction,
            owner,
            claim.lease(),
            expected,
            RunTerminal::Failed(code),
            LeaseCheck::Active,
        )
        .await;
    }
    if status != "delivering"
        || claimed_by.as_deref() != Some(owner)
        || u32::try_from(attempt).ok() != Some(claim.attempt())
    {
        return Err(RunRuntimeError::StaleLease);
    }
    let receipt = finish_run_in_transaction(
        transaction,
        owner,
        claim.lease(),
        expected,
        RunTerminal::Failed(code),
        LeaseCheck::Active,
    )
    .await?;
    let now = database_now(transaction).await?;
    transaction
        .execute(
            "UPDATE public.outbox SET status='delivered',delivered_at=$2,updated_at=$2, \
             claimed_by=NULL,claim_expires_at=NULL,last_error_code=$3 WHERE outbox_id=$1",
            &[&claim.outbox_id(), &now, &code.as_str()],
        )
        .await
        .map_err(|error| unavailable("收敛 rejected dispatch outbox", error))?;
    Ok(receipt)
}

async fn append_chunk_in_transaction(
    transaction: &Transaction<'_>,
    owner: &str,
    lease: &RunExecutionLease,
    expected_sequence: u64,
    channel: RunSemanticChannel,
    chunk: &str,
) -> Result<RunWriteReceipt, RunRuntimeError> {
    let expected = sequence_i64(expected_sequence)?;
    let payload = json!({"channel":channel.as_str(),"delta":chunk});
    let Some(locked) = lock_run(transaction, lease.run_id().as_str()).await? else {
        return Err(RunRuntimeError::StaleLease);
    };
    validate_lease_identity(&locked, lease)?;
    if locked.next_event_seq > expected {
        return replay_event(
            transaction,
            lease,
            expected,
            "semantic_chunk",
            &payload,
            false,
        )
        .await?
        .ok_or(RunRuntimeError::Conflict);
    }
    if locked.next_event_seq != expected || locked.status != "running" {
        return Err(RunRuntimeError::Conflict);
    }
    let now = database_now(transaction).await?;
    validate_active_lease(&locked, owner, lease, now)?;
    let thread_event = locked.thread_next_event_seq;
    let next_run = checked_increment(expected, "next_event_sequence")?;
    let next_thread = checked_increment(thread_event, "thread_next_event_sequence")?;
    transaction
        .execute(
            "INSERT INTO public.run_events( \
               run_id,seq,thread_id,event_seq,event_type,payload,terminal,created_at \
             ) VALUES($1,$2,$3,$4,'semantic_chunk',$5,false,$6)",
            &[
                &lease.run_id().as_str(),
                &expected,
                &lease.thread_id().as_str(),
                &thread_event,
                &payload,
                &now,
            ],
        )
        .await
        .map_err(|error| write_error("写 semantic chunk", error))?;
    transaction
        .execute(
            "UPDATE public.runs SET next_event_seq=$2 WHERE run_id=$1",
            &[&lease.run_id().as_str(), &next_run],
        )
        .await
        .map_err(|error| write_error("推进 run event sequence", error))?;
    transaction
        .execute(
            "UPDATE public.threads SET next_event_seq=$2,updated_at=$3 WHERE thread_id=$1",
            &[&lease.thread_id().as_str(), &next_thread, &now],
        )
        .await
        .map_err(|error| write_error("推进 thread event sequence", error))?;
    notify_thread(transaction).await?;
    Ok(RunWriteReceipt {
        run_event_sequence: expected_sequence,
        thread_event_sequence: sequence_u64(thread_event, "thread_event_sequence")?,
        message_sequence: None,
        replayed: false,
    })
}

async fn append_tool_exchange_in_transaction(
    transaction: &Transaction<'_>,
    owner: &str,
    lease: &RunExecutionLease,
    expected_sequence: u64,
    exchange: &RunToolExchange,
) -> Result<RunWriteReceipt, RunRuntimeError> {
    let arguments =
        serde_json::to_vec(exchange.arguments()).map_err(|_| RunRuntimeError::InvalidInput {
            field: "tool_arguments",
        })?;
    if exchange.provider_call_id().len() > 1024 * 1024
        || exchange.name().len() > 256
        || arguments.len() > 1024 * 1024
        || exchange.result().len() > 1024 * 1024
        || exchange.error_code().is_some_and(|value| value.len() > 256)
    {
        return Err(RunRuntimeError::InvalidInput {
            field: "tool_exchange",
        });
    }
    let expected = sequence_i64(expected_sequence)?;
    let payload = json!({
        "kind":"tool_exchange",
        "toolCallId":exchange.internal_call_id().as_str(),
        "providerCallIdHash":Sha256Digest::of(exchange.provider_call_id().as_bytes()).to_hex(),
        "toolName":exchange.name(),
        "argumentsHash":Sha256Digest::of(&arguments).to_hex(),
        "resultHash":Sha256Digest::of(exchange.result().as_bytes()).to_hex(),
        "errorCode":exchange.error_code(),
    });
    let Some(locked) = lock_run(transaction, lease.run_id().as_str()).await? else {
        return Err(RunRuntimeError::StaleLease);
    };
    validate_lease_identity(&locked, lease)?;
    if locked.next_event_seq > expected {
        let mut receipt = replay_event(transaction, lease, expected, "checkpoint", &payload, false)
            .await?
            .ok_or(RunRuntimeError::Conflict)?;
        receipt.message_sequence = replay_tool_messages(transaction, lease, exchange).await?;
        return Ok(receipt);
    }
    if locked.next_event_seq != expected || locked.status != "running" {
        return Err(RunRuntimeError::Conflict);
    }
    let now = database_now(transaction).await?;
    validate_active_lease(&locked, owner, lease, now)?;
    let assistant_text = aggregate_text_chunks(transaction, lease.run_id()).await?;
    let assistant_sequence = locked.thread_next_message_seq;
    let tool_sequence = checked_increment(assistant_sequence, "tool_message_sequence")?;
    let next_message = checked_increment(tool_sequence, "next_message_sequence")?;
    let assistant_id =
        tool_assistant_message_id(lease.run_id(), exchange.internal_call_id().as_str());
    let result_id = tool_result_message_id(lease.run_id(), exchange.internal_call_id().as_str());
    let assistant_content = json!({
        "text":assistant_text,
        "toolCalls":[{
            "id":exchange.provider_call_id(),
            "type":"function",
            "function":{
                "name":exchange.name(),
                "arguments":exchange.arguments(),
            }
        }]
    });
    let result_content = json!({
        "text":exchange.result(),
        "toolCallId":exchange.provider_call_id(),
        "toolName":exchange.name(),
        "errorCode":exchange.error_code(),
    });
    transaction
        .execute(
            "INSERT INTO public.messages( \
               message_id,thread_id,seq,role,content,search_text,run_id,actor_id,created_at \
             ) VALUES($1,$2,$3,'assistant',$4,$5,$6,NULL,$7), \
                     ($8,$2,$9,'tool',$10,$11,$6,NULL,$7)",
            &[
                &assistant_id,
                &lease.thread_id().as_str(),
                &assistant_sequence,
                &assistant_content,
                &assistant_text,
                &lease.run_id().as_str(),
                &now,
                &result_id,
                &tool_sequence,
                &result_content,
                &exchange.result(),
            ],
        )
        .await
        .map_err(|error| write_error("写 durable tool pair", error))?;
    let thread_event = locked.thread_next_event_seq;
    let next_run = checked_increment(expected, "next_event_sequence")?;
    let next_thread = checked_increment(thread_event, "thread_next_event_sequence")?;
    transaction
        .execute(
            "INSERT INTO public.run_events( \
               run_id,seq,thread_id,event_seq,event_type,payload,terminal,created_at \
             ) VALUES($1,$2,$3,$4,'checkpoint',$5,false,$6)",
            &[
                &lease.run_id().as_str(),
                &expected,
                &lease.thread_id().as_str(),
                &thread_event,
                &payload,
                &now,
            ],
        )
        .await
        .map_err(|error| write_error("写 tool exchange checkpoint", error))?;
    transaction
        .execute(
            "UPDATE public.runs SET next_event_seq=$2 WHERE run_id=$1",
            &[&lease.run_id().as_str(), &next_run],
        )
        .await
        .map_err(|error| write_error("推进 tool exchange run sequence", error))?;
    transaction
        .execute(
            "UPDATE public.threads SET next_event_seq=$2,next_message_seq=$3,updated_at=$4 \
             WHERE thread_id=$1",
            &[
                &lease.thread_id().as_str(),
                &next_thread,
                &next_message,
                &now,
            ],
        )
        .await
        .map_err(|error| write_error("推进 tool exchange thread sequence", error))?;
    notify_thread(transaction).await?;
    Ok(RunWriteReceipt {
        run_event_sequence: expected_sequence,
        thread_event_sequence: sequence_u64(thread_event, "thread_event_sequence")?,
        message_sequence: Some(sequence_u64(
            assistant_sequence,
            "assistant_message_sequence",
        )?),
        replayed: false,
    })
}

async fn replay_tool_messages(
    transaction: &Transaction<'_>,
    lease: &RunExecutionLease,
    exchange: &RunToolExchange,
) -> Result<Option<u64>, RunRuntimeError> {
    let assistant_id =
        tool_assistant_message_id(lease.run_id(), exchange.internal_call_id().as_str());
    let result_id = tool_result_message_id(lease.run_id(), exchange.internal_call_id().as_str());
    let message_ids = vec![assistant_id.clone(), result_id.clone()];
    let rows = transaction
        .query(
            "SELECT message_id,seq,role,content FROM public.messages \
             WHERE thread_id=$1 AND run_id=$2 AND message_id=ANY($3) ORDER BY seq",
            &[
                &lease.thread_id().as_str(),
                &lease.run_id().as_str(),
                &message_ids,
            ],
        )
        .await
        .map_err(|error| unavailable("核对 durable tool pair", error))?;
    if rows.len() != 2 {
        return Err(RunRuntimeError::Corrupt {
            field: "tool_messages",
        });
    }
    let first_id: String = decode(&rows[0], "message_id")?;
    let first_role: String = decode(&rows[0], "role")?;
    let first_content: Value = decode(&rows[0], "content")?;
    let second_id: String = decode(&rows[1], "message_id")?;
    let second_role: String = decode(&rows[1], "role")?;
    let second_content: Value = decode(&rows[1], "content")?;
    let first_call = first_content
        .get("toolCalls")
        .and_then(Value::as_array)
        .filter(|calls| calls.len() == 1)
        .and_then(|calls| calls.first());
    if first_id != assistant_id
        || first_role != "assistant"
        || second_id != result_id
        || second_role != "tool"
        || first_call
            .and_then(|call| call.get("id"))
            .and_then(Value::as_str)
            != Some(exchange.provider_call_id())
        || first_call
            .and_then(|call| call.get("function"))
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
            != Some(exchange.name())
        || first_call
            .and_then(|call| call.get("function"))
            .and_then(|function| function.get("arguments"))
            != Some(exchange.arguments())
        || second_content.get("toolCallId").and_then(Value::as_str)
            != Some(exchange.provider_call_id())
        || second_content.get("toolName").and_then(Value::as_str) != Some(exchange.name())
        || second_content.get("text").and_then(Value::as_str) != Some(exchange.result())
        || second_content.get("errorCode").and_then(Value::as_str) != exchange.error_code()
    {
        return Err(RunRuntimeError::Corrupt {
            field: "tool_messages",
        });
    }
    let sequence: i64 = decode(&rows[0], "seq")?;
    Ok(Some(sequence_u64(sequence, "assistant_message_sequence")?))
}

#[derive(Clone, Copy)]
enum LeaseCheck {
    Active,
    Recovery,
}

async fn finish_run_in_transaction(
    transaction: &Transaction<'_>,
    owner: &str,
    lease: &RunExecutionLease,
    expected_sequence: u64,
    terminal: RunTerminal,
    lease_check: LeaseCheck,
) -> Result<RunWriteReceipt, RunRuntimeError> {
    let expected = sequence_i64(expected_sequence)?;
    let Some(locked) = lock_run(transaction, lease.run_id().as_str()).await? else {
        return Err(RunRuntimeError::StaleLease);
    };
    validate_lease_identity(&locked, lease)?;
    if locked.next_event_seq > expected || is_terminal_status(&locked.status) {
        return replay_terminal(transaction, lease, expected_sequence, terminal)
            .await?
            .ok_or(RunRuntimeError::Conflict);
    }
    if locked.next_event_seq != expected || locked.status != "running" {
        return Err(RunRuntimeError::Conflict);
    }
    let now = database_now(transaction).await?;
    if matches!(lease_check, LeaseCheck::Active) {
        validate_active_lease(&locked, owner, lease, now)?;
    }

    let payload = terminal_payload(terminal);
    let thread_event = locked.thread_next_event_seq;
    let next_run = checked_increment(expected, "next_event_sequence")?;
    let next_thread = checked_increment(thread_event, "thread_next_event_sequence")?;
    let assistant_text = aggregate_text_chunks(transaction, lease.run_id()).await?;
    let message_sequence = if assistant_text.is_empty() {
        None
    } else {
        let message_seq = locked.thread_next_message_seq;
        let next_message = checked_increment(message_seq, "thread_next_message_sequence")?;
        let message_id = assistant_message_id(lease.run_id());
        let content = json!({
            "text": assistant_text,
            "incomplete": !matches!(terminal, RunTerminal::Completed),
        });
        transaction
            .execute(
                "INSERT INTO public.messages( \
                   message_id,thread_id,seq,role,content,search_text,run_id,actor_id,created_at \
                 ) VALUES($1,$2,$3,'assistant',$4,$5,$6,NULL,$7)",
                &[
                    &message_id,
                    &lease.thread_id().as_str(),
                    &message_seq,
                    &content,
                    &assistant_text,
                    &lease.run_id().as_str(),
                    &now,
                ],
            )
            .await
            .map_err(|error| write_error("materialize assistant message", error))?;
        transaction
            .execute(
                "UPDATE public.threads SET next_message_seq=$2 WHERE thread_id=$1",
                &[&lease.thread_id().as_str(), &next_message],
            )
            .await
            .map_err(|error| write_error("推进 assistant message sequence", error))?;
        Some(sequence_u64(message_seq, "message_sequence")?)
    };

    transaction
        .execute(
            "INSERT INTO public.run_events( \
               run_id,seq,thread_id,event_seq,event_type,payload,terminal,created_at \
             ) VALUES($1,$2,$3,$4,$5,$6,true,$7)",
            &[
                &lease.run_id().as_str(),
                &expected,
                &lease.thread_id().as_str(),
                &thread_event,
                &terminal.as_str(),
                &payload,
                &now,
            ],
        )
        .await
        .map_err(|error| write_error("写 terminal run event", error))?;
    let error_code = terminal.error_code().map(RunFailureCode::as_str);
    transaction
        .execute(
            "UPDATE public.runs SET status=$2,next_event_seq=$3,terminal_event_seq=$4, \
             error_code=$5,finished_at=$6 WHERE run_id=$1",
            &[
                &lease.run_id().as_str(),
                &terminal.as_str(),
                &next_run,
                &expected,
                &error_code,
                &now,
            ],
        )
        .await
        .map_err(|error| write_error("terminalize run", error))?;
    transaction
        .execute(
            "UPDATE public.threads SET next_event_seq=$2,updated_at=$3 WHERE thread_id=$1",
            &[&lease.thread_id().as_str(), &next_thread, &now],
        )
        .await
        .map_err(|error| write_error("推进 terminal thread sequence", error))?;
    release_lease(transaction, lease.thread_id(), now).await?;
    notify_thread(transaction).await?;
    Ok(RunWriteReceipt {
        run_event_sequence: expected_sequence,
        thread_event_sequence: sequence_u64(thread_event, "thread_event_sequence")?,
        message_sequence,
        replayed: false,
    })
}

async fn recover_one_in_transaction(
    transaction: &Transaction<'_>,
    owner: &str,
) -> Result<Option<RunWriteReceipt>, RunRuntimeError> {
    let now = database_now(transaction).await?;
    let row = transaction
        .query_opt(
            "SELECT r.run_id,o.status AS outbox_status \
             FROM public.runs r \
             JOIN public.thread_leases l ON l.thread_id=r.thread_id \
             JOIN public.outbox o ON o.outbox_id=r.run_id || ':agent_run_dispatch' \
             WHERE r.status='running' AND l.expires_at<=$1 \
               AND o.status IN ('delivered','dead_letter') \
             ORDER BY l.expires_at,r.run_id \
             FOR UPDATE OF r,l,o SKIP LOCKED LIMIT 1",
            &[&now],
        )
        .await
        .map_err(|error| unavailable("选择 stale running run", error))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let run_id: String = decode(&row, "run_id")?;
    let outbox_status: String = decode(&row, "outbox_status")?;
    let Some(mut locked) = lock_run(transaction, &run_id).await? else {
        return Err(RunRuntimeError::Corrupt { field: "stale_run" });
    };
    if locked.status != "running" || locked.lease_expires_at > now {
        return Ok(None);
    }
    if let Some(next) = locked.lease_fencing.checked_add(1) {
        let recovery_expiry = checked_add(now, Duration::microseconds(1), "recovery_expiry")?;
        transaction
            .execute(
                "UPDATE public.thread_leases SET owner_id=$2,fencing_token=$3,acquired_at=$4, \
                 expires_at=$5,updated_at=$4 WHERE thread_id=$1",
                &[&locked.thread_id, &owner, &next, &now, &recovery_expiry],
            )
            .await
            .map_err(|error| unavailable("接管 stale run fencing", error))?;
        transaction
            .execute(
                "UPDATE public.runs SET fencing_token=$2 WHERE run_id=$1 AND status='running'",
                &[&run_id, &next],
            )
            .await
            .map_err(|error| unavailable("重绑 stale run fencing", error))?;
        locked.fencing = next;
        locked.lease_fencing = next;
        locked.lease_owner = owner.to_owned();
    }
    let lease = lease_from_locked(&locked)?;
    let terminal = if outbox_status == "delivered" {
        RunTerminal::ReconciliationRequired(RunFailureCode::RuntimeLeaseExpired)
    } else {
        RunTerminal::Failed(RunFailureCode::DispatchDeadLetter)
    };
    finish_run_in_transaction(
        transaction,
        owner,
        &lease,
        lease.next_event_sequence(),
        terminal,
        LeaseCheck::Recovery,
    )
    .await
    .map(Some)
}

struct LockedRun {
    run_id: String,
    thread_id: String,
    bot_id: String,
    actor_id: String,
    status: String,
    fencing: i64,
    next_event_seq: i64,
    thread_next_event_seq: i64,
    thread_next_message_seq: i64,
    lease_owner: String,
    lease_fencing: i64,
    lease_expires_at: OffsetDateTime,
}

async fn lock_run(
    transaction: &Transaction<'_>,
    run_id: &str,
) -> Result<Option<LockedRun>, RunRuntimeError> {
    let row = transaction
        .query_opt(
            "SELECT r.run_id,r.thread_id,r.bot_id,r.actor_id,r.status,r.fencing_token, \
                    r.next_event_seq,t.next_event_seq AS thread_next_event_seq, \
                    t.next_message_seq AS thread_next_message_seq,l.owner_id AS lease_owner, \
                    l.fencing_token AS lease_fencing,l.expires_at AS lease_expires_at \
             FROM public.runs r \
             JOIN public.threads t ON t.thread_id=r.thread_id \
             JOIN public.thread_leases l ON l.thread_id=r.thread_id \
             WHERE r.run_id=$1 FOR UPDATE OF r,t,l",
            &[&run_id],
        )
        .await
        .map_err(|error| unavailable("锁定 run/thread/lease", error))?;
    row.as_ref().map(decode_locked_run).transpose()
}

fn decode_locked_run(row: &Row) -> Result<LockedRun, RunRuntimeError> {
    Ok(LockedRun {
        run_id: decode(row, "run_id")?,
        thread_id: decode(row, "thread_id")?,
        bot_id: decode(row, "bot_id")?,
        actor_id: decode(row, "actor_id")?,
        status: decode(row, "status")?,
        fencing: decode(row, "fencing_token")?,
        next_event_seq: decode(row, "next_event_seq")?,
        thread_next_event_seq: decode(row, "thread_next_event_seq")?,
        thread_next_message_seq: decode(row, "thread_next_message_seq")?,
        lease_owner: decode(row, "lease_owner")?,
        lease_fencing: decode(row, "lease_fencing")?,
        lease_expires_at: decode(row, "lease_expires_at")?,
    })
}

fn lease_from_locked(locked: &LockedRun) -> Result<RunExecutionLease, RunRuntimeError> {
    RunExecutionLease::new(
        RunId::new(&locked.run_id),
        ThreadId::new(&locked.thread_id),
        BotId::new(&locked.bot_id),
        ActorId::new(&locked.actor_id),
        FencingToken::new(locked.fencing).map_err(|_| RunRuntimeError::Corrupt {
            field: "fencing_token",
        })?,
        sequence_u64(locked.next_event_seq, "next_event_sequence")?,
    )
}

fn validate_claim_binding(
    locked: &LockedRun,
    claim: &ClaimedRunDispatch,
) -> Result<(), RunRuntimeError> {
    validate_lease_identity(locked, claim.lease())?;
    if locked.status != "running" {
        return Err(RunRuntimeError::StaleLease);
    }
    Ok(())
}

fn validate_lease_identity(
    locked: &LockedRun,
    lease: &RunExecutionLease,
) -> Result<(), RunRuntimeError> {
    if locked.run_id != lease.run_id().as_str()
        || locked.thread_id != lease.thread_id().as_str()
        || locked.bot_id != lease.bot_id().as_str()
        || locked.actor_id != lease.actor_id().as_str()
        || locked.fencing != lease.fencing().get()
    {
        return Err(RunRuntimeError::StaleLease);
    }
    Ok(())
}

fn validate_active_lease(
    locked: &LockedRun,
    owner: &str,
    lease: &RunExecutionLease,
    now: OffsetDateTime,
) -> Result<(), RunRuntimeError> {
    if locked.lease_owner != owner
        || locked.lease_fencing != lease.fencing().get()
        || locked.lease_expires_at <= now
    {
        return Err(RunRuntimeError::StaleLease);
    }
    Ok(())
}

async fn replay_event(
    transaction: &Transaction<'_>,
    lease: &RunExecutionLease,
    sequence: i64,
    event_type: &str,
    payload: &Value,
    terminal: bool,
) -> Result<Option<RunWriteReceipt>, RunRuntimeError> {
    let row = transaction
        .query_opt(
            "SELECT event_seq,event_type,payload,terminal FROM public.run_events \
             WHERE run_id=$1 AND seq=$2",
            &[&lease.run_id().as_str(), &sequence],
        )
        .await
        .map_err(|error| unavailable("核对 exact run event replay", error))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let stored_type: String = decode(&row, "event_type")?;
    let stored_payload: Value = decode(&row, "payload")?;
    let stored_terminal: bool = decode(&row, "terminal")?;
    if stored_type != event_type || stored_payload != *payload || stored_terminal != terminal {
        return Ok(None);
    }
    Ok(Some(RunWriteReceipt {
        run_event_sequence: sequence_u64(sequence, "run_event_sequence")?,
        thread_event_sequence: sequence_u64(decode(&row, "event_seq")?, "event_sequence")?,
        message_sequence: None,
        replayed: true,
    }))
}

async fn replay_terminal(
    transaction: &Transaction<'_>,
    lease: &RunExecutionLease,
    expected_sequence: u64,
    terminal: RunTerminal,
) -> Result<Option<RunWriteReceipt>, RunRuntimeError> {
    let expected = sequence_i64(expected_sequence)?;
    let payload = terminal_payload(terminal);
    let Some(mut receipt) = replay_event(
        transaction,
        lease,
        expected,
        terminal.as_str(),
        &payload,
        true,
    )
    .await?
    else {
        return Ok(None);
    };
    let message_id = assistant_message_id(lease.run_id());
    let materialized = transaction
        .query_opt(
            "SELECT seq,content FROM public.messages \
             WHERE message_id=$1 AND thread_id=$2 AND run_id=$3 AND role='assistant'",
            &[
                &message_id,
                &lease.thread_id().as_str(),
                &lease.run_id().as_str(),
            ],
        )
        .await
        .map_err(|error| unavailable("核对 terminal assistant message", error))?
        .map(|row| {
            let sequence = row
                .try_get::<_, i64>("seq")
                .map_err(|_| RunRuntimeError::Corrupt {
                    field: "message_sequence",
                })
                .and_then(|value| sequence_u64(value, "message_sequence"))?;
            let content: Value = decode(&row, "content")?;
            Ok((sequence, content))
        })
        .transpose()?;
    let expected_text = aggregate_text_chunks(transaction, lease.run_id()).await?;
    receipt.message_sequence = match (expected_text.is_empty(), materialized) {
        (true, None) => None,
        (false, Some((sequence, content)))
            if content.get("text").and_then(Value::as_str) == Some(expected_text.as_str())
                && content.get("incomplete").and_then(Value::as_bool)
                    == Some(!matches!(terminal, RunTerminal::Completed)) =>
        {
            Some(sequence)
        }
        _ => {
            return Err(RunRuntimeError::Corrupt {
                field: "terminal_assistant_message",
            });
        }
    };
    Ok(Some(receipt))
}

async fn aggregate_text_chunks(
    transaction: &Transaction<'_>,
    run_id: &RunId,
) -> Result<String, RunRuntimeError> {
    let rows = transaction
        .query(
            "SELECT payload FROM public.run_events \
             WHERE run_id=$1 AND event_type='semantic_chunk' \
               AND seq>coalesce((SELECT max(seq) FROM public.run_events \
                                 WHERE run_id=$1 AND event_type='checkpoint' \
                                   AND payload->>'kind'='tool_exchange'),-1) \
             ORDER BY seq",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|error| unavailable("读取 semantic chunks", error))?;
    let mut text = String::new();
    for row in rows {
        let payload: Value = decode(&row, "payload")?;
        if payload.get("channel").and_then(Value::as_str) != Some("text") {
            continue;
        }
        let delta =
            payload
                .get("delta")
                .and_then(Value::as_str)
                .ok_or(RunRuntimeError::Corrupt {
                    field: "semantic_chunk_delta",
                })?;
        text.push_str(delta);
    }
    Ok(text)
}

fn tool_assistant_message_id(run_id: &RunId, call_id: &str) -> String {
    format!("{}:tool:{call_id}:assistant", run_id.as_str())
}

fn tool_result_message_id(run_id: &RunId, call_id: &str) -> String {
    format!("{}:tool:{call_id}:result", run_id.as_str())
}

async fn release_lease(
    transaction: &Transaction<'_>,
    thread: &ThreadId,
    now: OffsetDateTime,
) -> Result<(), RunRuntimeError> {
    transaction
        .execute(
            "UPDATE public.thread_leases SET \
             expires_at=greatest($2,acquired_at + interval '1 microsecond'),updated_at=$2 \
             WHERE thread_id=$1",
            &[&thread.as_str(), &now],
        )
        .await
        .map(|_| ())
        .map_err(|error| unavailable("释放 terminal run lease", error))
}

async fn dead_letter_corrupt(
    transaction: &Transaction<'_>,
    outbox_id: &str,
    now: OffsetDateTime,
) -> Result<(), RunRuntimeError> {
    transaction
        .execute(
            "UPDATE public.outbox SET status='dead_letter',claimed_by=NULL,claim_expires_at=NULL, \
             last_error_code=$2,updated_at=$3 WHERE outbox_id=$1",
            &[
                &outbox_id,
                &RunFailureCode::DispatchPayloadCorrupt.as_str(),
                &now,
            ],
        )
        .await
        .map(|_| ())
        .map_err(|error| unavailable("dead-letter corrupt dispatch", error))
}

async fn dead_letter_dispatch(
    transaction: &Transaction<'_>,
    outbox_id: &str,
    now: OffsetDateTime,
    field: &'static str,
) -> Result<Option<ClaimedRunDispatch>, RunRuntimeError> {
    dead_letter_corrupt(transaction, outbox_id, now).await?;
    tracing::error!(field, "agent dispatch durable binding 损坏；已 dead-letter");
    Ok(None)
}

async fn notify_thread(transaction: &Transaction<'_>) -> Result<(), RunRuntimeError> {
    transaction
        .query_one("SELECT pg_notify($1,'')", &[&THREAD_EVENT_TOPIC])
        .await
        .map(|_| ())
        .map_err(|error| unavailable("通知 thread event", error))
}

fn terminal_payload(terminal: RunTerminal) -> Value {
    match terminal.error_code() {
        Some(code) => json!({"status":terminal.as_str(),"errorCode":code.as_str()}),
        None => json!({"status":terminal.as_str()}),
    }
}

fn assistant_message_id(run_id: &RunId) -> String {
    format!("{}{ASSISTANT_MESSAGE_SUFFIX}", run_id.as_str())
}

fn validate_chunk(chunk: &str) -> Result<(), RunRuntimeError> {
    if chunk.is_empty() || chunk.len() > SEMANTIC_CHUNK_MAX_BYTES || chunk.as_bytes().contains(&0) {
        Err(RunRuntimeError::InvalidInput {
            field: "semantic_chunk",
        })
    } else {
        Ok(())
    }
}

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "completed" | "failed" | "cancelled" | "reconciliation_required"
    )
}

fn sequence_i64(value: u64) -> Result<i64, RunRuntimeError> {
    i64::try_from(value).map_err(|_| RunRuntimeError::InvalidInput {
        field: "event_sequence",
    })
}

fn sequence_u64(value: i64, field: &'static str) -> Result<u64, RunRuntimeError> {
    u64::try_from(value).map_err(|_| RunRuntimeError::Corrupt { field })
}

fn checked_increment(value: i64, field: &'static str) -> Result<i64, RunRuntimeError> {
    value
        .checked_add(1)
        .ok_or(RunRuntimeError::Corrupt { field })
}

fn checked_add(
    value: OffsetDateTime,
    duration: Duration,
    field: &'static str,
) -> Result<OffsetDateTime, RunRuntimeError> {
    value
        .checked_add(duration)
        .ok_or(RunRuntimeError::Corrupt { field })
}

async fn database_now(transaction: &Transaction<'_>) -> Result<OffsetDateTime, RunRuntimeError> {
    transaction
        .query_one("SELECT now()", &[])
        .await
        .map_err(|error| unavailable("读取 run runtime 数据库时钟", error))?
        .try_get(0)
        .map_err(|_| RunRuntimeError::Corrupt {
            field: "database_now",
        })
}

async fn database_now_client(
    client: &deadpool_postgres::Client,
) -> Result<OffsetDateTime, RunRuntimeError> {
    client
        .query_one("SELECT now()", &[])
        .await
        .map_err(|error| unavailable("读取 run lease 数据库时钟", error))?
        .try_get(0)
        .map_err(|_| RunRuntimeError::Corrupt {
            field: "database_now",
        })
}

async fn finish_transaction<T>(
    transaction: deadpool_postgres::Transaction<'_>,
    result: Result<T, RunRuntimeError>,
) -> Result<T, RunRuntimeError> {
    match result {
        Ok(value) => {
            transaction.commit().await.map_err(|error| {
                tracing::error!(error = %error, "run runtime commit 结果未知");
                RunRuntimeError::CommitUnknown
            })?;
            Ok(value)
        }
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

fn decode<T>(row: &Row, column: &'static str) -> Result<T, RunRuntimeError>
where
    T: tokio_postgres::types::FromSqlOwned,
{
    row.try_get(column)
        .map_err(|_| RunRuntimeError::Corrupt { field: column })
}

fn unavailable(context: &'static str, error: tokio_postgres::Error) -> RunRuntimeError {
    tracing::error!(
        sqlstate = error.code().map_or("none", SqlState::code),
        connection_closed = error.is_closed(),
        context,
        "run runtime database operation failed"
    );
    RunRuntimeError::Unavailable
}

fn write_error(context: &'static str, error: tokio_postgres::Error) -> RunRuntimeError {
    tracing::error!(
        sqlstate = error.code().map_or("none", SqlState::code),
        connection_closed = error.is_closed(),
        context,
        "run runtime transaction write failed"
    );
    match error.code() {
        Some(code) if code == &SqlState::UNIQUE_VIOLATION => RunRuntimeError::Conflict,
        Some(code) if code == &SqlState::FOREIGN_KEY_VIOLATION => RunRuntimeError::StaleLease,
        _ => RunRuntimeError::Unavailable,
    }
}
