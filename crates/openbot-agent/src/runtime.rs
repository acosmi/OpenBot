//! Built-in Agent dispatch consumer/host；pure decisions remain in `openbot-domain::agent`。

use std::collections::BTreeMap;
use std::future::{Future, pending};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use openbot_application::{
    AgentAudit, AgentAuditKind, AgentContextError, AgentContextSource, DurableTextRun,
    ProviderAdapter, ProviderEvent, ProviderFailure, ProviderPortError, ProviderRateCard,
    RunCancellationDisposition, RunDispatchConsumer, RunDispatchDecision, RunExecutionLease,
    RunFailureCode, RunRuntime, RunRuntimeError, RunTerminal, RunTokenUsageReceipt,
    RunToolExchange,
};
use openbot_contracts::components::is_component_human_decision_name;
use openbot_domain::agent::{
    AgentEffect, AgentEvent, AgentFailure, AgentState, AgentTerminal, AgentToolCall, reduce,
};
use tokio::sync::{Mutex, Semaphore, mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};

use crate::gateway::{AgentToolInvokeError, AgentToolInvoker};

/// Upstream parity tool sampling step cap。
pub const TOOL_STEP_CAP: u8 = 8;
/// New absolute run deadline default（v3 §7.2）。
pub const DEFAULT_RUN_DEADLINE: Duration = Duration::from_millis(1_800_000);
/// Thread lease 30s 的三分之一。
pub const DEFAULT_LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(10);
/// Bounded shutdown（v3 §13.2）。
pub const AGENT_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);
/// Default reservation queue。
pub const DEFAULT_AGENT_QUEUE_CAPACITY: usize = 256;
/// Default simultaneous provider runs；独立于 per-thread foreground unique index。
pub const DEFAULT_AGENT_CONCURRENCY: usize = 8;

/// Built-in runtime configuration。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltInAgentConfig {
    /// Reserved + active runs cap。
    pub queue_capacity: usize,
    /// Provider concurrency。
    pub max_concurrency: usize,
    /// Lease heartbeat。
    pub lease_renew_interval: Duration,
    /// `None` = explicitly disabled by OPENBOT_RUN_DEADLINE_MS=0。
    pub run_deadline: Option<Duration>,
}

impl Default for BuiltInAgentConfig {
    fn default() -> Self {
        Self {
            queue_capacity: DEFAULT_AGENT_QUEUE_CAPACITY,
            max_concurrency: DEFAULT_AGENT_CONCURRENCY,
            lease_renew_interval: DEFAULT_LEASE_RENEW_INTERVAL,
            run_deadline: Some(DEFAULT_RUN_DEADLINE),
        }
    }
}

impl BuiltInAgentConfig {
    fn validate(self) -> Result<Self, RunFailureCode> {
        if self.queue_capacity == 0
            || self.queue_capacity > 4096
            || self.max_concurrency == 0
            || self.max_concurrency > self.queue_capacity
            || self.lease_renew_interval.is_zero()
            || self.run_deadline.is_some_and(|value| value.is_zero())
        {
            Err(RunFailureCode::AgentRuntimeUnavailable)
        } else {
            Ok(self)
        }
    }
}

/// Lifecycle owner；consumer clone is injected into RunRelay，handle stays in Server main。
pub struct BuiltInAgentRuntime {
    consumer: Arc<BuiltInAgentConsumer>,
    stop: watch::Sender<bool>,
    supervisor: JoinHandle<()>,
}

impl core::fmt::Debug for BuiltInAgentRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BuiltInAgentRuntime")
            .finish_non_exhaustive()
    }
}

impl BuiltInAgentRuntime {
    /// Start bounded supervisor。
    pub fn start(
        runtime: Arc<dyn RunRuntime>,
        context: Arc<dyn AgentContextSource>,
        provider: Arc<dyn ProviderAdapter>,
        tools: Arc<dyn AgentToolInvoker>,
        audit: Arc<dyn AgentAudit>,
        config: BuiltInAgentConfig,
    ) -> Result<Self, RunFailureCode> {
        let config = config.validate()?;
        let (sender, receiver) = mpsc::channel(config.queue_capacity);
        let (stop, stop_rx) = watch::channel(false);
        let inner = Arc::new(Inner {
            runtime,
            context,
            provider,
            tools,
            audit,
            config,
            sender,
            reservations: Mutex::new(BTreeMap::new()),
            semaphore: Arc::new(Semaphore::new(config.max_concurrency)),
            stopped: AtomicBool::new(false),
        });
        let consumer = Arc::new(BuiltInAgentConsumer {
            inner: inner.clone(),
        });
        let supervisor = tokio::spawn(supervise(inner, receiver, stop_rx));
        Ok(Self {
            consumer,
            stop,
            supervisor,
        })
    }

    /// Consumer for production RunRelay。
    #[must_use]
    pub fn consumer(&self) -> Arc<dyn RunDispatchConsumer> {
        self.consumer.clone()
    }

    /// Cancel every active run and wait at most 5 seconds。
    pub async fn stop(self) {
        self.consumer.inner.stopped.store(true, Ordering::SeqCst);
        self.stop.send_replace(true);
        let _ = self.supervisor.await;
    }
}

/// Idempotent reserve/activate/revoke implementation。
pub struct BuiltInAgentConsumer {
    inner: Arc<Inner>,
}

impl core::fmt::Debug for BuiltInAgentConsumer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BuiltInAgentConsumer")
            .field("config", &self.inner.config)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl RunDispatchConsumer for BuiltInAgentConsumer {
    async fn dispatch(&self, lease: RunExecutionLease) -> RunDispatchDecision {
        if self.inner.stopped.load(Ordering::SeqCst) {
            return RunDispatchDecision::Rejected(RunFailureCode::AgentRuntimeUnavailable);
        }
        let mut reservations = self.inner.reservations.lock().await;
        let key = lease.run_id().as_str().to_owned();
        if let Some(existing) = reservations.get(&key) {
            return if existing.lease == lease {
                RunDispatchDecision::Accepted
            } else {
                RunDispatchDecision::Rejected(RunFailureCode::DispatchPayloadCorrupt)
            };
        }
        if reservations.len() >= self.inner.config.queue_capacity {
            return RunDispatchDecision::RetryableBusy;
        }
        let (cancel, _) = watch::channel(false);
        reservations.insert(
            key,
            Reservation {
                lease,
                cancel,
                active: false,
            },
        );
        RunDispatchDecision::Accepted
    }

    async fn activate(&self, lease: &RunExecutionLease) -> Result<(), RunFailureCode> {
        let mut reservations = self.inner.reservations.lock().await;
        let Some(reservation) = reservations.get_mut(lease.run_id().as_str()) else {
            return Err(RunFailureCode::AgentRuntimeUnavailable);
        };
        if reservation.lease != *lease {
            return Err(RunFailureCode::DispatchPayloadCorrupt);
        }
        if reservation.active {
            return Ok(());
        }
        let activation = Activation {
            lease: lease.clone(),
            cancel: reservation.cancel.subscribe(),
        };
        if self.inner.sender.try_send(activation).is_err() {
            reservations.remove(lease.run_id().as_str());
            return Err(RunFailureCode::AgentRuntimeUnavailable);
        }
        reservation.active = true;
        Ok(())
    }

    async fn revoke(&self, lease: &RunExecutionLease) -> RunCancellationDisposition {
        let mut reservations = self.inner.reservations.lock().await;
        if reservations
            .get(lease.run_id().as_str())
            .is_some_and(|reservation| reservation.lease == *lease)
            && let Some(reservation) = reservations.remove(lease.run_id().as_str())
        {
            reservation.cancel.send_replace(true);
            return RunCancellationDisposition::ChildSignalled;
        }
        RunCancellationDisposition::NoLocalChild
    }
}

struct Inner {
    runtime: Arc<dyn RunRuntime>,
    context: Arc<dyn AgentContextSource>,
    provider: Arc<dyn ProviderAdapter>,
    tools: Arc<dyn AgentToolInvoker>,
    audit: Arc<dyn AgentAudit>,
    config: BuiltInAgentConfig,
    sender: mpsc::Sender<Activation>,
    reservations: Mutex<BTreeMap<String, Reservation>>,
    semaphore: Arc<Semaphore>,
    stopped: AtomicBool,
}

struct Reservation {
    lease: RunExecutionLease,
    cancel: watch::Sender<bool>,
    active: bool,
}

struct Activation {
    lease: RunExecutionLease,
    cancel: watch::Receiver<bool>,
}

#[derive(Clone, Copy)]
enum CancellationSource {
    User,
    Deadline,
}

enum ControlledChild<T> {
    Ready(T),
    Cancelled(CancellationSource),
    LeaseLost,
}

