//! PostgreSQL 17 evidence for component list and additive exact build-catalogue synchronization.

mod harness;

use harness::{admin_config, with_temp_database};
use openbot_application::{
    ComponentAdministration, ComponentAdministrationError, validate_manifest_entries,
};
use openbot_contracts::auth::{AuthContextBuilder, AuthGeneration, Role};
use openbot_contracts::components::{SHOW_QUOTE_COMPONENT_NAME, compiled_component_manifest};
use openbot_contracts::ids::{ActorId, DeploymentId, TenantId};
use openbot_infra::component_catalogue::PostgresComponentAdministration;
use openbot_infra::db::{baseline, native, pool};

#[tokio::test]
#[ignore = "需要真实 PostgreSQL：设 OPENBOT_TEST_DATABASE_URL 后加 --include-ignored 运行"]
async fn catalogue_sync_is_exact_additive_audited_and_list_is_closed() {
    let admin = admin_config("catalogue_sync_is_exact_additive_audited_and_list_is_closed");
    with_temp_database(&admin, "componentcatalogue", |config| async move {
        let pool = pool::connect(&config)
            .await
            .map_err(|error| error.to_string())?;
        let result = async {
            let mut client = pool.get().await.map_err(|error| error.to_string())?;
            baseline::apply(&client)
                .await
                .map_err(|error| error.to_string())?;
            native::apply(&mut client)
                .await
                .map_err(|error| error.to_string())?;
            client
                .batch_execute(
                    "INSERT INTO public.users(id,email,auth_generation)
                       VALUES('component-actor','component@example.test',0);
                     INSERT INTO public.components(
                       name,title,kind,draft_description,published_description,published,
                       published_at,updated_by,created_at,updated_at
                     ) VALUES
                       ('showLegacyWidget','Legacy widget','card','legacy draft','legacy published',
                        true,clock_timestamp(),'admin',clock_timestamp(),clock_timestamp()),
                       ('showFutureChart','Future chart','chart','future draft',NULL,false,NULL,'admin',
                        clock_timestamp(),clock_timestamp());
                     INSERT INTO public.agents(id,name,type,configuration) VALUES
                       ('agent-a','Agent A','built_in','{}'),
                       ('agent-z','Agent Z','built_in','{}');
                     INSERT INTO public.component_exclusions(component_name,agent_id)
                       VALUES('showLegacyWidget','agent-z'),('showLegacyWidget','agent-a');
                     INSERT INTO public.component_functions(component_name,function_name)
                       VALUES('showLegacyWidget','readZ'),('showLegacyWidget','readA');",
                )
                .await
                .map_err(|error| error.to_string())?;
            drop(client);

            let auth = AuthContextBuilder::from_verified_session(
                DeploymentId::new("component-deployment"),
                TenantId::new("component-tenant"),
                ActorId::new("component-actor"),
                AuthGeneration::new(0),
                false,
            )
            .with_roles([Role::User])
            .build();
            let catalogue = PostgresComponentAdministration::new(
                pool.clone(),
                b"component-catalogue-audit-key".to_vec(),
            )
            .map_err(|error| error.to_string())?;

            let before = catalogue
                .list_components(&auth)
                .await
                .map_err(|error| error.to_string())?;
            if before.components.len() != 2
                || before.components[0].name != "showLegacyWidget"
                || before.components[0].withheld_from != ["agent-a", "agent-z"]
                || before.components[0].functions != ["readA", "readZ"]
                || !before.components[1].has_unpublished_changes
            {
                return Err(format!("initial component projection drift: {before:?}"));
            }

            let client = pool.get().await.map_err(|error| error.to_string())?;
            client
                .batch_execute(
                    "CREATE FUNCTION public.reject_component_catalogue_audit() RETURNS trigger
                       LANGUAGE plpgsql AS $$ BEGIN
                         IF NEW.event_type='component.published' THEN
                           RAISE EXCEPTION 'forced component audit failure';
                         END IF;
                         RETURN NEW;
                       END $$;
                     CREATE TRIGGER reject_component_catalogue_audit
                       BEFORE INSERT ON public.audit_events FOR EACH ROW
                       EXECUTE FUNCTION public.reject_component_catalogue_audit();",
                )
                .await
                .map_err(|error| error.to_string())?;
            drop(client);
            if !matches!(
                catalogue
                    .sync_catalogue(&auth, &compiled_component_manifest())
                    .await,
                Err(ComponentAdministrationError::Unavailable)
            ) {
                return Err("forced audit failure did not fail catalogue sync".to_owned());
            }
            let client = pool.get().await.map_err(|error| error.to_string())?;
            let rolled_back: i64 = client
                .query_one(
                    "SELECT count(*)::bigint FROM public.components
                      WHERE name IN ('showChecklist','showMetrics','showNotice','showQuote','showRecord')",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?
                .try_get(0)
                .map_err(|error| error.to_string())?;
            if rolled_back != 0 {
                return Err("component insert survived failed audit transaction".to_owned());
            }
            client
                .batch_execute(
                    "DROP TRIGGER reject_component_catalogue_audit ON public.audit_events;
                     DROP FUNCTION public.reject_component_catalogue_audit();",
                )
                .await
                .map_err(|error| error.to_string())?;
            drop(client);

            let manifest = compiled_component_manifest();
            let expected_added = manifest
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>();
            let added = catalogue
                .sync_catalogue(&auth, &manifest)
                .await
                .map_err(|error| error.to_string())?;
            if added
                .added
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                != expected_added
            {
                return Err(format!("component first sync drift: {added:?}"));
            }
            let repeated = catalogue
                .sync_catalogue(&auth, &compiled_component_manifest())
                .await
                .map_err(|error| error.to_string())?;
            if !repeated.added.is_empty() {
                return Err(format!(
                    "component repeat was not additive no-op: {repeated:?}"
                ));
            }

            let mut tampered = compiled_component_manifest();
            tampered[0].title = "Browser-owned title".to_owned();
            if validate_manifest_entries(&tampered).is_ok()
                || !matches!(
                    catalogue.sync_catalogue(&auth, &tampered).await,
                    Err(ComponentAdministrationError::InvalidInput {
                        field: "component_identity"
                    })
                )
            {
                return Err("tampered manifest reached component persistence".to_owned());
            }

            let client = pool.get().await.map_err(|error| error.to_string())?;
            client
                .execute(
                    "UPDATE public.components SET published=false,draft_description='admin draft',
                       updated_by='admin' WHERE name='showQuote'",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?;
            drop(client);
            let no_overwrite = catalogue
                .sync_catalogue(&auth, &compiled_component_manifest())
                .await
                .map_err(|error| error.to_string())?;
            if !no_overwrite.added.is_empty() {
                return Err("existing governance row was treated as a new build row".to_owned());
            }
            let after = catalogue
                .list_components(&auth)
                .await
                .map_err(|error| error.to_string())?;
            let quote = after
                .components
                .iter()
                .find(|record| record.name == SHOW_QUOTE_COMPONENT_NAME)
                .ok_or_else(|| "showQuote missing after sync".to_owned())?;
            if quote.published
                || quote.draft_description != "admin draft"
                || quote.updated_by.as_deref() != Some("admin")
                || !quote.has_unpublished_changes
            {
                return Err(format!("catalogue overwrote governance: {quote:?}"));
            }
            let client = pool.get().await.map_err(|error| error.to_string())?;
            let audit_row = client
                .query_one(
                    "SELECT count(*)::bigint,
                            count(*) FILTER (WHERE target_id='showQuote')::bigint
                       FROM public.audit_events
                      WHERE event_type='component.published'
                        AND actor_user_id='component-actor'",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?;
            let audits: i64 = audit_row
                .try_get(0)
                .map_err(|error| error.to_string())?;
            let quote_audits: i64 = audit_row
                .try_get(1)
                .map_err(|error| error.to_string())?;
            if audits != 5 || quote_audits != 1 {
                return Err(format!(
                    "component catalogue audit count drift: total={audits} quote={quote_audits}"
                ));
            }
            client
                .execute(
                    "UPDATE public.components SET kind='unknown' WHERE name='showLegacyWidget'",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?;
            drop(client);
            if !matches!(
                catalogue.list_components(&auth).await,
                Err(ComponentAdministrationError::Corrupt { field: "kind" })
            ) {
                return Err("unknown durable component kind crossed the closed wire".to_owned());
            }
            Ok(())
        }
        .await;
        pool.close();
        result
    })
    .await;
}
