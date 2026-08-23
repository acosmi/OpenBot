//! [`OpenBotApplication`] —— [`ApplicationService`] 的具体实现，把端口与 use case 接起来。
//!
//! # tracing 从第一个垂直切片就生效（v3 §24 G1 判据第四条）
//!
//! 两个入口各挂一个 `#[tracing::instrument]` span，字段取 §16.4 的关联字段。三条纪律
//! 在这里是**代码形态**不是注释：
//!
//! 1. `skip_all` —— 参数一个都不自动记录。没有它，`#[instrument]` 会把 `auth` 与
//!    `command` 用 `Debug` 打出去，于是 `AuthContext` 的角色集合与 auth generation、
//!    以及不透明游标，全都进了日志。contracts 刻意不给 `AuthContext` 实现 `Serialize`，
//!    而 `Debug` 是那道防线上的缺口 —— `skip_all` 把它堵上。
//! 2. 身份只取**需要的 ID 字段**（deployment / tenant / actor），逐个用 `Display` 写入。
//! 3. 字段名全集登记在 [`APPLICATION_SPAN_FIELDS`]，每一项都必须有基数裁决
//!    （见 [`crate::service::TRACE_ONLY_SPAN_FIELDS`]）。

use core::time::Duration;

use async_trait::async_trait;
use openbot_contracts::auth::AuthContext;
use openbot_contracts::command::{AppCommand, AppReply, SubscriptionRequest};
use openbot_contracts::error::AppError;
use tracing::Span;

use crate::ports::{ChannelReader, NoPeopleAdministration, PeopleAdministration};
use crate::service::{AppEventStream, ApplicationService, command_kind, subscription_kind};
use crate::tool::{NoToolControlPlane, NoToolJournal, ToolControlPlane, ToolJournal, invoke_tool};
use crate::use_cases::{
    DEFAULT_HEARTBEAT_PERIOD, admin_status, change_person_access, change_person_role, current_user,
    health, health_stream, list_people, list_visible_channels,
};

/// [`ApplicationService`] 的生产实现。
///
/// 它对具体数据源一无所知：`R` 是任何满足 [`ChannelReader`] 的类型。`openbot-server` 与
/// `openbot-desktop` 各自注入 `openbot-infra` 的实现，测试注入内存 fake —— 三条路径穿的
/// 是同一份业务代码，这正是 §24 G1「ApplicationService 经 Axum/Tauri 结果一致」的前提。
pub struct OpenBotApplication<
    R,
    P = NoPeopleAdministration,
    C = NoToolControlPlane,
    J = NoToolJournal,
> {
    channels: R,
    people: P,
    tool_control: C,
    tool_journal: J,
    heartbeat_period: Duration,
}

impl<R> OpenBotApplication<R, NoPeopleAdministration, NoToolControlPlane, NoToolJournal> {
    /// 注入端口实现。
    pub fn new(channels: R) -> Self {
        Self {
            channels,
            people: NoPeopleAdministration,
            tool_control: NoToolControlPlane,
            tool_journal: NoToolJournal,
            heartbeat_period: DEFAULT_HEARTBEAT_PERIOD,
        }
    }
}

impl<R, P, C, J> OpenBotApplication<R, P, C, J> {
    /// 注入 people/auth 原子端口。
    #[must_use]
    pub fn with_people<Q>(self, people: Q) -> OpenBotApplication<R, Q, C, J> {
        OpenBotApplication {
            channels: self.channels,
            people,
            tool_control: self.tool_control,
            tool_journal: self.tool_journal,
            heartbeat_period: self.heartbeat_period,
        }
    }

    /// 注入 tool control plane 与 durable journal；二者分开，application 才能掌握固定顺序。
    #[must_use]
    pub fn with_tools<T, K>(self, control: T, journal: K) -> OpenBotApplication<R, P, T, K> {
        OpenBotApplication {
            channels: self.channels,
            people: self.people,
            tool_control: control,
            tool_journal: journal,
            heartbeat_period: self.heartbeat_period,
        }
    }

