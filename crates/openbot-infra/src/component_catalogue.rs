//! PostgreSQL compiled-component governance projection and additive build catalogue sync.

use async_trait::async_trait;
use deadpool_postgres::Pool;
use openbot_application::{
    ComponentAdministration, ComponentAdministrationError, validate_manifest_entries,
};
use openbot_contracts::auth::AuthContext;
use openbot_contracts::components::{
    CompiledComponentKind, CompiledComponentManifestEntry, ComponentCatalogueAdded,
    ComponentRecord, ComponentRecords,
};
use openbot_domain::audit::event::{AuditEvent, AuditEventType};
use openbot_domain::audit::payload::{AuditIdentifier, AuditLabel, AuditPayload};
use openbot_domain::vault::SecretBytes;
use tokio_postgres::Row;

use crate::repo::audit::{append_event_in_transaction, next_event_coordinates};

/// Production PostgreSQL adapter for compiled-component read/sync operations.
pub struct PostgresComponentAdministration {
    pool: Pool,
    checkpoint_key: SecretBytes,
}

impl PostgresComponentAdministration {
    /// Construct with the deployment's existing domain-separated audit checkpoint key.
    pub fn new(pool: Pool, checkpoint_key: Vec<u8>) -> Result<Self, ComponentAdministrationError> {
        if checkpoint_key.is_empty() {
            return Err(ComponentAdministrationError::Unavailable);
        }
        Ok(Self {
            pool,
            checkpoint_key: SecretBytes::new(checkpoint_key),
        })
    }
}

impl core::fmt::Debug for PostgresComponentAdministration {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PostgresComponentAdministration")
            .field("checkpoint_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ComponentAdministration for PostgresComponentAdministration {
    async fn list_components(
        &self,
        _auth: &AuthContext,
    ) -> Result<ComponentRecords, ComponentAdministrationError> {
        let client = self.pool.get().await.map_err(unavailable)?;
        let rows = client
            .query(
                "SELECT c.name,c.title,c.kind,c.draft_description,c.published_description,
                        c.published,c.published_at,c.updated_by,c.updated_at,
                        coalesce((SELECT array_agg(e.agent_id ORDER BY e.agent_id)
                                    FROM public.component_exclusions e
                                   WHERE e.component_name=c.name),ARRAY[]::text[]) AS withheld_from,
                        coalesce((SELECT array_agg(f.function_name ORDER BY f.function_name)
                                    FROM public.component_functions f
                                   WHERE f.component_name=c.name),ARRAY[]::text[]) AS functions
                   FROM public.components c
                  ORDER BY c.kind,c.title,c.name",
                &[],
            )
            .await
            .map_err(query_unavailable)?;
        let components = rows
            .iter()
            .map(decode_record)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ComponentRecords { components })
    }

    async fn sync_catalogue(
        &self,
        auth: &AuthContext,
        entries: &[CompiledComponentManifestEntry],
    ) -> Result<ComponentCatalogueAdded, ComponentAdministrationError> {
        validate_manifest_entries(entries)?;
        let mut client = self.pool.get().await.map_err(unavailable)?;
        let transaction = client.transaction().await.map_err(query_unavailable)?;
        let mut added = Vec::with_capacity(entries.len());
        for entry in entries {
            let inserted = transaction
                .query_opt(
                    "INSERT INTO public.components(
                       name,title,kind,draft_description,published_description,published,
                       published_at,updated_by,created_at,updated_at
                     ) VALUES($1,$2,$3,$4,$4,true,clock_timestamp(),'the build',
                              clock_timestamp(),clock_timestamp())
                     ON CONFLICT (name) DO NOTHING RETURNING name",
                    &[
                        &entry.name,
                        &entry.title,
                        &entry.kind.as_str(),
                        &entry.description,
                    ],
                )
                .await
                .map_err(query_unavailable)?;
            if inserted.is_none() {
                continue;
            }
            let target_id =
                AuditIdentifier::new(entry.name.clone()).map_err(|_| corrupt("component_name"))?;
            let (id, created_at) = next_event_coordinates(&transaction)
                .await
                .map_err(infra_unavailable)?;
            let event = AuditEvent {
                id,
                actor: Some(auth.actor().clone()),
                event_type: AuditEventType::parse("component.published")
                    .ok_or_else(|| corrupt("audit_event"))?,
                target_kind: AuditLabel::new("component"),
                target_id: Some(target_id),
                payload: AuditPayload::empty(),
                created_at,
            };
            append_event_in_transaction(&transaction, &event, self.checkpoint_key.expose())
                .await
                .map_err(infra_unavailable)?;
            added.push(entry.name.clone());
        }
        transaction.commit().await.map_err(|error| {
            tracing::error!(error = %error, "component catalogue commit result unknown");
            ComponentAdministrationError::CommitUnknown
        })?;
        Ok(ComponentCatalogueAdded { added })
    }
}

