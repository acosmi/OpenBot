//! Compiled-component list and additive build-catalogue HTTP framing.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use http::header::CACHE_CONTROL;
use http::{HeaderMap, HeaderValue};
use openbot_contracts::command::{AppCommand, AppReply};
use openbot_contracts::components::{
    ComponentCatalogueAdded, ComponentCatalogueRequest, ComponentRecords,
};
use openbot_contracts::error::AppError;

use crate::auth::{Authenticated, OriginAuthenticated};
use crate::error::HttpError;
use crate::http::ServerState;

/// `GET /api/components`; any authenticated person may inspect deployment governance facts.
pub async fn list_get(
    State(state): State<ServerState>,
    Authenticated(auth): Authenticated,
) -> Result<(HeaderMap, Json<ComponentRecords>), HttpError> {
    match state
        .application()
        .execute(auth, AppCommand::ListComponents)
        .await?
    {
        AppReply::Components(components) => Ok((no_store(), Json(components))),
        _ => Err(application_contract_error()),
    }
}

/// `PUT /api/components/catalogue`; the application accepts only byte-exact build manifest rows.
pub async fn catalogue_put(
    State(state): State<ServerState>,
    OriginAuthenticated(auth): OriginAuthenticated,
    body: Result<Json<ComponentCatalogueRequest>, JsonRejection>,
) -> Result<(HeaderMap, Json<ComponentCatalogueAdded>), HttpError> {
    let Json(request) = body.map_err(|error| {
        tracing::debug!(error = %error, "component catalogue body rejected");
        AppError::MalformedPayload { field: "body" }
    })?;
    match state
        .application()
        .execute(auth, AppCommand::SyncComponentCatalogue(request))
        .await?
    {
        AppReply::ComponentCatalogueAdded(added) => Ok((no_store(), Json(added))),
        _ => Err(application_contract_error()),
    }
}

fn no_store() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers
}

fn application_contract_error() -> HttpError {
    tracing::error!("component command received mismatched reply");
    AppError::DependencyUnavailable {
        dependency: "application",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use http::{Method, Request, StatusCode};
    use openbot_application::cursor::ChannelCursor;
    use openbot_application::{
        ApplicationService, ChannelReader, ComponentAdministration, ComponentAdministrationError,
        OpenBotApplication, PortError,
    };
    use openbot_contracts::auth::{AuthContext, AuthGeneration, Role};
    use openbot_contracts::command::ChannelSummary;
    use openbot_contracts::components::{
        CompiledComponentKind, CompiledComponentManifestEntry, ComponentRecord,
        SHOW_QUOTE_COMPONENT_NAME, compiled_component_manifest,
    };
    use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};
    use openbot_domain::identity::session::TrustedOrigins;
    use openbot_infra::auth::config::default_session_lifetime;
    use time::OffsetDateTime;
    use tower::ServiceExt as _;

    use crate::auth::{FixedAuthResolver, SensitiveWriteSecurity};
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

    #[derive(Clone, Default)]
    struct FakeComponents {
        syncs: Arc<Mutex<Vec<Vec<CompiledComponentManifestEntry>>>>,
    }

    #[async_trait]
    impl ComponentAdministration for FakeComponents {
        async fn list_components(
            &self,
            _auth: &AuthContext,
        ) -> Result<ComponentRecords, ComponentAdministrationError> {
            Ok(ComponentRecords {
                components: vec![ComponentRecord {
                    name: SHOW_QUOTE_COMPONENT_NAME.to_owned(),
                    title: "Quotation".to_owned(),
                    kind: CompiledComponentKind::Card,
                    draft_description: "quote".to_owned(),
                    published_description: Some("quote".to_owned()),
                    published: true,
                    published_at: Some(OffsetDateTime::UNIX_EPOCH),
                    updated_by: Some("the build".to_owned()),
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                    has_unpublished_changes: false,
                    withheld_from: Vec::new(),
                    functions: Vec::new(),
                }],
            })
        }

        async fn sync_catalogue(
            &self,
            _auth: &AuthContext,
            entries: &[CompiledComponentManifestEntry],
        ) -> Result<ComponentCatalogueAdded, ComponentAdministrationError> {
            self.syncs.lock().unwrap().push(entries.to_vec());
            Ok(ComponentCatalogueAdded {
                added: entries.iter().map(|entry| entry.name.clone()).collect(),
            })
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

    fn app(components: FakeComponents) -> axum::Router {
        let application: Arc<dyn ApplicationService> = Arc::new(
            OpenBotApplication::new(EmptyChannels)
                .with_component_administration(Arc::new(components)),
        );
        crate::router(
            ServerBuilder::new(application, Arc::new(FixedAuthResolver::granting(auth())))
                .with_sensitive_write_security(SensitiveWriteSecurity::new(
                    default_session_lifetime(),
                    TrustedOrigins::from_configured(["https://app.example.test"]).unwrap(),
                ))
                .build(),
        )
    }

    async fn send(
        router: axum::Router,
        method: Method,
        uri: &str,
        origin: Option<&str>,
        body: Body,
    ) -> axum::response::Response {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(origin) = origin {
            request = request.header(http::header::ORIGIN, origin);
        }
        request = request.header(http::header::CONTENT_TYPE, "application/json");
        router.oneshot(request.body(body).unwrap()).await.unwrap()
    }

    #[tokio::test]
    async fn list_and_catalogue_are_typed_no_store_and_origin_precedes_body() {
        let components = FakeComponents::default();
        let list = send(
            app(components.clone()),
            Method::GET,
            "/api/components",
            None,
            Body::empty(),
        )
        .await;
        assert_eq!(list.status(), StatusCode::OK);
        assert_eq!(list.headers()[CACHE_CONTROL], "no-store");
        let list_body = to_bytes(list.into_body(), 4096).await.unwrap();
        assert!(!String::from_utf8_lossy(&list_body).contains("secret"));

        let rejected = send(
            app(components.clone()),
            Method::PUT,
            "/api/components/catalogue",
            None,
            Body::from("{"),
        )
        .await;
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
        assert!(components.syncs.lock().unwrap().is_empty());

        let body = serde_json::to_vec(&ComponentCatalogueRequest {
            components: compiled_component_manifest(),
        })
        .unwrap();
        let accepted = send(
            app(components.clone()),
            Method::PUT,
            "/api/components/catalogue",
            Some("https://app.example.test"),
            Body::from(body),
        )
        .await;
        assert_eq!(accepted.status(), StatusCode::OK);
        assert_eq!(accepted.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(components.syncs.lock().unwrap().len(), 1);
    }
}