async fn supervise(
    inner: Arc<Inner>,
    mut receiver: mpsc::Receiver<Activation>,
    mut stop: watch::Receiver<bool>,
) {
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            activation = receiver.recv() => match activation {
                Some(activation) => {
                    let inner = inner.clone();
                    tasks.spawn(async move {
                        execute_activation(inner.clone(), activation).await;
                    });
                }
                None => break,
            },
            result = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Err(error)) = result {
                    tracing::error!(error = %error, "built-in Agent task panic/cancel");
                }
            }
        }
    }
    inner.stopped.store(true, Ordering::SeqCst);
    {
        let reservations = inner.reservations.lock().await;
        for reservation in reservations.values() {
            reservation.cancel.send_replace(true);
        }
    }
    let deadline = tokio::time::Instant::now() + AGENT_SHUTDOWN_DEADLINE;
    while !tasks.is_empty() {
        if tokio::time::timeout_at(deadline, tasks.join_next())
            .await
            .is_err()
        {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            break;
        }
    }
    inner.reservations.lock().await.clear();
}

async fn execute_activation(inner: Arc<Inner>, mut activation: Activation) {
    if *activation.cancel.borrow() {
        cleanup(&inner, &activation.lease).await;
        return;
    }
    let permit = tokio::select! {
        changed = activation.cancel.changed() => {
            if changed.is_err() || *activation.cancel.borrow() {
                cleanup(&inner, &activation.lease).await;
                return;
            }
            return;
        }
        permit = inner.semaphore.clone().acquire_owned() => match permit {
            Ok(permit) => permit,
            Err(_) => {
                cleanup(&inner, &activation.lease).await;
                return;
            }
        }
    };
    let _permit = permit;
    execute_run(&inner, &mut activation).await;
    inner.tools.release(activation.lease.run_id());
    cleanup(&inner, &activation.lease).await;
}

async fn cleanup(inner: &Inner, lease: &RunExecutionLease) {
    let mut reservations = inner.reservations.lock().await;
    if reservations
        .get(lease.run_id().as_str())
        .is_some_and(|reservation| reservation.lease == *lease)
    {
        reservations.remove(lease.run_id().as_str());
    }
}

async fn execute_run(inner: &Inner, activation: &mut Activation) {
    let lease = &activation.lease;
    let mut state = AgentState::queued(lease.run_id().clone());
    let Ok((next, effects)) = reduce(&state, AgentEvent::DispatchActivated) else {
        return;
    };
    state = next;
    if !matches!(effects.as_slice(), [AgentEffect::LoadContext]) {
        return;
    }
    let mut journal = DurableTextRun::new(inner.runtime.clone(), lease.clone());
    let mut sampling_index = 0_u32;
    let mut run_output_ceiling = None::<Option<u64>>;
    let mut run_rate_card = None::<Option<ProviderRateCard>>;
    if inner
        .audit
        .record(lease, AgentAuditKind::Invoked)
        .await
        .is_err()
    {
        drive_terminal_event(&mut state, &mut journal, AgentEvent::JournalCommitUnknown).await;
        return;
    }
    let mut renew = tokio::time::interval(inner.config.lease_renew_interval);
    renew.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    renew.tick().await;
    let deadline = inner
        .config
        .run_deadline
        .map(|duration| tokio::time::Instant::now() + duration);
    loop {
        let request = match await_run_child(
            inner.context.load(lease),
            &mut activation.cancel,
            &mut renew,
            deadline,
            inner.runtime.as_ref(),
            lease,
        )
        .await
        {
            ControlledChild::Ready(Ok(request)) => request,
            ControlledChild::Ready(Err(error)) => {
                drive_terminal_event(
                    &mut state,
                    &mut journal,
                    AgentEvent::ContextFailed(context_failure(error)),
                )
                .await;
                return;
            }
            ControlledChild::Cancelled(source) => {
                cancel_and_commit(
                    &mut state,
                    &mut journal,
                    inner.audit.as_ref(),
                    lease,
                    source,
                )
                .await;
                return;
            }
            ControlledChild::LeaseLost => {
                drive_terminal_event(&mut state, &mut journal, AgentEvent::LeaseLost).await;
                return;
            }
        };
        let max_output_tokens = request.max_output_tokens;
        let candidate_rate_card = request.rate_card.clone();
        let candidate_run_output_ceiling =
            max_output_tokens.map(|limit| u64::from(limit) * (u64::from(TOOL_STEP_CAP) + 1));
        match run_output_ceiling {
            None => run_output_ceiling = Some(candidate_run_output_ceiling),
            Some(existing) if existing == candidate_run_output_ceiling => {}
            Some(_) => {
                drive_terminal_event(
                    &mut state,
                    &mut journal,
                    AgentEvent::ContextFailed(AgentFailure::ProviderInvalidResponse),
                )
                .await;
                return;
            }
        }
        match &run_rate_card {
            None => run_rate_card = Some(candidate_rate_card),
            Some(existing) if *existing == candidate_rate_card => {}
            Some(_) => {
                drive_terminal_event(
                    &mut state,
                    &mut journal,
                    AgentEvent::ContextFailed(AgentFailure::ProviderInvalidResponse),
                )
                .await;
                return;
            }
        }
        let Ok((next, effects)) = reduce(&state, AgentEvent::ContextPrepared) else {
            return;
        };
        state = next;
        if !matches!(effects.as_slice(), [AgentEffect::StartProvider]) {
            return;
        }
        let session = match await_run_child(
            inner.provider.start(request),
            &mut activation.cancel,
            &mut renew,
            deadline,
            inner.runtime.as_ref(),
            lease,
        )
        .await
        {
            ControlledChild::Ready(result) => result,
            ControlledChild::Cancelled(source) => {
                cancel_and_commit(
                    &mut state,
                    &mut journal,
                    inner.audit.as_ref(),
                    lease,
                    source,
                )
                .await;
                return;
            }
            ControlledChild::LeaseLost => {
                drive_terminal_event(&mut state, &mut journal, AgentEvent::LeaseLost).await;
                return;
            }
        };
        let mut session = match session {
            Ok(session) => session,
            Err(error) => {
                drive_terminal_event(
                    &mut state,
                    &mut journal,
                    AgentEvent::ProviderFailed(provider_port_failure(error)),
                )
                .await;
                return;
            }
        };
        let mut usage_seen = false;
        let mut pending_tools = BTreeMap::<u32, AgentToolCall>::new();
        let calls = loop {
            let chunk_deadline = journal.next_deadline().map(tokio::time::Instant::from_std);
            tokio::select! {
                changed = activation.cancel.changed() => {
                    if changed.is_err() || *activation.cancel.borrow() {
                        drop(session);
                        cancel_and_commit(
                            &mut state,
                            &mut journal,
                            inner.audit.as_ref(),
                            lease,
                            CancellationSource::User,
                        ).await;
                        return;
                    }
                }
                () = wait_optional(deadline) => {
                    drop(session);
                    cancel_and_commit(
                        &mut state,
                        &mut journal,
                        inner.audit.as_ref(),
                        lease,
                        CancellationSource::Deadline,
                    ).await;
                    return;
                }
                _ = renew.tick() => {
                    if inner.runtime.renew_lease(lease).await.is_err() {
                        drop(session);
                        drive_terminal_event(&mut state, &mut journal, AgentEvent::LeaseLost).await;
                        return;
                    }
                }
                () = wait_optional(chunk_deadline) => {
                    if let Err(error) = journal.flush_due(Instant::now()).await {
                        drop(session);
                        journal_failure(&mut state, &mut journal, error).await;
                        return;
                    }
                }
                event = session.next_event() => match event {
                    Ok(Some(ProviderEvent::ToolCallCompleted {
                        index,
                        call_id,
                        name,
                        arguments,
                    })) => {
                        if pending_tools.values().any(|call| call.call_id == call_id)
                            || pending_tools
                                .insert(index, AgentToolCall { call_id, name, arguments })
                                .is_some()
                        {
                            drive_terminal_event(
                                &mut state,
                                &mut journal,
                                AgentEvent::ProviderFailed(AgentFailure::ProviderInvalidResponse),
                            ).await;
                            return;
                        }
                    }
                    Ok(Some(ProviderEvent::Completed)) if !pending_tools.is_empty() => {
                        if max_output_tokens.is_some() && !usage_seen {
                            drive_terminal_event(
                                &mut state,
                                &mut journal,
                                AgentEvent::ProviderFailed(AgentFailure::ProviderInvalidResponse),
                            ).await;
                            return;
                        }
                        break pending_tools.into_values().collect::<Vec<_>>();
                    }
                    Ok(Some(event)) => {
                        let sampling = ProviderSamplingContext {
                            max_output_tokens,
                            usage_seen: &mut usage_seen,
                            runtime: inner.runtime.as_ref(),
                            lease,
                            sampling_index,
                            max_run_output_tokens: run_output_ceiling.flatten(),
                            rate_card: run_rate_card.as_ref().and_then(Option::as_ref),
                            audit: inner.audit.as_ref(),
                        };
                        if handle_provider_event(
                            &mut state,
                            &mut journal,
                            event,
                            sampling,
                        ).await {
                            return;
                        }
                    }
                    Ok(None) => {
                        drive_terminal_event(
                            &mut state,
                            &mut journal,
                            AgentEvent::ProviderFailed(AgentFailure::ProviderInvalidResponse),
                        ).await;
                        return;
                    }
                    Err(error) => {
                        drive_terminal_event(
                            &mut state,
                            &mut journal,
                            AgentEvent::ProviderFailed(provider_port_failure(error)),
                        ).await;
                        return;
                    }
                }
            }
        };
        drop(session);
        let Ok((next, effects)) = reduce(&state, AgentEvent::ProviderToolCalls(calls)) else {
            return;
        };
        state = next;
        let mut effects = effects.into_iter();
        let Some(AgentEffect::InvokeTools(calls)) = effects.next() else {
            return;
        };
        if effects.next().is_some() {
            return;
        }
        for call in calls {
            let arguments = call.arguments.clone();
            let waits_for_human = is_component_human_decision_name(&call.name);
            if waits_for_human {
                let Ok((next, effects)) = reduce(&state, AgentEvent::HumanRequired) else {
                    return;
                };
                state = next;
                if !matches!(effects.as_slice(), [AgentEffect::AwaitHuman]) {
                    return;
                }
            }
            let tools = inner.tools.clone();
            let invocation_lease = lease.clone();
            let provider_call_id = call.call_id.clone();
            let tool_name = call.name.clone();
            let invocation = async move {
                tools
                    .invoke(
                        &invocation_lease,
                        &provider_call_id,
                        &tool_name,
                        call.arguments,
                    )
                    .await
            };
            let outcome = if waits_for_human {
                // Dropping a JoinHandle detaches rather than aborts. On cancellation the durable
                // waiter therefore survives long enough to observe the terminal run in PostgreSQL
                // and atomically retire/audit its pending row. Ordinary effect tools stay inline so
                // cancellation still drops them before the terminal commit.
                match await_run_child(
                    tokio::spawn(invocation),
                    &mut activation.cancel,
                    &mut renew,
                    deadline,
                    inner.runtime.as_ref(),
                    lease,
                )
                .await
                {
                    ControlledChild::Ready(Ok(result)) => ControlledChild::Ready(result),
                    ControlledChild::Ready(Err(error)) => {
                        tracing::error!(
                            cancelled = error.is_cancelled(),
                            "component human-decision task ended without a result"
                        );
                        ControlledChild::Ready(Err(AgentToolInvokeError::Unavailable))
                    }
                    ControlledChild::Cancelled(source) => ControlledChild::Cancelled(source),
                    ControlledChild::LeaseLost => ControlledChild::LeaseLost,
                }
            } else {
                await_run_child(
                    invocation,
                    &mut activation.cancel,
                    &mut renew,
                    deadline,
                    inner.runtime.as_ref(),
                    lease,
                )
                .await
            };
            if waits_for_human && matches!(&outcome, ControlledChild::Ready(_)) {
                let Ok((next, effects)) = reduce(&state, AgentEvent::HumanReleased) else {
                    return;
                };
                state = next;
                if !effects.is_empty() {
                    return;
                }
            }
            let reply = match outcome {
                ControlledChild::Ready(Ok(reply)) => reply,
                ControlledChild::Ready(Err(AgentToolInvokeError::ReconciliationRequired)) => {
                    drive_terminal_event(
                        &mut state,
                        &mut journal,
                        AgentEvent::JournalCommitUnknown,
                    )
                    .await;
                    return;
                }
                ControlledChild::Ready(Err(AgentToolInvokeError::Unavailable)) => {
                    drive_terminal_event(
                        &mut state,
                        &mut journal,
                        AgentEvent::ToolRuntimeUnavailable,
                    )
                    .await;
                    return;
                }
                ControlledChild::Cancelled(source) => {
                    if waits_for_human {
                        cancel_and_commit(
                            &mut state,
                            &mut journal,
                            inner.audit.as_ref(),
                            lease,
                            source,
                        )
                        .await;
                    } else {
                        tool_interrupted(
                            &mut state,
                            &mut journal,
                            inner.audit.as_ref(),
                            lease,
                            source,
                        )
                        .await;
                    }
                    return;
                }
                ControlledChild::LeaseLost => {
                    drive_terminal_event(&mut state, &mut journal, AgentEvent::LeaseLost).await;
                    return;
                }
            };
            let exchange = match RunToolExchange::new(
                reply.call_id().clone(),
                call.call_id,
                call.name,
                arguments,
                reply.content().to_owned(),
                reply.error_code().map(str::to_owned),
            ) {
                Ok(exchange) => exchange,
                Err(error) => {
                    journal_failure(&mut state, &mut journal, error).await;
                    return;
                }
            };
            if let Err(error) = journal.append_tool_exchange(&exchange).await {
                journal_failure(&mut state, &mut journal, error).await;
                return;
            }
        }
        let Ok((next, effects)) = reduce(&state, AgentEvent::ToolResultCommitted) else {
            return;
        };
        state = next;
        if !matches!(effects.as_slice(), [AgentEffect::LoadContext]) {
            return;
        }
        let Some(next_sampling_index) = sampling_index.checked_add(1) else {
            drive_terminal_event(
                &mut state,
                &mut journal,
                AgentEvent::ProviderFailed(AgentFailure::RunTokenBudgetExceeded),
            )
            .await;
            return;
        };
        sampling_index = next_sampling_index;
    }
}

