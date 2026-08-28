//! Pinned RMCP client over SafeDialer against a real pinned RMCP Streamable HTTP server.

mod harness {
    include!("../../../test-support/postgres_harness.rs");
}

use std::sync::{Arc, Mutex};

use openbot_agent::{AgentToolInvoker, AuthorizedAgentToolGateway};
use openbot_application::{
    AgentContextSource, ApplicationService, OpenBotApplication, RunExecutionLease,
    ToolApprovalAdministration, ToolCallSequence,
};
use openbot_contracts::auth::{AuthContextBuilder, AuthGeneration, Role};
use openbot_contracts::ids::{ActorId, BotId, DeploymentId, RunId, TenantId, ThreadId};
use openbot_contracts::tool::ToolApprovalDecision;
use openbot_domain::policy::{ActionPolicy, PolicyMode};
use openbot_domain::thread::FencingToken;
use openbot_infra::agent_tools::{
    PostgresAgentAuthorizationSource, PostgresAgentToolSequence, PostgresBuiltInToolControlPlane,
};
use openbot_infra::db::{baseline, native, pool};
use openbot_infra::mcp::{MAX_MCP_RESULT_CHARS, McpClientError, SafeRmcpClient, normalize_result};
use openbot_infra::mcp_catalog::PostgresMcpCatalog;
use openbot_infra::memory_admin::PostgresMemoryAdministration;
use openbot_infra::net::safe_http::{CidrAllowlist, EgressPolicy, SafeDialer, SchemePolicy};
use openbot_infra::policy::PolicyStore;
use openbot_infra::provider::context::PostgresAgentContextSource;
use openbot_infra::repo::ChannelRepo;
use openbot_infra::repo::tools::PostgresToolJournal;
use openbot_infra::tool_approval::PostgresToolApprovalCoordinator;
use rmcp::ErrorData as McpError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::{Value, json};

#[derive(Clone)]
struct RealMcpServer {
    received: Arc<Mutex<Vec<Value>>>,
    listed: Arc<Mutex<Vec<Tool>>>,
}

impl RealMcpServer {
    fn tools() -> Vec<Tool> {
        let schema = |required: bool| {
            let mut value = json!({
                "type":"object",
                "properties":{"query":{"type":"string"}},
            });
            if required {
                value["required"] = json!(["query"]);
            }
            value.as_object().cloned().unwrap()
        };
        vec![
            Tool::new(
                "search_issues",
                "Find issues matching a query.",
                schema(true),
            ),
            Tool::new_with_raw("bare", None, schema(false)),
            Tool::new("long_answer", "Returns far too much.", schema(false)),
            Tool::new("returns_an_image", "Not text.", schema(false)),
            Tool::new("mixed_content", "Mixed.", schema(false)),
            Tool::new("always_fails", "Tool-level failure.", schema(false)),
        ]
    }
}

impl ServerHandler for RealMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("openbot-test-rmcp", "3.1.4"))
            .with_protocol_version(ProtocolVersion::V_2026_07_28)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(
            self.listed.lock().unwrap().clone(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let result = match request.name.as_ref() {
            "search_issues" => {
                self.received.lock().unwrap().push(arguments.clone());
                let query = arguments
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                CallToolResult::success(vec![ContentBlock::text(format!(
                    "Found 2 issues for {query}"
                ))])
            }
            "bare" => CallToolResult::success(Vec::new()),
            "long_answer" => {
                self.received.lock().unwrap().push(arguments.clone());
                CallToolResult::success(vec![ContentBlock::text("x".repeat(25_000))])
            }
            "returns_an_image" => {
                CallToolResult::success(vec![ContentBlock::image("AA==", "image/png")])
            }
            "mixed_content" => CallToolResult::success(vec![
                ContentBlock::text("here is the chart"),
                ContentBlock::image("AA==", "image/png"),
            ]),
            "always_fails" => CallToolResult::error(vec![ContentBlock::text("vendor said no")]),
            _ => return Err(McpError::invalid_params("unknown tool", None)),
        };
        Ok(CallToolResponse::Complete(result))
    }
}

async fn spawn_server() -> Result<
    (
        String,
        Arc<Mutex<Vec<Value>>>,
        Arc<Mutex<Vec<Tool>>>,
        tokio::task::JoinHandle<()>,
    ),
    String,
