//! Authenticated compiled-component catalogue reads and additive build announcements.

use std::collections::BTreeSet;

use async_trait::async_trait;
use openbot_contracts::auth::AuthContext;
use openbot_contracts::components::{
    CompiledComponentManifestEntry, ComponentCatalogueAdded, ComponentCatalogueRequest,
    ComponentRecords, compiled_component_manifest,
};
use openbot_contracts::error::AppError;

/// Stable component-catalogue failures without SQL values or model/user source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ComponentAdministrationError {
    /// Closed manifest/request validation failed.
    #[error("component_invalid_input field={field}")]
    InvalidInput {
        /// Static field only.
        field: &'static str,
    },
    /// PostgreSQL or another required local dependency is unavailable.
    #[error("component_unavailable")]
    Unavailable,
    /// A durable row cannot be projected into the closed contract.
    #[error("component_corrupt field={field}")]
    Corrupt {
        /// Static field only.
        field: &'static str,
    },
    /// A catalogue/audit transaction may have committed and must be reconciled.
    #[error("component_commit_unknown")]
    CommitUnknown,
}

impl ComponentAdministrationError {
    /// Map into the stable application error taxonomy.
    #[must_use]
    pub const fn into_app_error(self) -> AppError {
        match self {
            Self::InvalidInput { field } => AppError::MalformedPayload { field },
            Self::Unavailable | Self::Corrupt { .. } => AppError::DependencyUnavailable {
                dependency: "components",
            },
            Self::CommitUnknown => AppError::ReconciliationRequired { accepted: true },
        }
    }
}

/// Production port for component governance reads and additive catalogue synchronization.
#[async_trait]
pub trait ComponentAdministration: Send + Sync {
    /// List all durable compiled-component governance records for an authenticated actor.
    async fn list_components(
        &self,
        auth: &AuthContext,
    ) -> Result<ComponentRecords, ComponentAdministrationError>;

    /// Insert missing exact build entries; existing governance rows must remain byte-for-byte owned
    /// by administration.
    async fn sync_catalogue(
        &self,
        auth: &AuthContext,
        entries: &[CompiledComponentManifestEntry],
    ) -> Result<ComponentCatalogueAdded, ComponentAdministrationError>;
}

/// Fail-closed default for hosts that have not attached component persistence.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoComponentAdministration;

#[async_trait]
impl ComponentAdministration for NoComponentAdministration {
    async fn list_components(
        &self,
        _auth: &AuthContext,
    ) -> Result<ComponentRecords, ComponentAdministrationError> {
        Err(ComponentAdministrationError::Unavailable)
    }

    async fn sync_catalogue(
        &self,
        _auth: &AuthContext,
        _entries: &[CompiledComponentManifestEntry],
    ) -> Result<ComponentCatalogueAdded, ComponentAdministrationError> {
        Err(ComponentAdministrationError::Unavailable)
    }
}

/// List the authenticated deployment's durable compiled-component governance rows.
pub async fn list_components(
    port: &dyn ComponentAdministration,
    auth: &AuthContext,
) -> Result<ComponentRecords, AppError> {
    port.list_components(auth)
        .await
        .map_err(ComponentAdministrationError::into_app_error)
}

/// Validate browser-repeated metadata against the build-owned manifest, then insert only missing
/// rows through the authority port.
pub async fn sync_component_catalogue(
    port: &dyn ComponentAdministration,
    auth: &AuthContext,
    request: ComponentCatalogueRequest,
) -> Result<ComponentCatalogueAdded, AppError> {
    validate_manifest_entries(&request.components).map_err(|error| error.into_app_error())?;
    port.sync_catalogue(auth, &request.components)
        .await
        .map_err(ComponentAdministrationError::into_app_error)
}

/// Check that every repeated entry is a unique, byte-exact member of this build's manifest.
pub fn validate_manifest_entries(
    entries: &[CompiledComponentManifestEntry],
) -> Result<(), ComponentAdministrationError> {
    let manifest = compiled_component_manifest();
    if entries.len() > manifest.len() {
        return Err(ComponentAdministrationError::InvalidInput {
            field: "components",
        });
    }
    let mut names = BTreeSet::new();
    for entry in entries {
        if !names.insert(entry.name.as_str()) {
            return Err(ComponentAdministrationError::InvalidInput {
                field: "component_name",
            });
        }
        if !manifest.iter().any(|known| known == entry) {
            return Err(ComponentAdministrationError::InvalidInput {
                field: "component_identity",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use openbot_contracts::auth::{AuthContext, AuthGeneration, Role};
    use openbot_contracts::components::{
        CompiledComponentKind, ComponentRecord, SHOW_QUOTE_COMPONENT_NAME,
    };
    use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};
    use time::OffsetDateTime;

    use super::*;

    #[derive(Default)]
    struct FakeComponents {
        syncs: Mutex<Vec<Vec<CompiledComponentManifestEntry>>>,
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

    #[tokio::test]
    async fn any_authenticated_role_can_list_and_only_exact_manifest_reaches_the_port() {
        let port = FakeComponents::default();
        assert_eq!(
            list_components(&port, &auth())
                .await
                .unwrap()
                .components
                .len(),
            1
        );
        let manifest = compiled_component_manifest();
        assert_eq!(
            sync_component_catalogue(
                &port,
                &auth(),
                ComponentCatalogueRequest {
                    components: manifest.clone(),
                },
            )
            .await
            .unwrap()
            .added,
            [SHOW_QUOTE_COMPONENT_NAME]
        );
        assert_eq!(port.syncs.lock().unwrap().as_slice(), [manifest]);
    }

    #[tokio::test]
    async fn unknown_tampered_and_duplicate_entries_fail_before_the_port() {
        let port = FakeComponents::default();
        let mut tampered = compiled_component_manifest();
        tampered[0].description.push_str(" attacker");
        let duplicate = vec![
            compiled_component_manifest().remove(0),
            compiled_component_manifest().remove(0),
        ];
        for components in [tampered, duplicate] {
            assert_eq!(
                sync_component_catalogue(&port, &auth(), ComponentCatalogueRequest { components },)
                    .await
                    .unwrap_err()
                    .code()
                    .as_str(),
                "malformed_payload"
            );
        }
        assert!(port.syncs.lock().unwrap().is_empty());
    }
}