async fn wait_optional(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => pending::<()>().await,
    }
}

async fn await_run_child<F, T>(
    child: F,
    cancel: &mut watch::Receiver<bool>,
    renew: &mut tokio::time::Interval,
    deadline: Option<tokio::time::Instant>,
    runtime: &dyn RunRuntime,
    lease: &RunExecutionLease,
) -> ControlledChild<T>
where
    F: Future<Output = T>,
{
    tokio::pin!(child);
    loop {
        tokio::select! {
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    return ControlledChild::Cancelled(CancellationSource::User);
                }
            }
            () = wait_optional(deadline) => {
                return ControlledChild::Cancelled(CancellationSource::Deadline);
            }
            _ = renew.tick() => {
                if runtime.renew_lease(lease).await.is_err() {
                    return ControlledChild::LeaseLost;
                }
            }
            result = &mut child => return ControlledChild::Ready(result),
        }
    }
}

struct ProviderSamplingContext<'a> {
    max_output_tokens: Option<u32>,
    usage_seen: &'a mut bool,
    runtime: &'a dyn RunRuntime,
    lease: &'a RunExecutionLease,
    sampling_index: u32,
    max_run_output_tokens: Option<u64>,
    rate_card: Option<&'a ProviderRateCard>,
    audit: &'a dyn AgentAudit,
}

