//! Host-owned multiplexing from typed Desktop sessions into Tauri structured IPC channels.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use openbot_contracts::auth::AuthContext;
use openbot_contracts::command::SubscriptionRequest;
use openbot_contracts::ids::ThreadId;
use tauri::ipc::Channel;

use crate::broker::DisconnectReason;
use crate::budget::DeliveryClass;
use crate::event::{AppEventRef, GapCause, SequenceError, SequenceGap, SequenceTracker};
use crate::transport::{InProcessTransport, OpenSessionError};
use crate::window::{ThreadSubscriptions, WindowLabel};

pub use openbot_contracts::desktop::{
    DESKTOP_STRUCTURED_SUBSCRIPTION_ID_EXCLUSIVE_LIMIT, DesktopStructuredDeliveryClass,
    DesktopStructuredEventFrame, DesktopStructuredGapCause, DesktopStructuredSequenceGap,
    DesktopStructuredStreamKind, DesktopStructuredTerminalReason,
};

type ActiveSubscriptions = BTreeMap<WindowLabel, BTreeMap<u64, WindowLabel>>;

/// Opening a host-owned structured subscription failed.
#[derive(Debug, thiserror::Error)]
pub enum DesktopStructuredOpenError {
    /// Host subscription counter is exhausted; wrapping could collide with a live route.
    #[error("structured_subscription_counter_exhausted")]
    CounterExhausted,
    /// Application or in-process transport rejected the subscription.
    #[error(transparent)]
    Session(#[from] OpenSessionError),
}

/// Reading/projecting one subscription violated the sequence contract.
#[derive(Debug, thiserror::Error)]
pub enum DesktopStructuredFrameError {
    /// Broker sequence/gap invariants were violated.
    #[error(transparent)]
    Sequence(#[from] SequenceError),
    /// Sender disappeared without the mandatory terminal frame.
    #[error("structured_stream_ended_without_terminal")]
    MissingTerminal,
    /// Application emitted an event outside the closed requested stream family/scope.
    #[error("structured_stream_event_mismatch")]
    EventMismatch,
}

/// Final outcome of a Tauri Channel pump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopStructuredPumpExit {
    /// Terminal frame was delivered to the renderer.
    Terminal(DesktopStructuredTerminalReason),
    /// Renderer/Tauri callback disappeared; subscription was dropped and cancelled.
    SinkClosed,
    /// Internal sequence or closed-stream integrity failed; no misleading frame was sent.
    IntegrityViolation,
    /// Session ended without its mandatory terminal frame.
    MissingTerminal,
}

/// One process-wide bridge; internal broker labels and subscription IDs are host-owned.
#[derive(Clone)]
pub struct DesktopStructuredEventBridge {
    transport: Arc<InProcessTransport>,
    next_subscription_id: Arc<AtomicU64>,
    active: Arc<Mutex<ActiveSubscriptions>>,
}

impl DesktopStructuredEventBridge {
    /// Wrap the exact transport used by unary custom-protocol requests.
    #[must_use]
    pub fn new(transport: Arc<InProcessTransport>) -> Self {
        Self {
            transport,
            next_subscription_id: Arc::new(AtomicU64::new(0)),
            active: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Open one typed subscription for an already authenticated native window.
    pub async fn open(
        &self,
        window: WindowLabel,
        auth: &AuthContext,
        request: SubscriptionRequest,
    ) -> Result<DesktopStructuredSubscription, DesktopStructuredOpenError> {
        let subscription_id = self
            .next_subscription_id
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current
                    .checked_add(1)
                    .filter(|next| *next <= DESKTOP_STRUCTURED_SUBSCRIPTION_ID_EXCLUSIVE_LIMIT)
            })
            .map_err(|_| DesktopStructuredOpenError::CounterExhausted)?;
        let stream = DesktopStructuredStreamKind::from_request(&request);
        let expected_thread = match &request {
            SubscriptionRequest::ThreadEvents { thread_id, .. } => Some(thread_id.clone()),
            SubscriptionRequest::Health
            | SubscriptionRequest::ChannelActivity
            | SubscriptionRequest::ToolApprovalActivity => None,
        };
        let thread_subscriptions = expected_thread
            .clone()
            .map_or_else(ThreadSubscriptions::none, |thread_id| {
                ThreadSubscriptions::from_threads([thread_id])
            });
        // Length-prefixing makes `(window, id)` injective even when a host label contains separators.
        let internal_label = WindowLabel::new(format!(
            "{}:{}:{}",
            window.as_str().len(),
            window.as_str(),
            subscription_id
        ));
        let session = self
            .transport
            .open_session(internal_label.clone(), auth, request, thread_subscriptions)
            .await?;
        self.active
            .lock()
            .expect("structured subscription registry lock must not be poisoned")
            .entry(window.clone())
            .or_default()
            .insert(subscription_id, internal_label.clone());
        Ok(DesktopStructuredSubscription {
            subscription_id,
            stream,
            expected_thread,
            window,
            internal_label,
            transport: Arc::clone(&self.transport),
            active: Arc::clone(&self.active),
            session,
            tracker: SequenceTracker::new(),
            terminal_seen: false,
        })
    }

