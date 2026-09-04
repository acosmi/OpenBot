//! Server same-origin ScreenSession issuance and read-only binary WebSocket framing.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::extract::{OriginalUri, State};
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::Response;
use openbot_computer::screen::{
    SCREEN_VIEWER_MAX_BINARY_BYTES, SCREEN_VIEWER_PROTOCOL, ScreenHubError, ScreenViewer,
    ScreenViewerBinding, ScreenViewerFrame,
};
use openbot_contracts::command::{AppCommand, AppReply};
use openbot_contracts::error::AppError;
use openbot_contracts::screen::{
    ScreenSessionRequest, ScreenSessionTarget, ScreenSessionTicket, ScreenViewerBindingRequest,
};
use time::OffsetDateTime;

use crate::auth::OriginBoundAuthenticated;
use crate::error::HttpError;
use crate::http::ServerState;

const SCREEN_WS_INPUT_LIMIT: usize = 1024;

#[cfg(test)]
const SERVER_SCREEN_FIXTURE: &str =
    include_str!("../../../../fixtures/computer/server-screen-websocket-v1.json");

/// `POST /api/screen/sessions`: issue one ticket after same-origin auth, never from body identity.
pub async fn issue_session(
    State(state): State<ServerState>,
    bound: OriginBoundAuthenticated,
    body: Result<Json<ScreenSessionTarget>, JsonRejection>,
) -> Result<(HeaderMap, Json<ScreenSessionTicket>), HttpError> {
    let Json(target) = body.map_err(|rejection| {
        tracing::debug!(rejection = %rejection, "screen session body rejected");
        AppError::MalformedPayload { field: "body" }
    })?;
    let (auth, origin) = bound.into_parts();
    let reply = state
        .application()
        .execute(
            auth,
            AppCommand::IssueScreenSession(ScreenSessionRequest {
                target,
                binding: ScreenViewerBindingRequest::Server { origin },
            }),
        )
        .await?;
    let AppReply::ScreenSession(ticket) = reply else {
        tracing::error!("IssueScreenSession received a non-ScreenSession application reply");
        return Err(AppError::DependencyUnavailable {
            dependency: "application",
        }
        .into());
    };
    Ok((no_store(), Json(ticket)))
}

/// `GET /api/screen`: consume one ticket from requested subprotocols and stream binary latest-frame.
pub async fn websocket(
    State(state): State<ServerState>,
    bound: OriginBoundAuthenticated,
    OriginalUri(uri): OriginalUri,
    ws: WebSocketUpgrade,
) -> Result<Response, HttpError> {
    if uri.query().is_some() {
        return Err(AppError::MalformedPayload {
            field: "websocket_query",
        }
        .into());
    }
    let ticket_protocol = requested_ticket(&ws)?;
    let (auth, origin) = bound.into_parts();
    let binding = ScreenViewerBinding::verified_server(origin)
        .map_err(|_| AppError::MalformedPayload { field: "origin" })?;
    let hub = state.screen_hub().ok_or(AppError::DependencyUnavailable {
        dependency: "screen_hub",
    })?;
    let mut viewer = hub
        .consume_ticket(
            &auth,
            &binding,
            ticket_protocol.as_str(),
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(screen_error)?;
    let first = viewer.current().map_err(screen_error)?;
    Ok(ws
        .protocols([SCREEN_VIEWER_PROTOCOL])
        .max_message_size(SCREEN_WS_INPUT_LIMIT)
        .max_frame_size(SCREEN_WS_INPUT_LIMIT)
        .on_failed_upgrade(|error| {
            tracing::debug!(error = %error, "screen websocket upgrade failed");
        })
        .on_upgrade(move |socket| drive_screen_socket(socket, viewer, first)))
}

fn requested_ticket(ws: &WebSocketUpgrade) -> Result<String, AppError> {
    let mut count = 0_u8;
    let mut base_seen = false;
    let mut ticket = None;
    for protocol in ws.requested_protocols() {
        count = count.saturating_add(1);
        if count > 2 {
            return Err(AppError::MalformedPayload {
                field: "websocket_protocol",
            });
        }
        if protocol == SCREEN_VIEWER_PROTOCOL {
            if base_seen {
                return Err(AppError::MalformedPayload {
                    field: "websocket_protocol",
                });
            }
            base_seen = true;
        } else {
            let value = protocol.to_str().map_err(|_| AppError::MalformedPayload {
                field: "websocket_protocol",
            })?;
            if ticket.replace(value.to_owned()).is_some() {
                return Err(AppError::MalformedPayload {
                    field: "websocket_protocol",
                });
            }
        }
    }
    if count != 2 || !base_seen {
        return Err(AppError::MalformedPayload {
            field: "websocket_protocol",
        });
    }
    ticket.ok_or(AppError::MalformedPayload {
        field: "websocket_protocol",
    })
}

async fn drive_screen_socket(
    mut socket: WebSocket,
    mut viewer: ScreenViewer,
    first: std::sync::Arc<ScreenViewerFrame>,
) {
    if send_frame(&mut socket, &first).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            frame = viewer.next() => match frame {
                Ok(frame) => {
                    if send_frame(&mut socket, &frame).await.is_err() {
                        return;
                    }
                }
                Err(_) => {
                    close(&mut socket, close_code::POLICY, "screen_revoked").await;
                    return;
                }
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Ping(payload))) => {
                    if socket.send(Message::Pong(payload)).await.is_err() {
                        return;
                    }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(Message::Text(_) | Message::Binary(_))) => {
                    close(&mut socket, close_code::POLICY, "screen_input_not_enabled").await;
                    return;
                }
            }
        }
    }
}