async fn handle_provider_event(
    state: &mut AgentState,
    journal: &mut DurableTextRun,
    event: ProviderEvent,
    sampling: ProviderSamplingContext<'_>,
) -> bool {
    match event {
        ProviderEvent::ResponseStarted { .. }
        | ProviderEvent::OutputItemAdded { .. }
        | ProviderEvent::ToolCallStarted { .. }
        | ProviderEvent::ToolArgumentsDelta { .. } => false,
        ProviderEvent::TextDelta { delta, .. } => {
            let Ok((next, effects)) = reduce(state, AgentEvent::ProviderTextDelta(delta)) else {
                return true;
            };
            *state = next;
            let [AgentEffect::PersistText(delta)] = effects.as_slice() else {
                return true;
            };
            if let Err(error) = journal.push(delta, Instant::now()).await {
                journal_failure(state, journal, error).await;
                return true;
            }
            false
        }
        ProviderEvent::ReasoningDelta { delta, .. } => {
            let Ok((next, effects)) = reduce(state, AgentEvent::ProviderReasoningDelta(delta))
            else {
                return true;
            };
            *state = next;
            let [AgentEffect::PersistReasoning(delta)] = effects.as_slice() else {
                return true;
            };
            if let Err(error) = journal.push_reasoning(delta, Instant::now()).await {
                journal_failure(state, journal, error).await;
                return true;
            }
            false
        }
        ProviderEvent::ToolCallCompleted { .. } => {
            drive_terminal_event(
                state,
                journal,
                AgentEvent::ProviderFailed(AgentFailure::ProviderInvalidResponse),
            )
            .await;
            true
        }
        ProviderEvent::Usage(usage) => {
            let usage_is_invalid = *sampling.usage_seen
                || usage
                    .input_tokens
                    .checked_add(usage.output_tokens)
                    .is_none_or(|known| usage.total_tokens < known);
            if usage_is_invalid {
                drive_terminal_event(
                    state,
                    journal,
                    AgentEvent::ProviderFailed(AgentFailure::ProviderInvalidResponse),
                )
                .await;
                true
            } else {
                let sampling_budget_exceeded = sampling
                    .max_output_tokens
                    .is_some_and(|limit| usage.output_tokens > u64::from(limit));
                match sampling
                    .runtime
                    .record_provider_usage(
                        sampling.lease,
                        sampling.sampling_index,
                        usage,
                        sampling.max_run_output_tokens,
                        sampling.rate_card,
                    )
                    .await
                {
                    Ok(RunTokenUsageReceipt::Recorded(_) | RunTokenUsageReceipt::Replayed(_)) => {
                        *sampling.usage_seen = true;
                        if sampling_budget_exceeded {
                            drive_terminal_event(
                                state,
                                journal,
                                AgentEvent::ProviderFailed(
                                    AgentFailure::ProviderTokenBudgetExceeded,
                                ),
                            )
                            .await;
                            true
                        } else {
                            false
                        }
                    }
                    Ok(RunTokenUsageReceipt::BudgetExceeded(_)) => {
                        *sampling.usage_seen = true;
                        drive_terminal_event(
                            state,
                            journal,
                            AgentEvent::ProviderFailed(AgentFailure::RunTokenBudgetExceeded),
                        )
                        .await;
                        true
                    }
                    Err(RunRuntimeError::StaleLease) => {
                        drive_terminal_event(state, journal, AgentEvent::LeaseLost).await;
                        true
                    }
                    Err(_) => {
                        drive_terminal_event(state, journal, AgentEvent::JournalCommitUnknown)
                            .await;
                        true
                    }
                }
            }
        }
        ProviderEvent::Completed => {
            if sampling.max_output_tokens.is_some() && !*sampling.usage_seen {
                drive_terminal_event(
                    state,
                    journal,
                    AgentEvent::ProviderFailed(AgentFailure::ProviderInvalidResponse),
                )
                .await;
                return true;
            }
            let Ok((next, effects)) = reduce(state, AgentEvent::ProviderCompleted) else {
                return true;
            };
            *state = next;
            let [AgentEffect::CommitTerminal(terminal)] = effects.as_slice() else {
                return true;
            };
            commit_terminal(state, journal, *terminal).await;
            true
        }
        ProviderEvent::Failed(failure) => {
            if failure == ProviderFailure::StreamStalled && {
                metrics::counter!("openbot_agent_stream_stalled_total").increment(1);
                sampling
                    .audit
                    .record(sampling.lease, AgentAuditKind::StreamStalled)
                    .await
                    .is_err()
            } {
                drive_terminal_event(state, journal, AgentEvent::JournalCommitUnknown).await;
                return true;
            }
            let failure = provider_failure(failure);
            let Ok((next, effects)) = reduce(state, AgentEvent::ProviderFailed(failure)) else {
                return true;
            };
            *state = next;
            let [AgentEffect::CommitTerminal(terminal)] = effects.as_slice() else {
                return true;
            };
            commit_terminal(state, journal, *terminal).await;
            true
        }
    }
}

async fn commit_terminal(
    state: &mut AgentState,
    journal: &mut DurableTextRun,
    terminal: AgentTerminal,
) {
    if journal.finish(run_terminal(terminal)).await.is_ok()
        && let Ok((next, _)) = reduce(state, AgentEvent::TerminalCommitted)
    {
        *state = next;
    }
}

async fn drive_terminal_event(
    state: &mut AgentState,
    journal: &mut DurableTextRun,
    event: AgentEvent,
) {
    let Ok((next, effects)) = reduce(state, event) else {
        return;
    };
    *state = next;
    let [AgentEffect::CommitTerminal(terminal)] = effects.as_slice() else {
        return;
    };
    commit_terminal(state, journal, *terminal).await;
}

async fn cancel_and_commit(
    state: &mut AgentState,
    journal: &mut DurableTextRun,
    audit: &dyn AgentAudit,
    lease: &RunExecutionLease,
    source: CancellationSource,
) {
    if matches!(source, CancellationSource::Deadline) && {
        metrics::counter!("openbot_agent_run_deadline_total").increment(1);
        audit
            .record(lease, AgentAuditKind::RunDeadlineExceeded)
            .await
            .is_err()
    } {
        drive_terminal_event(state, journal, AgentEvent::JournalCommitUnknown).await;
        return;
    }
    let Ok((next, effects)) = reduce(state, AgentEvent::CancelRequested) else {
        return;
    };
    if !matches!(effects.as_slice(), [AgentEffect::CancelChildren]) {
        return;
    }
    *state = next;
    drive_terminal_event(state, journal, AgentEvent::ChildrenStopped).await;
}

async fn tool_interrupted(
    state: &mut AgentState,
    journal: &mut DurableTextRun,
    audit: &dyn AgentAudit,
    lease: &RunExecutionLease,
    source: CancellationSource,
) {
    if matches!(source, CancellationSource::Deadline) {
        metrics::counter!("openbot_agent_run_deadline_total").increment(1);
        if audit
            .record(lease, AgentAuditKind::RunDeadlineExceeded)
            .await
            .is_err()
        {
            drive_terminal_event(state, journal, AgentEvent::JournalCommitUnknown).await;
            return;
        }
    }
    // Dropping an in-flight tool future cannot prove whether its non-idempotent effect committed.
    drive_terminal_event(state, journal, AgentEvent::JournalCommitUnknown).await;
}

async fn journal_failure(
    state: &mut AgentState,
    journal: &mut DurableTextRun,
    error: RunRuntimeError,
) {
    let event = match error {
        RunRuntimeError::StaleLease => AgentEvent::LeaseLost,
        RunRuntimeError::CommitUnknown => AgentEvent::JournalCommitUnknown,
        RunRuntimeError::Unavailable
        | RunRuntimeError::Corrupt { .. }
        | RunRuntimeError::InvalidInput { .. }
        | RunRuntimeError::Conflict => AgentEvent::JournalCommitUnknown,
    };
    drive_terminal_event(state, journal, event).await;
}

const fn provider_failure(failure: ProviderFailure) -> AgentFailure {
    match failure {
        ProviderFailure::Authentication => AgentFailure::ProviderAuthentication,
        ProviderFailure::RateLimited { .. } => AgentFailure::ProviderRateLimited,
        ProviderFailure::ServerUnavailable { .. } | ProviderFailure::Transport => {
            AgentFailure::ProviderUnavailable
        }
        ProviderFailure::InvalidResponse => AgentFailure::ProviderInvalidResponse,
        ProviderFailure::StreamStalled => AgentFailure::ProviderStreamStalled,
        ProviderFailure::GenerationFailed => AgentFailure::ProviderGenerationFailed,
    }
}

const fn provider_port_failure(error: ProviderPortError) -> AgentFailure {
    match error {
        ProviderPortError::InvalidRequest { .. } => AgentFailure::ProviderInvalidResponse,
        ProviderPortError::Unavailable => AgentFailure::ProviderUnavailable,
        ProviderPortError::CommitUnknown => AgentFailure::ProviderCommitUnknown,
    }
}

const fn context_failure(error: AgentContextError) -> AgentFailure {
    match error {
        AgentContextError::Stale => AgentFailure::RuntimeLeaseLost,
        AgentContextError::ToolHistoryUnsupported => AgentFailure::ToolLoopUnavailable,
        AgentContextError::Unavailable
        | AgentContextError::Corrupt { .. }
        | AgentContextError::TooLarge => AgentFailure::ProviderInvalidResponse,
    }
}

const fn run_terminal(terminal: AgentTerminal) -> RunTerminal {
    match terminal {
        AgentTerminal::Succeeded => RunTerminal::Completed,
        AgentTerminal::Cancelled => RunTerminal::Cancelled,
        AgentTerminal::Failed(failure) => RunTerminal::Failed(failure_code(failure)),
        AgentTerminal::ReconciliationRequired(failure) => {
            RunTerminal::ReconciliationRequired(failure_code(failure))
        }
    }
}

