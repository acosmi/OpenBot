//! Axum + production session + ApplicationService + PostgreSQL remote callback security slice.

mod harness {
    include!("../../../test-support/postgres_harness.rs");
}

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use harness::{admin_config, with_temp_database};
use http::{Method, Request, StatusCode};
use openbot_application::{ApplicationService, OpenBotApplication};
use openbot_contracts::ids::{ActorId, BotId, DeploymentId, RunId, TenantId};
use openbot_domain::identity::session::{
    SessionHashKey, SessionToken, SessionTokenHash, TrustedOrigins,
};
use openbot_domain::remote_callback::{
    RemoteRunAssertionSigner, RemoteRunScope, RemoteToolSet, callback_token_hash,
};
use openbot_infra::agent_callback::{
    PostgresAgentCallbackTokens, PostgresRemoteCallbackAuthenticator,
};
use openbot_infra::auth::config::default_session_lifetime;
use openbot_infra::db::{baseline, native, pool};
use openbot_infra::repo::ChannelRepo;
use openbot_server::{PostgresSessionAuthResolver, SensitiveWriteSecurity, ServerBuilder, router};
use time::{Duration, OffsetDateTime};
use tower::ServiceExt as _;

const RAW_SESSION: &str = "callback-http-session-token-with-enough-entropy";
const SESSION_KEY: &[u8] = b"callback-http-session-hash-key";
const AUDIT_KEY: &[u8] = b"callback-http-audit-checkpoint-key";
const ORIGIN: &str = "https://app.example.test";

