//! Remote AG-UI 0.0.57 adapter into the provider-neutral host loop.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use openbot_application::{
    ProviderAdapter, ProviderEvent, ProviderFailure, ProviderPortError, ProviderRequest,
    ProviderRoute, ProviderSession, RemoteAguiEventStream, RemoteAguiTransport,
    RemoteAguiTransportError,
};
use serde_json::json;

use crate::agui::{AguiDecoder, AguiEvent, AguiRole, encode_run_agent_input};

/// Production semantic adapter; HTTP/DNS/TLS/SSE framing is supplied by the infra transport port.
pub struct RemoteAguiProvider {
    transport: Arc<dyn RemoteAguiTransport>,
}

impl RemoteAguiProvider {
    /// Bind the unique safe remote transport.
    #[must_use]
    pub fn new(transport: Arc<dyn RemoteAguiTransport>) -> Self {
        Self { transport }
    }
}

impl core::fmt::Debug for RemoteAguiProvider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RemoteAguiProvider")
            .field("transport", &"<safe-remote-transport>")
            .finish()
    }
}

#[async_trait]
impl ProviderAdapter for RemoteAguiProvider {
    async fn start(
        &self,
        request: ProviderRequest,
    ) -> Result<Box<dyn ProviderSession>, ProviderPortError> {
        let ProviderRoute::RemoteAgUi(route) = &request.route else {
            return Err(ProviderPortError::InvalidRequest {
                field: "remote_route",
            });
        };
        if route.run_assertion().is_none() && !request.tools.is_empty() {
            return Err(ProviderPortError::InvalidRequest {
                field: "remote_tool_assertion",
            });
        }
        let mut forwarded = serde_json::Map::new();
        forwarded.insert(
            "openbotBotId".to_owned(),
            serde_json::Value::String(route.bot_id().to_owned()),
        );
        forwarded.insert(
            "openbotDeploymentTools".to_owned(),
            serde_json::Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| serde_json::Value::String(tool.name.clone()))
                    .collect(),
            ),
        );
        if let Some(assertion) = route.run_assertion() {
            forwarded.insert(
                "openbotRun".to_owned(),
                serde_json::Value::String(assertion.to_owned()),
            );
        }
        let body = encode_run_agent_input(
            route.thread_id(),
            route.run_id(),
            &request.messages,
            &request.tools,
            serde_json::Value::Object(forwarded),
        )
        .map_err(|_| ProviderPortError::InvalidRequest {
            field: "remote_run_input",
        })?;
        let stream = match self
            .transport
            .start(route.endpoint(), route.authorization(), body)
            .await
        {
            Ok(stream) => stream,
            Err(RemoteAguiTransportError::Unavailable) => {
                return Err(ProviderPortError::Unavailable);
            }
            Err(RemoteAguiTransportError::CommitUnknown) => {
                return Err(ProviderPortError::CommitUnknown);
            }
            Err(error) => {
                return Ok(Box::new(OneRemoteEvent::new(transport_failure(error))));
            }
        };
        let offered_tools = request.tools.iter().map(|tool| tool.name.clone()).collect();
        Ok(Box::new(RemoteAguiSession {
            stream,
            decoder: AguiDecoder::new(route.thread_id(), route.run_id(), json!({})).map_err(
                |_| ProviderPortError::InvalidRequest {
                    field: "remote_run_identity",
                },
            )?,
            pending: VecDeque::new(),
            assistant_messages: BTreeSet::new(),
            completed_tools: Vec::new(),
            remote_tool_results: BTreeSet::new(),
            offered_tools,
            response_id: format!("remote-agui:{}", route.run_id()),
            terminal: false,
            stream_ended: false,
        }))
    }
}

struct OneRemoteEvent(Option<ProviderEvent>);

impl OneRemoteEvent {
    const fn new(event: ProviderEvent) -> Self {
        Self(Some(event))
    }
}

#[async_trait]
impl ProviderSession for OneRemoteEvent {
    async fn next_event(&mut self) -> Result<Option<ProviderEvent>, ProviderPortError> {
        Ok(self.0.take())
    }
}

struct RemoteAguiSession {
    stream: Box<dyn RemoteAguiEventStream>,
    decoder: AguiDecoder,
    pending: VecDeque<ProviderEvent>,
    assistant_messages: BTreeSet<String>,
    completed_tools: Vec<(u32, String, String, serde_json::Value)>,
    remote_tool_results: BTreeSet<String>,
    offered_tools: BTreeSet<String>,
    response_id: String,
    terminal: bool,
    stream_ended: bool,
}