    /// Close every subscription owned by one actual native window.
    ///
    /// The registry entry is removed before any route is cancelled, so a concurrently finishing
    /// pump cannot resurrect stale ownership. The returned count is the exact number of registered
    /// subscriptions selected for cleanup.
    pub fn close_window(&self, window: &WindowLabel) -> usize {
        let registrations = self
            .active
            .lock()
            .expect("structured subscription registry lock must not be poisoned")
            .remove(window)
            .unwrap_or_default();
        let count = registrations.len();
        for internal_label in registrations.into_values() {
            let _ = self.transport.close_session(&internal_label);
        }
        count
    }

    /// Close one exact subscription only when it belongs to the host-observed actual window.
    ///
    /// Returning `false` covers unknown, already-finished, and another-window identities without
    /// revealing which case occurred.
    pub fn close_subscription(&self, window: &WindowLabel, subscription_id: u64) -> bool {
        let Some(internal_label) = remove_registration(&self.active, window, subscription_id)
        else {
            return false;
        };
        let _ = self.transport.close_session(&internal_label);
        true
    }

    /// Number of subscriptions still registered under actual host window labels.
    #[must_use]
    pub fn active_subscription_count(&self) -> usize {
        self.active
            .lock()
            .expect("structured subscription registry lock must not be poisoned")
            .values()
            .map(BTreeMap::len)
            .sum()
    }
}

/// One live subscription. Dropping it immediately cancels its host-owned event pump.
pub struct DesktopStructuredSubscription {
    subscription_id: u64,
    stream: DesktopStructuredStreamKind,
    expected_thread: Option<ThreadId>,
    window: WindowLabel,
    internal_label: WindowLabel,
    transport: Arc<InProcessTransport>,
    active: Arc<Mutex<ActiveSubscriptions>>,
    session: crate::DesktopSession,
    tracker: SequenceTracker,
    terminal_seen: bool,
}

impl DesktopStructuredSubscription {
    /// Host-visible actual window label, never the internal broker route label.
    #[must_use]
    pub fn window(&self) -> &WindowLabel {
        &self.window
    }

    /// Host-minted identity.
    #[must_use]
    pub const fn subscription_id(&self) -> u64 {
        self.subscription_id
    }