async fn provision(pool: &deadpool_postgres::Pool) -> Result<(), String> {
    let mut client = pool.get().await.map_err(|error| error.to_string())?;
    baseline::apply(&client)
        .await
        .map_err(|error| error.to_string())?;
    native::apply(&mut client)
        .await
        .map_err(|error| error.to_string())?;
    let now = OffsetDateTime::now_utc();
    let token_hash = SessionTokenHash::compute(
        SessionToken::new(RAW_SESSION.as_bytes()),
        SessionHashKey::new(SESSION_KEY),
    )
    .to_column_value();
    client
        .execute(
            "INSERT INTO public.users(id,email,name,auth_generation) VALUES($1,$2,$3,0)",
            &[&"owner-a", &"owner@example.test", &"Owner"],
        )
        .await
        .map_err(|error| error.to_string())?;
    client
        .execute(
            "INSERT INTO public.user_roles(user_id,role) VALUES('owner-a','user')",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?;
    client
        .execute(
            "INSERT INTO public.sessions(
               id,user_id,token,expires_at,created_at,updated_at,auth_generation
             ) VALUES($1,'owner-a',$2,$3,$4,$4,0)",
            &[
                &"session-callback",
                &token_hash,
                &(now + Duration::hours(1)),
                &(now - Duration::minutes(1)),
            ],
        )
        .await
        .map_err(|error| error.to_string())?;
    client
        .batch_execute(
            "INSERT INTO public.agents(id,name,type,configuration,package_id) VALUES(
               'remote-owner','Owner Remote','remote_ag_ui',
               '{\"endpoint\":\"https://remote.invalid\"}',NULL
             );
             INSERT INTO public.agent_profiles(
               agent_id,owner_user_id,title,role_description,avatar_seed,visibility,deleted_at
             ) VALUES('remote-owner','owner-a','Owner Remote','role','seed','private',NULL);
             INSERT INTO public.threads(
               thread_id,tenant_id,deployment_id,created_by,anchor_kind,anchor_id,status,
               next_message_seq,next_event_seq,created_at,updated_at
             ) VALUES(
               'thread-callback','tenant-a','deployment-a','owner-a','direct_bot',
               'remote-owner','active',0,0,clock_timestamp(),clock_timestamp()
             );
             INSERT INTO public.thread_memberships(thread_id,user_id)
               VALUES('thread-callback','owner-a');
             INSERT INTO public.runs(
               run_id,thread_id,bot_id,actor_id,foreground,status,fencing_token,
               next_event_seq,created_at,started_at
             ) VALUES(
               'run-callback','thread-callback','remote-owner','owner-a',true,'running',1,
               0,clock_timestamp(),clock_timestamp()
             );
             INSERT INTO public.thread_leases(
               thread_id,owner_id,fencing_token,acquired_at,expires_at,updated_at
             ) VALUES(
               'thread-callback','runtime-a',1,clock_timestamp(),
               clock_timestamp()+interval '10 minutes',clock_timestamp()
             );",
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn send(
    app: axum::Router,
    method: Method,
    path: &str,
    origin: Option<&str>,
    agent_token: Option<&str>,
    body: Option<String>,
) -> Result<(StatusCode, Vec<u8>), String> {
    let mut request = Request::builder().method(method).uri(path).header(
        http::header::COOKIE,
        format!("openbot_session={RAW_SESSION}"),
    );
    if let Some(origin) = origin {
        request = request.header(http::header::ORIGIN, origin);
    }
    if let Some(token) = agent_token {
        request = request.header("x-openbot-agent-token", token);
    }
    let body = match body {
        Some(body) => {
            request = request.header(http::header::CONTENT_TYPE, "application/json");
            Body::from(body)
        }
        None => Body::empty(),
    };
    let response = app
        .oneshot(request.body(body).map_err(|error| error.to_string())?)
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .map_err(|error| error.to_string())?
        .to_vec();
    Ok((status, bytes))
}

#[tokio::test]
#[ignore = "需要真实 PostgreSQL：设 OPENBOT_TEST_DATABASE_URL 后加 --include-ignored 运行"]
async fn production_http_issues_hash_only_token_and_refuses_ungranted_callback() {
    let admin =
        admin_config("production_http_issues_hash_only_token_and_refuses_ungranted_callback");
    with_temp_database(&admin, "callbackhttp", |config| async move {
        let pool = pool::connect(&config)
            .await
            .map_err(|error| error.to_string())?;
        let result = async {
            provision(&pool).await?;
            let deployment = DeploymentId::new("deployment-a");
            let tenant = TenantId::new("tenant-a");
            let signer = Arc::new(
                RemoteRunAssertionSigner::new(b"callback-http-master".to_vec())
                    .map_err(|error| error.to_string())?,
            );
            let token_store = PostgresAgentCallbackTokens::new(
                pool.clone(),
                deployment.clone(),
                tenant.clone(),
                AUDIT_KEY.to_vec(),
            )
            .map_err(|error| error.to_string())?;
            let application: Arc<dyn ApplicationService> = Arc::new(
                OpenBotApplication::new(ChannelRepo::new(pool.clone()))
                    .with_agent_callback_tokens(token_store),
            );
            let resolver = PostgresSessionAuthResolver::new(
                pool.clone(),
                SESSION_KEY,
                default_session_lifetime(),
                deployment.clone(),
                tenant.clone(),
            )
            .map_err(|error| error.to_string())?;
            let authenticator = Arc::new(
                PostgresRemoteCallbackAuthenticator::new(
                    pool.clone(),
                    deployment.clone(),
                    tenant.clone(),
                    false,
                    signer.clone(),
                    AUDIT_KEY.to_vec(),
                )
                .map_err(|error| error.to_string())?,
            );
            let app = router(
                ServerBuilder::new(application, Arc::new(resolver))
                    .with_sensitive_write_security(SensitiveWriteSecurity::new(
                        default_session_lifetime(),
                        TrustedOrigins::from_configured([ORIGIN])
                            .map_err(|error| error.to_string())?,
                    ))
                    .with_remote_callback_authenticator(authenticator)
                    .build(),
            );

            let (status, _) = send(
                app.clone(),
                Method::POST,
                "/api/agents/remote-owner/callback-token",
                None,
                None,
                None,
            )
            .await?;
            if status != StatusCode::FORBIDDEN {
                return Err(format!("missing Origin status drift: {status}"));
            }

            let (status, body) = send(
                app.clone(),
                Method::POST,
                "/api/agents/remote-owner/callback-token",
                Some(ORIGIN),
                None,
                None,
            )
            .await?;
            if status != StatusCode::CREATED {
                return Err(format!("callback token issue status drift: {status}"));
            }
            let body: serde_json::Value =
                serde_json::from_slice(&body).map_err(|error| error.to_string())?;
            let token = body["token"]
                .as_str()
                .ok_or_else(|| "callback token response missing".to_owned())?
                .to_owned();
            let expected_hash = callback_token_hash(&token)
                .map_err(|error| error.to_string())?
                .to_hex();
            let client = pool.get().await.map_err(|error| error.to_string())?;
            let stored: Option<String> = client
                .query_one(
                    "SELECT callback_token_hash FROM public.agent_profiles \
                      WHERE agent_id='remote-owner'",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?
                .try_get(0)
                .map_err(|error| error.to_string())?;
            let now_millis: i64 = client
                .query_one(
                    "SELECT floor(extract(epoch FROM clock_timestamp())*1000)::bigint",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?
                .try_get(0)
                .map_err(|error| error.to_string())?;
            if stored.as_deref() != Some(expected_hash.as_str())
                || stored.as_deref() == Some(token.as_str())
            {
                return Err("HTTP issue did not store hash-only".to_owned());
            }
            drop(client);
            let assertion = signer
                .mint(
                    RemoteRunScope {
                        deployment,
                        tenant,
                        bot: BotId::new("remote-owner"),
                        actor: ActorId::new("owner-a"),
                        run: RunId::new("run-callback"),
                    },
                    &RemoteToolSet::empty(),
                    now_millis,
                )
                .map_err(|error| error.to_string())?;
            let callback_body = serde_json::json!({
                "name":"mcp__drive__search",
                "args":{},
                "run":assertion,
            })
            .to_string();
            let (status, _) = send(
                app.clone(),
                Method::POST,
                "/api/agent-tools/call",
                None,
                Some(&token),
                Some(callback_body.clone()),
            )
            .await?;
            if status != StatusCode::NOT_FOUND {
                return Err(format!("ungranted callback status drift: {status}"));
            }
            let unknown = openbot_domain::remote_callback::callback_token_from_entropy([0x44; 32]);
            let (status, _) = send(
                app.clone(),
                Method::POST,
                "/api/agent-tools/call",
                None,
                Some(&unknown),
                Some(callback_body),
            )
            .await?;
            if status != StatusCode::UNAUTHORIZED {
                return Err(format!("unknown callback status drift: {status}"));
            }

            let (status, body) = send(
                app,
                Method::DELETE,
                "/api/agents/remote-owner/callback-token",
                Some(ORIGIN),
                None,
                None,
            )
            .await?;
            if status != StatusCode::NO_CONTENT || !body.is_empty() {
                return Err(format!("callback revoke response drift: {status}/{body:?}"));
            }
            let client = pool.get().await.map_err(|error| error.to_string())?;
            let evidence = client
                .query_one(
                    "SELECT callback_token_hash IS NULL,callback_token_issued_at IS NULL,
                            (SELECT count(*)::bigint FROM public.audit_events
                              WHERE event_type='bot.callback_token_issued'
                                AND target_id='remote-owner'),
                            (SELECT count(*)::bigint FROM public.audit_events
                              WHERE event_type='bot.callback_token_revoked'
                                AND target_id='remote-owner'),
                            (SELECT count(*)::bigint FROM public.audit_events
                              WHERE event_type='mcp.callback_refused')
                       FROM public.agent_profiles WHERE agent_id='remote-owner'",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?;
            let hash_null: bool = evidence.try_get(0).map_err(|error| error.to_string())?;
            let issued_null: bool = evidence.try_get(1).map_err(|error| error.to_string())?;
            let issued: i64 = evidence.try_get(2).map_err(|error| error.to_string())?;
            let revoked: i64 = evidence.try_get(3).map_err(|error| error.to_string())?;
            let refused: i64 = evidence.try_get(4).map_err(|error| error.to_string())?;
            if !hash_null || !issued_null || issued != 1 || revoked != 1 || refused != 2 {
                return Err(format!(
                    "HTTP callback durable evidence drift: hash_null={hash_null} issued_null={issued_null} issued={issued} revoked={revoked} refused={refused}"
                ));
            }
            Ok(())
        }
        .await;
        pool.close();
        result
    })
    .await;
}