#[async_trait]
impl ProviderSession for RemoteAguiSession {
    async fn next_event(&mut self) -> Result<Option<ProviderEvent>, ProviderPortError> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }
            if self.terminal || self.stream_ended {
                return Ok(None);
            }
            match self.stream.next_data().await {
                Ok(Some(data)) => {
                    let events = match self.decoder.ingest(&data) {
                        Ok(events) => events,
                        Err(_) => {
                            self.fail(ProviderFailure::InvalidResponse);
                            continue;
                        }
                    };
                    for event in events {
                        self.accept(event);
                    }
                }
                Ok(None) => {
                    self.stream_ended = true;
                    if self.decoder.finish().is_err() {
                        self.fail(ProviderFailure::InvalidResponse);
                    }
                }
                Err(RemoteAguiTransportError::StreamStalled) => {
                    self.fail(ProviderFailure::StreamStalled);
                }
                Err(RemoteAguiTransportError::InvalidResponse) => {
                    self.fail(ProviderFailure::InvalidResponse);
                }
                Err(RemoteAguiTransportError::DestinationRejected) => {
                    self.fail(ProviderFailure::InvalidResponse);
                }
                Err(RemoteAguiTransportError::Authentication) => {
                    self.fail(ProviderFailure::Authentication);
                }
                Err(RemoteAguiTransportError::RateLimited) => {
                    self.fail(ProviderFailure::RateLimited { retry_after: None });
                }
                Err(RemoteAguiTransportError::ServerUnavailable) => {
                    self.fail(ProviderFailure::ServerUnavailable { retry_after: None });
                }
                Err(RemoteAguiTransportError::Unavailable) => {
                    return Err(ProviderPortError::Unavailable);
                }
                Err(RemoteAguiTransportError::CommitUnknown) => {
                    return Err(ProviderPortError::CommitUnknown);
                }
            }
        }
    }
}

impl RemoteAguiSession {
    fn accept(&mut self, event: AguiEvent) {
        match event {
            AguiEvent::RunStarted => self.pending.push_back(ProviderEvent::ResponseStarted {
                response_id: self.response_id.clone(),
            }),
            AguiEvent::TextStarted {
                id,
                role: AguiRole::Assistant,
                ..
            } => {
                self.assistant_messages.insert(id);
            }
            AguiEvent::TextStarted { .. } => {}
            AguiEvent::TextDelta { id, delta }
                if self.assistant_messages.contains(&id) && !delta.is_empty() =>
            {
                self.pending
                    .push_back(ProviderEvent::TextDelta { index: 0, delta });
            }
            AguiEvent::TextDelta { .. } => {}
            AguiEvent::TextEnded { id } => {
                self.assistant_messages.remove(&id);
            }
            AguiEvent::ReasoningDelta { delta, .. } if !delta.is_empty() => {
                self.pending
                    .push_back(ProviderEvent::ReasoningDelta { index: 0, delta });
            }
            AguiEvent::ReasoningDelta { .. } => {}
            AguiEvent::ToolCompleted {
                id,
                name,
                arguments,
            } => {
                let index = u32::try_from(self.completed_tools.len()).unwrap_or(u32::MAX);
                self.completed_tools.push((index, id, name, arguments));
            }
            AguiEvent::ToolResult { call_id, .. } => {
                self.remote_tool_results.insert(call_id);
            }
            AguiEvent::RunFinished => self.complete(),
            AguiEvent::RunInterrupted { .. } => {
                // Decoder supports the pinned interrupt shape; durable human resume is a later G7 slice.
                self.fail(ProviderFailure::GenerationFailed);
            }
            // Remote prose/code are untrusted display inputs. Collapse them at this boundary so
            // neither the durable run journal nor logs/audit can receive provider-controlled text.
            AguiEvent::RunError { .. } => self.fail(ProviderFailure::GenerationFailed),
            AguiEvent::StepStarted { .. }
            | AguiEvent::StepFinished { .. }
            | AguiEvent::ToolStarted { .. }
            | AguiEvent::ToolArguments { .. }
            | AguiEvent::StateSnapshot { .. }
            | AguiEvent::StateDelta { .. }
            | AguiEvent::MessagesSnapshot { .. }
            | AguiEvent::ActivitySnapshot { .. }
            | AguiEvent::ActivityDelta { .. }
            | AguiEvent::ReasoningStarted { .. }
            | AguiEvent::ReasoningMessageStarted { .. }
            | AguiEvent::ReasoningMessageEnded { .. }
            | AguiEvent::ReasoningEnded { .. }
            | AguiEvent::ReasoningEncrypted { .. }
            | AguiEvent::Raw { .. }
            | AguiEvent::Custom { .. } => {}
        }
    }

