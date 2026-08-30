//! G3 thread/message/run/event/outbox 同事务 append 的 PostgreSQL 17 真库证据。

mod harness;

use harness::{admin_config, with_temp_database};
use openbot_application::{BeginThreadRunRequest, ThreadDirectory, ThreadDirectoryError};
use openbot_contracts::command::{BeginThreadRun, ThreadRunAnchor};
use openbot_contracts::ids::thread::ThreadIdentity;
use openbot_contracts::ids::{ActorId, BotId, DeploymentId, RunId, TenantId};
use openbot_infra::db::{baseline, native, pool};
use openbot_infra::thread_directory::PostgresThreadDirectory;
use time::Duration;

async fn provision(pool: &deadpool_postgres::Pool) -> Result<(), String> {
    let mut client = pool.get().await.map_err(|error| error.to_string())?;
    baseline::apply(&client)
        .await
        .map_err(|error| error.to_string())?;
    native::apply(&mut client)
        .await
        .map_err(|error| error.to_string())?;
    client
        .batch_execute(
            "INSERT INTO public.users(id,email) VALUES
               ('actor-a','a@example.test'),('actor-b','b@example.test'),
               ('actor-admin','admin@example.test');
             INSERT INTO public.user_roles(user_id,role) VALUES('actor-admin','admin');
             INSERT INTO public.agents(id,name,type,configuration)
               VALUES('bot-1','Bot 1','built_in','{}'::jsonb),
                     ('bot-private','Private Bot','built_in','{}'::jsonb);
             INSERT INTO public.agent_profiles(
               agent_id,owner_user_id,title,role_description,avatar_seed,visibility,deleted_at
             ) VALUES('bot-1',NULL,'Bot 1','test role','seed','public',NULL),
                     ('bot-private','actor-b','Private Bot','private role','private-seed','private',NULL);",
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn request(
    deployment: &DeploymentId,
    actor: &str,
    thread_entropy: u64,
    run_id: &str,
    message: &str,
) -> BeginThreadRunRequest {
    let mut entropy = [0_u8; 16];
    entropy[8..].copy_from_slice(&thread_entropy.to_be_bytes());
    BeginThreadRunRequest {
        deployment: deployment.clone(),
        tenant: TenantId::new("tenant-a"),
        actor: ActorId::new(actor),
        command: BeginThreadRun {
            thread_id: ThreadIdentity::new(deployment).mint_from_entropy(entropy),
            run_id: RunId::new(run_id),
            bot_id: BotId::new("bot-1"),
            anchor: ThreadRunAnchor::DirectBot,
            message: message.to_owned(),
        },
    }
}

async fn count(client: &tokio_postgres::Client, table: &str) -> Result<i64, String> {
    let sql = format!("SELECT count(*)::bigint FROM public.{table}");
    client
        .query_one(&sql, &[])
        .await
        .map_err(|error| error.to_string())?
        .try_get(0)
        .map_err(|error| error.to_string())
}

#[tokio::test]
#[ignore = "需要真实 PostgreSQL：设 OPENBOT_TEST_DATABASE_URL 后加 --include-ignored 运行"]
async fn begin_commits_every_surface_and_exact_replay_writes_nothing() {
    let admin = admin_config("begin_commits_every_surface_and_exact_replay_writes_nothing");
    with_temp_database(&admin, "threadbegin", |config| async move {
        let pool = pool::connect(&config)
            .await
            .map_err(|error| error.to_string())?;
        let result = async {
            provision(&pool).await?;
            let deployment = DeploymentId::new("dep-a");
            let directory = PostgresThreadDirectory::with_runtime(
                pool.clone(),
                config.clone(),
                "runtime-a".to_owned(),
                Duration::seconds(30),
            )
            .map_err(|error| error.to_string())?;
            let request = request(&deployment, "actor-a", 1, "run-1", "hello native thread");
            let first = directory
                .begin_thread_run(request.clone())
                .await
                .map_err(|error| error.to_string())?;
            if first.replayed || first.message_sequence != 0 || first.event_sequence != 0 {
                return Err(format!("首个 receipt 形状错误：{first:?}"));
            }

            let client = pool.get().await.map_err(|error| error.to_string())?;
            for (table, expected) in [
                ("threads", 1),
                ("thread_memberships", 1),
                ("thread_leases", 1),
                ("messages", 1),
                ("runs", 1),
                ("run_events", 1),
                ("outbox", 1),
            ] {
                let actual = count(&client, table).await?;
                if actual != expected {
                    return Err(format!("{table} 应为 {expected} 行，实际 {actual}"));
                }
            }
            let shape = client
                .query_one(
                    "SELECT t.next_message_seq,t.next_event_seq,r.status,r.started_at IS NOT NULL,
                            e.event_type,e.terminal,o.destination,o.delivery_class,
                            o.payload ? 'message' AS leaks_message
                     FROM public.threads t
                     JOIN public.runs r ON r.thread_id=t.thread_id
                     JOIN public.run_events e ON e.run_id=r.run_id
                     JOIN public.outbox o ON o.aggregate_id=t.thread_id",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?;
            let actual: (i64, i64, String, bool, String, bool, String, String, bool) = (
                shape.try_get(0).map_err(|error| error.to_string())?,
                shape.try_get(1).map_err(|error| error.to_string())?,
                shape.try_get(2).map_err(|error| error.to_string())?,
                shape.try_get(3).map_err(|error| error.to_string())?,
                shape.try_get(4).map_err(|error| error.to_string())?,
                shape.try_get(5).map_err(|error| error.to_string())?,
                shape.try_get(6).map_err(|error| error.to_string())?,
                shape.try_get(7).map_err(|error| error.to_string())?,
                shape.try_get(8).map_err(|error| error.to_string())?,
            );
            if actual
                != (
                    1,
                    1,
                    "running".to_owned(),
                    true,
                    "started".to_owned(),
                    false,
                    "agent_run_dispatch".to_owned(),
                    "internal".to_owned(),
                    false,
                )
            {
                return Err(format!("同事务终态错误：{actual:?}"));
            }
            drop(client);

            let replay = directory
                .begin_thread_run(request.clone())
                .await
                .map_err(|error| error.to_string())?;
            if !replay.replayed
                || replay.message_sequence != first.message_sequence
                || replay.event_sequence != first.event_sequence
            {
                return Err(format!("精确 replay 没返回原 receipt：{replay:?}"));
            }
            let client = pool.get().await.map_err(|error| error.to_string())?;
            for table in [
                "threads",
                "thread_memberships",
                "thread_leases",
                "messages",
                "runs",
                "run_events",
                "outbox",
            ] {
                if count(&client, table).await? != 1 {
                    return Err(format!("幂等 replay 改写了 {table}"));
                }
            }
            drop(client);

            let mut changed = request.clone();
            changed.command.message = "different payload".to_owned();
            if directory.begin_thread_run(changed).await
                != Err(ThreadDirectoryError::RequestConflict)
            {
                return Err("同 run_id 不同内容必须 request_conflict".to_owned());
            }
            let mut second = request.clone();
            second.command.run_id = RunId::new("run-2");
            if directory.begin_thread_run(second).await != Err(ThreadDirectoryError::LeaseConflict)
            {
                return Err("active foreground 必须 lease_conflict".to_owned());
            }
            let client = pool.get().await.map_err(|error| error.to_string())?;
            client
                .execute(
                    "UPDATE public.threads SET status='deleted',deleted_at=now(),updated_at=now() \
                     WHERE thread_id=$1",
                    &[&request.command.thread_id.as_str()],
                )
                .await
                .map_err(|error| error.to_string())?;
            drop(client);
            if directory.begin_thread_run(request).await != Err(ThreadDirectoryError::NotVisible) {
                return Err("deleted thread 的精确 replay 也不得伪装成重新开始".to_owned());
            }

            let mut refused = crate::request(
                &deployment,
                "actor-a",
                11,
                "run-private-refused",
                "must not run private",
            );
            refused.command.bot_id = BotId::new("bot-private");
            if directory.begin_thread_run(refused).await != Err(ThreadDirectoryError::NotVisible) {
                return Err("non-owner must not run a private Agent".to_owned());
            }
            let mut admin = crate::request(
                &deployment,
                "actor-admin",
                12,
                "run-private-admin",
                "admin may run private",
            );
            admin.command.bot_id = BotId::new("bot-private");
            directory
                .begin_thread_run(admin)
                .await
                .map_err(|error| format!("admin private Agent run failed: {error}"))?;
            Ok(())
        }
        .await;
        pool.close();
        result
    })
    .await;
}

#[tokio::test]
#[ignore = "需要真实 PostgreSQL：设 OPENBOT_TEST_DATABASE_URL 后加 --include-ignored 运行"]
async fn a_late_outbox_conflict_rolls_back_thread_message_run_event_and_lease() {
    let admin =
        admin_config("a_late_outbox_conflict_rolls_back_thread_message_run_event_and_lease");
    with_temp_database(&admin, "threadrollback", |config| async move {
        let pool = pool::connect(&config)
            .await
            .map_err(|error| error.to_string())?;
        let result = async {
            provision(&pool).await?;
            let client = pool.get().await.map_err(|error| error.to_string())?;
            client
                .execute(
                    "INSERT INTO public.outbox(
                       outbox_id,aggregate_kind,aggregate_id,seq,destination,delivery_class,payload
                     ) VALUES(
                       'run-rollback:agent_run_dispatch','test','preexisting',0,
                       'agent_run_dispatch','internal','{}'::jsonb
                     )",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?;
            drop(client);

            let deployment = DeploymentId::new("dep-a");
            let directory = PostgresThreadDirectory::with_runtime(
                pool.clone(),
                config.clone(),
                "runtime-a".to_owned(),
                Duration::seconds(30),
            )
            .map_err(|error| error.to_string())?;
            let error = directory
                .begin_thread_run(request(
                    &deployment,
                    "actor-a",
                    2,
                    "run-rollback",
                    "must rollback",
                ))
                .await;
            if error != Err(ThreadDirectoryError::RequestConflict) {
                return Err(format!(
                    "末段 outbox collision 应 request_conflict：{error:?}"
                ));
            }

            let client = pool.get().await.map_err(|error| error.to_string())?;
            for table in [
                "threads",
                "thread_memberships",
                "thread_leases",
                "messages",
                "runs",
                "run_events",
            ] {
                let actual = count(&client, table).await?;
                if actual != 0 {
                    return Err(format!("outbox 末段失败后 {table} 仍有 {actual} 行"));
                }
            }
            if count(&client, "outbox").await? != 1 {
                return Err("预置 outbox 之外不应留下新行".to_owned());
            }
            Ok(())
        }
        .await;
        pool.close();
        result
    })
    .await;
}

#[tokio::test]
#[ignore = "需要真实 PostgreSQL：设 OPENBOT_TEST_DATABASE_URL 后加 --include-ignored 运行"]
async fn existing_legacy_uuid_is_writable_but_a_new_foreign_uuid_is_rejected() {
    let admin = admin_config("existing_legacy_uuid_is_writable_but_a_new_foreign_uuid_is_rejected");
    with_temp_database(&admin, "threadlegacy", |config| async move {
        let pool = pool::connect(&config)
            .await
            .map_err(|error| error.to_string())?;
        let result = async {
            provision(&pool).await?;
            let client = pool.get().await.map_err(|error| error.to_string())?;
            client
                .batch_execute(
                    "INSERT INTO public.threads(
                       thread_id,tenant_id,deployment_id,created_by,anchor_kind,anchor_id
                     ) VALUES(
                       '550e8400-e29b-41d4-a716-446655440000','tenant-a','dep-a','actor-a',
                       'direct_bot','bot-1'
                     );
                     INSERT INTO public.thread_memberships(thread_id,user_id)
                       VALUES('550e8400-e29b-41d4-a716-446655440000','actor-a');",
                )
                .await
                .map_err(|error| error.to_string())?;
            drop(client);

            let deployment = DeploymentId::new("dep-a");
            let directory = PostgresThreadDirectory::with_runtime(
                pool.clone(),
                config.clone(),
                "runtime-a".to_owned(),
                Duration::seconds(30),
            )
            .map_err(|error| error.to_string())?;
            let mut legacy = request(&deployment, "actor-a", 3, "legacy-run", "legacy works");
            legacy.command.thread_id =
                openbot_contracts::ids::ThreadId::new("550e8400-e29b-41d4-a716-446655440000");
            if ThreadIdentity::new(&deployment).owns(&legacy.command.thread_id) {
                return Err("前提失效：UUIDv4 不应被 dep-a owns".to_owned());
            }
            directory
                .begin_thread_run(legacy)
                .await
                .map_err(|error| error.to_string())?;

            let foreign_deployment = DeploymentId::new("dep-foreign");
            let mut entropy = [0_u8; 16];
            entropy[15] = 9;
            let foreign = BeginThreadRunRequest {
                deployment: deployment.clone(),
                tenant: TenantId::new("tenant-a"),
                actor: ActorId::new("actor-a"),
                command: BeginThreadRun {
                    thread_id: ThreadIdentity::new(&foreign_deployment).mint_from_entropy(entropy),
                    run_id: RunId::new("foreign-run"),
                    bot_id: BotId::new("bot-1"),
                    anchor: ThreadRunAnchor::DirectBot,
                    message: "must not create".to_owned(),
                },
            };
            if directory.begin_thread_run(foreign).await
                != Err(ThreadDirectoryError::InvalidInput { field: "thread_id" })
            {
                return Err("不存在的 foreign UUID 不得在当前 deployment 创建".to_owned());
            }
            Ok(())
        }
        .await;
        pool.close();
        result
    })
    .await;
}