async fn send_frame(socket: &mut WebSocket, frame: &ScreenViewerFrame) -> Result<(), ()> {
    if frame.binary().len() > SCREEN_VIEWER_MAX_BINARY_BYTES {
        close(socket, close_code::ERROR, "screen_frame_too_large").await;
        return Err(());
    }
    socket
        .send(Message::Binary(frame.binary().to_vec().into()))
        .await
        .map_err(|_| ())
}

async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

fn screen_error(error: ScreenHubError) -> AppError {
    match error {
        ScreenHubError::NotVisible
        | ScreenHubError::SourceClosed
        | ScreenHubError::TicketInvalid
        | ScreenHubError::TicketExpired
        | ScreenHubError::ViewerRevoked => AppError::NotVisible,
        ScreenHubError::InvalidBinding | ScreenHubError::InvalidViewerLimit => {
            AppError::MalformedPayload { field: "screen" }
        }
        ScreenHubError::ViewerLimit => AppError::RequestConflict {
            resource: "screen_viewers",
        },
        ScreenHubError::DuplicateStream
        | ScreenHubError::RandomFailed
        | ScreenHubError::RandomCollision
        | ScreenHubError::ClockOverflow
        | ScreenHubError::Frame => AppError::DependencyUnavailable {
            dependency: "screen_hub",
        },
    }
}