fn decode_record(row: &Row) -> Result<ComponentRecord, ComponentAdministrationError> {
    let name: String = row.try_get("name").map_err(|_| corrupt("name"))?;
    let title: String = row.try_get("title").map_err(|_| corrupt("title"))?;
    let kind: String = row.try_get("kind").map_err(|_| corrupt("kind"))?;
    let draft_description: String = row
        .try_get("draft_description")
        .map_err(|_| corrupt("draft_description"))?;
    let published_description: Option<String> = row
        .try_get("published_description")
        .map_err(|_| corrupt("published_description"))?;
    let published: bool = row.try_get("published").map_err(|_| corrupt("published"))?;
    let published_at: Option<time::OffsetDateTime> = row
        .try_get("published_at")
        .map_err(|_| corrupt("published_at"))?;
    let updated_by: Option<String> = row
        .try_get("updated_by")
        .map_err(|_| corrupt("updated_by"))?;
    let updated_at: time::OffsetDateTime = row
        .try_get("updated_at")
        .map_err(|_| corrupt("updated_at"))?;
    let withheld_from: Vec<String> = row
        .try_get("withheld_from")
        .map_err(|_| corrupt("withheld_from"))?;
    let functions: Vec<String> = row.try_get("functions").map_err(|_| corrupt("functions"))?;
    validate_name(&name)?;
    validate_text(&title, 512, "title")?;
    validate_description(&draft_description, "draft_description")?;
    if let Some(value) = published_description.as_deref() {
        validate_description(value, "published_description")?;
    }
    if published && (published_description.is_none() || published_at.is_none()) {
        return Err(corrupt("publication"));
    }
    if let Some(value) = updated_by.as_deref() {
        validate_text(value, 512, "updated_by")?;
    }
    validate_identifiers(&withheld_from, "withheld_from")?;
    validate_identifiers(&functions, "functions")?;
    let kind = match kind.as_str() {
        "chart" => CompiledComponentKind::Chart,
        "card" => CompiledComponentKind::Card,
        "decision" => CompiledComponentKind::Decision,
        _ => return Err(corrupt("kind")),
    };
    let has_unpublished_changes =
        draft_description != published_description.as_deref().unwrap_or("");
    Ok(ComponentRecord {
        name,
        title,
        kind,
        draft_description,
        published_description,
        published,
        published_at,
        updated_by,
        updated_at,
        has_unpublished_changes,
        withheld_from,
        functions,
    })
}

fn validate_identifiers(
    values: &[String],
    field: &'static str,
) -> Result<(), ComponentAdministrationError> {
    if values.len() > 1024 {
        return Err(corrupt(field));
    }
    for value in values {
        validate_text(value, 512, field)?;
    }
    Ok(())
}

fn validate_text(
    value: &str,
    max: usize,
    field: &'static str,
) -> Result<(), ComponentAdministrationError> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        Err(corrupt(field))
    } else {
        Ok(())
    }
}

fn validate_name(value: &str) -> Result<(), ComponentAdministrationError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Err(corrupt("name"))
    } else {
        Ok(())
    }
}

fn validate_description(
    value: &str,
    field: &'static str,
) -> Result<(), ComponentAdministrationError> {
    if value.is_empty() || value.len() > 64 * 1024 || value.as_bytes().contains(&0) {
        Err(corrupt(field))
    } else {
        Ok(())
    }
}

fn corrupt(field: &'static str) -> ComponentAdministrationError {
    ComponentAdministrationError::Corrupt { field }
}

fn unavailable(error: deadpool_postgres::PoolError) -> ComponentAdministrationError {
    tracing::warn!(error = %error, "component catalogue pool unavailable");
    ComponentAdministrationError::Unavailable
}

fn query_unavailable(error: tokio_postgres::Error) -> ComponentAdministrationError {
    tracing::warn!(error = %error, "component catalogue query unavailable");
    ComponentAdministrationError::Unavailable
}

fn infra_unavailable(error: crate::db::InfraError) -> ComponentAdministrationError {
    tracing::warn!(error = %error, "component catalogue audit unavailable");
    ComponentAdministrationError::Unavailable
}