    /// Read, sequence-check, and project the next frame.
    pub async fn next_frame(
        &mut self,
    ) -> Result<Option<DesktopStructuredEventFrame>, DesktopStructuredFrameError> {
        if self.terminal_seen {
            return Ok(None);
        }
        let frame = self
            .session
            .next_frame()
            .await
            .ok_or(DesktopStructuredFrameError::MissingTerminal)?;
        self.tracker.observe(&frame)?;
        if frame.event().is_some_and(|event| {
            !self
                .stream
                .accepts_event(event, self.expected_thread.as_ref())
        }) {
            return Err(DesktopStructuredFrameError::EventMismatch);
        }
        let projected = project_frame(self.subscription_id, self.stream, &frame);
        self.terminal_seen = projected.terminal_reason().is_some();
        Ok(Some(projected))
    }
}

impl Drop for DesktopStructuredSubscription {
    fn drop(&mut self) {
        let _ = remove_registration(&self.active, &self.window, self.subscription_id);
        let _ = self.transport.close_session(&self.internal_label);
    }
}

fn remove_registration(
    active: &Mutex<ActiveSubscriptions>,
    window: &WindowLabel,
    subscription_id: u64,
) -> Option<WindowLabel> {
    let mut active = active
        .lock()
        .expect("structured subscription registry lock must not be poisoned");
    let registrations = active.get_mut(window)?;
    let internal_label = registrations.remove(&subscription_id)?;
    if registrations.is_empty() {
        active.remove(window);
    }
    Some(internal_label)
}

/// Pump one typed subscription into an actual Tauri IPC Channel.
pub async fn pump_tauri_structured_events(
    mut subscription: DesktopStructuredSubscription,
    channel: Channel<String>,
) -> DesktopStructuredPumpExit {
    loop {
        let frame = match subscription.next_frame().await {
            Ok(Some(frame)) => frame,
            Ok(None) => return DesktopStructuredPumpExit::MissingTerminal,
            Err(DesktopStructuredFrameError::Sequence(_)) => {
                return DesktopStructuredPumpExit::IntegrityViolation;
            }
            Err(DesktopStructuredFrameError::EventMismatch) => {
                return DesktopStructuredPumpExit::IntegrityViolation;
            }
            Err(DesktopStructuredFrameError::MissingTerminal) => {
                return DesktopStructuredPumpExit::MissingTerminal;
            }
        };
        let terminal = frame.terminal_reason();
        let Ok(frame) = serde_json::to_string(&frame) else {
            return DesktopStructuredPumpExit::IntegrityViolation;
        };
        if channel.send(frame).is_err() {
            return DesktopStructuredPumpExit::SinkClosed;
        }
        if let Some(reason) = terminal {
            return DesktopStructuredPumpExit::Terminal(reason);
        }
    }
}

fn project_frame(
    subscription_id: u64,
    stream: DesktopStructuredStreamKind,
    frame: &AppEventRef,
) -> DesktopStructuredEventFrame {
    let skipped = frame.skipped().map(project_gap);
    if let Some(event) = frame.event() {
        return DesktopStructuredEventFrame::Event {
            subscription_id,
            stream,
            sequence: frame.seq(),
            skipped,
            event: event.clone(),
        };
    }
    let reason = frame
        .terminal_reason()
        .expect("non-event Desktop frame must be terminal");
    let (reason, overflow_class) = match reason {
        DisconnectReason::QueueOverflow { class } => (
            DesktopStructuredTerminalReason::QueueOverflow,
            Some(project_delivery_class(class)),
        ),
        DisconnectReason::Shutdown => (DesktopStructuredTerminalReason::Shutdown, None),
        DisconnectReason::UpstreamEnded => (DesktopStructuredTerminalReason::UpstreamEnded, None),
        DisconnectReason::SubscriptionClosed => {
            (DesktopStructuredTerminalReason::SubscriptionClosed, None)
        }
    };
    DesktopStructuredEventFrame::Terminal {
        subscription_id,
        stream,
        sequence: frame.seq(),
        skipped,
        reason,
        overflow_class,
    }
}

fn project_gap(gap: SequenceGap) -> DesktopStructuredSequenceGap {
    DesktopStructuredSequenceGap {
        from_sequence: gap.from_seq,
        through_sequence: gap.through_seq,
        cause: match gap.cause {
            GapCause::Superseded => DesktopStructuredGapCause::Superseded,
            GapCause::Dropped => DesktopStructuredGapCause::Dropped,
        },
    }
}

fn project_delivery_class(class: DeliveryClass) -> DesktopStructuredDeliveryClass {
    match class {
        DeliveryClass::Critical => DesktopStructuredDeliveryClass::Critical,
        DeliveryClass::Coalescable => DesktopStructuredDeliveryClass::Coalescable,
        DeliveryClass::LatestValue => DesktopStructuredDeliveryClass::LatestValue,
        DeliveryClass::Screen => DesktopStructuredDeliveryClass::Screen,
    }
}

#[cfg(test)]
mod tests {
    use core::pin::Pin;
    use core::task::{Context, Poll};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use openbot_application::{AppEventStream, ApplicationService};
    use openbot_contracts::command::{
        AppCommand, AppEvent, AppReply, ChannelActivityEvent, HealthReport, ThreadRunEvent,
        ThreadRunEventKind,
    };
    use openbot_contracts::error::AppError;
    use openbot_contracts::ids::{BotId, ChannelId, RunId, ThreadId};
    use tauri::ipc::InvokeResponseBody;

    use super::*;
    use crate::testing::auth_for;

    struct FiniteStream(VecDeque<AppEvent>);

    impl futures_core::Stream for FiniteStream {
        type Item = AppEvent;

        fn poll_next(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Option<Self::Item>> {
            Poll::Ready(self.0.pop_front())
        }
    }

    struct ScriptedService;

    #[async_trait]
    impl ApplicationService for ScriptedService {
        async fn execute(
            &self,
            _auth: AuthContext,
            _command: AppCommand,
        ) -> Result<AppReply, AppError> {
            Ok(AppReply::Health(HealthReport { ok: true }))
        }