    fn complete(&mut self) {
        for (index, call_id, name, arguments) in &self.completed_tools {
            if self.remote_tool_results.contains(call_id) {
                continue;
            }
            if !self.offered_tools.contains(name) {
                self.fail(ProviderFailure::InvalidResponse);
                return;
            }
            self.pending.push_back(ProviderEvent::ToolCallCompleted {
                index: *index,
                call_id: call_id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            });
        }
        self.pending.push_back(ProviderEvent::Completed);
        self.terminal = true;
    }

    fn fail(&mut self, failure: ProviderFailure) {
        self.pending.clear();
        self.pending.push_back(ProviderEvent::Failed(failure));
        self.terminal = true;
    }
}

fn transport_failure(error: RemoteAguiTransportError) -> ProviderEvent {
    ProviderEvent::Failed(match error {
        RemoteAguiTransportError::DestinationRejected => ProviderFailure::InvalidResponse,
        RemoteAguiTransportError::Authentication => ProviderFailure::Authentication,
        RemoteAguiTransportError::RateLimited => ProviderFailure::RateLimited { retry_after: None },
        RemoteAguiTransportError::ServerUnavailable | RemoteAguiTransportError::Unavailable => {
            ProviderFailure::ServerUnavailable { retry_after: None }
        }
        RemoteAguiTransportError::InvalidResponse => ProviderFailure::InvalidResponse,
        RemoteAguiTransportError::StreamStalled => ProviderFailure::StreamStalled,
        RemoteAguiTransportError::CommitUnknown => ProviderFailure::Transport,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use openbot_application::{
        ProviderMessage, ProviderMessageRole, RemoteAguiRoute, RemoteAguiTransportError,
    };

    use super::*;

    struct FakeTransport {
        body: Mutex<Option<Vec<u8>>>,
        events: Vec<String>,
    }

    struct FakeStream(VecDeque<String>);

    #[async_trait]
    impl RemoteAguiEventStream for FakeStream {
        async fn next_data(&mut self) -> Result<Option<String>, RemoteAguiTransportError> {
            Ok(self.0.pop_front())
        }
    }

    #[async_trait]
    impl RemoteAguiTransport for FakeTransport {
        async fn validate_endpoint(&self, _endpoint: &str) -> Result<(), RemoteAguiTransportError> {
            Ok(())
        }

        async fn start(
            &self,
            _endpoint: &str,
            _authorization: Option<&openbot_application::RemoteAguiAuthorization>,
            body: Vec<u8>,
        ) -> Result<Box<dyn RemoteAguiEventStream>, RemoteAguiTransportError> {
            *self.body.lock().unwrap() = Some(body);
            Ok(Box::new(FakeStream(self.events.clone().into())))
        }
    }

    fn request(tools: Vec<openbot_application::ProviderToolDefinition>) -> ProviderRequest {
        ProviderRequest {
            route: ProviderRoute::RemoteAgUi(
                RemoteAguiRoute::new(
                    "https://agent.example/run".to_owned(),
                    "thread-1".to_owned(),
                    "run-1".to_owned(),
                    "bot-1".to_owned(),
                    None,
                )
                .unwrap(),
            ),
            messages: vec![ProviderMessage {
                role: ProviderMessageRole::User,
                content: "hello".to_owned(),
                tool_call_id: None,
                tool_name: None,
                tool_calls: Vec::new(),
            }],
            tools,
            max_output_tokens: None,
            rate_card: None,
            cost_cap: None,
        }
    }

    #[tokio::test]
    async fn remote_text_and_reasoning_map_only_after_exact_lifecycle_validation() {
        let transport = Arc::new(FakeTransport {
            body: Mutex::new(None),
            events: vec![
                json!({"type":"RUN_STARTED","threadId":"thread-1","runId":"run-1"}).to_string(),
                json!({"type":"TEXT_MESSAGE_START","messageId":"m","role":"assistant"}).to_string(),
                json!({"type":"TEXT_MESSAGE_CONTENT","messageId":"m","delta":"hello"}).to_string(),
                json!({"type":"TEXT_MESSAGE_END","messageId":"m"}).to_string(),
                json!({"type":"REASONING_START","messageId":"r"}).to_string(),
                json!({"type":"REASONING_MESSAGE_START","messageId":"rm","role":"reasoning"})
                    .to_string(),
                json!({"type":"REASONING_MESSAGE_CONTENT","messageId":"rm","delta":"summary"})
                    .to_string(),
                json!({"type":"REASONING_MESSAGE_END","messageId":"rm"}).to_string(),
                json!({"type":"REASONING_END","messageId":"r"}).to_string(),
                json!({"type":"RUN_FINISHED","threadId":"thread-1","runId":"run-1"}).to_string(),
            ],
        });
        let provider = RemoteAguiProvider::new(transport.clone());
        let mut session = provider.start(request(Vec::new())).await.unwrap();
        let mut events = Vec::new();
        while let Some(event) = session.next_event().await.unwrap() {
            events.push(event);
        }
        assert!(matches!(events[0], ProviderEvent::ResponseStarted { .. }));
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderEvent::TextDelta { delta, .. } if delta == "hello"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ProviderEvent::ReasoningDelta { delta, .. } if delta == "summary"
        )));
        assert_eq!(events.last(), Some(&ProviderEvent::Completed));
        let body: serde_json::Value =
            serde_json::from_slice(transport.body.lock().unwrap().as_ref().unwrap()).unwrap();
        assert_eq!(body["forwardedProps"]["openbotBotId"], "bot-1");
        assert_eq!(body["tools"], json!([]));
    }

    #[tokio::test]
    async fn remote_cannot_call_an_unoffered_deployment_tool_without_an_assertion() {
        let transport = Arc::new(FakeTransport {
            body: Mutex::new(None),
            events: Vec::new(),
        });
        let provider = RemoteAguiProvider::new(transport);
        assert!(matches!(
            provider
                .start(request(vec![openbot_application::ProviderToolDefinition {
                    name: "remember".to_owned(),
                    description: "remember".to_owned(),
                    input_schema: json!({"type":"object"}),
                }]))
                .await,
            Err(ProviderPortError::InvalidRequest {
                field: "remote_tool_assertion"
            })
        ));
    }

    #[tokio::test]
    async fn remote_error_and_malformed_message_become_closed_failures_without_remote_prose() {
        let cases = [
            (
                vec![
                    json!({"type":"RUN_STARTED","threadId":"thread-1","runId":"run-1"}).to_string(),
                    json!({
                        "type":"RUN_ERROR",
                        "message":"REMOTE_ERROR_SECRET_CANARY",
                        "code":"vendor-secret-code"
                    })
                    .to_string(),
                ],
                ProviderFailure::GenerationFailed,
            ),
            (
                vec![
                    json!({"type":"RUN_STARTED","threadId":"thread-1","runId":"run-1"}).to_string(),
                    json!({
                        "type":"MESSAGES_SNAPSHOT",
                        "messages":[{"id":"m","role":"assistant","content":{"bad":true}}]
                    })
                    .to_string(),
                ],
                ProviderFailure::InvalidResponse,
            ),
        ];
        for (events, expected) in &cases {
            let transport = Arc::new(FakeTransport {
                body: Mutex::new(None),
                events: events.clone(),
            });
            let provider = RemoteAguiProvider::new(transport);
            let mut session = provider.start(request(Vec::new())).await.unwrap();
            assert!(matches!(
                session.next_event().await.unwrap(),
                Some(ProviderEvent::ResponseStarted { .. })
            ));
            assert_eq!(
                session.next_event().await.unwrap(),
                Some(ProviderEvent::Failed(*expected))
            );
            assert_eq!(session.next_event().await.unwrap(), None);
        }
        let rendered = format!("{cases:?}");
        assert!(rendered.contains("REMOTE_ERROR_SECRET_CANARY"));
        let local = format!(
            "{:?}",
            ProviderEvent::Failed(ProviderFailure::GenerationFailed)
        );
        assert!(!local.contains("REMOTE_ERROR_SECRET_CANARY"));
        assert!(!local.contains("vendor-secret-code"));
    }
}
