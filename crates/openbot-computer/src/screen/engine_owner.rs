//! Sole owner of a live Engine session and its ScreenHub demand lifecycle.

use std::sync::Arc;
use std::time::Duration;

use openbot_contracts::auth::AuthContext;

use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::browser::BrowserInput;
use crate::control::{ControlService, HumanInputTicket};
use crate::engine::{EngineProcess, EngineProcessError, ScreenIngressStats, ScreenStreamKey};

use super::{ScreenDemandObserver, ScreenHub};

/// One in-flight input plus one capture transition fit within the first-source two-second bound.
const OPERATION_DEADLINE: Duration = Duration::from_millis(750);
const SHUTDOWN_DEADLINE: Duration = Duration::from_millis(1500);
const COMMAND_CAPACITY: usize = 16;

/// Rust-owned observation state. No external frame or renderer can set these values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenEngineState {
    /// Source attached; the owner has not yet reconciled initial demand.
    Starting,
    /// At least one viewer is attached and capture is enabled.
    Running,
    /// Capture is stopped and all frames ACKed; the document and renderer are retained.
    Paused,
    /// Explicit shutdown/source invalidation completed.
    Closed,
    /// An operation failed or timed out; the process is retired, never reused.
    Failed,
}

/// Closed errors without engine prose, paths, user input, or scope identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScreenEngineError {
    /// Bounded command queue is full; nothing was enqueued.
    #[error("screen_engine_busy")]
    Busy,
    /// Owner or source is no longer available.
    #[error("screen_engine_closed")]
    Closed,
    /// Input was rejected by Rust authority before writing to the engine.
    #[error("screen_engine_input_refused")]
    InputRefused,
    /// Engine operation failed or exceeded its bound. Commit must not be inferred.
    #[error("screen_engine_unavailable")]
    Unavailable,
}

enum Command {
    Input {
        auth: Box<AuthContext>,
        ticket: HumanInputTicket,
        input: BrowserInput,
        reply: oneshot::Sender<Result<(), ScreenEngineError>>,
    },
    Stats(oneshot::Sender<Result<ScreenIngressStats, ScreenEngineError>>),
    Shutdown,
}

/// Non-Clone supervisor handle. It owns the task; Drop aborts it and drops the EngineProcess.
/// The source may be started by either Browser or Component role, but only one owner consumes it.
pub struct ScreenEngineOwner {
    commands: mpsc::Sender<Command>,
    state: watch::Receiver<ScreenEngineState>,
    task: Option<JoinHandle<Result<(), ScreenEngineError>>>,
}

impl ScreenEngineOwner {
    /// Consume an already started, authenticated EngineProcess and attach its unique source.
    /// Production ComputerManager must supply that process; this does not manufacture scope.
    pub async fn attach(
        mut engine: EngineProcess,
        hub: ScreenHub,
        control: Arc<Mutex<ControlService>>,
    ) -> Result<Self, ScreenEngineError> {
        let source = engine
            .take_screen_source()
            .map_err(|_| ScreenEngineError::Unavailable)?;
        let key = source.stream_key().clone();
        let demand = hub
            .attach(source)
            .await
            .map_err(|_| ScreenEngineError::Unavailable)?;
        let (commands, receiver) = mpsc::channel(COMMAND_CAPACITY);
        let (state, observer) = watch::channel(ScreenEngineState::Starting);
        let task = tokio::spawn(own_engine(
            engine, hub, key, demand, receiver, state, control,
        ));
        Ok(Self {
            commands,
            state: observer,
            task: Some(task),
        })
    }

    /// Read-only lifecycle observer for the Rust host. It carries no frame or authority payload.
    #[must_use]
    pub fn observe(&self) -> watch::Receiver<ScreenEngineState> {
        self.state.clone()
    }

    /// Submit one fresh typed input. Queue saturation and caller abandonment before execution
    /// produce zero engine effect. Once an input has started, failure remains an unknown outcome.
    pub async fn apply_input(
        &self,
        auth: AuthContext,
        ticket: HumanInputTicket,
        input: BrowserInput,
    ) -> Result<(), ScreenEngineError> {
        let (reply, result) = oneshot::channel();
        self.enqueue(Command::Input {
            auth: Box::new(auth),
            ticket,
            input,
            reply,
        })?;
        result.await.map_err(|_| ScreenEngineError::Closed)?
    }