const fn failure_code(failure: AgentFailure) -> RunFailureCode {
    match failure {
        AgentFailure::ProviderAuthentication => RunFailureCode::ProviderAuthentication,
        AgentFailure::ProviderRateLimited => RunFailureCode::ProviderRateLimited,
        AgentFailure::ProviderUnavailable => RunFailureCode::ProviderUnavailable,
        AgentFailure::ProviderCommitUnknown => RunFailureCode::ProviderCommitUnknown,
        AgentFailure::ProviderInvalidResponse => RunFailureCode::ProviderInvalidResponse,
        AgentFailure::ProviderStreamStalled => RunFailureCode::ProviderStreamStalled,
        AgentFailure::ProviderGenerationFailed => RunFailureCode::ProviderGenerationFailed,
        AgentFailure::ProviderTokenBudgetExceeded => RunFailureCode::ProviderTokenBudgetExceeded,
        AgentFailure::RunTokenBudgetExceeded => RunFailureCode::RunTokenBudgetExceeded,
        AgentFailure::ToolStepLimit => RunFailureCode::ToolStepLimit,
        AgentFailure::ToolLoopUnavailable => RunFailureCode::ToolLoopUnavailable,
        AgentFailure::ToolDenied => RunFailureCode::ToolDenied,
        AgentFailure::RuntimeLeaseLost => RunFailureCode::RuntimeLeaseExpired,
        AgentFailure::JournalCommitUnknown => RunFailureCode::JournalCommitUnknown,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use openbot_application::{
        AgentAuditError, ClaimedRunDispatch, NoAgentAudit, ProviderBillingFamily, ProviderMessage,
        ProviderMessageRole, ProviderPortError, ProviderRateCardInput, ProviderRequest,
        ProviderSession, ProviderUsage, RunSemanticChannel, RunTokenUsage, RunToolExchange,
        RunWriteReceipt,
    };
    use openbot_contracts::ids::{ActorId, BotId, RunId, ThreadId, ToolCallId};
    use openbot_domain::thread::FencingToken;
    use time::macros::datetime;
    use tokio::sync::Notify;

    use super::*;
    use crate::NoAgentToolInvoker;

    struct FakeRuntime {
        calls: StdMutex<Vec<RuntimeCall>>,
        usage: StdMutex<FakeUsage>,
        force_budget_exceeded: bool,
        terminal: Notify,
    }

    #[derive(Default)]
    struct FakeUsage {
        ceiling_initialized: bool,
        ceiling: Option<u64>,
        rate_card_initialized: bool,
        rate_card: Option<ProviderRateCard>,
        next_sampling: u32,
        aggregate: RunTokenUsage,
        last: Option<(u32, ProviderUsage)>,
    }

    #[derive(Default)]
    struct FakeAudit {
        kinds: StdMutex<Vec<AgentAuditKind>>,
    }

    #[async_trait]
    impl AgentAudit for FakeAudit {
        async fn record(
            &self,
            _lease: &RunExecutionLease,
            kind: AgentAuditKind,
        ) -> Result<(), AgentAuditError> {
            self.kinds.lock().expect("audit lock").push(kind);
            Ok(())
        }
    }

    impl FakeAudit {
        fn kinds(&self) -> Vec<AgentAuditKind> {
            self.kinds.lock().expect("audit lock").clone()
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum RuntimeCall {
        Chunk(u64, RunSemanticChannel, String),
        ToolExchange(u64, String, String, String),
        Finish(u64, RunTerminal),
        Renew,
    }

    impl FakeRuntime {
        fn new() -> Self {
            Self {
                calls: StdMutex::new(Vec::new()),
                usage: StdMutex::new(FakeUsage::default()),
                force_budget_exceeded: false,
                terminal: Notify::new(),
            }
        }

        fn rejecting_usage_budget() -> Self {
            Self {
                force_budget_exceeded: true,
                ..Self::new()
            }
        }

        fn calls(&self) -> Vec<RuntimeCall> {
            self.calls.lock().expect("fake lock").clone()
        }

        fn usage(&self) -> (Option<u64>, u32, RunTokenUsage) {
            let usage = self.usage.lock().expect("usage lock");
            (usage.ceiling, usage.next_sampling, usage.aggregate)
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
            self.calls
                .lock()
                .expect("fake lock")
                .push(RuntimeCall::Renew);
            Ok(())
        }

        async fn append_semantic_chunk(
            &self,
            _lease: &RunExecutionLease,
            expected_sequence: u64,
            channel: RunSemanticChannel,
            chunk: &str,
        ) -> Result<RunWriteReceipt, RunRuntimeError> {
            self.calls
                .lock()
                .expect("fake lock")
                .push(RuntimeCall::Chunk(
                    expected_sequence,
                    channel,
                    chunk.to_owned(),
                ));
            Ok(receipt(expected_sequence))
        }

        async fn append_tool_exchange(
            &self,
            _lease: &RunExecutionLease,
            expected_sequence: u64,
            exchange: &RunToolExchange,
        ) -> Result<RunWriteReceipt, RunRuntimeError> {
            self.calls
                .lock()
                .expect("fake lock")
                .push(RuntimeCall::ToolExchange(
                    expected_sequence,
                    exchange.provider_call_id().to_owned(),
                    exchange.name().to_owned(),
                    exchange.result().to_owned(),
                ));
            Ok(receipt(expected_sequence))
        }

        async fn record_provider_usage(
            &self,
            _lease: &RunExecutionLease,
            sampling_index: u32,
            usage: ProviderUsage,
            max_run_output_tokens: Option<u64>,
            rate_card: Option<&ProviderRateCard>,
        ) -> Result<RunTokenUsageReceipt, RunRuntimeError> {
            let mut state = self.usage.lock().expect("usage lock");
            if state.ceiling_initialized {
                if state.ceiling != max_run_output_tokens {
                    return Err(RunRuntimeError::Conflict);
                }
            } else {
                state.ceiling_initialized = true;
                state.ceiling = max_run_output_tokens;
            }
            if state.rate_card_initialized {
                if state.rate_card.as_ref() != rate_card {
                    return Err(RunRuntimeError::Conflict);
                }
            } else {
                state.rate_card_initialized = true;
                state.rate_card = rate_card.cloned();
            }
            if self.force_budget_exceeded {
                return Ok(RunTokenUsageReceipt::BudgetExceeded(state.aggregate));
            }
            if sampling_index < state.next_sampling {
                return if state.last == Some((sampling_index, usage))
                    && sampling_index.checked_add(1) == Some(state.next_sampling)
                {
                    if state
                        .ceiling
                        .is_some_and(|limit| state.aggregate.output_tokens > limit)
                    {
                        Ok(RunTokenUsageReceipt::BudgetExceeded(state.aggregate))
                    } else {
                        Ok(RunTokenUsageReceipt::Replayed(state.aggregate))
                    }
                } else {
                    Err(RunRuntimeError::Conflict)
                };
            }
            if sampling_index != state.next_sampling {
                return Err(RunRuntimeError::Conflict);
            }
            let next = RunTokenUsage {
                input_tokens: state
                    .aggregate
                    .input_tokens
                    .checked_add(usage.input_tokens)
                    .ok_or(RunRuntimeError::InvalidInput {
                        field: "provider_usage",
                    })?,
                output_tokens: state
                    .aggregate
                    .output_tokens
                    .checked_add(usage.output_tokens)
                    .ok_or(RunRuntimeError::InvalidInput {
                        field: "provider_usage",
                    })?,
                total_tokens: state
                    .aggregate
                    .total_tokens
                    .checked_add(usage.total_tokens)
                    .ok_or(RunRuntimeError::InvalidInput {
                        field: "provider_usage",
                    })?,
            };
            if max_run_output_tokens.is_some_and(|limit| next.output_tokens > limit) {
                state.aggregate = next;
                state.last = Some((sampling_index, usage));
                state.next_sampling =
                    sampling_index
                        .checked_add(1)
                        .ok_or(RunRuntimeError::InvalidInput {
                            field: "sampling_index",
                        })?;
                return Ok(RunTokenUsageReceipt::BudgetExceeded(next));
            }
            state.aggregate = next;
            state.last = Some((sampling_index, usage));
            state.next_sampling =
                sampling_index
                    .checked_add(1)
                    .ok_or(RunRuntimeError::InvalidInput {
                        field: "sampling_index",
                    })?;
            Ok(RunTokenUsageReceipt::Recorded(next))
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
                .push(RuntimeCall::Finish(expected_sequence, terminal));
            self.terminal.notify_one();
            Ok(receipt(expected_sequence))
        }

        async fn recover_one_stale_run(&self) -> Result<Option<RunWriteReceipt>, RunRuntimeError> {
            Ok(None)
        }
    }

    struct FakeContext;

    #[async_trait]
    impl AgentContextSource for FakeContext {
        async fn load(
            &self,
            _lease: &RunExecutionLease,
        ) -> Result<ProviderRequest, AgentContextError> {
            Ok(ProviderRequest {
                route: openbot_application::ProviderRoute::PackageOpenAi,
                messages: vec![ProviderMessage {
                    role: ProviderMessageRole::User,
                    content: "hello".to_owned(),
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                }],
                tools: Vec::new(),
                max_output_tokens: Some(32),
                rate_card: None,
            })
        }
    }

    struct HoldingContext;

    #[derive(Default)]
    struct CountingContext {
        loads: AtomicUsize,
    }

    #[derive(Default)]
    struct DriftingRateContext {
        loads: AtomicUsize,
    }

    #[async_trait]
    impl AgentContextSource for CountingContext {
        async fn load(
            &self,
            lease: &RunExecutionLease,
        ) -> Result<ProviderRequest, AgentContextError> {
            self.loads.fetch_add(1, AtomicOrdering::SeqCst);
            FakeContext.load(lease).await
        }
    }

    #[async_trait]
    impl AgentContextSource for DriftingRateContext {
        async fn load(
            &self,
            lease: &RunExecutionLease,
        ) -> Result<ProviderRequest, AgentContextError> {
            let index = self.loads.fetch_add(1, AtomicOrdering::SeqCst);
            let mut request = FakeContext.load(lease).await?;
            request.rate_card = Some(
                ProviderRateCard::new(ProviderRateCardInput {
                    family: ProviderBillingFamily::OpenAiCompatible,
                    model: "model-1".to_owned(),
                    currency: "USD".to_owned(),
                    max_input_micro_units_per_million_tokens: if index == 0 {
                        1_500_000
                    } else {
                        1_500_001
                    },
                    max_output_micro_units_per_million_tokens: 2_000_000,
                    source_url: "https://prices.example.test/archive/2026-08-30".to_owned(),
                    source_sha256: if index == 0 {
                        "a".repeat(64)
                    } else {
                        "b".repeat(64)
                    },
                    observed_at: datetime!(2026-08-30 12:00 UTC),
                })
                .expect("test rate card"),
            );
            Ok(request)
        }
    }

    #[async_trait]
    impl AgentContextSource for HoldingContext {
        async fn load(
            &self,
            _lease: &RunExecutionLease,
        ) -> Result<ProviderRequest, AgentContextError> {
            pending().await
        }
    }

    struct FakeProvider {
        events: Vec<ProviderEvent>,
        hold: bool,
    }

    struct FailingStartProvider(ProviderPortError);

    struct SequencedProvider {
        sessions: StdMutex<VecDeque<Vec<ProviderEvent>>>,
        starts: AtomicUsize,
    }

    #[async_trait]
    impl ProviderAdapter for SequencedProvider {
        async fn start(
            &self,
            _request: ProviderRequest,
        ) -> Result<Box<dyn ProviderSession>, ProviderPortError> {
            self.starts.fetch_add(1, AtomicOrdering::SeqCst);
            let events = self
                .sessions
                .lock()
                .expect("provider sessions")
                .pop_front()
                .ok_or(ProviderPortError::InvalidRequest {
                    field: "test_sessions",
                })?;
            Ok(Box::new(FakeSession {
                events: events.into(),
                hold: false,
            }))
        }
    }

    #[derive(Default)]
    struct FakeToolInvoker {
        calls: AtomicUsize,
        releases: AtomicUsize,
    }

    #[async_trait]
    impl AgentToolInvoker for FakeToolInvoker {
        async fn invoke(
            &self,
            _lease: &RunExecutionLease,
            _provider_call_id: &str,
            tool_name: &str,
            arguments: serde_json::Value,
        ) -> Result<crate::AgentToolReply, AgentToolInvokeError> {
            assert_eq!(tool_name, "remember");
            assert_eq!(arguments, serde_json::json!({"content":"tea"}));
            let index = self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            crate::AgentToolReply::new(
                ToolCallId::new(format!("internal-{index}")),
                r#"{"status":"remembered"}"#.to_owned(),
                None,
            )
        }

        fn release(&self, _run_id: &RunId) {
            self.releases.fetch_add(1, AtomicOrdering::SeqCst);
        }
    }

    #[derive(Default)]
    struct OrderedToolInvoker {
        seen: StdMutex<Vec<u64>>,
    }

    struct HumanToolInvoker {
        started: Notify,
        answer: Notify,
        provider_calls: StdMutex<Vec<String>>,
        completed: AtomicUsize,
    }

    impl HumanToolInvoker {
        fn new() -> Self {
            Self {
                started: Notify::new(),
                answer: Notify::new(),
                provider_calls: StdMutex::new(Vec::new()),
                completed: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl AgentToolInvoker for HumanToolInvoker {
        async fn invoke(
            &self,
            _lease: &RunExecutionLease,
            provider_call_id: &str,
            tool_name: &str,
            arguments: serde_json::Value,
        ) -> Result<crate::AgentToolReply, AgentToolInvokeError> {
            assert_eq!(tool_name, "askApproval");
            assert_eq!(
                arguments,
                serde_json::json!({"title":"Refund?","summary":"Duplicate"})
            );
            self.provider_calls
                .lock()
                .expect("human provider calls")
                .push(provider_call_id.to_owned());
            self.started.notify_one();
            self.answer.notified().await;
            self.completed.fetch_add(1, AtomicOrdering::SeqCst);
            crate::AgentToolReply::new(
                ToolCallId::new("decision-1"),
                r#"{"decision":"approved"}"#.to_owned(),
                None,
            )
        }
    }

    #[async_trait]
    impl AgentToolInvoker for OrderedToolInvoker {
        async fn invoke(
            &self,
            _lease: &RunExecutionLease,
            _provider_call_id: &str,
            _tool_name: &str,
            arguments: serde_json::Value,
        ) -> Result<crate::AgentToolReply, AgentToolInvokeError> {
            let order = arguments
                .get("order")
                .and_then(serde_json::Value::as_u64)
                .expect("test order");
            self.seen.lock().expect("ordered tools").push(order);
            crate::AgentToolReply::new(
                ToolCallId::new(format!("internal-{order}")),
                format!("result-{order}"),
                None,
            )
        }
    }

    #[async_trait]
    impl ProviderAdapter for FailingStartProvider {
        async fn start(
            &self,
            _request: ProviderRequest,
        ) -> Result<Box<dyn ProviderSession>, ProviderPortError> {
            Err(self.0)
        }
    }

    #[async_trait]
    impl ProviderAdapter for FakeProvider {
        async fn start(
            &self,
            _request: ProviderRequest,
        ) -> Result<Box<dyn ProviderSession>, ProviderPortError> {
            Ok(Box::new(FakeSession {
                events: self.events.clone().into(),
                hold: self.hold,
            }))
        }
    }

    struct FakeSession {
        events: VecDeque<ProviderEvent>,
        hold: bool,
    }

    #[async_trait]
    impl ProviderSession for FakeSession {
        async fn next_event(&mut self) -> Result<Option<ProviderEvent>, ProviderPortError> {
            if let Some(event) = self.events.pop_front() {
                Ok(Some(event))
            } else if self.hold {
                pending().await
            } else {
                Ok(None)
            }
        }
    }

    fn lease(run: &str) -> RunExecutionLease {
        RunExecutionLease::new(
            RunId::new(run),
            ThreadId::new("550e8400-e29b-41d4-a716-446655440000"),
            BotId::new("bot-1"),
            ActorId::new("actor-1"),
            FencingToken::new(1).unwrap(),
            1,
        )
        .unwrap()
    }

    const fn receipt(sequence: u64) -> RunWriteReceipt {
        RunWriteReceipt {
            run_event_sequence: sequence,
            thread_event_sequence: sequence,
            message_sequence: None,
            replayed: false,
        }
    }

    fn test_config() -> BuiltInAgentConfig {
        BuiltInAgentConfig {
            queue_capacity: 4,
            max_concurrency: 2,
            lease_renew_interval: Duration::from_secs(1),
            run_deadline: Some(Duration::from_secs(2)),
        }
    }

    #[tokio::test]
    async fn accepted_text_stream_reaches_durable_chunk_and_completed_terminal() {
        let runtime = Arc::new(FakeRuntime::new());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            Arc::new(FakeContext),
            Arc::new(FakeProvider {
                events: vec![
                    ProviderEvent::ResponseStarted {
                        response_id: "response-1".to_owned(),
                    },
                    ProviderEvent::TextDelta {
                        index: 0,
                        delta: "hello".to_owned(),
                    },
                    ProviderEvent::Usage(ProviderUsage {
                        input_tokens: 1,
                        output_tokens: 1,
                        total_tokens: 2,
                    }),
                    ProviderEvent::Completed,
                ],
                hold: false,
            }),
            Arc::new(NoAgentToolInvoker),
            Arc::new(NoAgentAudit),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-text");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert_eq!(
            runtime.calls(),
            [
                RuntimeCall::Chunk(1, RunSemanticChannel::Text, "hello".to_owned()),
                RuntimeCall::Finish(2, RunTerminal::Completed),
            ]
        );
        agent.stop().await;
    }

    #[tokio::test]
    async fn complete_tool_turn_is_durable_then_reloads_context_and_resamples() {
        let runtime = Arc::new(FakeRuntime::new());
        let context = Arc::new(CountingContext::default());
        let provider = Arc::new(SequencedProvider {
            sessions: StdMutex::new(
                vec![
                    vec![
                        ProviderEvent::ToolCallCompleted {
                            index: 0,
                            call_id: "provider-call-1".to_owned(),
                            name: "remember".to_owned(),
                            arguments: serde_json::json!({"content":"tea"}),
                        },
                        ProviderEvent::Usage(ProviderUsage {
                            input_tokens: 3,
                            output_tokens: 2,
                            total_tokens: 5,
                        }),
                        ProviderEvent::Completed,
                    ],
                    vec![
                        ProviderEvent::TextDelta {
                            index: 0,
                            delta: "remembered".to_owned(),
                        },
                        ProviderEvent::Usage(ProviderUsage {
                            input_tokens: 5,
                            output_tokens: 1,
                            total_tokens: 6,
                        }),
                        ProviderEvent::Completed,
                    ],
                ]
                .into(),
            ),
            starts: AtomicUsize::new(0),
        });
        let tools = Arc::new(FakeToolInvoker::default());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            context.clone(),
            provider.clone(),
            tools.clone(),
            Arc::new(NoAgentAudit),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-tool-loop");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert_eq!(
            runtime.calls(),
            [
                RuntimeCall::ToolExchange(
                    1,
                    "provider-call-1".to_owned(),
                    "remember".to_owned(),
                    r#"{"status":"remembered"}"#.to_owned(),
                ),
                RuntimeCall::Chunk(2, RunSemanticChannel::Text, "remembered".to_owned()),
                RuntimeCall::Finish(3, RunTerminal::Completed),
            ]
        );
        assert_eq!(context.loads.load(AtomicOrdering::SeqCst), 2);
        assert_eq!(provider.starts.load(AtomicOrdering::SeqCst), 2);
        assert_eq!(tools.calls.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(
            runtime.usage(),
            (
                Some(288),
                2,
                RunTokenUsage {
                    input_tokens: 8,
                    output_tokens: 3,
                    total_tokens: 11,
                },
            )
        );
        agent.stop().await;
        assert_eq!(tools.releases.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test]
    async fn provider_rate_snapshot_cannot_drift_between_tool_samplings() {
        let runtime = Arc::new(FakeRuntime::new());
        let context = Arc::new(DriftingRateContext::default());
        let provider = Arc::new(SequencedProvider {
            sessions: StdMutex::new(
                vec![vec![
                    ProviderEvent::ToolCallCompleted {
                        index: 0,
                        call_id: "provider-call-priced".to_owned(),
                        name: "remember".to_owned(),
                        arguments: serde_json::json!({"content":"tea"}),
                    },
                    ProviderEvent::Usage(ProviderUsage {
                        input_tokens: 3,
                        output_tokens: 2,
                        total_tokens: 5,
                    }),
                    ProviderEvent::Completed,
                ]]
                .into(),
            ),
            starts: AtomicUsize::new(0),
        });
        let tools = Arc::new(FakeToolInvoker::default());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            context.clone(),
            provider.clone(),
            tools,
            Arc::new(NoAgentAudit),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-price-drift");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert_eq!(context.loads.load(AtomicOrdering::SeqCst), 2);
        assert_eq!(provider.starts.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(
            runtime.calls(),
            [
                RuntimeCall::ToolExchange(
                    1,
                    "provider-call-priced".to_owned(),
                    "remember".to_owned(),
                    r#"{"status":"remembered"}"#.to_owned(),
                ),
                RuntimeCall::Finish(
                    2,
                    RunTerminal::Failed(RunFailureCode::ProviderInvalidResponse),
                ),
            ]
        );
        agent.stop().await;
    }

    #[tokio::test]
    async fn run_wide_usage_budget_exhaustion_commits_stable_terminal() {
        let runtime = Arc::new(FakeRuntime::rejecting_usage_budget());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            Arc::new(FakeContext),
            Arc::new(FakeProvider {
                events: vec![ProviderEvent::Usage(ProviderUsage {
                    input_tokens: 3,
                    output_tokens: 2,
                    total_tokens: 5,
                })],
                hold: false,
            }),
            Arc::new(NoAgentToolInvoker),
            Arc::new(NoAgentAudit),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-wide-budget");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert_eq!(
            runtime.calls(),
            [RuntimeCall::Finish(
                1,
                RunTerminal::Failed(RunFailureCode::RunTokenBudgetExceeded),
            )]
        );
        agent.stop().await;
    }

    #[tokio::test]
    async fn human_decision_waits_before_exchange_then_resamples_only_after_commit() {
        let runtime = Arc::new(FakeRuntime::new());
        let context = Arc::new(CountingContext::default());
        let provider = Arc::new(SequencedProvider {
            sessions: StdMutex::new(
                vec![
                    vec![
                        ProviderEvent::ToolCallCompleted {
                            index: 0,
                            call_id: "provider-human-1".to_owned(),
                            name: "askApproval".to_owned(),
                            arguments: serde_json::json!({
                                "title":"Refund?",
                                "summary":"Duplicate"
                            }),
                        },
                        ProviderEvent::Usage(ProviderUsage {
                            input_tokens: 3,
                            output_tokens: 2,
                            total_tokens: 5,
                        }),
                        ProviderEvent::Completed,
                    ],
                    vec![
                        ProviderEvent::TextDelta {
                            index: 0,
                            delta: "approved".to_owned(),
                        },
                        ProviderEvent::Usage(ProviderUsage {
                            input_tokens: 5,
                            output_tokens: 1,
                            total_tokens: 6,
                        }),
                        ProviderEvent::Completed,
                    ],
                ]
                .into(),
            ),
            starts: AtomicUsize::new(0),
        });
        let tools = Arc::new(HumanToolInvoker::new());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            context.clone(),
            provider.clone(),
            tools.clone(),
            Arc::new(NoAgentAudit),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-human");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), tools.started.notified())
            .await
            .unwrap();
        assert_eq!(provider.starts.load(AtomicOrdering::SeqCst), 1);
        assert!(runtime.calls().is_empty());
        tools.answer.notify_one();
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert_eq!(
            runtime.calls(),
            [
                RuntimeCall::ToolExchange(
                    1,
                    "provider-human-1".to_owned(),
                    "askApproval".to_owned(),
                    r#"{"decision":"approved"}"#.to_owned(),
                ),
                RuntimeCall::Chunk(2, RunSemanticChannel::Text, "approved".to_owned()),
                RuntimeCall::Finish(3, RunTerminal::Completed),
            ]
        );
        assert_eq!(
            tools.provider_calls.lock().unwrap().as_slice(),
            ["provider-human-1"]
        );
        assert_eq!(tools.completed.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(context.loads.load(AtomicOrdering::SeqCst), 2);
        assert_eq!(provider.starts.load(AtomicOrdering::SeqCst), 2);
        agent.stop().await;
    }

    #[tokio::test]
    async fn cancelled_human_wait_is_detached_long_enough_for_durable_retirement() {
        let runtime = Arc::new(FakeRuntime::new());
        let tools = Arc::new(HumanToolInvoker::new());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            Arc::new(FakeContext),
            Arc::new(FakeProvider {
                events: vec![
                    ProviderEvent::ToolCallCompleted {
                        index: 0,
                        call_id: "provider-human-cancel".to_owned(),
                        name: "askApproval".to_owned(),
                        arguments: serde_json::json!({
                            "title":"Refund?",
                            "summary":"Duplicate"
                        }),
                    },
                    ProviderEvent::Usage(ProviderUsage {
                        input_tokens: 3,
                        output_tokens: 2,
                        total_tokens: 5,
                    }),
                    ProviderEvent::Completed,
                ],
                hold: false,
            }),
            tools.clone(),
            Arc::new(NoAgentAudit),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-human-cancel");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), tools.started.notified())
            .await
            .unwrap();
        assert_eq!(
            consumer.revoke(&lease).await,
            RunCancellationDisposition::ChildSignalled
        );
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert!(
            runtime
                .calls()
                .iter()
                .any(|call| matches!(call, RuntimeCall::Finish(_, RunTerminal::Cancelled)))
        );
        tools.answer.notify_one();
        tokio::time::timeout(Duration::from_secs(1), async {
            while tools.completed.load(AtomicOrdering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(tools.completed.load(AtomicOrdering::SeqCst), 1);
        agent.stop().await;
    }

    #[tokio::test]
    async fn non_parallel_tool_batch_executes_by_stable_output_index_and_reinjects_in_order() {
        let runtime = Arc::new(FakeRuntime::new());
        let provider = Arc::new(SequencedProvider {
            sessions: StdMutex::new(
                vec![
                    vec![
                        ProviderEvent::ToolCallCompleted {
                            index: 5,
                            call_id: "provider-five".to_owned(),
                            name: "remember".to_owned(),
                            arguments: serde_json::json!({"order":5}),
                        },
                        ProviderEvent::ToolCallCompleted {
                            index: 1,
                            call_id: "provider-one".to_owned(),
                            name: "remember".to_owned(),
                            arguments: serde_json::json!({"order":1}),
                        },
                        ProviderEvent::Usage(ProviderUsage {
                            input_tokens: 3,
                            output_tokens: 2,
                            total_tokens: 5,
                        }),
                        ProviderEvent::Completed,
                    ],
                    vec![
                        ProviderEvent::Usage(ProviderUsage {
                            input_tokens: 5,
                            output_tokens: 1,
                            total_tokens: 6,
                        }),
                        ProviderEvent::Completed,
                    ],
                ]
                .into(),
            ),
            starts: AtomicUsize::new(0),
        });
        let tools = Arc::new(OrderedToolInvoker::default());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            Arc::new(FakeContext),
            provider,
            tools.clone(),
            Arc::new(NoAgentAudit),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-tool-order");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert_eq!(*tools.seen.lock().expect("ordered tools"), [1, 5]);
        assert_eq!(
            runtime.calls(),
            [
                RuntimeCall::ToolExchange(
                    1,
                    "provider-one".to_owned(),
                    "remember".to_owned(),
                    "result-1".to_owned(),
                ),
                RuntimeCall::ToolExchange(
                    2,
                    "provider-five".to_owned(),
                    "remember".to_owned(),
                    "result-5".to_owned(),
                ),
                RuntimeCall::Finish(3, RunTerminal::Completed),
            ]
        );
        agent.stop().await;
    }

    #[tokio::test]
    async fn reported_output_above_authoritative_token_cap_fails_before_completed() {
        let runtime = Arc::new(FakeRuntime::new());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            Arc::new(FakeContext),
            Arc::new(FakeProvider {
                events: vec![
                    ProviderEvent::ResponseStarted {
                        response_id: "response-budget".to_owned(),
                    },
                    ProviderEvent::Usage(ProviderUsage {
                        input_tokens: 10,
                        output_tokens: 33,
                        total_tokens: 43,
                    }),
                    ProviderEvent::Completed,
                ],
                hold: false,
            }),
            Arc::new(NoAgentToolInvoker),
            Arc::new(NoAgentAudit),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-token-budget");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert_eq!(
            runtime.calls(),
            [RuntimeCall::Finish(
                1,
                RunTerminal::Failed(RunFailureCode::ProviderTokenBudgetExceeded)
            )]
        );
        assert_eq!(
            runtime.usage(),
            (
                Some(288),
                1,
                RunTokenUsage {
                    input_tokens: 10,
                    output_tokens: 33,
                    total_tokens: 43,
                },
            )
        );
        agent.stop().await;
    }

    #[tokio::test]
    async fn completed_without_usage_cannot_bypass_authoritative_token_cap() {
        let runtime = Arc::new(FakeRuntime::new());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            Arc::new(FakeContext),
            Arc::new(FakeProvider {
                events: vec![
                    ProviderEvent::ResponseStarted {
                        response_id: "response-no-usage".to_owned(),
                    },
                    ProviderEvent::Completed,
                ],
                hold: false,
            }),
            Arc::new(NoAgentToolInvoker),
            Arc::new(NoAgentAudit),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-no-usage");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert_eq!(
            runtime.calls(),
            [RuntimeCall::Finish(
                1,
                RunTerminal::Failed(RunFailureCode::ProviderInvalidResponse)
            )]
        );
        agent.stop().await;
    }

    #[tokio::test]
    async fn tool_call_fails_closed_until_unique_tool_loop_is_connected() {
        let runtime = Arc::new(FakeRuntime::new());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            Arc::new(FakeContext),
            Arc::new(FakeProvider {
                events: vec![
                    ProviderEvent::ToolCallCompleted {
                        index: 0,
                        call_id: "call-1".to_owned(),
                        name: "remember".to_owned(),
                        arguments: serde_json::json!({"content":"x"}),
                    },
                    ProviderEvent::Usage(ProviderUsage {
                        input_tokens: 1,
                        output_tokens: 1,
                        total_tokens: 2,
                    }),
                    ProviderEvent::Completed,
                ],
                hold: false,
            }),
            Arc::new(NoAgentToolInvoker),
            Arc::new(NoAgentAudit),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-tool");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert_eq!(
            runtime.calls(),
            [RuntimeCall::Finish(
                1,
                RunTerminal::Failed(RunFailureCode::ToolLoopUnavailable)
            )]
        );
        agent.stop().await;
    }

    #[tokio::test]
    async fn revoke_active_provider_waits_for_drop_then_commits_cancelled() {
        let runtime = Arc::new(FakeRuntime::new());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            Arc::new(FakeContext),
            Arc::new(FakeProvider {
                events: Vec::new(),
                hold: true,
            }),
            Arc::new(NoAgentToolInvoker),
            Arc::new(NoAgentAudit),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-cancel");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::task::yield_now().await;
        assert_eq!(
            consumer.revoke(&lease).await,
            RunCancellationDisposition::ChildSignalled
        );
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert_eq!(
            runtime.calls(),
            [RuntimeCall::Finish(1, RunTerminal::Cancelled)]
        );
        agent.stop().await;
    }

    #[tokio::test]
    async fn absolute_deadline_and_lease_heartbeat_start_before_context_finishes() {
        let runtime = Arc::new(FakeRuntime::new());
        let audit = Arc::new(FakeAudit::default());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            Arc::new(HoldingContext),
            Arc::new(FakeProvider {
                events: Vec::new(),
                hold: true,
            }),
            Arc::new(NoAgentToolInvoker),
            audit.clone(),
            BuiltInAgentConfig {
                queue_capacity: 4,
                max_concurrency: 2,
                lease_renew_interval: Duration::from_millis(5),
                run_deadline: Some(Duration::from_millis(30)),
            },
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-context-deadline");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        let calls = runtime.calls();
        assert!(calls.iter().any(|call| call == &RuntimeCall::Renew));
        assert_eq!(
            calls.last(),
            Some(&RuntimeCall::Finish(1, RunTerminal::Cancelled))
        );
        assert_eq!(
            audit.kinds(),
            [AgentAuditKind::Invoked, AgentAuditKind::RunDeadlineExceeded]
        );
        agent.stop().await;
    }

    #[tokio::test]
    async fn stream_stall_is_audited_before_failed_terminal() {
        let runtime = Arc::new(FakeRuntime::new());
        let audit = Arc::new(FakeAudit::default());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            Arc::new(FakeContext),
            Arc::new(FakeProvider {
                events: vec![ProviderEvent::Failed(ProviderFailure::StreamStalled)],
                hold: false,
            }),
            Arc::new(NoAgentToolInvoker),
            audit.clone(),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-stall-audit");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert_eq!(
            runtime.calls(),
            [RuntimeCall::Finish(
                1,
                RunTerminal::Failed(RunFailureCode::ProviderStreamStalled)
            )]
        );
        assert_eq!(
            audit.kinds(),
            [AgentAuditKind::Invoked, AgentAuditKind::StreamStalled]
        );
        agent.stop().await;
    }

    #[tokio::test]
    async fn provider_start_commit_unknown_is_reconciliation_not_retry_or_failure() {
        let runtime = Arc::new(FakeRuntime::new());
        let agent = BuiltInAgentRuntime::start(
            runtime.clone(),
            Arc::new(FakeContext),
            Arc::new(FailingStartProvider(ProviderPortError::CommitUnknown)),
            Arc::new(NoAgentToolInvoker),
            Arc::new(NoAgentAudit),
            test_config(),
        )
        .unwrap();
        let consumer = agent.consumer();
        let lease = lease("run-provider-unknown");
        assert_eq!(
            consumer.dispatch(lease.clone()).await,
            RunDispatchDecision::Accepted
        );
        consumer.activate(&lease).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), runtime.terminal.notified())
            .await
            .unwrap();
        assert_eq!(
            runtime.calls(),
            [RuntimeCall::Finish(
                1,
                RunTerminal::ReconciliationRequired(RunFailureCode::ProviderCommitUnknown)
            )]
        );
        agent.stop().await;
    }
}
