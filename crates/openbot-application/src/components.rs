//! Authenticated compiled-component catalogue reads and additive build announcements.

use std::collections::BTreeSet;

use async_trait::async_trait;
use openbot_contracts::auth::AuthContext;
use openbot_contracts::components::{
    CompiledComponentManifestEntry, ComponentCatalogueAdded, ComponentCatalogueRequest,
    ComponentDecision, ComponentDecisionRefusal, ComponentDecisionRequest, ComponentRecords,
    GrantedCompiledComponents, compiled_component_manifest,
};
use openbot_contracts::error::AppError;
use openbot_contracts::ids::{ActorId, BotId, TenantId};

const MAX_RUNTIME_IDENTIFIERS: usize = 1024;
const MAX_RUNTIME_IDENTIFIER_BYTES: usize = 256;

/// Authority already resolved for one compiled-component runtime request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentRuntimeScope {
    /// Verified tenant, used as the outermost Agent/package boundary.
    pub tenant: TenantId,
    /// Verified current actor.
    pub actor: ActorId,
    /// Whether verified roles include administrator.
    pub admin: bool,
    /// Untrusted Agent target after bounded identifier validation.
    pub agent_id: BotId,
}

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
    /// The named Agent is absent or not runnable by the verified actor in this tenant.
    #[error("component_agent_not_visible")]
    NotVisible,
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
            Self::NotVisible => AppError::NotVisible,
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

    /// List the current build's published, non-withheld renderer grants for one runnable Agent.
    async fn list_components_for_agent(
        &self,
        _scope: &ComponentRuntimeScope,
        _renderer_names: &[String],
    ) -> Result<GrantedCompiledComponents, ComponentAdministrationError> {
        Err(ComponentAdministrationError::Unavailable)
    }

    /// Re-authorize one component and all data functions declared by this exact invocation.
    async fn decide_component(
        &self,
        _scope: &ComponentRuntimeScope,
        _component_name: &str,
        _build_has_renderer: bool,
        _functions: &[String],
    ) -> Result<ComponentDecision, ComponentAdministrationError> {
        Err(ComponentAdministrationError::Unavailable)
    }
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

/// List the current build's actual runtime grants for one current-actor-runnable Agent.
pub async fn list_components_for_agent(
    port: &dyn ComponentAdministration,
    auth: &AuthContext,
    agent_id: BotId,
) -> Result<GrantedCompiledComponents, AppError> {
    validate_runtime_identifier(agent_id.as_str(), "agent_id")
        .map_err(ComponentAdministrationError::into_app_error)?;
    let scope = runtime_scope(auth, agent_id);
    let renderer_names = compiled_component_manifest()
        .into_iter()
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    let granted = port
        .list_components_for_agent(&scope, &renderer_names)
        .await
        .map_err(ComponentAdministrationError::into_app_error)?;
    validate_granted_components(&granted, &renderer_names)
        .map_err(ComponentAdministrationError::into_app_error)?;
    Ok(granted)
}

/// Re-authorize a component immediately before accepting one renderer tool call.
pub async fn decide_component(
    port: &dyn ComponentAdministration,
    auth: &AuthContext,
    component_name: String,
    request: ComponentDecisionRequest,
) -> Result<ComponentDecision, AppError> {
    validate_component_name(&component_name)
        .map_err(ComponentAdministrationError::into_app_error)?;
    validate_runtime_identifier(request.agent_id.as_str(), "agent_id")
        .map_err(ComponentAdministrationError::into_app_error)?;
    let functions = canonicalize_functions(request.functions)
        .map_err(ComponentAdministrationError::into_app_error)?;
    let build_has_renderer = compiled_component_manifest()
        .iter()
        .any(|entry| entry.name == component_name);
    let scope = runtime_scope(auth, request.agent_id);
    let decision = port
        .decide_component(&scope, &component_name, build_has_renderer, &functions)
        .await
        .map_err(ComponentAdministrationError::into_app_error)?;
    validate_decision(&decision, &functions)
        .map_err(ComponentAdministrationError::into_app_error)?;
    Ok(decision)
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

fn runtime_scope(auth: &AuthContext, agent_id: BotId) -> ComponentRuntimeScope {
    ComponentRuntimeScope {
        tenant: auth.tenant().clone(),
        actor: auth.actor().clone(),
        admin: auth.has_role(openbot_contracts::auth::Role::Admin),
        agent_id,
    }
}

fn canonicalize_functions(
    functions: Vec<String>,
) -> Result<Vec<String>, ComponentAdministrationError> {
    if functions.len() > MAX_RUNTIME_IDENTIFIERS {
        return Err(ComponentAdministrationError::InvalidInput { field: "functions" });
    }
    let mut unique = BTreeSet::new();
    for function in functions {
        validate_component_name(&function)?;
        unique.insert(function);
    }
    Ok(unique.into_iter().collect())
}

fn validate_component_name(value: &str) -> Result<(), ComponentAdministrationError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Err(ComponentAdministrationError::InvalidInput {
            field: "component_name",
        })
    } else {
        Ok(())
    }
}

fn validate_runtime_identifier(
    value: &str,
    field: &'static str,
) -> Result<(), ComponentAdministrationError> {
    if value.is_empty()
        || value.len() > MAX_RUNTIME_IDENTIFIER_BYTES
        || value.chars().any(char::is_control)
    {
        Err(ComponentAdministrationError::InvalidInput { field })
    } else {
        Ok(())
    }
}

