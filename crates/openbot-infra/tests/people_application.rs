//! W-3 people/auth application adapter 的 PostgreSQL 17 真库矩阵。

mod harness;

use harness::{admin_config, with_temp_database};
use openbot_application::{PeopleAdministration, PeoplePageRequest, PeoplePortError};
use openbot_contracts::auth::Role;
use openbot_contracts::error::IdentityConflictReason;
use openbot_contracts::ids::ActorId;
use openbot_domain::identity::roles::AdminFloor;
use serde_json::json;

use openbot_infra::db::{baseline, native, pool};
use openbot_infra::repo::people_admin::PostgresPeopleAdministration;

const AUDIT_KEY: &[u8] = b"people-application-postgres17-test-key";

async fn provision(pool: &deadpool_postgres::Pool, people_sql: &str) -> Result<(), String> {
    let mut client = pool
        .get()
        .await
        .map_err(|error| format!("取连接失败：{error}"))?;
    baseline::apply(&client)
        .await
        .map_err(|error| format!("应用 baseline 失败：{error}"))?;
    native::apply(&mut client)
        .await
        .map_err(|error| format!("应用 native migration 失败：{error}"))?;
    client
        .batch_execute(people_sql)
        .await
        .map_err(|error| format!("灌入 people fixture 失败：{error}"))
}

fn adapter(
    pool: &deadpool_postgres::Pool,
    floor: Option<AdminFloor>,
) -> Result<PostgresPeopleAdministration, String> {
    PostgresPeopleAdministration::new(pool.clone(), floor, AUDIT_KEY)
        .map_err(|error| error.to_string())
}

async fn scalar_i64(pool: &deadpool_postgres::Pool, sql: &str) -> Result<i64, String> {
    pool.get()
        .await
        .map_err(|error| error.to_string())?
        .query_one(sql, &[])
        .await
        .map_err(|error| error.to_string())?
        .try_get(0)
        .map_err(|error| error.to_string())
}