    /// 覆盖心跳间隔。
    ///
    /// 存在的理由只有一个：让测试不必与 30 秒的默认节拍赛跑。生产侧应当用默认值 ——
    /// 改它会改变客户端与中间设备看到的保活频率，属于产品决定。
    #[must_use]
    pub const fn with_heartbeat_period(mut self, period: Duration) -> Self {
        self.heartbeat_period = period;
        self
    }
}

impl<R, P, C, J> OpenBotApplication<R, P, C, J>
where
    R: ChannelReader,
    P: PeopleAdministration,
    C: ToolControlPlane,
    J: ToolJournal,
{
    /// 命令派发。**穷举 match 无通配** —— 新增 `AppCommand` 变体会在这里编译失败，
    /// 而不是落进一个 `_ => Err(unknown_method)` 分支。那个分支正是 §5.2 逐字禁止的
    /// 「自由 method string」在 Rust 侧的形态。
    async fn dispatch(
        &self,
        auth: &AuthContext,
        command: AppCommand,
    ) -> Result<AppReply, AppError> {
        match command {
            AppCommand::Health => Ok(AppReply::Health(health())),
            AppCommand::ListVisibleChannels { limit, cursor } => {
                let page =
                    list_visible_channels(&self.channels, auth, limit, cursor.as_deref()).await?;
                Ok(AppReply::Channels(page))
            }
            AppCommand::GetCurrentUser => Ok(AppReply::CurrentUser(
                current_user(&self.people, auth).await?,
            )),
            AppCommand::AdminStatus => Ok(AppReply::AdminStatus(admin_status(auth)?)),
            AppCommand::ListPeople {
                search,
                cursor,
                limit,
            } => Ok(AppReply::People(
                list_people(&self.people, auth, search, cursor, limit).await?,
            )),
            AppCommand::ChangePersonRole { user_id, role } => Ok(AppReply::Person(
                change_person_role(&self.people, auth, &user_id, role).await?,
            )),
            AppCommand::ChangePersonAccess { user_id, revoked } => Ok(AppReply::Person(
                change_person_access(&self.people, auth, &user_id, revoked).await?,
            )),
            AppCommand::InvokeTool(invocation) => Ok(AppReply::Tool(
                invoke_tool(&self.tool_control, &self.tool_journal, auth, invocation).await?,
            )),
        }
    }
}