fn validate_granted_components(
    granted: &GrantedCompiledComponents,
    renderer_names: &[String],
) -> Result<(), ComponentAdministrationError> {
    if granted.components.len() > renderer_names.len() {
        return Err(ComponentAdministrationError::Corrupt {
            field: "component_grants",
        });
    }
    let renderers = renderer_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut previous = None::<&str>;
    for component in &granted.components {
        validate_component_name(&component.name).map_err(|_| {
            ComponentAdministrationError::Corrupt {
                field: "component_name",
            }
        })?;
        if !renderers.contains(component.name.as_str())
            || previous.is_some_and(|previous| previous >= component.name.as_str())
            || component.description.is_empty()
            || component.description.len() > 64 * 1024
            || component.description.as_bytes().contains(&0)
        {
            return Err(ComponentAdministrationError::Corrupt {
                field: "component_grants",
            });
        }
        previous = Some(&component.name);
    }
    Ok(())
}

fn validate_decision(
    decision: &ComponentDecision,
    functions: &[String],
) -> Result<(), ComponentAdministrationError> {
    if !decision.is_consistent() {
        return Err(ComponentAdministrationError::Corrupt {
            field: "component_decision",
        });
    }
    if let Some(ComponentDecisionRefusal::FunctionNotGranted { function }) = &decision.refusal
        && functions.binary_search(function).is_err()
    {
        return Err(ComponentAdministrationError::Corrupt {
            field: "component_function",
        });
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

    type RuntimeDecisionCall = (ComponentRuntimeScope, String, bool, Vec<String>);

    #[derive(Default)]
    struct FakeComponents {
        syncs: Mutex<Vec<Vec<CompiledComponentManifestEntry>>>,
        runtime_lists: Mutex<Vec<(ComponentRuntimeScope, Vec<String>)>>,
        runtime_decisions: Mutex<Vec<RuntimeDecisionCall>>,
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

        async fn list_components_for_agent(
            &self,
            scope: &ComponentRuntimeScope,
            renderer_names: &[String],
        ) -> Result<GrantedCompiledComponents, ComponentAdministrationError> {
            self.runtime_lists
                .lock()
                .unwrap()
                .push((scope.clone(), renderer_names.to_vec()));
            Ok(GrantedCompiledComponents {
                components: vec![openbot_contracts::components::GrantedCompiledComponent {
                    name: SHOW_QUOTE_COMPONENT_NAME.to_owned(),
                    description: "published quote".to_owned(),
                }],
            })
        }

        async fn decide_component(
            &self,
            scope: &ComponentRuntimeScope,
            component_name: &str,
            build_has_renderer: bool,
            functions: &[String],
        ) -> Result<ComponentDecision, ComponentAdministrationError> {
            self.runtime_decisions.lock().unwrap().push((
                scope.clone(),
                component_name.to_owned(),
                build_has_renderer,
                functions.to_vec(),
            ));
            if !build_has_renderer {
                return Ok(ComponentDecision::refused(
                    ComponentDecisionRefusal::UnknownComponent,
                ));
            }
            if let Some(function) = functions.iter().find(|name| name.as_str() == "missing") {
                return Ok(ComponentDecision::refused(
                    ComponentDecisionRefusal::FunctionNotGranted {
                        function: function.clone(),
                    },
                ));
            }
            Ok(ComponentDecision::allowed())
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
        let expected = manifest
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>();
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
            expected
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

    #[tokio::test]
    async fn runtime_scope_manifest_and_function_set_are_authoritative_and_bounded() {
        let port = FakeComponents::default();
        let grants = list_components_for_agent(&port, &auth(), BotId::new("agent-one"))
            .await
            .unwrap();
        assert_eq!(grants.components[0].name, SHOW_QUOTE_COMPONENT_NAME);
        {
            let lists = port.runtime_lists.lock().unwrap();
            assert_eq!(lists.len(), 1);
            assert_eq!(lists[0].0.actor.as_str(), "actor");
            assert_eq!(lists[0].0.tenant.as_str(), "tenant");
            assert!(!lists[0].0.admin);
            assert_eq!(lists[0].1.len(), compiled_component_manifest().len());
        }

        let allowed = decide_component(
            &port,
            &auth(),
            SHOW_QUOTE_COMPONENT_NAME.to_owned(),
            ComponentDecisionRequest {
                agent_id: BotId::new("agent-one"),
                functions: vec!["readZ".to_owned(), "readA".to_owned(), "readA".to_owned()],
            },
        )
        .await
        .unwrap();
        assert_eq!(allowed, ComponentDecision::allowed());
        let unknown = decide_component(
            &port,
            &auth(),
            "showStale".to_owned(),
            ComponentDecisionRequest {
                agent_id: BotId::new("agent-one"),
                functions: Vec::new(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            unknown,
            ComponentDecision::refused(ComponentDecisionRefusal::UnknownComponent)
        );
        let decisions = port.runtime_decisions.lock().unwrap();
        assert_eq!(decisions[0].3, ["readA", "readZ"]);
        assert!(decisions[0].2);
        assert!(!decisions[1].2);
    }

    #[tokio::test]
    async fn malformed_runtime_identifiers_never_reach_authority_port() {
        let port = FakeComponents::default();
        assert!(
            list_components_for_agent(&port, &auth(), BotId::new("\n"))
                .await
                .is_err()
        );
        assert!(
            decide_component(
                &port,
                &auth(),
                "bad/name".to_owned(),
                ComponentDecisionRequest {
                    agent_id: BotId::new("agent-one"),
                    functions: Vec::new(),
                },
            )
            .await
            .is_err()
        );
        assert!(port.runtime_lists.lock().unwrap().is_empty());
        assert!(port.runtime_decisions.lock().unwrap().is_empty());
    }
}