#[tokio::test]
#[ignore = "需要真实 PostgreSQL：设 OPENBOT_TEST_DATABASE_URL 后加 --include-ignored 运行"]
async fn people_projection_search_and_cursor_match_the_fixed_upstream_shape() {
    let admin = admin_config("people_projection_search_and_cursor_match_the_fixed_upstream_shape");
    with_temp_database(&admin, "people_projection", |config| async move {
        let pool = pool::connect(&config)
            .await
            .map_err(|error| format!("连接临时库失败：{error}"))?;
        let outcome = async {
            provision(
                &pool,
                r#"
                INSERT INTO public.users(id,email,name) VALUES
                  ('admin-a','admin@example.com','Admin'),
                  ('alpha','alpha@example.com','100%_match'),
                  ('beta','beta@example.com','Beta'),
                  ('never','never@example.com','Never');
                INSERT INTO public.user_roles(user_id,role) VALUES
                  ('admin-a','admin'),('alpha','user'),('beta','user');
                INSERT INTO public.sessions(id,user_id,token,expires_at,created_at) VALUES
                  ('s-alpha','alpha','token-alpha','2099-01-01T00:00:00Z','2026-04-04T00:00:00Z'),
                  ('s-beta','beta','token-beta','2099-01-01T00:00:00Z','2026-03-03T00:00:00Z'),
                  ('s-admin','admin-a','token-admin','2099-01-01T00:00:00Z','2026-02-02T00:00:00Z');
                INSERT INTO public.accounts(id,account_id,provider_id,user_id) VALUES
                  ('acct-z','alpha-z','zeta','alpha'),
                  ('acct-a','alpha-a','alpha','alpha');
                "#,
            )
            .await?;
            let floor = AdminFloor::from_configured([" ADMIN@example.com "])
                .map_err(|error| error.to_string())?;
            let people = adapter(&pool, Some(floor))?;

            let current = people
                .current_user(&ActorId::new("admin-a"))
                .await
                .map_err(|error| error.to_string())?;
            if current.id.as_str() != "admin-a"
                || current.email != "admin@example.com"
                || current.role != Role::Admin
            {
                return Err(format!("/api/me 投影不符：{current:?}"));
            }

            let first = people
                .list_people(PeoplePageRequest {
                    search: None,
                    cursor: None,
                    limit: 2,
                })
                .await
                .map_err(|error| error.to_string())?;
            let first_ids: Vec<&str> = first
                .people
                .iter()
                .map(|person| person.id.as_str())
                .collect();
            if first_ids != ["alpha", "beta"] || first.next_cursor.is_none() {
                return Err(format!("people 第一页定序/游标不符：{first:?}"));
            }
            if first.people[0].providers != vec!["alpha".to_owned(), "zeta".to_owned()] {
                return Err(format!(
                    "provider 没有稳定排序：{:?}",
                    first.people[0].providers
                ));
            }

            let second = people
                .list_people(PeoplePageRequest {
                    search: None,
                    cursor: first.next_cursor,
                    limit: 2,
                })
                .await
                .map_err(|error| error.to_string())?;
            let second_ids: Vec<&str> = second
                .people
                .iter()
                .map(|person| person.id.as_str())
                .collect();
            if second_ids != ["admin-a", "never"] || second.next_cursor.is_some() {
                return Err(format!("people 第二页定序/终止游标不符：{second:?}"));
            }
            if !second.people[0].configured_admin || second.people[1].role != Role::User {
                return Err("configured floor 或缺角色降级投影与上游不符".to_owned());
            }

            let escaped = people
                .list_people(PeoplePageRequest {
                    search: Some("100%_".to_owned()),
                    cursor: None,
                    limit: 50,
                })
                .await
                .map_err(|error| error.to_string())?;
            if escaped.people.len() != 1 || escaped.people[0].id.as_str() != "alpha" {
                return Err(format!("LIKE wildcard 没有按字面转义：{escaped:?}"));
            }

            let malformed = people
                .list_people(PeoplePageRequest {
                    search: None,
                    cursor: Some("not-base64url-json".to_owned()),
                    limit: 2,
                })
                .await
                .map_err(|error| error.to_string())?;
            if malformed.people != first.people {
                return Err("坏 cursor 没有按固定上游语义回到第一页".to_owned());
            }
            Ok(())
        }
        .await;
        pool.close();
        outcome
    })
    .await;
}

