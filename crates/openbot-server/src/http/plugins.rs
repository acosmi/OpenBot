//! MCP per-user connection HTTP framing. Business, OAuth and persistence stay behind typed ports.

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::header::{CACHE_CONTROL, LOCATION, REFERRER_POLICY};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use openbot_application::{McpOAuthCallbackInput, McpOAuthCallbackOutcome};
use openbot_contracts::command::{AppCommand, AppReply};
use openbot_contracts::error::AppError;
use openbot_contracts::mcp::{
    McpConnectionDisconnected, McpConnections, McpOAuthAuthorization, McpOAuthClientRegistered,
    McpOAuthClientRegistration, McpOAuthReturnTo, McpServerMutation,
};
use serde::Deserialize;

use crate::auth::{Authenticated, OriginAuthenticated, SensitiveAuthenticated};
use crate::error::HttpError;
use crate::http::ServerState;

/// `GET /api/plugins/connections`.
pub async fn connections_get(
    State(state): State<ServerState>,
    Authenticated(auth): Authenticated,
) -> Result<(HeaderMap, Json<McpConnections>), HttpError> {
    match state
        .application()
        .execute(auth, AppCommand::ListMcpConnections)
        .await?
    {
        AppReply::McpConnections(connections) => Ok((no_store_headers(), Json(connections))),
        _ => Err(application_contract_error()),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Closed connect-origin query; any value except `admin` falls back to settings.
pub struct BeginConnectQuery {
    return_to: Option<String>,
}

/// `POST /api/plugins/servers/{id}/connect`; answers with URL, never redirects automatically.
pub async fn servers_connect_post(
    State(state): State<ServerState>,
    OriginAuthenticated(auth): OriginAuthenticated,
    Path(server_id): Path<String>,
    query: Result<Query<BeginConnectQuery>, QueryRejection>,
) -> Result<(HeaderMap, Json<McpOAuthAuthorization>), HttpError> {
    let Query(query) = query.map_err(|rejection| {
        tracing::debug!(rejection = %rejection, "MCP connect query 解析失败");
        AppError::MalformedPayload { field: "query" }
    })?;
    let return_to = if query.return_to.as_deref() == Some("admin") {
        McpOAuthReturnTo::Admin
    } else {
        McpOAuthReturnTo::Settings
    };
    match state
        .application()
        .execute(
            auth,
            AppCommand::BeginMcpOAuth {
                server_id,
                return_to,
            },
        )
        .await?
    {
        AppReply::McpOAuthAuthorization(authorization) => {
            Ok((no_store_headers(), Json(authorization)))
        }
        _ => Err(application_contract_error()),
    }
}

/// `DELETE /api/plugins/connections/{id}`; local deny is committed before vendor reconciliation.
pub async fn connections_delete(
    State(state): State<ServerState>,
    OriginAuthenticated(auth): OriginAuthenticated,
    Path(server_id): Path<String>,
) -> Result<(HeaderMap, Json<McpConnectionDisconnected>), HttpError> {
    match state
        .application()
        .execute(auth, AppCommand::DisconnectMcpConnection { server_id })
        .await?
    {
        AppReply::McpConnectionDisconnected(receipt) => Ok((no_store_headers(), Json(receipt))),
        _ => Err(application_contract_error()),
    }
}

/// `POST /api/plugins/servers/{id}/oauth-client`; fresh admin/origin guard runs before body parse.
pub async fn servers_oauth_client_post(
    State(state): State<ServerState>,
    SensitiveAuthenticated(resolved): SensitiveAuthenticated,
    headers: HeaderMap,
    Path(server_id): Path<String>,
    body: Result<Json<McpOAuthClientRegistration>, JsonRejection>,
) -> Result<Json<McpOAuthClientRegistered>, HttpError> {
    state
        .authorize_fresh_origin_write(&resolved, request_origin(&headers))
        .await?;
    let Json(registration) = body.map_err(|rejection| {
        tracing::debug!(rejection = %rejection, "MCP OAuth client body 解析失败");
        AppError::MalformedPayload { field: "body" }
    })?;
    match state
        .application()
        .execute(
            resolved.into_context(),
            AppCommand::RegisterMcpOAuthClient {
                server_id,
                registration,
            },
        )
        .await?
    {
        AppReply::McpOAuthClientRegistered(registered) => Ok(Json(registered)),
        _ => Err(application_contract_error()),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
/// Closed curated-server request; endpoint/vendor/transport are not caller-controlled.
pub struct AddCuratedServerBody {
    key: String,
}

/// `POST /api/plugins/servers`; the request selects a reviewed key and can never supply a URL.
pub async fn servers_post(
    State(state): State<ServerState>,
    SensitiveAuthenticated(resolved): SensitiveAuthenticated,
    headers: HeaderMap,
    body: Result<Json<AddCuratedServerBody>, JsonRejection>,
) -> Result<Json<McpServerMutation>, HttpError> {
    state
        .authorize_fresh_origin_write(&resolved, request_origin(&headers))
        .await?;
    let Json(body) = body.map_err(|rejection| {
        tracing::debug!(rejection = %rejection, "curated plugin server body 解析失败");
        AppError::MalformedPayload { field: "body" }
    })?;
    match state
        .application()
        .execute(
            resolved.into_context(),
            AppCommand::AddCuratedMcpServer { key: body.key },
        )
        .await?
    {
        AppReply::McpServerMutation(receipt) => Ok(Json(receipt)),
        _ => Err(application_contract_error()),
    }
}

/// `POST /api/plugins/servers/{id}/refresh`; Drive is static and needs no personal token.
pub async fn servers_refresh_post(
    State(state): State<ServerState>,
    SensitiveAuthenticated(resolved): SensitiveAuthenticated,
    headers: HeaderMap,
    Path(server_id): Path<String>,
) -> Result<Json<McpServerMutation>, HttpError> {
    state
        .authorize_fresh_origin_write(&resolved, request_origin(&headers))
        .await?;
    match state
        .application()
        .execute(
            resolved.into_context(),
            AppCommand::RefreshMcpServer { server_id },
        )
        .await?
    {
        AppReply::McpServerMutation(receipt) => Ok(Json(receipt)),
        _ => Err(application_contract_error()),
    }
}

#[derive(Debug, Default, Deserialize)]
/// OAuth callback parameters accepted for framing; error details are consumed but never rendered.
pub struct OAuthCallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    iss: Option<String>,
    #[serde(default)]
    #[serde(rename = "error")]
    _error: Option<String>,
    #[serde(default, rename = "error_description")]
    _error_description: Option<String>,
    #[serde(default, rename = "error_uri")]
    _error_uri: Option<String>,
}

/// Public OAuth callback. Every outcome is a no-store redirect; response bodies never contain
/// code/state/vendor error material.
pub async fn oauth_callback_get(
    State(state): State<ServerState>,
    query: Result<Query<OAuthCallbackQuery>, QueryRejection>,
) -> Response {
    let query = query.map(|Query(query)| query).unwrap_or_default();
    let bounded = |value: Option<String>, max: usize| {
        value
            .filter(|value| value.len() <= max && !value.as_bytes().contains(&0))
            .unwrap_or_default()
    };
    let input = McpOAuthCallbackInput::new(
        bounded(query.code, 16 * 1024).into_bytes(),
        bounded(query.state, 512).into_bytes(),
        query
            .iss
            .filter(|value| value.len() <= 8 * 1024 && !value.as_bytes().contains(&0)),
    );
    let outcome = match state.mcp_oauth_callback() {
        Some(coordinator) => coordinator.complete(input).await,
        None => McpOAuthCallbackOutcome {
            redirect_to: "/settings/connected-accounts?connected=failed".to_owned(),
        },
    };
    redirect_no_store(outcome)
}

fn redirect_no_store(outcome: McpOAuthCallbackOutcome) -> Response {
    let Ok(location) = HeaderValue::from_str(&outcome.redirect_to) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let mut response = StatusCode::FOUND.into_response();
    response.headers_mut().insert(LOCATION, location);
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

fn no_store_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers
}

fn application_contract_error() -> HttpError {
    AppError::DependencyUnavailable {
        dependency: "application",
    }
    .into()
}

fn request_origin(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(http::header::ORIGIN)
        .map(|value| value.to_str().unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use http::{Method, Request};
    use openbot_application::cursor::ChannelCursor;
    use openbot_application::{
        ApplicationService, ChannelReader, McpConnectionAdministration, McpConnectionError,
        McpOAuthCallback, OpenBotApplication, PortError,
    };
    use openbot_contracts::auth::{AuthContext, AuthGeneration, Role};
    use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};
    use openbot_contracts::mcp::McpVendorRevocationStatus;
    use openbot_domain::identity::session::{SessionState, TrustedOrigins, evaluate_session};
    use openbot_infra::auth::config::default_session_lifetime;
    use std::sync::{Arc, Mutex};
    use time::{Duration, OffsetDateTime};
    use tower::ServiceExt as _;

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
        ) -> Result<Vec<openbot_contracts::command::ChannelSummary>, PortError> {
            Ok(Vec::new())
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Call {
        List(ActorId),
        Begin(ActorId, String, McpOAuthReturnTo),
        Disconnect(ActorId, String),
        Register(ActorId, String),
        Add(ActorId, String),
        Refresh(ActorId, String),
    }

    #[derive(Clone, Default)]
    struct FakeConnections {
        calls: Arc<Mutex<Vec<Call>>>,
    }

    #[async_trait]
    impl McpConnectionAdministration for FakeConnections {
        async fn list_connections(
            &self,
            auth: &AuthContext,
        ) -> Result<McpConnections, McpConnectionError> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::List(auth.actor().clone()));
            Ok(McpConnections {
                available_server_ids: vec!["notes".to_owned()],
                connections: vec![openbot_contracts::mcp::McpConnection {
                    server_id: "notes".to_owned(),
                    scope: "notes:read".to_owned(),
                    connected_at: OffsetDateTime::UNIX_EPOCH,
                }],
                redirect_uri: Some(
                    "https://api.example.test/api/plugins/oauth/callback".to_owned(),
                ),
            })
        }

        async fn begin_oauth(
            &self,
            auth: &AuthContext,
            server_id: &str,
            return_to: McpOAuthReturnTo,
        ) -> Result<McpOAuthAuthorization, McpConnectionError> {
            self.calls.lock().unwrap().push(Call::Begin(
                auth.actor().clone(),
                server_id.to_owned(),
                return_to,
            ));
            Ok(McpOAuthAuthorization {
                authorization_url: "https://auth.example.test/authorize?state=redacted".to_owned(),
            })
        }

        async fn disconnect(
            &self,
            auth: &AuthContext,
            server_id: &str,
        ) -> Result<McpConnectionDisconnected, McpConnectionError> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Disconnect(auth.actor().clone(), server_id.to_owned()));
            Ok(McpConnectionDisconnected {
                server_id: server_id.to_owned(),
                vendor_revocation: McpVendorRevocationStatus::Pending,
            })
        }

        async fn register_oauth_client(
            &self,
            auth: &AuthContext,
            server_id: &str,
            _registration: &McpOAuthClientRegistration,
        ) -> Result<McpOAuthClientRegistered, McpConnectionError> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Register(auth.actor().clone(), server_id.to_owned()));
            Ok(McpOAuthClientRegistered::success())
        }

        async fn add_curated_server(
            &self,
            auth: &AuthContext,
            key: &str,
        ) -> Result<McpServerMutation, McpConnectionError> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Add(auth.actor().clone(), key.to_owned()));
            Ok(McpServerMutation {
                server_id: key.to_owned(),
                catalog_generation: 1,
                tool_count: 4,
                suspended_grants: 0,
            })
        }

        async fn refresh_server(
            &self,
            auth: &AuthContext,
            server_id: &str,
        ) -> Result<McpServerMutation, McpConnectionError> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Refresh(auth.actor().clone(), server_id.to_owned()));
            Ok(McpServerMutation {
                server_id: server_id.to_owned(),
                catalog_generation: 2,
                tool_count: 4,
                suspended_grants: 0,
            })
        }
    }

    type CallbackCall = (Vec<u8>, Vec<u8>, Option<String>);

    struct FakeCallback {
        calls: Mutex<Vec<CallbackCall>>,
    }

    #[async_trait]
    impl McpOAuthCallback for FakeCallback {
        async fn complete(&self, input: McpOAuthCallbackInput) -> McpOAuthCallbackOutcome {
            self.calls.lock().unwrap().push((
                input.code().to_vec(),
                input.state().to_vec(),
                input.issuer().map(str::to_owned),
            ));
            McpOAuthCallbackOutcome {
                redirect_to: "https://app.example.test/settings/connected-accounts/notes"
                    .to_owned(),
            }
        }
    }

    fn auth() -> AuthContext {
        AuthContext::for_test(
            DeploymentId::new("dep"),
            TenantId::new("tenant"),
            ActorId::new("actor"),
            [Role::User],
            AuthGeneration::new(1),
            false,
        )
    }

    fn app(connections: FakeConnections, callback: Arc<FakeCallback>) -> (Router, FakeConnections) {
        let application: Arc<dyn ApplicationService> = Arc::new(
            OpenBotApplication::new(EmptyChannels)
                .with_mcp_connections(Arc::new(connections.clone())),
        );
        let router = crate::router(
            ServerBuilder::new(application, Arc::new(FixedAuthResolver::granting(auth())))
                .with_sensitive_write_security(SensitiveWriteSecurity::new(
                    default_session_lifetime(),
                    TrustedOrigins::from_configured(["https://app.example.test"]).unwrap(),
                ))
                .with_mcp_oauth_callback(callback)
                .build(),
        );
        (router, connections)
    }

    async fn send(router: Router, method: Method, uri: &str, origin: Option<&str>) -> Response {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(origin) = origin {
            request = request.header(http::header::ORIGIN, origin);
        }
        router
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn send_body(
        router: Router,
        uri: &str,
        origin: Option<&str>,
        body: &'static str,
    ) -> Response {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header(http::header::CONTENT_TYPE, "application/json");
        if let Some(origin) = origin {
            request = request.header(http::header::ORIGIN, origin);
        }
        router
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }

    fn registration_app(connections: FakeConnections) -> Router {
        let generation = AuthGeneration::new(1);
        let context = AuthContext::for_test(
            DeploymentId::new("dep"),
            TenantId::new("tenant"),
            ActorId::new("actor"),
            [Role::Admin],
            generation,
            false,
        );
        let now = OffsetDateTime::now_utc();
        let lifetime = default_session_lifetime();
        let live = evaluate_session(
            lifetime,
            SessionState::rehydrate(now - Duration::minutes(1), now, generation),
            generation,
            now,
        )
        .unwrap();
        let resolver = FixedAuthResolver::granting_resolved(ResolvedAuth::from_live_session(
            context, live, None,
        ));
        crate::router(
            ServerBuilder::new(
                Arc::new(
                    OpenBotApplication::new(EmptyChannels)
                        .with_mcp_connections(Arc::new(connections)),
                ),
                Arc::new(resolver),
            )
            .with_sensitive_write_security(SensitiveWriteSecurity::new(
                lifetime,
                TrustedOrigins::from_configured(["https://app.example.test"]).unwrap(),
            ))
            .build(),
        )
    }

    #[tokio::test]
    async fn authenticated_routes_only_frame_typed_commands_and_origin_guards_writes() {
        let callback = Arc::new(FakeCallback {
            calls: Mutex::new(Vec::new()),
        });
        let (router, connections) = app(FakeConnections::default(), callback);
        let list = send(
            router.clone(),
            Method::GET,
            "/api/plugins/connections",
            None,
        )
        .await;
        assert_eq!(list.status(), StatusCode::OK);
        assert_eq!(list.headers().get(CACHE_CONTROL).unwrap(), "no-store");

        let rejected = send(
            router.clone(),
            Method::POST,
            "/api/plugins/servers/notes/connect?returnTo=admin",
            None,
        )
        .await;
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
        let begin = send(
            router.clone(),
            Method::POST,
            "/api/plugins/servers/notes/connect?returnTo=admin",
            Some("https://app.example.test"),
        )
        .await;
        assert_eq!(begin.status(), StatusCode::OK);
        assert_eq!(begin.headers().get(CACHE_CONTROL).unwrap(), "no-store");
        let untrusted_return = send(
            router.clone(),
            Method::POST,
            "/api/plugins/servers/notes/connect?returnTo=https%3A%2F%2Fevil.test",
            Some("https://app.example.test"),
        )
        .await;
        assert_eq!(untrusted_return.status(), StatusCode::OK);
        assert_eq!(
            untrusted_return.headers().get(CACHE_CONTROL).unwrap(),
            "no-store"
        );
        let disconnected = send(
            router,
            Method::DELETE,
            "/api/plugins/connections/notes",
            Some("https://app.example.test"),
        )
        .await;
        assert_eq!(disconnected.status(), StatusCode::OK);
        assert_eq!(
            disconnected.headers().get(CACHE_CONTROL).unwrap(),
            "no-store"
        );
        assert_eq!(
            connections.calls.lock().unwrap().as_slice(),
            [
                Call::List(ActorId::new("actor")),
                Call::Begin(
                    ActorId::new("actor"),
                    "notes".to_owned(),
                    McpOAuthReturnTo::Admin
                ),
                Call::Begin(
                    ActorId::new("actor"),
                    "notes".to_owned(),
                    McpOAuthReturnTo::Settings
                ),
                Call::Disconnect(ActorId::new("actor"), "notes".to_owned())
            ]
        );
    }

    #[tokio::test]
    async fn callback_is_public_uniform_bodyless_no_store_redirect() {
        let callback = Arc::new(FakeCallback {
            calls: Mutex::new(Vec::new()),
        });
        let (router, _) = app(FakeConnections::default(), callback.clone());
        let response = send(
            router,
            Method::GET,
            "/api/plugins/oauth/callback?code=CODE-CANARY&state=STATE-CANARY&iss=https%3A%2F%2Fissuer.example",
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response.headers().get(LOCATION).unwrap(),
            "https://app.example.test/settings/connected-accounts/notes"
        );
        assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
        assert_eq!(
            response.headers().get(REFERRER_POLICY).unwrap(),
            "no-referrer"
        );
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert!(body.is_empty());
        assert_eq!(
            callback.calls.lock().unwrap().as_slice(),
            [(
                b"CODE-CANARY".to_vec(),
                b"STATE-CANARY".to_vec(),
                Some("https://issuer.example".to_owned())
            )]
        );
    }

    #[tokio::test]
    async fn oauth_client_registration_guards_fresh_origin_before_secret_body_parse() {
        let connections = FakeConnections::default();
        let uri = "/api/plugins/servers/notes/oauth-client";
        let before_body = send_body(registration_app(connections.clone()), uri, None, "{").await;
        assert_eq!(before_body.status(), StatusCode::FORBIDDEN);
        assert!(connections.calls.lock().unwrap().is_empty());

        let malformed = send_body(
            registration_app(connections.clone()),
            uri,
            Some("https://app.example.test"),
            "{",
        )
        .await;
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
        assert!(connections.calls.lock().unwrap().is_empty());

        let valid = send_body(
            registration_app(connections.clone()),
            uri,
            Some("https://app.example.test"),
            r#"{"clientId":"client","clientSecret":"CLIENT-SECRET-CANARY","issuer":"https://issuer.example","tokenEndpointAuthMethod":"client_secret_basic"}"#,
        )
        .await;
        assert_eq!(valid.status(), StatusCode::OK);
        let body = to_bytes(valid.into_body(), 1024).await.unwrap();
        assert_eq!(body.as_ref(), br#"{"ok":true}"#);
        assert_eq!(
            connections.calls.lock().unwrap().as_slice(),
            [Call::Register(ActorId::new("actor"), "notes".to_owned())]
        );
    }

    #[tokio::test]
    async fn curated_add_and_refresh_are_fresh_admin_typed_commands_without_a_url_input() {
        let connections = FakeConnections::default();
        let rejected = send_body(
            registration_app(connections.clone()),
            "/api/plugins/servers",
            None,
            "{",
        )
        .await;
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
        assert!(connections.calls.lock().unwrap().is_empty());

        let unknown_field = send_body(
            registration_app(connections.clone()),
            "/api/plugins/servers",
            Some("https://app.example.test"),
            r#"{"key":"google-drive","url":"https://evil.example"}"#,
        )
        .await;
        assert_eq!(unknown_field.status(), StatusCode::BAD_REQUEST);
        assert!(connections.calls.lock().unwrap().is_empty());

        let added = send_body(
            registration_app(connections.clone()),
            "/api/plugins/servers",
            Some("https://app.example.test"),
            r#"{"key":"google-drive"}"#,
        )
        .await;
        assert_eq!(added.status(), StatusCode::OK);
        let body = to_bytes(added.into_body(), 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({
                "serverId":"google-drive","catalogGeneration":1,
                "toolCount":4,"suspendedGrants":0
            })
        );

        let refreshed = send(
            registration_app(connections.clone()),
            Method::POST,
            "/api/plugins/servers/google-drive/refresh",
            Some("https://app.example.test"),
        )
        .await;
        assert_eq!(refreshed.status(), StatusCode::OK);
        assert_eq!(
            connections.calls.lock().unwrap().as_slice(),
            [
                Call::Add(ActorId::new("actor"), "google-drive".to_owned()),
                Call::Refresh(ActorId::new("actor"), "google-drive".to_owned())
            ]
        );
    }
}