> {
    let received = Arc::new(Mutex::new(Vec::new()));
    let listed = Arc::new(Mutex::new(RealMcpServer::tools()));
    let server = RealMcpServer {
        received: received.clone(),
        listed: listed.clone(),
    };
    let service = StreamableHttpService::new(
        move || Ok::<_, std::io::Error>(server.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok((format!("http://{address}/mcp"), received, listed, handle))
}

fn client() -> SafeRmcpClient {
    SafeRmcpClient::new(
        SafeDialer::new(EgressPolicy::new(
            CidrAllowlist::parse_exact(["127.0.0.1/32"]).unwrap(),
        )),
        SchemePolicy::HttpOrHttps,
        Some(std::time::Duration::from_secs(2)),
    )
}

#[tokio::test]
async fn real_rmcp_server_lists_calls_normalizes_and_closes_per_operation() {
    let (url, received, listed, server) = spawn_server().await.unwrap();
    let client = client();
    let tools = client.list_tools(&url, None).await.unwrap();
    assert_eq!(tools.len(), 6);
    let search = tools
        .iter()
        .find(|tool| tool.name == "search_issues")
        .unwrap();
    assert_eq!(search.description, "Find issues matching a query.");
    assert_eq!(search.input_schema["type"], "object");
    assert_eq!(
        tools
            .iter()
            .find(|tool| tool.name == "bare")
            .unwrap()
            .description,
        ""
    );

    let answer = client
        .call_tool(&url, None, "search_issues", json!({"query":"billing"}))
        .await
        .unwrap();
    assert_eq!(answer.text, "Found 2 issues for billing");
    assert!(!answer.is_error);
    assert!(!answer.truncated);
    assert_eq!(
        received.lock().unwrap().as_slice(),
        [json!({"query":"billing"})]
    );

    let image = client
        .call_tool(&url, None, "returns_an_image", json!({}))
        .await
        .unwrap();
    assert_eq!(image.text, "[image]");
    let mixed = client
        .call_tool(&url, None, "mixed_content", json!({}))
        .await
        .unwrap();
    assert_eq!(mixed.text, "here is the chart\n[image]");
    let failed = client
        .call_tool(&url, None, "always_fails", json!({}))
        .await
        .unwrap();
    assert!(failed.is_error);
    assert_eq!(failed.text, "vendor said no");
    let empty = client
        .call_tool(&url, None, "bare", json!({}))
        .await
        .unwrap();
    assert!(empty.text.to_ascii_lowercase().contains("nothing"));

    let long = client
        .call_tool(&url, None, "long_answer", json!({}))
        .await
        .unwrap();
    assert!(long.truncated);
    assert!(long.text.contains("25000 characters"));
    assert_eq!(
        long.text.chars().take(MAX_MCP_RESULT_CHARS).count(),
        MAX_MCP_RESULT_CHARS
    );
    assert_eq!(
        client
            .call_tool(&url, None, "no_such_tool", json!({}))
            .await,
        Err(McpClientError::ToolMissing)
    );
    let expected_schema_hash = openbot_domain::audit::hash::Sha256Digest::of(
        &serde_json::to_vec(&search.input_schema).unwrap(),
    );
    let before = received.lock().unwrap().len();
    listed
        .lock()
        .unwrap()
        .iter_mut()
        .find(|tool| tool.name == "search_issues")
        .unwrap()
        .input_schema = Arc::new(
        json!({"type":"object","properties":{"term":{"type":"string"}},"required":["term"]})
            .as_object()
            .cloned()
            .unwrap(),
    );
    assert_eq!(
        client
            .call_tool_bound(
                &url,
                None,
                "search_issues",
                expected_schema_hash,
                json!({"query":"must-not-send"}),
            )
            .await,
        Err(McpClientError::CatalogChanged)
    );
    assert_eq!(received.lock().unwrap().len(), before);
    server.abort();
}

#[tokio::test]
async fn unreachable_server_is_an_error_not_an_empty_catalog() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    assert!(
        client()
            .list_tools(&format!("http://{address}/mcp"), None)
            .await
            .is_err()
    );
}

#[test]
fn pure_result_projection_preserves_whitespace_next_to_real_content() {
    let result = normalize_result(
        &[ContentBlock::text(" "), ContentBlock::text("real")],
        false,
    )
    .unwrap();
    assert!(result.text.contains("real"));
    assert!(!result.truncated);
    let exact = "x".repeat(MAX_MCP_RESULT_CHARS);
    let result = normalize_result(&[ContentBlock::text(exact.clone())], false).unwrap();
    assert_eq!(result.text, exact);
    assert!(!result.truncated);
}

#[tokio::test]
#[ignore = "需要真实 PostgreSQL 与 loopback socket：设 OPENBOT_TEST_DATABASE_URL 后加 --include-ignored 运行"]
async fn catalog_generation_suspends_missing_and_changed_grants_without_auto_revival() {
    let admin = harness::admin_config(
        "catalog_generation_suspends_missing_and_changed_grants_without_auto_revival",
    );
    harness::with_temp_database(&admin, "mcpcatalog", |config| async move {
        let pool = pool::connect(&config)
            .await
            .map_err(|error| error.to_string())?;
        let result = async {
            let mut pg = pool.get().await.map_err(|error| error.to_string())?;
            baseline::apply(&pg)
                .await
                .map_err(|error| error.to_string())?;
            native::apply(&mut pg)
                .await
                .map_err(|error| error.to_string())?;
            let (url, _received, listed, server) = spawn_server().await?;
            let search = RealMcpServer::tools().remove(0);
            let schema = Value::Object((*search.input_schema).clone());
            let schema_hash = openbot_domain::audit::hash::Sha256Digest::of(
                &serde_json::to_vec(&schema).map_err(|error| error.to_string())?,
            )
            .to_hex();
            pg.execute(
                "INSERT INTO public.users(id,email) VALUES('actor-a','actor@example.test')",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            pg.execute(
                "INSERT INTO public.agents(id,name,type,configuration) VALUES
                   ('holder','Holder','remote_ag_ui','{}'),
                   ('stranger','Stranger','remote_ag_ui','{}')",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            pg.execute(
                "INSERT INTO public.mcp_servers(
                   id,title,vendor,url,provenance,catalog_generation,catalog_hash,
                   catalog_transport_fingerprint
                 ) VALUES('notes','Notes','notes',$1,'custom',0,repeat('0',64),repeat('0',64))",
                &[&url],
            )
            .await
            .map_err(|error| error.to_string())?;
            pg.execute(
                "INSERT INTO public.mcp_tools(
                   server_id,name,description,input_schema,schema_hash,effect,catalog_generation,
                   first_seen_at,last_seen_at,available
                 ) VALUES('notes','search_issues','Find issues matching a query.',$1,$2,'read',0,
                          clock_timestamp(),clock_timestamp(),true)",
                &[&schema, &schema_hash],
            )
            .await
            .map_err(|error| error.to_string())?;
            pg.execute(
                "INSERT INTO public.plugin_grants(kind,ref,agent_id)
                   VALUES('mcp','notes/search_issues','holder')",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            drop(pg);

            let catalog = PostgresMcpCatalog::new(pool.clone(), client(), vec![0x99; 32])
                .map_err(|error| error.to_string())?;
            let first = catalog
                .refresh("notes", None)
                .await
                .map_err(|error| error.to_string())?;
            if first.generation.get() != 1 || first.suspended_grants != 0 {
                return Err(format!("first catalog refresh drift: {first:?}"));
            }
            let holder = catalog
                .granted_tools(
                    &openbot_contracts::ids::BotId::new("holder"),
                    &openbot_contracts::ids::ActorId::new("actor-a"),
                )
                .await
                .map_err(|error| error.to_string())?;
            if holder.len() != 1
                || holder[0].model_name != "mcp__notes__search_issues"
                || holder[0].input_schema["required"] != json!(["query"])
                || holder[0].effect != openbot_domain::tool::metadata::Effect::Read
            {
                return Err(format!("reviewed grant projection drift: {holder:?}"));
            }
            if !catalog
                .granted_tools(
                    &openbot_contracts::ids::BotId::new("stranger"),
                    &openbot_contracts::ids::ActorId::new("actor-a"),
                )
                .await
                .map_err(|error| error.to_string())?
                .is_empty()
            {
                return Err("Bot without grant received MCP tool".to_owned());
            }
            let second = catalog
                .refresh("notes", None)
                .await
                .map_err(|error| error.to_string())?;
            if second.generation.get() != 2 || second.suspended_grants != 0 {
                return Err(format!("unchanged refresh drift: {second:?}"));
            }

            listed
                .lock()
                .unwrap()
                .retain(|tool| tool.name != "search_issues");
            let missing = catalog
                .refresh("notes", None)
                .await
                .map_err(|error| error.to_string())?;
            if missing.generation.get() != 3 || missing.suspended_grants != 1 {
                return Err(format!("missing suspension drift: {missing:?}"));
            }
            listed.lock().unwrap().push(search.clone());
            let reappeared = catalog
                .refresh("notes", None)
                .await
                .map_err(|error| error.to_string())?;
            if reappeared.generation.get() != 4 || reappeared.suspended_grants != 0 {
                return Err(format!("reappearance drift: {reappeared:?}"));
            }
            if !catalog
                .granted_tools(
                    &openbot_contracts::ids::BotId::new("holder"),
                    &openbot_contracts::ids::ActorId::new("actor-a"),
                )
                .await
                .map_err(|error| error.to_string())?
                .is_empty()
            {
                return Err("suspended grant auto-revived".to_owned());
            }

            let pg = pool.get().await.map_err(|error| error.to_string())?;
            pg.execute(
                "UPDATE public.plugin_grants g SET state='active',
                   catalog_generation=s.catalog_generation,schema_hash=t.schema_hash,effect=t.effect
                  FROM public.mcp_tools t JOIN public.mcp_servers s ON s.id=t.server_id
                 WHERE g.kind='mcp' AND g.ref='notes/search_issues' AND g.agent_id='holder'
                   AND t.server_id='notes' AND t.name='search_issues'",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            drop(pg);
            let mut changed = search.clone();
            changed.input_schema = Arc::new(
                json!({"type":"object","properties":{"term":{"type":"string"}},"required":["term"]})
                    .as_object()
                    .cloned()
                    .unwrap(),
            );
            *listed.lock().unwrap() = vec![changed];
            let changed = catalog
                .refresh("notes", None)
                .await
                .map_err(|error| error.to_string())?;
            if changed.generation.get() != 5 || changed.suspended_grants != 1 {
                return Err(format!("schema-change suspension drift: {changed:?}"));
            }
            let pg = pool.get().await.map_err(|error| error.to_string())?;
            pg.execute(
                "UPDATE public.plugin_grants g SET state='active',
                   catalog_generation=s.catalog_generation,schema_hash=t.schema_hash,effect=t.effect,
                   transport_fingerprint=s.catalog_transport_fingerprint
                  FROM public.mcp_tools t JOIN public.mcp_servers s ON s.id=t.server_id
                 WHERE g.kind='mcp' AND g.ref='notes/search_issues' AND g.agent_id='holder'
                   AND t.server_id='notes' AND t.name='search_issues'",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            pg.execute(
                "UPDATE public.mcp_servers SET provenance='custom-v2' WHERE id='notes'",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            drop(pg);
            if !matches!(
                catalog
                    .granted_tools(
                        &openbot_contracts::ids::BotId::new("holder"),
                        &openbot_contracts::ids::ActorId::new("actor-a"),
                    )
                    .await,
                Err(openbot_infra::mcp_catalog::McpCatalogError::Corrupt {
                    field: "catalog_transport_fingerprint"
                })
            ) {
                return Err("transport changed before refresh but old grant remained visible".to_owned());
            }
            let transport_changed = catalog
                .refresh("notes", None)
                .await
                .map_err(|error| error.to_string())?;
            if transport_changed.generation.get() != 6
                || transport_changed.suspended_grants != 1
            {
                return Err(format!(
                    "transport/provenance suspension drift: {transport_changed:?}"
                ));
            }
            let pg = pool.get().await.map_err(|error| error.to_string())?;
            let audit_count: i64 = pg
                .query_one(
                    "SELECT count(*)::bigint FROM public.audit_events
                      WHERE event_type='mcp.tool_suspended_missing'
                        AND target_id='notes/search_issues'",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?
                .try_get(0)
                .map_err(|error| error.to_string())?;
            let state: String = pg
                .query_one(
                    "SELECT state FROM public.plugin_grants
                      WHERE kind='mcp' AND ref='notes/search_issues' AND agent_id='holder'",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?
                .try_get(0)
                .map_err(|error| error.to_string())?;
            if audit_count != 3 || state != "suspended_missing" {
                return Err(format!(
                    "suspension audit/state drift: {audit_count}/{state}"
                ));
            }
            server.abort();
            Ok(())
        }
        .await;
        pool.close();
        result
    })
    .await;
}

#[tokio::test]
#[ignore = "需要真实 PostgreSQL 与 loopback socket：设 OPENBOT_TEST_DATABASE_URL 后加 --include-ignored 运行"]
async fn server_side_tools_cover_no_grant_vendor_schema_real_rmcp_audit_and_policy_refusal() {
    let admin = harness::admin_config(
        "server_side_tools_cover_no_grant_vendor_schema_real_rmcp_audit_and_policy_refusal",
    );
    harness::with_temp_database(&admin, "mcptoolplane", |config| async move {
        let pool = pool::connect(&config)
            .await
            .map_err(|error| error.to_string())?;
        let result = async {
            let mut pg = pool.get().await.map_err(|error| error.to_string())?;
            baseline::apply(&pg)
                .await
                .map_err(|error| error.to_string())?;
            native::apply(&mut pg)
                .await
                .map_err(|error| error.to_string())?;
            let (url, received, _listed, server) = spawn_server().await?;
            pg.execute(
                "INSERT INTO public.users(id,email,auth_generation)
                   VALUES('actor-mcp','actor-mcp@example.test',0)",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            pg.execute(
                "INSERT INTO public.user_roles(user_id,role) VALUES('actor-mcp','user')",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            pg.batch_execute(
                "INSERT INTO public.deployment_packages(tenant_id,source_path,checksum)
                   VALUES('tenant-mcp','/fixture',repeat('a',64));
                 INSERT INTO public.agents(id,name,type,configuration,package_id)
                   SELECT 'holder-mcp','Holder MCP','built_in',
                          '{\"systemPrompt\":\"Use granted tools.\",\"providerSource\":\"package\"}',id
                     FROM public.deployment_packages WHERE tenant_id='tenant-mcp';
                 INSERT INTO public.agents(id,name,type,configuration,package_id)
                   SELECT 'stranger-mcp','Stranger MCP','built_in',
                          '{\"systemPrompt\":\"No tools.\",\"providerSource\":\"package\"}',id
                     FROM public.deployment_packages WHERE tenant_id='tenant-mcp';
                 INSERT INTO public.agent_profiles(
                   agent_id,owner_user_id,title,role_description,avatar_seed,visibility,deleted_at
                 ) VALUES
                   ('holder-mcp',NULL,'Holder MCP','role','seed-a','public',NULL),
                   ('stranger-mcp',NULL,'Stranger MCP','role','seed-b','public',NULL);
                 INSERT INTO public.threads(
                   thread_id,tenant_id,deployment_id,created_by,anchor_kind,anchor_id,status,
                   next_message_seq,next_event_seq,created_at,updated_at
                 ) VALUES(
                   'thread-mcp','tenant-mcp','deployment-mcp','actor-mcp','direct_bot',
                   'holder-mcp','active',1,0,clock_timestamp(),clock_timestamp()
                 );
                 INSERT INTO public.thread_memberships(thread_id,user_id)
                   VALUES('thread-mcp','actor-mcp');
                 INSERT INTO public.runs(
                   run_id,thread_id,bot_id,actor_id,foreground,status,fencing_token,
                   next_event_seq,created_at,started_at
                 ) VALUES(
                   'run-mcp','thread-mcp','holder-mcp','actor-mcp',true,'running',1,
                   0,clock_timestamp(),clock_timestamp()
                 );
                 INSERT INTO public.thread_leases(
                   thread_id,owner_id,fencing_token,acquired_at,expires_at,updated_at
                 ) VALUES(
                   'thread-mcp','runtime-mcp',1,clock_timestamp(),
                   clock_timestamp()+interval '10 minutes',clock_timestamp()
                 );
                 INSERT INTO public.messages(
                   message_id,thread_id,seq,role,content,search_text,run_id,actor_id,created_at
                 ) VALUES(
                   'message-mcp','thread-mcp',0,'user','{\"text\":\"Search invoices.\"}',
                   'Search invoices.','run-mcp','actor-mcp',clock_timestamp()
                 );",
            )
            .await
            .map_err(|error| error.to_string())?;
            pg.execute(
                "INSERT INTO public.mcp_servers(id,title,vendor,url,provenance)
                   VALUES('notes','Notes','notes',$1,'custom')",
                &[&url],
            )
            .await
            .map_err(|error| error.to_string())?;
            pg.execute(
                "INSERT INTO public.plugin_grants(kind,ref,agent_id,granted_by)
                   VALUES('mcp','notes/search_issues','holder-mcp','admin-mcp')",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            drop(pg);

            let rmcp = client();
            let catalog = Arc::new(
                PostgresMcpCatalog::new(pool.clone(), rmcp.clone(), vec![0x71; 32])
                    .map_err(|error| error.to_string())?,
            );
            catalog
                .refresh("notes", None)
                .await
                .map_err(|error| error.to_string())?;
            let pg = pool.get().await.map_err(|error| error.to_string())?;
            pg.execute(
                "UPDATE public.mcp_tools SET effect='read'
                   WHERE server_id='notes' AND name='search_issues'",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            drop(pg);
            catalog
                .refresh("notes", None)
                .await
                .map_err(|error| error.to_string())?;
            let pg = pool.get().await.map_err(|error| error.to_string())?;
            pg.execute(
                "UPDATE public.plugin_grants g SET state='active',
                   catalog_generation=s.catalog_generation,schema_hash=t.schema_hash,
                   effect=t.effect,transport_fingerprint=s.catalog_transport_fingerprint,
                   updated_at=clock_timestamp()
                  FROM public.mcp_tools t JOIN public.mcp_servers s ON s.id=t.server_id
                 WHERE g.kind='mcp' AND g.ref='notes/search_issues'
                   AND g.agent_id='holder-mcp' AND t.server_id='notes'
                   AND t.name='search_issues'",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            drop(pg);

            let holder = BotId::new("holder-mcp");
            let stranger = BotId::new("stranger-mcp");
            let actor = ActorId::new("actor-mcp");
            if !catalog
                .granted_tools(&stranger, &actor)
                .await
                .map_err(|error| error.to_string())?
                .is_empty()
            {
                return Err("Bot with no grant was offered an MCP tool".to_owned());
            }
            let granted = catalog
                .granted_tools(&holder, &actor)
                .await
                .map_err(|error| error.to_string())?;
            if granted.len() != 1
                || !catalog
                    .validate_arguments(&granted[0], &json!({"query":"invoices"}))
                    .await
                    .map_err(|error| error.to_string())?
                || catalog
                    .validate_arguments(&granted[0], &json!({}))
                    .await
                    .map_err(|error| error.to_string())?
            {
                return Err(format!("vendor schema projection/validation drift: {granted:?}"));
            }

            let lease = RunExecutionLease::new(
                RunId::new("run-mcp"),
                ThreadId::new("thread-mcp"),
                holder.clone(),
                actor.clone(),
                FencingToken::new(1).map_err(|error| error.to_string())?,
                0,
            )
            .map_err(|error| error.to_string())?;
            let context = PostgresAgentContextSource::new(
                pool.clone(),
                DeploymentId::new("deployment-mcp"),
                TenantId::new("tenant-mcp"),
                Some(128),
            )
            .map_err(|error| error.to_string())?
            .with_mcp_catalog(catalog.clone());
            let request = context
                .load(&lease)
                .await
                .map_err(|error| error.to_string())?;
            if request.tools.len() != 1
                || request.tools[0].name != "mcp__notes__search_issues"
                || request.tools[0].description != "Find issues matching a query."
                || request.tools[0].input_schema != granted[0].input_schema
                || !request.messages[0]
                    .content
                    .contains("- notes: search_issues")
            {
                return Err(format!("provider tool projection drift: {:?}", request.tools));
            }

            let policy = PolicyStore::postgres(pool.clone(), None);
            policy.load().await.map_err(|error| error.to_string())?;
            policy
                .set(
                    ActionPolicy {
                        mode: PolicyMode::Enforce,
                        deny: Vec::new(),
                        allow: vec!["true".to_owned()],
                    },
                    Some("actor-mcp"),
                )
                .await
                .map_err(|error| error.to_string())?;
            let approvals = Arc::new(
                PostgresToolApprovalCoordinator::new(
                    pool.clone(),
                    DeploymentId::new("deployment-mcp"),
                    TenantId::new("tenant-mcp"),
                    vec![0x71; 32],
                )
                .map_err(|error| error.to_string())?,
            );
            let memory = PostgresMemoryAdministration::new(pool.clone());
            let control = PostgresBuiltInToolControlPlane::new(
                pool.clone(),
                DeploymentId::new("deployment-mcp"),
                TenantId::new("tenant-mcp"),
                policy.clone(),
                Arc::new(memory.clone()),
            )
            .with_mcp(catalog.clone(), rmcp)
            .with_tool_approvals(approvals.clone());
            let application: Arc<dyn ApplicationService> = Arc::new(
                OpenBotApplication::new(ChannelRepo::new(pool.clone()))
                    .with_policy(policy.clone())
                    .with_tools(
                        control,
                        PostgresToolJournal::new(pool.clone(), vec![0x71; 32])
                            .map_err(|error| error.to_string())?,
                    )
                    .with_memory(memory)
                    .with_tool_approvals(approvals.clone()),
            );
            let gateway = Arc::new(AuthorizedAgentToolGateway::with_sequence(
                application,
                Arc::new(PostgresAgentAuthorizationSource::new(
                    pool.clone(),
                    DeploymentId::new("deployment-mcp"),
                    TenantId::new("tenant-mcp"),
                    false,
                )),
                Arc::new(PostgresAgentToolSequence::new(pool.clone())),
            ));
            let reply = gateway
                .invoke(
                    &lease,
                    "provider-mcp-search-1",
                    "mcp__notes__search_issues",
                    json!({"query":"invoices"}),
                )
                .await
                .map_err(|error| error.to_string())?;
            if reply.error_code().is_some()
                || !reply.content().contains("Found 2 issues for invoices")
                || received.lock().unwrap().as_slice() != [json!({"query":"invoices"})]
            {
                return Err(format!("real governed MCP call drift: {reply:?}"));
            }
            let pg = pool.get().await.map_err(|error| error.to_string())?;
            let audit = pg
                .query_one(
                    "SELECT actor_user_id,payload->>'bot',
                            (SELECT next_tool_call_seq FROM public.runs WHERE run_id='run-mcp')
                       FROM public.audit_events
                      WHERE event_type='mcp.call_succeeded'
                        AND target_id='notes/search_issues'",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?;
            let audit_actor: Option<String> =
                audit.try_get(0).map_err(|error| error.to_string())?;
            let audit_bot: Option<String> =
                audit.try_get(1).map_err(|error| error.to_string())?;
            let next_sequence: Option<i64> =
                audit.try_get(2).map_err(|error| error.to_string())?;
            if audit_actor.as_deref() != Some("actor-mcp")
                || audit_bot.as_deref() != Some("holder-mcp")
                || next_sequence != Some(1)
            {
                return Err(format!(
                    "MCP audit/sequence drift: {audit_actor:?}/{audit_bot:?}/{next_sequence:?}"
                ));
            }
            drop(pg);

            let secret_refused = gateway
                .invoke(
                    &lease,
                    "provider-mcp-search-secret",
                    "mcp__notes__search_issues",
                    json!({"query":"OPENBOT_SECRET_CANARY-do-not-send"}),
                )
                .await
                .map_err(|error| error.to_string())?;
            if secret_refused.error_code() != Some("policy_refused")
                || !secret_refused.content().starts_with("Refused.")
                || received.lock().unwrap().len() != 1
            {
                return Err(format!(
                    "content-governance refusal drift: {secret_refused:?}"
                ));
            }
            let pg = pool.get().await.map_err(|error| error.to_string())?;
            let secret_audit: i64 = pg
                .query_one(
                    "SELECT count(*)::bigint FROM public.audit_events
                      WHERE event_type='mcp.call_rejected'
                        AND target_id='notes/search_issues'
                        AND payload->>'error_code'='content_secret_blocked'",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?
                .try_get(0)
                .map_err(|error| error.to_string())?;
            if secret_audit != 1 {
                return Err(format!("secret refusal audit drift: {secret_audit}"));
            }
            drop(pg);

            let pg = pool.get().await.map_err(|error| error.to_string())?;
            pg.execute(
                "UPDATE public.mcp_tools SET effect='read'
                   WHERE server_id='notes' AND name='always_fails'",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            pg.execute(
                "INSERT INTO public.plugin_grants(kind,ref,agent_id,granted_by)
                   VALUES('mcp','notes/always_fails','holder-mcp','admin-mcp')",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            drop(pg);
            catalog
                .refresh("notes", None)
                .await
                .map_err(|error| error.to_string())?;
            let vendor_failed = gateway
                .invoke(
                    &lease,
                    "provider-mcp-vendor-failed",
                    "mcp__notes__always_fails",
                    json!({}),
                )
                .await
                .map_err(|error| error.to_string())?;
            if vendor_failed.error_code() != Some("mcp_vendor_error")
                || !vendor_failed
                    .content()
                    .contains("The vendor reported an error: vendor said no")
                || received.lock().unwrap().len() != 1
            {
                return Err(format!("vendor failure projection drift: {vendor_failed:?}"));
            }
            let pg = pool.get().await.map_err(|error| error.to_string())?;
            let failed_audit: i64 = pg
                .query_one(
                    "SELECT count(*)::bigint FROM public.audit_events
                      WHERE event_type='mcp.call_failed'
                        AND target_id='notes/always_fails'
                        AND payload->>'bot'='holder-mcp'
                        AND payload->>'error_code'='mcp_vendor_error'",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?
                .try_get(0)
                .map_err(|error| error.to_string())?;
            if failed_audit != 1 {
                return Err(format!("vendor failure audit drift: {failed_audit}"));
            }
            drop(pg);

            let pg = pool.get().await.map_err(|error| error.to_string())?;
            pg.execute(
                "INSERT INTO public.plugin_grants(kind,ref,agent_id,granted_by)
                   VALUES('mcp','notes/long_answer','holder-mcp','admin-mcp')",
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
            drop(pg);
            catalog
                .refresh("notes", None)
                .await
                .map_err(|error| error.to_string())?;
            let acting_waiter = {
                let gateway = gateway.clone();
                let lease = lease.clone();
                tokio::spawn(async move {
                    gateway
                        .invoke(
                            &lease,
                            "provider-mcp-approved",
                            "mcp__notes__long_answer",
                            json!({}),
                        )
                        .await
                })
            };
            let approval_auth = AuthContextBuilder::from_verified_session(
                DeploymentId::new("deployment-mcp"),
                TenantId::new("tenant-mcp"),
                ActorId::new("actor-mcp"),
                AuthGeneration::new(0),
                false,
            )
            .with_role(Role::User)
            .build();
            let pending = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    let page = approvals
                        .list_pending(&approval_auth)
                        .await
                        .map_err(|error| error.to_string())?;
                    if let Some(pending) = page.approvals.into_iter().next() {
                        return Ok::<_, String>(pending);
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .map_err(|_| "acting approval did not become pending".to_owned())??;
            if pending.tool_name != "mcp__notes__long_answer"
                || pending.target_id != "notes/long_answer"
                || pending.effect != openbot_contracts::tool::ToolApprovalEffect::Execute
                || pending.arguments_summary != json!({})
            {
                return Err(format!("acting approval presentation drift: {pending:?}"));
            }
            approvals
                .decide(
                    &approval_auth,
                    &pending.approval_id,
                    ToolApprovalDecision::Grant,
                )
                .await
                .map_err(|error| error.to_string())?;
            let approved = acting_waiter
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
            if approved.error_code().is_some()
                || !approved.content().contains("[truncated:")
                || received.lock().unwrap().len() != 2
            {
                return Err(format!("approved acting MCP call drift: {approved:?}"));
            }
            let pg = pool.get().await.map_err(|error| error.to_string())?;
            let approval_evidence = pg
                .query_one(
                    "SELECT
                       (SELECT count(*)::bigint FROM public.audit_events
                         WHERE event_type='tool.approval_requested'
                           AND target_id=$1),
                       (SELECT count(*)::bigint FROM public.audit_events
                         WHERE event_type='tool.approval_granted'
                           AND target_id=$1),
                       (SELECT count(*)::bigint FROM public.audit_events
                         WHERE event_type='mcp.call_succeeded'
                           AND target_id='notes/long_answer'),
                       (SELECT count(*)::bigint FROM public.tool_calls
                         WHERE run_id='run-mcp' AND tool_name='mcp__notes__long_answer'),
                       (SELECT approval_id FROM public.tool_calls
                         WHERE run_id='run-mcp' AND tool_name='mcp__notes__long_answer'),
                       (SELECT payload->>'approval_id' FROM public.audit_events
                         WHERE event_type='mcp.call_succeeded'
                           AND target_id='notes/long_answer')",
                    &[&pending.approval_id],
                )
                .await
                .map_err(|error| error.to_string())?;
            let requested_audit: i64 = approval_evidence
                .try_get(0)
                .map_err(|error| error.to_string())?;
            let granted_audit: i64 = approval_evidence
                .try_get(1)
                .map_err(|error| error.to_string())?;
            let succeeded_audit: i64 = approval_evidence
                .try_get(2)
                .map_err(|error| error.to_string())?;
            let acting_decisions: i64 = approval_evidence
                .try_get(3)
                .map_err(|error| error.to_string())?;
            let decision_approval: Option<String> = approval_evidence
                .try_get(4)
                .map_err(|error| error.to_string())?;
            let audit_approval: Option<String> = approval_evidence
                .try_get(5)
                .map_err(|error| error.to_string())?;
            if requested_audit != 1
                || granted_audit != 1
                || succeeded_audit != 1
                || acting_decisions != 1
                || decision_approval.as_deref() != Some(pending.approval_id.as_str())
                || audit_approval.as_deref() != Some(pending.approval_id.as_str())
            {
                return Err(format!(
                    "acting approval boundary drift: {requested_audit}/{granted_audit}/{succeeded_audit}/{acting_decisions}/{decision_approval:?}/{audit_approval:?}"
                ));
            }
            drop(pg);

            let denied_waiter = {
                let gateway = gateway.clone();
                let lease = lease.clone();
                tokio::spawn(async move {
                    gateway
                        .invoke(
                            &lease,
                            "provider-mcp-denied",
                            "mcp__notes__long_answer",
                            json!({}),
                        )
                        .await
                })
            };
            let denied_pending = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    let page = approvals
                        .list_pending(&approval_auth)
                        .await
                        .map_err(|error| error.to_string())?;
                    if let Some(pending) = page.approvals.into_iter().next() {
                        return Ok::<_, String>(pending);
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .map_err(|_| "denial approval did not become pending".to_owned())??;
            approvals
                .decide(
                    &approval_auth,
                    &denied_pending.approval_id,
                    ToolApprovalDecision::Deny,
                )
                .await
                .map_err(|error| error.to_string())?;
            let denied = denied_waiter
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
            if denied.error_code() != Some("policy_refused")
                || !denied.content().starts_with("Refused.")
                || received.lock().unwrap().len() != 2
            {
                return Err(format!("denied acting MCP call drift: {denied:?}"));
            }
            let pg = pool.get().await.map_err(|error| error.to_string())?;
            let denial_evidence = pg
                .query_one(
                    "SELECT
                       (SELECT count(*)::bigint FROM public.audit_events
                         WHERE event_type='tool.approval_denied' AND target_id=$1),
                       (SELECT count(*)::bigint FROM public.audit_events
                         WHERE event_type='mcp.call_rejected'
                           AND target_id='notes/long_answer'
                           AND payload->>'error_code'='approval_denied'),
                       (SELECT count(*)::bigint FROM public.tool_calls
                         WHERE run_id='run-mcp' AND tool_name='mcp__notes__long_answer')",
                    &[&denied_pending.approval_id],
                )
                .await
                .map_err(|error| error.to_string())?;
            let denied_audit: i64 = denial_evidence
                .try_get(0)
                .map_err(|error| error.to_string())?;
            let rejected_audit: i64 = denial_evidence
                .try_get(1)
                .map_err(|error| error.to_string())?;
            let acting_decisions: i64 = denial_evidence
                .try_get(2)
                .map_err(|error| error.to_string())?;
            if denied_audit != 1 || rejected_audit != 1 || acting_decisions != 1 {
                return Err(format!(
                    "acting denial boundary drift: {denied_audit}/{rejected_audit}/{acting_decisions}"
                ));
            }
            drop(pg);

            policy
                .set(
                    ActionPolicy {
                        mode: PolicyMode::Enforce,
                        deny: vec![r#"mcp.server == "notes""#.to_owned()],
                        allow: vec!["true".to_owned()],
                    },
                    Some("actor-mcp"),
                )
                .await
                .map_err(|error| error.to_string())?;
            let refused = gateway
                .invoke(
                    &lease,
                    "provider-mcp-policy-refused",
                    "mcp__notes__search_issues",
                    json!({"query":"must-not-send"}),
                )
                .await
                .map_err(|error| error.to_string())?;
            if refused.error_code() != Some("policy_refused")
                || !refused.content().starts_with("Refused.")
                || received.lock().unwrap().len() != 2
            {
                return Err(format!("policy refusal projection drift: {refused:?}"));
            }
            let pg = pool.get().await.map_err(|error| error.to_string())?;
            let rejected: i64 = pg
                .query_one(
                    "SELECT count(*)::bigint FROM public.audit_events
                      WHERE event_type='mcp.call_rejected'
                        AND target_id='notes/search_issues'
                        AND payload->>'bot'='holder-mcp'
                        AND payload->>'error_code'='policy_refused'",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?
                .try_get(0)
                .map_err(|error| error.to_string())?;
            let sequence: Option<i64> = pg
                .query_one(
                    "SELECT next_tool_call_seq FROM public.runs WHERE run_id='run-mcp'",
                    &[],
                )
                .await
                .map_err(|error| error.to_string())?
                .try_get(0)
                .map_err(|error| error.to_string())?;
            if rejected != 1 || sequence != Some(6) {
                return Err(format!(
                    "policy audit/durable sequence drift: {rejected}/{sequence:?}"
                ));
            }
            drop(pg);
            let first_replica = PostgresAgentToolSequence::new(pool.clone());
            let second_replica = PostgresAgentToolSequence::new(pool.clone());
            let sequence_run = RunId::new("run-mcp");
            let (left, right) = tokio::join!(
                first_replica.next(&sequence_run),
                second_replica.next(&sequence_run),
            );
            let mut allocated = [
                left.map_err(|error| error.to_string())?,
                right.map_err(|error| error.to_string())?,
            ];
            allocated.sort_unstable();
            let after_reconstruction = PostgresAgentToolSequence::new(pool.clone())
                .next(&sequence_run)
                .await
                .map_err(|error| error.to_string())?;
            if allocated != [6, 7] || after_reconstruction != 8 {
                return Err(format!(
                    "cross-replica sequence allocation drift: {allocated:?}/{after_reconstruction}"
                ));
            }
            server.abort();
            Ok(())
        }
        .await;
        pool.close();
        result
    })
    .await;
}