#[tokio::test]
#[ignore = "需要真实 PostgreSQL：设 OPENBOT_TEST_DATABASE_URL 后加 --include-ignored 运行"]
async fn role_change_is_atomic_idempotent_audited_and_respects_floor_and_self() {
    let admin =
        admin_config("role_change_is_atomic_idempotent_audited_and_respects_floor_and_self");
    with_temp_database(&admin, "people_role", |config| async move {
        let pool = pool::connect(&config)
            .await
            .map_err(|error| format!("连接临时库失败：{error}"))?;
        let outcome = async {
            provision(
                &pool,
                r#"
                INSERT INTO public.users(id,email,name) VALUES
                  ('admin-a','admin-a@example.com','Admin A'),
                  ('admin-b','admin-b@example.com','Admin B'),
                  ('member','member@example.com','Member'),
                  ('floor-admin','floor@example.com','Floor');
                INSERT INTO public.user_roles(user_id,role) VALUES
                  ('admin-a','admin'),('admin-b','admin'),('member','user'),('floor-admin','admin');
                "#,
            )
            .await?;
            let floor = AdminFloor::from_configured(["floor@example.com"])
                .map_err(|error| error.to_string())?;
            let people = adapter(&pool, Some(floor))?;
            let actor = ActorId::new("admin-a");
            let member = ActorId::new("member");

            let promoted = people
                .change_role(&actor, &member, Role::Admin)
                .await
                .map_err(|error| error.to_string())?;
            if promoted.role != Role::Admin || promoted.id != member {
                return Err(format!("提升后的 person 不符：{promoted:?}"));
            }
            let unchanged = people
                .change_role(&actor, &member, Role::Admin)
                .await
                .map_err(|error| error.to_string())?;
            if unchanged != promoted {
                return Err("幂等角色请求改变了 person".to_owned());
            }
            let demoted = people
                .change_role(&actor, &member, Role::User)
                .await
                .map_err(|error| error.to_string())?;
            if demoted.role != Role::User {
                return Err("降权后仍是 admin".to_owned());
            }

            let self_error = people
                .change_role(&actor, &actor, Role::User)
                .await
                .expect_err("admin 不得自我降权");
            if self_error
                != (PeoplePortError::IdentityConflict {
                    reason: IdentityConflictReason::RoleSelfDemotion,
                })
            {
                return Err(format!("自我降权错误码不符：{self_error:?}"));
            }
            let floor_error = people
                .change_role(&actor, &ActorId::new("floor-admin"), Role::User)
                .await
                .expect_err("configured admin 不得降权");
            if floor_error
                != (PeoplePortError::IdentityConflict {
                    reason: IdentityConflictReason::RoleConfiguredAdmin,
                })
            {
                return Err(format!("floor 降权错误码不符：{floor_error:?}"));
            }

            if scalar_i64(
                &pool,
                "SELECT coalesce(auth_generation,0) FROM public.users WHERE id='member'",
            )
            .await?
                != 2
            {
                return Err("两次真实角色变化必须把 auth generation 从 NULL 推到 2".to_owned());
            }
            if scalar_i64(&pool, "SELECT count(*)::bigint FROM public.audit_events").await? != 2 {
                return Err("幂等请求或被拒请求写了额外 audit event".to_owned());
            }
            let client = pool.get().await.map_err(|error| error.to_string())?;
            let rows = client
                .query(
                    "SELECT event_type,target_id,payload FROM public.audit_events ORDER BY created_at,id",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?;
            let payloads: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| row.get::<_, serde_json::Value>("payload"))
                .collect();
            if rows.iter().any(|row| row.get::<_, &str>("event_type") != "person.role_changed")
                || rows.iter().any(|row| row.get::<_, &str>("target_id") != "member")
                || payloads != [
                    json!({"new_role":"admin","previous_role":"user"}),
                    json!({"new_role":"user","previous_role":"admin"}),
                ]
            {
                return Err(format!("角色 audit 投影不符：{payloads:?}"));
            }
            if scalar_i64(&pool, "SELECT count(*)::bigint FROM public.audit_checkpoints").await?
                != 1
            {
                return Err("首条 acting audit 必须同事务创建唯一 genesis checkpoint".to_owned());
            }
            Ok(())
        }
        .await;
        pool.close();
        outcome
    })
    .await;
}