    /// Read counters from the owned real ingress, including while capture is paused.
    pub async fn stats(&self) -> Result<ScreenIngressStats, ScreenEngineError> {
        let (reply, result) = oneshot::channel();
        self.enqueue(Command::Stats(reply))?;
        result.await.map_err(|_| ScreenEngineError::Closed)?
    }

    /// Retire the owner and its source. A saturated queue fails closed via this handle's Drop.
    pub async fn shutdown(mut self) -> Result<(), ScreenEngineError> {
        match self.enqueue(Command::Shutdown) {
            Ok(()) | Err(ScreenEngineError::Closed) => {}
            Err(error) => return Err(error),
        }
        self.task
            .as_mut()
            .ok_or(ScreenEngineError::Closed)?
            .await
            .map_err(|_| ScreenEngineError::Unavailable)?
    }

    fn enqueue(&self, command: Command) -> Result<(), ScreenEngineError> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => ScreenEngineError::Busy,
                mpsc::error::TrySendError::Closed(_) => ScreenEngineError::Closed,
            })
    }
}

impl Drop for ScreenEngineOwner {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn own_engine(
    mut engine: EngineProcess,
    hub: ScreenHub,
    key: ScreenStreamKey,
    mut demand: ScreenDemandObserver,
    mut commands: mpsc::Receiver<Command>,
    state: watch::Sender<ScreenEngineState>,
    control: Arc<Mutex<ControlService>>,
) -> Result<(), ScreenEngineError> {
    let mut current = demand.current();
    let mut casting = None;
    let healthy = loop {
        if current.is_closed() {
            break true;
        }
        let enabled = current.has_viewers();
        if casting != Some(enabled) {
            if !matches!(
                tokio::time::timeout(
                    OPERATION_DEADLINE,
                    engine.set_screencast(key.tab_id(), enabled)
                )
                .await,
                Ok(Ok(()))
            ) {
                break false;
            }
            casting = Some(enabled);
            state.send_replace(if enabled {
                ScreenEngineState::Running
            } else {
                ScreenEngineState::Paused
            });
        }
        tokio::select! {
            biased;
            next = demand.changed() => current = next,
            command = commands.recv() => match command {
                Some(Command::Input { auth, ticket, input, reply }) => {
                    if reply.is_closed() { continue; }
                    // Queue only a non-authority ticket. Revalidate the current lease at execution
                    // and hold its guard through the bounded write/ACK, so release/transfer cannot
                    // race between receipt minting and engine dispatch.
                    let outcome = tokio::time::timeout(OPERATION_DEADLINE, async {
                        let mut control = control.lock().await;
                        if reply.is_closed() { return Ok(()); }
                        if demand.is_closed() { return Err(ScreenEngineError::Closed); }
                        let now = time::OffsetDateTime::now_utc();
                        let authority = control.authorize_human_input_receipt(&auth, &ticket, now)
                            .map_err(|_| ScreenEngineError::InputRefused)?;
                        engine.apply_human_input(authority, &input, now).await.map_err(|error| match error {
                            EngineProcessError::InputAuthority | EngineProcessError::InputPlan(_) => ScreenEngineError::InputRefused,
                            _ => ScreenEngineError::Unavailable,
                        })
                    }).await;
                    match outcome {
                        Ok(Ok(())) => { let _ = reply.send(Ok(())); }
                        Ok(Err(ScreenEngineError::InputRefused)) => {
                            let _ = reply.send(Err(ScreenEngineError::InputRefused));
                        }
                        _ => {
                            let _ = reply.send(Err(ScreenEngineError::Unavailable));
                            break false;
                        }
                    }
                }
                Some(Command::Stats(reply)) => {
                    if reply.is_closed() { continue; }
                    let stats = engine.screen_stats().await.map_err(|_| ScreenEngineError::Unavailable);
                    let failed = stats.is_err();
                    let _ = reply.send(stats);
                    if failed { break false; }
                }
                Some(Command::Shutdown) | None => break true,
            }
        }
    };
    commands.close();
    hub.detach_registered(&key, &demand).await;
    // A timed-out wire operation is never followed by another command: its late response could
    // be mistaken for that command's response. Drop retires the process instead.
    let result = if healthy {
        match tokio::time::timeout(SHUTDOWN_DEADLINE, engine.shutdown()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) | Err(_) => Err(ScreenEngineError::Unavailable),
        }
    } else {
        drop(engine);
        Err(ScreenEngineError::Unavailable)
    };
    state.send_replace(if result.is_ok() {
        ScreenEngineState::Closed
    } else {
        ScreenEngineState::Failed
    });
    result
}