        async fn subscribe(
            &self,
            _auth: AuthContext,
            request: SubscriptionRequest,
        ) -> Result<AppEventStream, AppError> {
            let event = match request {
                SubscriptionRequest::ThreadEvents { thread_id, .. } => {
                    AppEvent::ThreadRunEvent(ThreadRunEvent {
                        thread_id,
                        run_id: RunId::new("run-1"),
                        event_sequence: 1,
                        event_type: ThreadRunEventKind::Started,
                        payload: serde_json::json!({}),
                        terminal: false,
                        created_at: time::OffsetDateTime::UNIX_EPOCH,
                    })
                }
                SubscriptionRequest::ChannelActivity => {
                    AppEvent::ChannelActivity(ChannelActivityEvent {
                        channel_id: ChannelId::new("channel-1"),
                        last_message: Some("updated".to_owned()),
                        last_message_at: Some(time::OffsetDateTime::UNIX_EPOCH),
                        last_message_agent_id: Some(BotId::new("agent-1")),
                    })
                }
                SubscriptionRequest::Health => AppEvent::Heartbeat { seq: 1 },
                SubscriptionRequest::ToolApprovalActivity => AppEvent::ToolApprovalActivity(
                    openbot_contracts::tool::ToolApprovalActivityEvent { pending_count: 1 },
                ),
            };
            Ok(Box::pin(FiniteStream(VecDeque::from([event]))))
        }
    }

    struct PendingStream;

    impl futures_core::Stream for PendingStream {
        type Item = AppEvent;

        fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Pending
        }
    }

    struct PendingService;

    #[async_trait]
    impl ApplicationService for PendingService {
        async fn execute(
            &self,
            _auth: AuthContext,
            _command: AppCommand,
        ) -> Result<AppReply, AppError> {
            Ok(AppReply::Health(HealthReport { ok: true }))
        }

        async fn subscribe(
            &self,
            _auth: AuthContext,
            _request: SubscriptionRequest,
        ) -> Result<AppEventStream, AppError> {
            Ok(Box::pin(PendingStream))
        }
    }

    struct MismatchedService;

    #[async_trait]
    impl ApplicationService for MismatchedService {
        async fn execute(
            &self,
            _auth: AuthContext,
            _command: AppCommand,
        ) -> Result<AppReply, AppError> {
            Ok(AppReply::Health(HealthReport { ok: true }))
        }

        async fn subscribe(
            &self,
            _auth: AuthContext,
            request: SubscriptionRequest,
        ) -> Result<AppEventStream, AppError> {
            let event = match request {
                SubscriptionRequest::ThreadEvents { .. } => {
                    AppEvent::ThreadRunEvent(ThreadRunEvent {
                        thread_id: ThreadId::new("550e8400-e29b-81d4-a716-446655440099"),
                        run_id: RunId::new("run-mismatch"),
                        event_sequence: 1,
                        event_type: ThreadRunEventKind::Started,
                        payload: serde_json::json!({}),
                        terminal: false,
                        created_at: time::OffsetDateTime::UNIX_EPOCH,
                    })
                }
                SubscriptionRequest::Health
                | SubscriptionRequest::ChannelActivity
                | SubscriptionRequest::ToolApprovalActivity => {
                    AppEvent::ChannelActivity(ChannelActivityEvent {
                        channel_id: ChannelId::new("channel-mismatch"),
                        last_message: None,
                        last_message_at: None,
                        last_message_agent_id: None,
                    })
                }
            };
            Ok(Box::pin(FiniteStream(VecDeque::from([event]))))
        }
    }