#[tokio::test]
#[ignore = "需要真实 PostgreSQL：设 OPENBOT_TEST_DATABASE_URL 后加 --include-ignored 运行"]
async fn concurrent_cross_demotion_leaves_exactly_one_effective_admin() {
    let admin = admin_config("concurrent_cross_demotion_leaves_exactly_one_effective_admin");
    with_temp_database(&admin, "people_concurrent", |config| async move {
        let pool = pool::connect(&config)
            .await
            .map_err(|error| format!("连接临时库失败：{error}"))?;
        let outcome = async {
            provision(
                &pool,
                r#"
                INSERT INTO public.users(id,email) VALUES
                  ('admin-a','admin-a@example.com'),('admin-b','admin-b@example.com');
                INSERT INTO public.user_roles(user_id,role) VALUES
                  ('admin-a','admin'),('admin-b','admin');
                "#,
            )
            .await?;
            let people = adapter(&pool, None)?;
            let admin_a = ActorId::new("admin-a");
            let admin_b = ActorId::new("admin-b");
            let left = people.change_role(&admin_a, &admin_b, Role::User);
            let right = people.change_role(&admin_b, &admin_a, Role::User);
            let (left, right) = tokio::join!(left, right);
            let successes = usize::from(left.is_ok()) + usize::from(right.is_ok());
            let failures = [&left, &right]
                .into_iter()
                .filter(|result| {
                    matches!(
                        result,
                        Err(PeoplePortError::IdentityConflict {
                            reason: IdentityConflictReason::RoleLastAdmin
                        })
                    )
                })
                .count();
            if successes != 1 || failures != 1 {
                return Err(format!("并发互降结果不符：left={left:?}, right={right:?}"));
            }
            if scalar_i64(
                &pool,
                "SELECT count(*)::bigint FROM public.user_roles ur \
                 LEFT JOIN public.revoked_access ra ON ra.email=(SELECT lower(email) FROM public.users WHERE id=ur.user_id) \
                 WHERE ur.role='admin' AND ra.email IS NULL",
            )
            .await?
                != 1
            {
                return Err("并发互降后有效管理员不是恰好 1".to_owned());
            }
            if scalar_i64(
                &pool,
                "SELECT coalesce(sum(coalesce(auth_generation,0)),0)::bigint FROM public.users",
            )
            .await?
                != 1
                || scalar_i64(&pool, "SELECT count(*)::bigint FROM public.audit_events").await?
                    != 1
            {
                return Err("失败事务留下 generation/audit 副作用".to_owned());
            }
            Ok(())
        }
        .await;
        pool.close();
        outcome
    })
    .await;
}