fn no_store() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use futures_util::{SinkExt as _, StreamExt as _};
    use openbot_application::{
        ApplicationService, ChannelCursor, ChannelReader, OpenBotApplication, PortError,
    };
    use openbot_computer::screen::testing::attach_test_stream;
    use openbot_computer::screen::{SCREEN_VIEWER_PROTOCOL, ScreenHub, ScreenSessionService};
    use openbot_contracts::auth::{AuthContext, AuthGeneration, Role};
    use openbot_contracts::command::ChannelSummary;
    use openbot_contracts::ids::{
        ActorId, ComputerGeneration, ComputerId, DeploymentId, TabId, TenantId,
    };
    use openbot_contracts::screen::{ScreenSessionTarget, ScreenSessionTicket};
    use openbot_domain::identity::session::{SessionState, TrustedOrigins, evaluate_session};
    use openbot_infra::auth::config::default_session_lifetime;
    use time::{Duration, OffsetDateTime};
    use tokio_tungstenite::tungstenite::Message as ClientMessage;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
    use tower::ServiceExt as _;

    use super::SERVER_SCREEN_FIXTURE;
    use crate::auth::{FixedAuthResolver, ResolvedAuth, SensitiveWriteSecurity};
    use crate::http::ServerBuilder;

    struct EmptyChannels;

    #[async_trait]
    impl ChannelReader for EmptyChannels {
        async fn list_visible_channels(
            &self,
            _actor: &ActorId,
            _limit: u32,
            _cursor: Option<ChannelCursor>,
        ) -> Result<Vec<ChannelSummary>, PortError> {
            Ok(Vec::new())
        }
    }

    fn auth() -> AuthContext {
        AuthContext::for_test(
            DeploymentId::new("dep"),
            TenantId::new("tenant"),
            ActorId::new("actor"),
            [Role::User],
            AuthGeneration::new(4),
            false,
        )
    }

    fn router(hub: ScreenHub, auth: AuthContext) -> Router {
        let now = OffsetDateTime::now_utc();
        let lifetime = default_session_lifetime();
        let live = evaluate_session(
            lifetime,
            SessionState::rehydrate(now - Duration::minutes(1), now, auth.auth_generation()),
            auth.auth_generation(),
            now,
        )
        .expect("live session");
        let resolver =
            FixedAuthResolver::granting_resolved(ResolvedAuth::from_live_session(auth, live, None));
        let application: Arc<dyn ApplicationService> = Arc::new(
            OpenBotApplication::new(EmptyChannels)
                .with_screen_sessions(Arc::new(ScreenSessionService::new(hub.clone()))),
        );
        crate::router(
            ServerBuilder::new(application, Arc::new(resolver))
                .with_sensitive_write_security(SensitiveWriteSecurity::new(
                    lifetime,
                    TrustedOrigins::from_configured(["https://app.example.test"])
                        .expect("trusted origin"),
                ))
                .with_screen_hub(hub)
                .build(),
        )
    }

    async fn issue(
        router: Router,
        target: &ScreenSessionTarget,
    ) -> (StatusCode, ScreenSessionTicket) {
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/screen/sessions")
                    .header(http::header::ORIGIN, "https://app.example.test")
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(target).expect("screen target wire"),
                    ))
                    .expect("screen session request"),
            )
            .await
            .expect("screen session response");
        let status = response.status();
        assert_eq!(response.headers()[http::header::CACHE_CONTROL], "no-store");
        let bytes = to_bytes(response.into_body(), 4096)
            .await
            .expect("screen session body");
        let ticket = serde_json::from_slice(&bytes).expect("screen ticket");
        (status, ticket)
    }

    type ClientSocket = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

    async fn connect(
        address: std::net::SocketAddr,
        protocols: &str,
    ) -> Result<
        (ClientSocket, http::Response<Option<Vec<u8>>>),
        tokio_tungstenite::tungstenite::Error,
    > {
        connect_path(address, "/api/screen", protocols).await
    }

    async fn connect_path(
        address: std::net::SocketAddr,
        path: &str,
        protocols: &str,
    ) -> Result<
        (ClientSocket, http::Response<Option<Vec<u8>>>),
        tokio_tungstenite::tungstenite::Error,
    > {
        let mut request = format!("ws://{address}{path}")
            .into_client_request()
            .expect("screen websocket request");
        request.headers_mut().insert(
            http::header::ORIGIN,
            http::HeaderValue::from_static("https://app.example.test"),
        );
        request.headers_mut().insert(
            http::header::SEC_WEBSOCKET_PROTOCOL,
            http::HeaderValue::from_str(protocols).expect("protocol header"),
        );
        let stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("connect screen websocket");
        tokio_tungstenite::client_async(request, stream).await
    }

    #[tokio::test]
    async fn same_origin_ticket_streams_binary_selects_only_base_and_replay_fails() {
        let auth = auth();
        let hub = ScreenHub::new(3).expect("hub");
        let target = ScreenSessionTarget {
            computer_id: ComputerId::new("computer"),
            computer_generation: ComputerGeneration::new(3),
            tab_id: TabId::new("tab"),
        };
        let feed = attach_test_stream(
            &hub,
            &auth,
            target.computer_id.clone(),
            target.computer_generation,
            target.tab_id.clone(),
        )
        .await
        .expect("test stream");
        let router = router(hub.clone(), auth.clone());
        let (status, ticket) = issue(router.clone(), &target).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ticket.base_protocol(), SCREEN_VIEWER_PROTOCOL);
        assert!(!format!("{ticket:?}").contains(ticket.ticket_protocol()));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind screen websocket");
        let address = listener.local_addr().expect("screen address");
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = stopped.await;
                })
                .await
        });
        let requested = format!("{}, {}", ticket.base_protocol(), ticket.ticket_protocol());
        let (mut socket, response) = connect(address, requested.as_str())
            .await
            .expect("screen handshake");
        assert_eq!(
            response.headers()[http::header::SEC_WEBSOCKET_PROTOCOL],
            SCREEN_VIEWER_PROTOCOL
        );
        assert!(
            !response.headers()[http::header::SEC_WEBSOCKET_PROTOCOL]
                .to_str()
                .expect("selected protocol")
                .contains(ticket.ticket_protocol())
        );
        let first = socket.next().await.expect("first frame").expect("frame");
        assert!(matches!(first, ClientMessage::Binary(bytes) if bytes.starts_with(b"OBSCRN01")));

        feed.publish(2, 20.0);
        let next = socket.next().await.expect("next frame").expect("frame");
        assert!(matches!(next, ClientMessage::Binary(bytes) if bytes.starts_with(b"OBSCRN01")));
        socket
            .send(ClientMessage::Text("forged input".into()))
            .await
            .expect("send rejected input");
        let close = socket.next().await.expect("policy close").expect("close");
        assert!(
            matches!(close, ClientMessage::Close(Some(frame)) if frame.code == tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy)
        );

        assert!(connect(address, requested.as_str()).await.is_err());
        let _ = stop.send(());
        server.await.expect("server task").expect("server result");
    }

    #[tokio::test]
    async fn handshake_rejects_protocol_shape_and_generation_invalidation_closes_viewer() {
        let auth = auth();
        let hub = ScreenHub::new(2).expect("hub");
        let target = ScreenSessionTarget {
            computer_id: ComputerId::new("computer-generation"),
            computer_generation: ComputerGeneration::new(8),
            tab_id: TabId::new("tab-generation"),
        };
        let _feed = attach_test_stream(
            &hub,
            &auth,
            target.computer_id.clone(),
            target.computer_generation,
            target.tab_id.clone(),
        )
        .await
        .expect("test stream");
        let router = router(hub.clone(), auth.clone());
        let (_, ticket) = issue(router.clone(), &target).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind screen websocket");
        let address = listener.local_addr().expect("screen address");
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = stopped.await;
                })
                .await
        });
        assert!(connect(address, SCREEN_VIEWER_PROTOCOL).await.is_err());
        assert!(
            connect(
                address,
                format!(
                    "{}, {}, extra",
                    SCREEN_VIEWER_PROTOCOL,
                    ticket.ticket_protocol()
                )
                .as_str(),
            )
            .await
            .is_err()
        );
        let requested = format!("{}, {}", SCREEN_VIEWER_PROTOCOL, ticket.ticket_protocol());
        assert!(
            connect_path(
                address,
                "/api/screen?ticket=must-not-be-read",
                requested.as_str(),
            )
            .await
            .is_err()
        );
        let (mut socket, _) = connect(address, requested.as_str())
            .await
            .expect("valid screen handshake");
        let _ = socket.next().await.expect("initial frame").expect("frame");
        assert_eq!(
            hub.invalidate_actor(auth.tenant(), auth.actor(), AuthGeneration::new(5))
                .await,
            1
        );
        let close = socket
            .next()
            .await
            .expect("revocation close")
            .expect("close");
        assert!(
            matches!(close, ClientMessage::Close(Some(frame)) if frame.code == tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy)
        );
        let _ = stop.send(());
        server.await.expect("server task").expect("server result");
    }

    #[tokio::test]
    async fn session_issue_rejects_origin_before_parsing_an_attacker_body() {
        let response = router(ScreenHub::new(1).expect("hub"), auth())
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/screen/sessions")
                    .header(http::header::ORIGIN, "https://evil.example.test")
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{"))
                    .expect("wrong-origin request"),
            )
            .await
            .expect("wrong-origin response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("error body");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).expect("error JSON"),
            serde_json::json!({"code":"identity_sensitive_write_origin_untrusted"})
        );
    }

    #[test]
    fn fixture_locks_server_transport_and_remaining_production_boundary() {
        let fixture = serde_json::from_str::<serde_json::Value>(SERVER_SCREEN_FIXTURE)
            .expect("server screen fixture");
        assert_eq!(fixture["schema"], "openbot-server-screen-websocket-v1");
        assert_eq!(fixture["application"]["bindingFromJsonBody"], false);
        assert_eq!(fixture["server"]["ticketInUrlOrQuery"], false);
        assert_eq!(fixture["server"]["queryRejected"], true);
        assert_eq!(
            fixture["server"]["selectedProtocol"],
            SCREEN_VIEWER_PROTOCOL
        );
        assert_eq!(fixture["server"]["selectedProtocolContainsTicket"], false);
        assert_eq!(fixture["server"]["ticketSingleUse"], true);
        assert_eq!(fixture["server"]["defaultViewersPerStream"], 8);
        for completed in [
            "typedApplicationTicket",
            "serverSameOriginWebSocket",
            "realEngineToServerBinaryFrame",
            "productionServerHubAndPortComposition",
        ] {
            assert_eq!(fixture["evidenceBoundary"][completed], true);
        }
        for unfinished in [
            "productionManagerAttachesEngineSource",
            "postgresSessionCookieBrowser",
            "externalTlsTermination",
            "desktopLoopbackWebSocket",
            "viewerInputOverWebSocket",
            "bandwidthOrIdleLimit",
            "lastViewerStopsScreencastWithinTwoSeconds",
            "windowsRuntime",
            "linuxRunscRuntime",
        ] {
            assert_eq!(fixture["evidenceBoundary"][unfinished], false);
        }
    }
}