    fn collecting_channel(frames: Arc<Mutex<Vec<DesktopStructuredEventFrame>>>) -> Channel<String> {
        Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                panic!("structured frame must use the JSON IPC lane");
            };
            let frame_json = serde_json::from_str::<String>(&json)?;
            frames
                .lock()
                .unwrap()
                .push(serde_json::from_str(&frame_json)?);
            Ok(())
        })
    }

    #[tokio::test]
    async fn subscription_identity_exhaustion_fails_before_opening_a_route() {
        let transport = Arc::new(InProcessTransport::new(Arc::new(PendingService)));
        let bridge = DesktopStructuredEventBridge::new(Arc::clone(&transport));
        bridge.next_subscription_id.store(
            DESKTOP_STRUCTURED_SUBSCRIPTION_ID_EXCLUSIVE_LIMIT,
            Ordering::SeqCst,
        );

        assert!(matches!(
            bridge
                .open(
                    WindowLabel::new("main"),
                    &auth_for("actor-1"),
                    SubscriptionRequest::Health,
                )
                .await,
            Err(DesktopStructuredOpenError::CounterExhausted)
        ));
        assert_eq!(transport.broker().window_count(), 0);
        assert_eq!(bridge.active_subscription_count(), 0);
    }

    #[tokio::test]
    async fn one_window_can_hold_multiple_host_owned_tauri_channels() {
        let transport = Arc::new(InProcessTransport::new(Arc::new(ScriptedService)));
        let bridge = DesktopStructuredEventBridge::new(Arc::clone(&transport));
        let auth = auth_for("actor-1");
        let actual_window = WindowLabel::new("main");
        let thread = ThreadId::new("550e8400-e29b-81d4-a716-446655440000");
        let thread_subscription = bridge
            .open(
                actual_window.clone(),
                &auth,
                SubscriptionRequest::ThreadEvents {
                    thread_id: thread.clone(),
                    after_event_sequence: None,
                },
            )
            .await
            .unwrap();
        let channel_subscription = bridge
            .open(
                actual_window.clone(),
                &auth,
                SubscriptionRequest::ChannelActivity,
            )
            .await
            .unwrap();
        assert_eq!(thread_subscription.window(), &actual_window);
        assert_eq!(channel_subscription.window(), &actual_window);
        assert_ne!(
            thread_subscription.subscription_id(),
            channel_subscription.subscription_id()
        );
        assert_eq!(transport.broker().window_count(), 2);
        assert_eq!(bridge.active_subscription_count(), 2);

        let thread_frames = Arc::new(Mutex::new(Vec::new()));
        let channel_frames = Arc::new(Mutex::new(Vec::new()));
        let (thread_exit, channel_exit) = tokio::join!(
            pump_tauri_structured_events(
                thread_subscription,
                collecting_channel(Arc::clone(&thread_frames)),
            ),
            pump_tauri_structured_events(
                channel_subscription,
                collecting_channel(Arc::clone(&channel_frames)),
            ),
        );
        assert_eq!(
            thread_exit,
            DesktopStructuredPumpExit::Terminal(DesktopStructuredTerminalReason::UpstreamEnded)
        );
        assert_eq!(
            channel_exit,
            DesktopStructuredPumpExit::Terminal(DesktopStructuredTerminalReason::UpstreamEnded)
        );
        assert_eq!(transport.broker().window_count(), 0);
        assert_eq!(bridge.active_subscription_count(), 0);

        let thread_frames = thread_frames.lock().unwrap();
        let channel_frames = channel_frames.lock().unwrap();
        assert_eq!(thread_frames.len(), 2);
        assert_eq!(channel_frames.len(), 2);
        assert!(matches!(
            &thread_frames[0],
            DesktopStructuredEventFrame::Event {
                stream: DesktopStructuredStreamKind::ThreadEvents,
                event: AppEvent::ThreadRunEvent(event),
                ..
            } if event.thread_id == thread
        ));
        assert!(matches!(
            &channel_frames[0],
            DesktopStructuredEventFrame::Event {
                stream: DesktopStructuredStreamKind::ChannelActivity,
                event: AppEvent::ChannelActivity(_),
                ..
            }
        ));
        assert!(matches!(
            thread_frames[1],
            DesktopStructuredEventFrame::Terminal {
                reason: DesktopStructuredTerminalReason::UpstreamEnded,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn dropping_one_subscription_cancels_only_its_internal_route() {
        let transport = Arc::new(InProcessTransport::new(Arc::new(PendingService)));
        let bridge = DesktopStructuredEventBridge::new(Arc::clone(&transport));
        let auth = auth_for("actor-1");
        let main = WindowLabel::new("main");
        let mut first = bridge
            .open(main.clone(), &auth, SubscriptionRequest::ChannelActivity)
            .await
            .unwrap();
        let second = bridge
            .open(
                main.clone(),
                &auth,
                SubscriptionRequest::ToolApprovalActivity,
            )
            .await
            .unwrap();
        assert_eq!(transport.broker().window_count(), 2);
        assert_eq!(bridge.active_subscription_count(), 2);
        assert!(
            !bridge
                .close_subscription(&WindowLabel::new("another-window"), first.subscription_id())
        );
        assert!(bridge.close_subscription(&main, first.subscription_id()));
        assert!(!bridge.close_subscription(&main, first.subscription_id()));
        let terminal = first.next_frame().await.unwrap().unwrap();
        assert_eq!(
            terminal.terminal_reason(),
            Some(DesktopStructuredTerminalReason::SubscriptionClosed)
        );
        assert_eq!(transport.broker().window_count(), 1);
        assert_eq!(bridge.active_subscription_count(), 1);
        drop(first);
        assert_eq!(transport.broker().window_count(), 1);
        assert_eq!(bridge.active_subscription_count(), 1);
        drop(second);
        assert_eq!(transport.broker().window_count(), 0);
        assert_eq!(bridge.active_subscription_count(), 0);
    }

    #[tokio::test]
    async fn closing_one_actual_window_cancels_all_and_only_its_subscriptions() {
        let transport = Arc::new(InProcessTransport::new(Arc::new(PendingService)));
        let bridge = DesktopStructuredEventBridge::new(Arc::clone(&transport));
        let auth = auth_for("actor-1");
        let main = WindowLabel::new("main");
        let auxiliary = WindowLabel::new("auxiliary");
        let mut main_channels = bridge
            .open(main.clone(), &auth, SubscriptionRequest::ChannelActivity)
            .await
            .unwrap();
        let mut main_approvals = bridge
            .open(
                main.clone(),
                &auth,
                SubscriptionRequest::ToolApprovalActivity,
            )
            .await
            .unwrap();
        let auxiliary_health = bridge
            .open(auxiliary.clone(), &auth, SubscriptionRequest::Health)
            .await
            .unwrap();
        assert_eq!(transport.broker().window_count(), 3);
        assert_eq!(bridge.active_subscription_count(), 3);

        assert_eq!(bridge.close_window(&main), 2);
        assert_eq!(bridge.close_window(&main), 0, "window close is idempotent");
        assert_eq!(transport.broker().window_count(), 1);
        assert_eq!(bridge.active_subscription_count(), 1);
        for subscription in [&mut main_channels, &mut main_approvals] {
            let terminal = subscription.next_frame().await.unwrap().unwrap();
            assert_eq!(
                terminal.terminal_reason(),
                Some(DesktopStructuredTerminalReason::SubscriptionClosed)
            );
            assert!(subscription.next_frame().await.unwrap().is_none());
        }

        drop(auxiliary_health);
        assert_eq!(transport.broker().window_count(), 0);
        assert_eq!(bridge.active_subscription_count(), 0);
    }

    #[tokio::test]
    async fn stream_family_and_exact_thread_are_checked_before_ipc_projection() {
        let transport = Arc::new(InProcessTransport::new(Arc::new(MismatchedService)));
        let bridge = DesktopStructuredEventBridge::new(Arc::clone(&transport));
        let auth = auth_for("actor-1");
        let requests = [
            SubscriptionRequest::Health,
            SubscriptionRequest::ThreadEvents {
                thread_id: ThreadId::new("550e8400-e29b-81d4-a716-446655440000"),
                after_event_sequence: None,
            },
        ];

        for (index, request) in requests.into_iter().enumerate() {
            let mut subscription = bridge
                .open(WindowLabel::new(format!("window-{index}")), &auth, request)
                .await
                .unwrap();
            assert!(matches!(
                subscription.next_frame().await,
                Err(DesktopStructuredFrameError::EventMismatch)
            ));
            drop(subscription);
        }
        assert_eq!(transport.broker().window_count(), 0);
        assert_eq!(bridge.active_subscription_count(), 0);
    }

    #[tokio::test]
    async fn a_closed_tauri_sink_cancels_the_host_subscription() {
        let transport = Arc::new(InProcessTransport::new(Arc::new(ScriptedService)));
        let bridge = DesktopStructuredEventBridge::new(Arc::clone(&transport));
        let subscription = bridge
            .open(
                WindowLabel::new("main"),
                &auth_for("actor-1"),
                SubscriptionRequest::Health,
            )
            .await
            .unwrap();
        let channel = Channel::new(|_| Err(tauri::Error::FailedToReceiveMessage));

        assert_eq!(
            pump_tauri_structured_events(subscription, channel).await,
            DesktopStructuredPumpExit::SinkClosed
        );
        assert_eq!(transport.broker().window_count(), 0);
        assert_eq!(bridge.active_subscription_count(), 0);
    }

    #[test]
    fn non_overflow_terminal_wire_omits_the_overflow_class() {
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
}