#[tokio::test]
#[ignore = "需要真实 PostgreSQL：设 OPENBOT_TEST_DATABASE_URL 后加 --include-ignored 运行"]
async fn revoke_and_restore_are_session_generation_and_audit_atomic() {
    let admin = admin_config("revoke_and_restore_are_session_generation_and_audit_atomic");
    with_temp_database(&admin, "people_access", |config| async move {
        let pool = pool::connect(&config)
            .await
            .map_err(|error| format!("连接临时库失败：{error}"))?;
        let outcome = async {
            provision(
                &pool,
                r#"
                INSERT INTO public.users(id,email) VALUES
                  ('admin-a','admin-a@example.com'),('member','MiXeD@example.com');
                INSERT INTO public.user_roles(user_id,role) VALUES
                  ('admin-a','admin'),('member','user');
                INSERT INTO public.sessions(id,user_id,token,expires_at) VALUES
                  ('member-session','member','member-token','2099-01-01T00:00:00Z');
                "#,
            )
            .await?;
            let people = adapter(&pool, None)?;
            let actor = ActorId::new("admin-a");
            let subject = ActorId::new("member");
            let revoked = people
                .change_access(&actor, &subject, true)
                .await
                .map_err(|error| error.to_string())?;
            let revoked_wire =
                serde_json::to_value(&revoked).map_err(|error| error.to_string())?;
            if !revoked.revoked || revoked_wire.get("authGeneration").is_some() {
                return Err("撤权后的公开 person 状态不符".to_owned());
            }
            if scalar_i64(&pool, "SELECT count(*)::bigint FROM public.sessions WHERE user_id='member'").await? != 0
                || scalar_i64(&pool, "SELECT count(*)::bigint FROM public.revoked_access WHERE email='mixed@example.com'").await? != 1
                || scalar_i64(&pool, "SELECT auth_generation FROM public.users WHERE id='member'").await? != 1
            {
                return Err("deny/session/generation 没有同批落地".to_owned());
            }
            let unchanged = people
                .change_access(&actor, &subject, true)
                .await
                .map_err(|error| error.to_string())?;
            if unchanged != revoked
                || scalar_i64(&pool, "SELECT count(*)::bigint FROM public.audit_events").await? != 1
            {
                return Err("重复撤权不应推进 generation 或 audit".to_owned());
            }
            let restored = people
                .change_access(&actor, &subject, false)
                .await
                .map_err(|error| error.to_string())?;
            if restored.revoked
                || scalar_i64(&pool, "SELECT count(*)::bigint FROM public.revoked_access").await? != 0
                || scalar_i64(&pool, "SELECT auth_generation FROM public.users WHERE id='member'").await? != 1
            {
                return Err("恢复访问应只删 deny 行、不回退代际".to_owned());
            }
            let client = pool.get().await.map_err(|error| error.to_string())?;
            let rows = client
                .query(
                    "SELECT event_type,payload FROM public.audit_events ORDER BY created_at,id",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?;
            let projected: Vec<(String, serde_json::Value)> = rows
                .iter()
                .map(|row| (row.get("event_type"), row.get("payload")))
                .collect();
            if projected
                != [
                    ("person.access_revoked".to_owned(), json!({"access_revoked":true})),
                    ("person.access_restored".to_owned(), json!({"access_revoked":false})),
                ]
            {
                return Err(format!("访问 audit 投影不符：{projected:?}"));
            }
            Ok(())
        }
        .await;
        pool.close();
        outcome
    })
    .await;
}

#[tokio::test]
#[ignore = "需要真实 PostgreSQL：设 OPENBOT_TEST_DATABASE_URL 后加 --include-ignored 运行"]
async fn audit_invariant_failure_rolls_back_the_people_business_write() {
    let admin = admin_config("audit_invariant_failure_rolls_back_the_people_business_write");
    with_temp_database(&admin, "people_audit_rollback", |config| async move {
        let pool = pool::connect(&config)
            .await
            .map_err(|error| format!("连接临时库失败：{error}"))?;
        let outcome = async {
            provision(
                &pool,
                r#"
                INSERT INTO public.users(id,email) VALUES
                  ('admin-a','admin-a@example.com'),('member','member@example.com');
                INSERT INTO public.user_roles(user_id,role) VALUES
                  ('admin-a','admin'),('member','user');
                INSERT INTO public.audit_events
                  (id,actor_user_id,event_type,target_type,target_id,payload,created_at,prev_hash,row_hash)
                VALUES
                  ('018f47d2-2c00-7a00-8000-000000000099',NULL,'legacy.linked','legacy',NULL,'{}',
                   '2026-01-01T00:00:00Z',NULL,repeat('0',64));
                "#,
            )
            .await?;
            let people = adapter(&pool, None)?;
            let error = people
                .change_role(
                    &ActorId::new("admin-a"),
                    &ActorId::new("member"),
                    Role::Admin,
                )
                .await
                .expect_err("链已有 linked row 却没有 genesis checkpoint 必须 fail-closed");
            if !matches!(error, PeoplePortError::Corrupt { field: "people" }) {
                return Err(format!("audit 不变量失败映射不符：{error:?}"));
            }
            let client = pool.get().await.map_err(|error| error.to_string())?;
            let row = client
                .query_one(
                    "SELECT string_agg(role::text,',' ORDER BY role::text),auth_generation \
                     FROM public.users u JOIN public.user_roles ur ON ur.user_id=u.id \
                     WHERE u.id='member' GROUP BY u.auth_generation",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?;
            let roles: String = row.get(0);
            let generation: Option<i64> = row.get(1);
            if roles != "user" || generation.is_some() {
                return Err(format!("audit 失败却提交了业务写：roles={roles}, generation={generation:?}"));
            }
            if scalar_i64(&pool, "SELECT count(*)::bigint FROM public.audit_events").await? != 1
                || scalar_i64(&pool, "SELECT count(*)::bigint FROM public.audit_checkpoints").await? != 0
            {
                return Err("audit 失败事务留下了新 event/checkpoint".to_owned());
            }
            Ok(())
        }
        .await;
        pool.close();
        outcome
    })
    .await;
}