#[async_trait]
impl<R, P, C, J> ApplicationService for OpenBotApplication<R, P, C, J>
where
    R: ChannelReader + 'static,
    P: PeopleAdministration + 'static,
    C: ToolControlPlane + 'static,
    J: ToolJournal + 'static,
{
    #[tracing::instrument(
        name = "application.execute",
        skip_all,
        fields(
            deployment_id = %auth.deployment(),
            tenant_id = %auth.tenant(),
            actor_id = %auth.actor(),
            operation = command_kind(&command),
            error.code = tracing::field::Empty,
        )
    )]
    async fn execute(&self, auth: AuthContext, command: AppCommand) -> Result<AppReply, AppError> {
        let result = self.dispatch(&auth, command).await;
        if let Err(error) = &result {
            // 只记**稳定码**，不记 `Display`：`AppError` 的 Display 会带上 policy rule id
            // 与 lease holder 这类上下文，那些属于受控诊断，不该由一条恒开的 span 字段
            // 无差别带出去。code 是 §15.3 定死的、与文案解耦的那一样东西。
            Span::current().record("error.code", error.code().as_str());
        }
        result
    }

    #[tracing::instrument(
        name = "application.subscribe",
        skip_all,
        fields(
            deployment_id = %auth.deployment(),
            tenant_id = %auth.tenant(),
            actor_id = %auth.actor(),
            operation = subscription_kind(&request),
            error.code = tracing::field::Empty,
        )
    )]
    // `auth` **是被用了的** —— 上面三个 span 字段就在读它（`subscribe_is_instrumented_too`
    // 断言了这三个值确实落地）。但 `#[async_trait]` 与 `#[instrument]` 叠在一起时，字段
    // 表达式落在展开后的另一个卫生上下文里，`unused_variables` 看不见它，于是误报。
    //
    // 用 `expect` 而不是 `allow`：`expect` 在这条 lint **不再触发**时会自己报 unfulfilled。
    // 也就是说，等哪天 `auth` 在方法体里有了真实用途、或者宏的展开方式变了，这行会主动
    // 提醒把它删掉 —— 一个会自己退休的抑制，而不是一条永久失效的静音。
    #[expect(
        unused_variables,
        reason = "auth 由 #[instrument] 的 span 字段消费；async-trait 展开后 lint 看不见"
    )]
    async fn subscribe(
        &self,
        auth: AuthContext,
        request: SubscriptionRequest,
    ) -> Result<AppEventStream, AppError> {
        // 穷举 match，理由同 `dispatch`。
        match request {
            SubscriptionRequest::Health => Ok(health_stream(self.heartbeat_period)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{
        FakeChannelReader, FakePeopleAdministration, SENTINEL_AUTH_GENERATION, auth_for,
        sample_person, summary_at,
    };
    use crate::service::{APPLICATION_SPAN_FIELDS, EXECUTE_SPAN_NAME, SUBSCRIBE_SPAN_NAME};
    use core::fmt;
    use core::future::Future;
    use openbot_contracts::auth::{AuthContext, Role};
    use openbot_contracts::command::AppEvent;
    use openbot_contracts::error::ErrorCode;
    use openbot_contracts::ids::{ActorId, BotId, DeploymentId, RunId, TenantId, ToolCallId};
    use openbot_contracts::tool::ToolInvocation;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::registry::Registry;

    // -----------------------------------------------------------------------
    // span 捕获层
    // -----------------------------------------------------------------------

    /// 捕获到的 span：名字 + 全部被记录的字段。
    #[derive(Clone, Debug, Default)]
    struct Captured {
        names: Vec<String>,
        fields: Vec<(String, String)>,
    }

    impl Captured {
        fn names_of_fields(&self) -> Vec<&str> {
            self.fields.iter().map(|(k, _)| k.as_str()).collect()
        }

        fn value_of(&self, key: &str) -> Option<&str> {
            self.fields
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        }
    }

    /// 把 span 字段收进一个 `Vec`。
    ///
    /// `record_str` 与 `record_debug` 都实现：`Display` 值（`%expr`）走 `record_debug`，
    /// `&'static str` 值走 `record_str`。只实现一个会漏掉另一半 —— 那会让"span 里没有
    /// 敏感字段"这条断言在"我根本没看见任何字段"的情况下也成立。
    struct Collector<'a>(&'a mut Vec<(String, String)>);

    impl Visit for Collector<'_> {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.push((field.name().to_owned(), value.to_owned()));
        }

        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.0.push((field.name().to_owned(), format!("{value:?}")));
        }
    }

    #[derive(Clone, Default)]
    struct CaptureLayer(Arc<Mutex<Captured>>);

    impl<S: tracing::Subscriber> Layer<S> for CaptureLayer {
        fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {
            let mut captured = self.0.lock().expect("捕获层的互斥锁不会中毒");
            captured.names.push(attrs.metadata().name().to_owned());
            let mut fields = core::mem::take(&mut captured.fields);
            attrs.record(&mut Collector(&mut fields));
            captured.fields = fields;
        }

        fn on_record(&self, _id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
            let mut captured = self.0.lock().expect("捕获层的互斥锁不会中毒");
            let mut fields = core::mem::take(&mut captured.fields);
            values.record(&mut Collector(&mut fields));
            captured.fields = fields;
        }
    }

    /// 在捕获层下跑一段 async 工作，返回工作结果与捕获到的 span。
    fn capture<F, T>(work: F) -> (T, Captured)
    where
        F: Future<Output = T>,
    {
        let layer = CaptureLayer::default();
        let sink = Arc::clone(&layer.0);
        let subscriber = Registry::default().with(layer);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("构建当前线程运行时");
        let out = tracing::subscriber::with_default(subscriber, || runtime.block_on(work));
        let captured = sink.lock().expect("捕获层的互斥锁不会中毒").clone();
        (out, captured)
    }

    fn app() -> OpenBotApplication<FakeChannelReader, FakePeopleAdministration> {
        OpenBotApplication::new(
            FakeChannelReader::empty()
                .with_visible("actor-1", vec![summary_at("c-1", "2026-08-22T04:00:00Z")]),
        )
        .with_people(FakePeopleAdministration::seeded([
            sample_person("actor-1", Role::Admin),
            sample_person("actor-2", Role::User),
        ]))
        .with_heartbeat_period(Duration::from_millis(1))
    }

    // -----------------------------------------------------------------------
    // 派发
    // -----------------------------------------------------------------------

    #[test]
    fn execute_dispatches_every_command_variant() {
        let service = app();
        let auth = auth_for("actor-1");

        let (health_reply, _) = capture(service.execute(auth.clone(), AppCommand::Health));
        assert_eq!(
            health_reply.expect("探活必须成功"),
            AppReply::Health(openbot_contracts::command::HealthReport { ok: true })
        );

        let (channels_reply, _) = capture(service.execute(
            auth.clone(),
            AppCommand::ListVisibleChannels {
                limit: None,
                cursor: None,
            },
        ));
        match channels_reply.expect("列表必须成功") {
            AppReply::Channels(page) => {
                assert_eq!(page.channels.len(), 1);
                assert!(page.next_cursor.is_none());
            }
            other => panic!("命令与应答必须一一对应，拿到 {other:?}"),
        }

        let (me, _) = capture(service.execute(auth.clone(), AppCommand::GetCurrentUser));
        assert!(matches!(me, Ok(AppReply::CurrentUser(_))));

        let (status, _) = capture(service.execute(auth.clone(), AppCommand::AdminStatus));
        assert!(matches!(status, Ok(AppReply::AdminStatus(_))));

        let (people, _) = capture(service.execute(
            auth.clone(),
            AppCommand::ListPeople {
                search: None,
                cursor: None,
                limit: None,
            },
        ));
        assert!(matches!(people, Ok(AppReply::People(_))));

        let (role, _) = capture(service.execute(
            auth.clone(),
            AppCommand::ChangePersonRole {
                user_id: ActorId::new("actor-2"),
                role: Role::Admin,
            },
        ));
        assert!(matches!(role, Ok(AppReply::Person(_))));

        let (access, _) = capture(service.execute(
            auth.clone(),
            AppCommand::ChangePersonAccess {
                user_id: ActorId::new("actor-2"),
                revoked: true,
            },
        ));
        assert!(matches!(access, Ok(AppReply::Person(_))));

        let (tool, _) = capture(service.execute(
            auth,
            AppCommand::InvokeTool(ToolInvocation {
                call_id: ToolCallId::new("call-1"),
                run_id: RunId::new("run-1"),
                bot_id: BotId::new("bot-1"),
                call_seq: 0,
                tool_name: "computer.write".to_owned(),
                arguments: json!({}),
            }),
        ));
        assert!(matches!(
            tool,
            Err(AppError::DependencyUnavailable {
                dependency: "tool_catalog"
            })
        ));
    }

    /// 订阅回来的流是**活的**：拿到就能取到第一拍。
    ///
    /// 订阅与轮询必须在**同一个运行时**里完成 —— `tokio::time::Interval` 把定时器注册在
    /// 创建它的那个运行时上，换一个运行时去 poll 会撞上
    /// "A Tokio 1.x context was found, but it is being shutdown"。这不是测试技巧，
    /// 而是这条流对宿主的真实要求，transport 侧同样适用。
    #[test]
    fn subscribe_returns_a_live_heartbeat_stream() {
        let service = app();
        let (first, captured) = capture(async {
            let mut stream = service
                .subscribe(auth_for("actor-1"), SubscriptionRequest::Health)
                .await
                .expect("订阅必须成功");
            core::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await
        });
        assert_eq!(first, Some(AppEvent::Heartbeat { seq: 0 }));
        assert_eq!(captured.names, vec![SUBSCRIBE_SPAN_NAME.to_owned()]);
    }

    // -----------------------------------------------------------------------
    // tracing
    // -----------------------------------------------------------------------

    /// 正向对照（同时是 §16.4 关联字段确实落地的证据）：span 名与四个字段都在。
    ///
    /// 没有这一条，下面那条"没有敏感字段"的断言在"捕获层什么都没看见"的世界里恒真。
    #[test]
    fn execute_span_carries_the_declared_correlation_fields() {
        let service = app();
        let (_, captured) = capture(service.execute(
            auth_for("actor-1"),
            AppCommand::ListVisibleChannels {
                limit: None,
                cursor: None,
            },
        ));

        assert_eq!(captured.names, vec![EXECUTE_SPAN_NAME.to_owned()]);
        assert_eq!(captured.value_of("deployment_id"), Some("dep-g1"));
        assert_eq!(captured.value_of("tenant_id"), Some("tenant-g1"));
        assert_eq!(captured.value_of("actor_id"), Some("actor-1"));
        assert_eq!(
            captured.value_of("operation"),
            Some("list_visible_channels")
        );
    }

    /// 负向：`AuthContext` 整体、角色集合、auth generation 一个都不进 span。
    #[test]
    fn span_never_carries_the_auth_context_or_its_credentials() {
        let service = app();
        let (_, captured) = capture(service.execute(auth_for("actor-1"), AppCommand::Health));

        let tool_secret = "SENTINEL-TOOL-ARGUMENT-SECRET";
        let (_, tool_captured) = capture(service.execute(
            auth_for("actor-1"),
            AppCommand::InvokeTool(ToolInvocation {
                call_id: ToolCallId::new("call-secret"),
                run_id: RunId::new("run-1"),
                bot_id: BotId::new("bot-1"),
                call_seq: 0,
                tool_name: "computer.write".to_owned(),
                arguments: json!({"password":tool_secret}),
            }),
        ));

        let sentinel = SENTINEL_AUTH_GENERATION.to_string();
        for (name, value) in captured.fields.iter().chain(&tool_captured.fields) {
            assert!(
                !value.contains(&sentinel),
                "auth_generation 不得出现在 span 里：{name}={value}"
            );
            assert!(
                !value.contains(tool_secret),
                "tool arguments 不得出现在 span 里：{name}={value}"
            );
            for forbidden in ["AuthContext", "roles", "Admin", "single_user"] {
                assert!(
                    !value.contains(forbidden),
                    "span 字段 {name} 泄漏了 {forbidden}：{value}"
                );
                assert!(
                    !name.contains(forbidden),
                    "span 字段名本身就不该是 {forbidden}"
                );
            }
        }

        // 正向对照：捕获层确实看见了字段（否则上面的循环体一次都不执行）。
        assert!(!captured.fields.is_empty(), "捕获层必须真的看到字段");
        assert_eq!(captured.value_of("actor_id"), Some("actor-1"));
        assert_eq!(tool_captured.value_of("operation"), Some("invoke_tool"));
    }

    /// span 字段集合恰好是登记过的那些 —— 多记一个就判红，逼作者去做基数裁决。
    #[test]
    fn span_fields_are_exactly_the_declared_ledger() {
        let service = app();

        // 成功路径：`error.code` 是 Empty，不会被记录。
        let (_, ok) = capture(service.execute(auth_for("actor-1"), AppCommand::Health));
        for name in ok.names_of_fields() {
            assert!(
                APPLICATION_SPAN_FIELDS.contains(&name),
                "未登记的 span 字段：{name}"
            );
        }

        // 失败路径：`error.code` 被记录，于是并集恰好覆盖整份台账。
        let (_, err) = capture(service.execute(
            auth_for("actor-1"),
            AppCommand::ListVisibleChannels {
                limit: None,
                cursor: Some("@@@bad".to_owned()),
            },
        ));
        let mut union: Vec<&str> = ok.names_of_fields();
        union.extend(err.names_of_fields());
        union.sort_unstable();
        union.dedup();
        let mut expected: Vec<&str> = APPLICATION_SPAN_FIELDS.to_vec();
        expected.sort_unstable();
        assert_eq!(union, expected, "span 字段集合必须与台账逐项相等");
    }

    /// 失败时把**稳定码**记到 span 上（不是 Display，不是内部细节）。
    #[test]
    fn failed_execute_records_the_stable_error_code() {
        let service = app();
        let (result, captured) = capture(service.execute(
            auth_for("actor-1"),
            AppCommand::ListVisibleChannels {
                limit: None,
                cursor: Some("@@@bad".to_owned()),
            },
        ));

        let err = result.expect_err("坏游标必须失败");
        assert_eq!(err.code(), ErrorCode::MALFORMED_PAYLOAD);
        assert_eq!(captured.value_of("error.code"), Some("malformed_payload"));

        // 负向：policy rule / holder 这类 Display 上下文不得随之进来。
        assert!(
            captured.value_of("error").is_none(),
            "只记 code，不记整条 Display"
        );
    }

    #[test]
    fn subscribe_is_instrumented_too() {
        let service = app();
        let (_, captured) =
            capture(service.subscribe(auth_for("actor-1"), SubscriptionRequest::Health));
        assert_eq!(captured.names, vec![SUBSCRIBE_SPAN_NAME.to_owned()]);
        assert_eq!(captured.value_of("operation"), Some("health"));
        assert_eq!(captured.value_of("actor_id"), Some("actor-1"));
    }

    // -----------------------------------------------------------------------
    // G1 没有生产者的两个错误变体（见 crate 文档）
    // -----------------------------------------------------------------------

    /// 零角色的**已认证** actor 不被拒绝：G1 的两个用例都不设角色门（parity）。
    ///
    /// 同一个 roleless actor：普通读取不凭空加角色门，admin 用例则必须产出 403。
    #[test]
    fn a_roleless_authenticated_actor_is_not_rejected() {
        let roleless = AuthContext::for_test(
            DeploymentId::new("dep-g1"),
            TenantId::new("tenant-g1"),
            ActorId::new("actor-1"),
            [],
            1,
            false,
        );
        assert!(roleless.roles().is_empty());

        let service = app();
        let (health_reply, _) = capture(service.execute(roleless.clone(), AppCommand::Health));
        assert!(health_reply.is_ok(), "探活不看角色");

        let (list_reply, _) = capture(service.execute(
            roleless.clone(),
            AppCommand::ListVisibleChannels {
                limit: None,
                cursor: None,
            },
        ));
        assert!(list_reply.is_ok(), "列表只看 membership，不看角色");

        let (admin_reply, _) = capture(service.execute(roleless, AppCommand::AdminStatus));
        assert!(matches!(
            admin_reply,
            Err(AppError::ForbiddenRole {
                required: Role::Admin
            })
        ));
    }

    /// 正向对照：本 crate **确实**会返回错误 —— 上一条的 `is_ok()` 不是靠
    /// 「这个入口永远成功」成立的。
    #[test]
    fn the_service_does_produce_errors_on_other_paths() {
        let service = app();
        let (result, _) = capture(service.execute(
            auth_for("actor-1"),
            AppCommand::ListVisibleChannels {
                limit: None,
                cursor: Some("@@@bad".to_owned()),
            },
        ));
        assert!(result.is_err());
    }

    /// `dyn ApplicationService` 必须可用：transport 持有的是 trait 对象。
    #[test]
    fn the_service_is_object_safe() {
        let service: Box<dyn ApplicationService> = Box::new(app());
        let (reply, _) = capture(service.execute(auth_for("actor-1"), AppCommand::Health));
        assert!(reply.is_ok());
    }
}
