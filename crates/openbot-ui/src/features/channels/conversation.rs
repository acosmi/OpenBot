//! Native channel conversation: atomic snapshot, durable SSE replay/live, and idle sends.

#![cfg_attr(
    not(any(test, target_arch = "wasm32")),
    allow(dead_code, unused_variables)
)]

use core::fmt::Write as _;

use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use openbot_contracts::command::AppEvent;
use openbot_contracts::command::{
    ChannelDetail, ThreadConversationSnapshot, ThreadHistoryMessage, ThreadHistoryRole,
    ThreadRunEvent, ThreadRunEventKind,
};
use openbot_contracts::ids::{RunId, ThreadId};
use openbot_contracts::text::trim_ecmascript;
use sha2::{Digest, Sha256};

#[cfg(target_arch = "wasm32")]
use crate::api::{
    begin_channel_run, load_thread_conversation, mint_run_id, mint_thread_id,
    thread_event_stream_path,
};
use crate::features::agents::{AgentPresence, AgentPresenceState};
use crate::features::threads::tool_name::read_tool_name;
use crate::features::threads::tool_result::for_display;
use crate::i18n::{t, t_string, use_i18n};
use crate::icons::Icon;
use crate::primitives::{
    Avatar, AvatarSize, Bubble, BubbleKind, Button, ButtonSize, ButtonVariant, IconSize, IconView,
    Message, MessageAlign, MessageAvatar, MessageContent, MessageHeader, MessageScroller,
    MessageScrollerButton, MessageScrollerContent, MessageScrollerItem, MessageScrollerViewport,
    Textarea,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast as _;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use web_sys::{Event, EventSource, MessageEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TranscriptKind {
    User,
    Assistant,
    ToolCall,
    ToolResult,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TranscriptLine {
    id: String,
    kind: TranscriptKind,
    content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalNotice {
    Failed,
    Cancelled,
    ReconciliationRequired,
}

impl TerminalNotice {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::ReconciliationRequired => "reconciliation_required",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ConversationState {
    messages: Vec<TranscriptLine>,
    active_run_id: Option<RunId>,
    streaming_text: String,
    cursor: Option<u64>,
    terminal_notice: Option<TerminalNotice>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LiveEffect {
    None,
    ReloadSnapshot,
}

impl ConversationState {
    fn install_snapshot(&mut self, snapshot: ThreadConversationSnapshot) {
        self.messages = project_history(&snapshot.messages);
        self.active_run_id = snapshot.active_run_id;
        self.streaming_text = snapshot.active_run_text;
        self.cursor = snapshot.last_event_sequence;
        if self.active_run_id.is_some() {
            self.terminal_notice = None;
        }
    }
}

fn apply_live_event(
    state: &mut ConversationState,
    expected_thread: &ThreadId,
    event: &ThreadRunEvent,
) -> Result<LiveEffect, ()> {
    if &event.thread_id != expected_thread || event.terminal != event.event_type.is_terminal() {
        return Err(());
    }
    if state
        .cursor
        .is_some_and(|cursor| event.event_sequence <= cursor)
    {
        return Ok(LiveEffect::None);
    }
    if state
        .cursor
        .is_some_and(|cursor| cursor.checked_add(1) != Some(event.event_sequence))
    {
        return Ok(LiveEffect::ReloadSnapshot);
    }
    state.cursor = Some(event.event_sequence);
    match event.event_type {
        ThreadRunEventKind::Started => {
            state.active_run_id = Some(event.run_id.clone());
            state.streaming_text.clear();
            state.terminal_notice = None;
            Ok(LiveEffect::None)
        }
        ThreadRunEventKind::SemanticChunk => {
            if state.active_run_id.as_ref() != Some(&event.run_id) {
                return Ok(LiveEffect::ReloadSnapshot);
            }
            let Some(channel) = event
                .payload
                .get("channel")
                .and_then(serde_json::Value::as_str)
            else {
                return Err(());
            };
            let Some(delta) = event
                .payload
                .get("delta")
                .and_then(serde_json::Value::as_str)
            else {
                return Err(());
            };
            match channel {
                "text" => state.streaming_text.push_str(delta),
                "reasoning" => {}
                _ => return Err(()),
            }
            Ok(LiveEffect::None)
        }
        ThreadRunEventKind::Checkpoint => Ok(LiveEffect::None),
        ThreadRunEventKind::Completed => {
            state.active_run_id = None;
            state.terminal_notice = None;
            Ok(LiveEffect::ReloadSnapshot)
        }
        ThreadRunEventKind::Failed => {
            state.active_run_id = None;
            state.terminal_notice = Some(TerminalNotice::Failed);
            Ok(LiveEffect::ReloadSnapshot)
        }
        ThreadRunEventKind::Cancelled => {
            state.active_run_id = None;
            state.terminal_notice = Some(TerminalNotice::Cancelled);
            Ok(LiveEffect::ReloadSnapshot)
        }
        ThreadRunEventKind::ReconciliationRequired => {
            state.active_run_id = Some(event.run_id.clone());
            state.terminal_notice = Some(TerminalNotice::ReconciliationRequired);
            Ok(LiveEffect::ReloadSnapshot)
        }
    }
}

fn project_history(messages: &[ThreadHistoryMessage]) -> Vec<TranscriptLine> {
    let mut projected = Vec::new();
    for message in messages {
        match message.role {
            ThreadHistoryRole::System => {}
            ThreadHistoryRole::User => projected.push(TranscriptLine {
                id: message.id.clone(),
                kind: TranscriptKind::User,
                content: message.content.clone(),
            }),
            ThreadHistoryRole::Assistant => {
                if !message.content.is_empty() {
                    projected.push(TranscriptLine {
                        id: message.id.clone(),
                        kind: TranscriptKind::Assistant,
                        content: message.content.clone(),
                    });
                }
                if let Some(tool_calls) = &message.tool_calls {
                    let names = tool_calls
                        .iter()
                        .filter_map(|call| {
                            call.get("function")
                                .and_then(|function| function.get("name"))
                                .and_then(serde_json::Value::as_str)
                        })
                        .map(|name| {
                            let display = read_tool_name(name);
                            display.detail.map_or(display.label.clone(), |detail| {
                                format!("{} · {detail}", display.label)
                            })
                        })
                        .collect::<Vec<_>>();
                    if !names.is_empty() {
                        projected.push(TranscriptLine {
                            id: format!("{}:tools", message.id),
                            kind: TranscriptKind::ToolCall,
                            content: names.join("\n"),
                        });
                    }
                }
            }
            ThreadHistoryRole::Tool => projected.push(TranscriptLine {
                id: message.id.clone(),
                kind: TranscriptKind::ToolResult,
                content: for_display(&message.content),
            }),
        }
    }
    projected
}

#[derive(Clone)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
struct PendingTurn {
    thread_id: ThreadId,
    run_id: RunId,
    message: String,
}

/// Data-backed channel transcript and idle-send surface. Queue/Stop remain absent until cancellation lands.
#[component]
pub fn ChannelConversation(
    /// Current membership-authorized channel projection from the Server.
    channel: ChannelDetail,
) -> impl IntoView {
    let i18n = use_i18n();
    let channel_id = StoredValue::new(channel.id.clone());
    #[cfg(not(target_arch = "wasm32"))]
    let _ = channel_id;
    let selected_agent = channel.agent_ids.first().cloned();
    let agent_seed = selected_agent.as_ref().map_or_else(
        || channel.id.as_str().to_owned(),
        |id| id.as_str().to_owned(),
    );
    let agent_id = StoredValue::new(selected_agent);
    let agent_name = channel.name.clone();
    let streaming_agent_seed = StoredValue::new(agent_seed.clone());
    let streaming_agent_name = StoredValue::new(agent_name.clone());
    let channel_active = channel.active;
    let thread_id = RwSignal::new(channel.thread_id);
    let state = RwSignal::new(ConversationState::default());
    let loading = RwSignal::new(true);
    let snapshot_error = RwSignal::new(false);
    let stream_error = RwSignal::new(false);
    let reload_generation = RwSignal::new(0_u64);
    install_conversation_sync(
        thread_id,
        state,
        loading,
        snapshot_error,
        stream_error,
        reload_generation,
    );

    let draft = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);
    let send_error = RwSignal::new(false);
    let resumable = RwSignal::new(None::<PendingTurn>);
    Effect::new(move |_| {
        let Some(attempt) = resumable.get() else {
            return;
        };
        if state.get().active_run_id.as_ref() == Some(&attempt.run_id) {
            resumable.set(None);
            draft.set(String::new());
            send_error.set(false);
        }
    });
    let busy = Signal::derive(move || state.get().active_run_id.is_some() || submitting.get());
    let input_locked = Signal::derive(move || submitting.get() || resumable.get().is_some());
    let textarea_disabled = Signal::derive(move || input_locked.get() || !channel_active);
    let send_disabled = Signal::derive(move || {
        busy.get()
            || !channel_active
            || agent_id.get_value().is_none()
            || snapshot_error.get()
            || loading.get()
            || trim_ecmascript(&draft.get()).is_empty()
    });
    let submit = UnsyncCallback::new(move |_| {
        if send_disabled.get_untracked() {
            return;
        }
        let Some(agent_id) = agent_id.get_value() else {
            return;
        };
        let message = draft.get_untracked();
        if trim_ecmascript(&message).is_empty() {
            return;
        }
        submitting.set(true);
        send_error.set(false);
        #[cfg(target_arch = "wasm32")]
        leptos::task::spawn_local_scoped_with_cancellation(async move {
            let attempt = match resumable.get_untracked() {
                Some(attempt) => attempt,
                None => {
                    let resolved_thread = match thread_id.get_untracked() {
                        Some(thread) => thread,
                        None => match mint_thread_id().await {
                            Ok(thread) => thread,
                            Err(_) => {
                                send_error.set(true);
                                submitting.set(false);
                                return;
                            }
                        },
                    };
                    let attempt = PendingTurn {
                        thread_id: resolved_thread,
                        run_id: mint_run_id(),
                        message,
                    };
                    resumable.set(Some(attempt.clone()));
                    attempt
                }
            };
            match begin_channel_run(
                &attempt.thread_id,
                &channel_id.get_value(),
                &agent_id,
                &attempt.run_id,
                &attempt.message,
            )
            .await
            {
                Ok(_) => {
                    thread_id.set(Some(attempt.thread_id));
                    state.update(|state| {
                        state.active_run_id = Some(attempt.run_id);
                        state.streaming_text.clear();
                        state.terminal_notice = None;
                    });
                    draft.set(String::new());
                    resumable.set(None);
                    reload_generation.update(|value| *value = value.saturating_add(1));
                }
                Err(_) => {
                    send_error.set(true);
                    reload_generation.update(|value| *value = value.saturating_add(1));
                }
            }
            submitting.set(false);
        });
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (agent_id, message);
            submitting.set(false);
            send_error.set(true);
        }
    });
    let retry_snapshot = move |_| {
        reload_generation.update(|value| *value = value.saturating_add(1));
    };

    view! {
        <div class="ob-channel-conversation">
            <Show when=move || loading.get()>
                <div class="ob-loading" role="status">{move || t!(i18n, common.loading)}</div>
            </Show>
            <Show when=move || snapshot_error.get()>
                <div class="ob-alert" role="alert">
                    <span>{move || t!(i18n, channels.conversation_load_error)}</span>
                    <Button
                        variant=ButtonVariant::Ghost
                        size=ButtonSize::Small
                        on_activate=retry_snapshot
                    >{move || t!(i18n, common.retry)}</Button>
                </div>
            </Show>
            <Show when=move || stream_error.get() && !snapshot_error.get()>
                <p class="ob-alert" role="status">{move || t!(i18n, channels.conversation_stream_error)}</p>
            </Show>
            <MessageScroller
                id="channel-transcript"
                aria_label=move || t_string!(i18n, channels.transcript_label).to_owned()
            >
                <MessageScrollerViewport>
                    <MessageScrollerContent busy=busy>
                        <Show when=move || {
                            !loading.get()
                                && state.get().messages.is_empty()
                                && state.get().streaming_text.is_empty()
                        }>
                            <p class="ob-page-empty">{move || t!(i18n, channels.conversation_empty)}</p>
                        </Show>
                        <For
                            each=move || state.get().messages
                            key=|message| message.id.clone()
                            children={
                                let agent_seed = agent_seed.clone();
                                let agent_name = agent_name.clone();
                                move |message| view! {
                                    <TranscriptMessage
                                        message
                                        agent_seed=agent_seed.clone()
                                        agent_name=agent_name.clone()
                                    />
                                }
                            }
                        />
                        <Show when=move || !state.get().streaming_text.is_empty()>
                            {move || state.get().active_run_id.map(|run_id| view! {
                                <MessageScrollerItem
                                    message_id=transcript_dom_id(run_id.as_str())
                                >
                                    <div data-streaming-message="">
                                        <Message
                                            aria_label=move || t_string!(i18n, channels.streaming_reply_label).to_owned()
                                        >
                                            <MessageAvatar>
                                                <span aria-hidden="true">
                                                    <Avatar
                                                        principal_id=streaming_agent_seed.get_value()
                                                        name=streaming_agent_name.get_value()
                                                        size=AvatarSize::Small
                                                    />
                                                </span>
                                            </MessageAvatar>
                                            <MessageContent>
                                                <MessageHeader>{streaming_agent_name.get_value()}</MessageHeader>
                                                <Bubble kind=BubbleKind::Assistant>
                                                    <p class="ob-transcript-text">{move || state.get().streaming_text}</p>
                                                </Bubble>
                                            </MessageContent>
                                        </Message>
                                    </div>
                                </MessageScrollerItem>
                            })}
                        </Show>
                        <Show when=move || busy.get() && state.get().streaming_text.is_empty()>
                            <div class="ob-conversation-thinking" role="status">
                                <AgentPresence state=Signal::derive(move || AgentPresenceState::Thinking) />
                                <span>{move || t!(i18n, channels.tool_running)}</span>
                            </div>
                        </Show>
                        <Show when=move || state.get().terminal_notice.is_some()>
                            <p class="ob-alert" role="status">{move || terminal_text(i18n, state.get().terminal_notice)}</p>
                        </Show>
                    </MessageScrollerContent>
                </MessageScrollerViewport>
                <MessageScrollerButton
                    aria_label=move || t_string!(i18n, channels.transcript_back_to_bottom).to_owned()
                />
            </MessageScroller>
            <div class="ob-channel-composer">
                <Textarea
                    value=draft
                    id="channel-message"
                    aria_label=move || t_string!(i18n, channels.composer_placeholder).to_owned()
                    placeholder=t_string!(i18n, channels.composer_placeholder).to_owned()
                    disabled=textarea_disabled
                    on_submit=submit
                />
                <Button
                    variant=ButtonVariant::Primary
                    size=ButtonSize::Medium
                    disabled=send_disabled
                    loading=submitting
                    on_activate=submit
                >
                    <IconView icon=Icon::Send size=IconSize::Inline />
                    <span>{move || if resumable.get().is_some() {
                        t_string!(i18n, common.retry).to_owned()
                    } else {
                        t_string!(i18n, channels.composer_send).to_owned()
                    }}</span>
                </Button>
            </div>
            <Show when=move || send_error.get()>
                <p class="ob-alert" role="alert">{move || t!(i18n, channels.send_error)}</p>
            </Show>
            <Show when=move || !channel_active>
                <p class="ob-page-empty">{move || t!(i18n, channels.detail_inactive)}</p>
            </Show>
        </div>
    }
}

#[component]
fn TranscriptMessage(
    message: TranscriptLine,
    agent_seed: String,
    agent_name: String,
) -> impl IntoView {
    let i18n = use_i18n();
    let user = message.kind == TranscriptKind::User;
    let kind = message.kind;
    let content = message.content;
    let avatar_seed = StoredValue::new(agent_seed);
    let avatar_name = StoredValue::new(agent_name);
    let label = move || match kind {
        TranscriptKind::User => t_string!(i18n, channels.user_message_label).to_owned(),
        TranscriptKind::Assistant => t_string!(i18n, channels.assistant_message_label).to_owned(),
        TranscriptKind::ToolCall => t_string!(i18n, channels.tool_call_label).to_owned(),
        TranscriptKind::ToolResult => t_string!(i18n, channels.tool_result_label).to_owned(),
    };
    view! {
        <MessageScrollerItem
            message_id=transcript_dom_id(&message.id)
            scroll_anchor=user
        >
            <Message
                align=if user { MessageAlign::End } else { MessageAlign::Start }
                aria_label=label
            >
                <MessageAvatar>
                    <span aria-hidden="true">
                        <Avatar
                            principal_id=if user { "current-user".to_owned() } else { avatar_seed.get_value() }
                            name=if user {
                                t_string!(i18n, channels.you).to_owned()
                            } else {
                                avatar_name.get_value()
                            }
                            size=AvatarSize::Small
                        />
                    </span>
                </MessageAvatar>
                <MessageContent>
                    <MessageHeader>{move || if user {
                        t_string!(i18n, channels.you).to_owned()
                    } else {
                        avatar_name.get_value()
                    }}</MessageHeader>
                    <Bubble kind=if user { BubbleKind::User } else { BubbleKind::Assistant }>
                        <p class="ob-transcript-text">{content}</p>
                    </Bubble>
                </MessageContent>
            </Message>
        </MessageScrollerItem>
    }
}

fn transcript_dom_id(source: &str) -> String {
    let mut id = String::from("transcript-");
    for byte in Sha256::digest(source.as_bytes()) {
        write!(&mut id, "{byte:02x}").expect("writing to String cannot fail");
    }
    id
}

fn terminal_text(
    i18n: leptos_i18n::I18nContext<crate::i18n::Locale>,
    notice: Option<TerminalNotice>,
) -> String {
    match notice {
        Some(TerminalNotice::Failed) => t_string!(i18n, channels.run_failed).to_owned(),
        Some(TerminalNotice::Cancelled) => t_string!(i18n, channels.run_cancelled).to_owned(),
        Some(TerminalNotice::ReconciliationRequired) => {
            t_string!(i18n, channels.run_reconciliation).to_owned()
        }
        None => String::new(),
    }
}

#[cfg(target_arch = "wasm32")]
struct EventConnection {
    source: EventSource,
    _message: Closure<dyn FnMut(MessageEvent)>,
    _open: Closure<dyn FnMut(Event)>,
    _error: Closure<dyn FnMut(Event)>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for EventConnection {
    fn drop(&mut self) {
        self.source.close();
    }
}

#[allow(clippy::too_many_arguments)]
fn install_conversation_sync(
    thread_id: RwSignal<Option<ThreadId>>,
    state: RwSignal<ConversationState>,
    loading: RwSignal<bool>,
    snapshot_error: RwSignal<bool>,
    stream_error: RwSignal<bool>,
    generation: RwSignal<u64>,
) {
    #[cfg(target_arch = "wasm32")]
    {
        let connection = StoredValue::new_local(None::<EventConnection>);
        Effect::new(move |_| {
            let current_generation = generation.get();
            let current_thread = thread_id.get();
            connection.update_value(|current| {
                _ = current.take();
            });
            snapshot_error.set(false);
            stream_error.set(false);
            let Some(thread) = current_thread else {
                state.set(ConversationState::default());
                loading.set(false);
                return;
            };
            loading.set(true);
            leptos::task::spawn_local_scoped_with_cancellation(async move {
                let snapshot = load_thread_conversation(&thread).await;
                if generation.get_untracked() != current_generation
                    || thread_id.get_untracked().as_ref() != Some(&thread)
                {
                    return;
                }
                let snapshot = match snapshot {
                    Ok(snapshot) => snapshot,
                    Err(_) => {
                        loading.set(false);
                        snapshot_error.set(true);
                        return;
                    }
                };
                let cursor = snapshot.last_event_sequence;
                state.update(|state| state.install_snapshot(snapshot));
                loading.set(false);
                match open_event_source(&thread, cursor, state, stream_error, generation) {
                    Ok(opened) => {
                        if generation.get_untracked() == current_generation {
                            connection.set_value(Some(opened));
                        }
                    }
                    Err(()) => stream_error.set(true),
                }
            });
        });
        on_cleanup(move || {
            connection.update_value(|current| {
                _ = current.take();
            });
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (
        thread_id,
        state,
        loading,
        snapshot_error,
        stream_error,
        generation,
    );
}

#[cfg(target_arch = "wasm32")]
fn open_event_source(
    thread: &ThreadId,
    cursor: Option<u64>,
    state: RwSignal<ConversationState>,
    stream_error: RwSignal<bool>,
    generation: RwSignal<u64>,
) -> Result<EventConnection, ()> {
    let path = thread_event_stream_path(thread, cursor).map_err(|_| ())?;
    let source = EventSource::new(&path).map_err(|_| ())?;
    let expected_thread = thread.clone();
    let event_source = source.clone();
    let message = Closure::<dyn FnMut(MessageEvent)>::new(move |message: MessageEvent| {
        let Some(text) = message.data().as_string() else {
            stream_error.set(true);
            event_source.close();
            return;
        };
        let Ok(event) = serde_json::from_str::<AppEvent>(&text) else {
            stream_error.set(true);
            event_source.close();
            return;
        };
        match event {
            AppEvent::ThreadRunEvent(event) => {
                let effect =
                    state.try_update(|state| apply_live_event(state, &expected_thread, &event));
                match effect {
                    Some(Ok(LiveEffect::ReloadSnapshot)) | Some(Err(())) | None => {
                        generation.update(|value| *value = value.saturating_add(1));
                    }
                    Some(Ok(LiveEffect::None)) => {}
                }
            }
            AppEvent::ThreadStreamError { .. } => {
                stream_error.set(true);
                event_source.close();
            }
            AppEvent::Heartbeat { .. }
            | AppEvent::ChannelActivity(_)
            | AppEvent::ChannelStreamError { .. } => {
                stream_error.set(true);
                event_source.close();
            }
        }
    });
    source
        .add_event_listener_with_callback("thread_run_event", message.as_ref().unchecked_ref())
        .map_err(|_| ())?;
    source
        .add_event_listener_with_callback("thread_stream_error", message.as_ref().unchecked_ref())
        .map_err(|_| ())?;
    let open = Closure::<dyn FnMut(Event)>::new(move |_| stream_error.set(false));
    source
        .add_event_listener_with_callback("open", open.as_ref().unchecked_ref())
        .map_err(|_| ())?;
    let error = Closure::<dyn FnMut(Event)>::new(move |_| stream_error.set(true));
    source
        .add_event_listener_with_callback("error", error.as_ref().unchecked_ref())
        .map_err(|_| ())?;
    Ok(EventConnection {
        source,
        _message: message,
        _open: open,
        _error: error,
    })
}

#[cfg(test)]
mod tests {
    use openbot_contracts::command::ThreadRunEvent;
    use time::OffsetDateTime;

    use super::*;

    fn event(
        sequence: u64,
        kind: ThreadRunEventKind,
        payload: serde_json::Value,
    ) -> ThreadRunEvent {
        ThreadRunEvent {
            thread_id: ThreadId::new("thread-1"),
            run_id: RunId::new("run-1"),
            event_sequence: sequence,
            event_type: kind,
            payload,
            terminal: kind.is_terminal(),
            created_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn snapshot_carries_durable_history_active_text_and_cursor_without_a_seed() {
        let mut state = ConversationState::default();
        state.install_snapshot(ThreadConversationSnapshot {
            messages: vec![ThreadHistoryMessage {
                id: "user-1".to_owned(),
                role: ThreadHistoryRole::User,
                content: "hello".to_owned(),
                tool_call_id: None,
                tool_calls: None,
            }],
            active_run_id: Some(RunId::new("run-1")),
            active_run_text: "partial".to_owned(),
            last_event_sequence: Some(3),
        });
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.streaming_text, "partial");
        assert_eq!(state.cursor, Some(3));
        assert_eq!(state.active_run_id, Some(RunId::new("run-1")));
    }

    #[test]
    fn history_projects_user_assistant_tool_activity_but_not_system_prompt() {
        let lines = project_history(&[
            ThreadHistoryMessage {
                id: "system".to_owned(),
                role: ThreadHistoryRole::System,
                content: "secret standing instruction".to_owned(),
                tool_call_id: None,
                tool_calls: None,
            },
            ThreadHistoryMessage {
                id: "user".to_owned(),
                role: ThreadHistoryRole::User,
                content: "hello".to_owned(),
                tool_call_id: None,
                tool_calls: None,
            },
            ThreadHistoryMessage {
                id: "assistant".to_owned(),
                role: ThreadHistoryRole::Assistant,
                content: String::new(),
                tool_call_id: None,
                tool_calls: Some(vec![
                    serde_json::json!({"function":{"name":"mcp__notes__search_notes"}}),
                ]),
            },
            ThreadHistoryMessage {
                id: "tool".to_owned(),
                role: ThreadHistoryRole::Tool,
                content: serde_json::to_string("found it").unwrap(),
                tool_call_id: Some("call-1".to_owned()),
                tool_calls: None,
            },
        ]);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].kind, TranscriptKind::User);
        assert_eq!(lines[1].content, "Search notes");
        assert_eq!(lines[2].content, "found it");
        assert!(!format!("{lines:?}").contains("secret standing instruction"));
    }

    #[test]
    fn live_text_is_ordered_deduplicated_and_terminal_requests_snapshot_reload() {
        let mut state = ConversationState {
            active_run_id: Some(RunId::new("run-1")),
            cursor: Some(0),
            ..ConversationState::default()
        };
        assert_eq!(
            apply_live_event(
                &mut state,
                &ThreadId::new("thread-1"),
                &event(
                    1,
                    ThreadRunEventKind::SemanticChunk,
                    serde_json::json!({"channel":"text","delta":"hel"})
                ),
            ),
            Ok(LiveEffect::None)
        );
        _ = apply_live_event(
            &mut state,
            &ThreadId::new("thread-1"),
            &event(
                1,
                ThreadRunEventKind::SemanticChunk,
                serde_json::json!({"channel":"text","delta":"duplicate"}),
            ),
        );
        _ = apply_live_event(
            &mut state,
            &ThreadId::new("thread-1"),
            &event(
                2,
                ThreadRunEventKind::SemanticChunk,
                serde_json::json!({"channel":"reasoning","delta":"hidden"}),
            ),
        );
        assert_eq!(state.streaming_text, "hel");
        assert_eq!(
            apply_live_event(
                &mut state,
                &ThreadId::new("thread-1"),
                &event(
                    3,
                    ThreadRunEventKind::Completed,
                    serde_json::json!({"status":"completed"})
                ),
            ),
            Ok(LiveEffect::ReloadSnapshot)
        );
        assert!(state.active_run_id.is_none());
    }

    #[test]
    fn gap_wrong_thread_and_invalid_payload_never_become_visible_text() {
        let mut state = ConversationState {
            active_run_id: Some(RunId::new("run-1")),
            cursor: Some(0),
            ..ConversationState::default()
        };
        assert_eq!(
            apply_live_event(
                &mut state,
                &ThreadId::new("thread-1"),
                &event(
                    2,
                    ThreadRunEventKind::SemanticChunk,
                    serde_json::json!({"channel":"text","delta":"gap"})
                ),
            ),
            Ok(LiveEffect::ReloadSnapshot)
        );
        let mut wrong = event(
            1,
            ThreadRunEventKind::SemanticChunk,
            serde_json::json!({"channel":"text","delta":"wrong"}),
        );
        wrong.thread_id = ThreadId::new("thread-2");
        assert_eq!(
            apply_live_event(&mut state, &ThreadId::new("thread-1"), &wrong),
            Err(())
        );
        assert!(state.streaming_text.is_empty());
    }

    #[test]
    fn transcript_dom_identity_is_bounded_and_not_controlled_by_message_id() {
        let id = transcript_dom_id("message/one?x=1\n");
        assert_eq!(id, transcript_dom_id("message/one?x=1\n"));
        assert!(id.len() <= 128);
        assert!(
            id.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        );
    }

    #[test]
    fn terminal_notices_are_closed_codes_and_cannot_carry_raw_error_words() {
        assert_eq!(TerminalNotice::Failed.as_str(), "failed");
        assert_eq!(TerminalNotice::Cancelled.as_str(), "cancelled");
        assert_eq!(
            TerminalNotice::ReconciliationRequired.as_str(),
            "reconciliation_required"
        );
        assert_eq!(core::mem::size_of::<TerminalNotice>(), 1);
    }
}
