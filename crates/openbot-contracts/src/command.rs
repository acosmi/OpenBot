//! 唯一业务入口的 typed 命令 / 应答 / 订阅 / 事件（v3 §5.2）。
//!
//! §5.2 固定了 `openbot-application::ApplicationService` 的两个签名：
//!
//! ```text
//! async fn execute(&self, auth: AuthContext, command: AppCommand) -> Result<AppReply, AppError>;
//! async fn subscribe(&self, auth: AuthContext, request: SubscriptionRequest) -> Result<AppEventStream, AppError>;
//! ```
//!
//! trait 本身住在 `openbot-application`（它需要 `async_trait` 与 `Stream`，那是 native 侧的
//! 依赖）；本模块只定义穿越边界的**类型**，因为它们必须同时编到 wasm 给 `openbot-ui` 用。
//!
//! # 为什么这些 enum 是封闭的
//!
//! §5.2 逐字禁止：「任何 transport 都不得接受自由 method string、renderer 自报角色、renderer
//! 自报 `principal=admin` 或任意数据库 query。」
//!
//! 一个 `{ method: String, params: Value }` 形状的命令**就是**自由 method string —— 它把
//! 「有哪些用例」这件事从编译期推到了运行期的一次字符串匹配，于是 dispatcher 必然长出一个
//! `_ => Err(unknown_method)` 分支，而那个分支是 transport 在替 application 做业务判定。
//! 封闭 enum 让「不存在的用例」在**反序列化阶段**就变成 400 malformed payload（§15.3），
//! 根本到不了 application。
//!
//! # 用例随已闭合 slice 扩展
//!
//! 没有 parity ledger/第一真源条目背书的用例不能进（CLAUDE.md §4）。G1 从 channel/health
//! 起步，W-3a 追加 people，W-3b 追加 tool pipeline；thread 订阅仍是 G3，届时同批加入。

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::auth::Role;
use crate::ids::{BotId, ChannelId, ThreadId};
use crate::people::{AdminStatus, CurrentUser, PeoplePage, Person};
use crate::tool::{ToolInvocation, ToolResult};

/// 单页 channel 的条数上限。
///
/// **parity 值**（不是新增）：出处是上游 `server/src/routes/channels/routes.ts` 的
/// `MAX_CHANNEL_PAGE` 常量。这里逐字沿用它，不擅自放大或收紧 —— 改动分页上限会改变
/// 既有客户端的翻页轮次，属于行为变更，需要单独的 ledger 条目。
///
/// 语义：[`AppCommand::ListVisibleChannels::limit`] 为 `None` 或大于本值时，application
/// 按本值截断；本 crate 只定义常量，不在这里做钳制 —— 钳制是 use case 的职责。
pub const MAX_CHANNEL_PAGE: u32 = 200;

/// 应用层命令。封闭 enum。
///
/// 线上表示是 internally tagged（`kind` 字段），并且 `deny_unknown_fields`：多送一个字段
/// 就是 400，不静默忽略。静默忽略未知字段等于允许调用方以为自己传了个参数而实际没有 ——
/// 那是一类特别难查的行为分歧。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppCommand {
    /// 最小只读用例：探活。不读任何租户数据，也不产生 audit 事件。
    Health,

    /// 列出**当前 actor 可见的** channel。
    ///
    /// 「可见」由 application 依据 materialized membership 判定（§6.5 条 5），不由调用方
    /// 传入任何过滤条件决定 —— 那会把访问控制的判定权交给调用方。
    ListVisibleChannels {
        /// 本页最多返回多少条。`None` = 由 application 取默认值；超过
        /// [`MAX_CHANNEL_PAGE`] 由 application 截断。
        limit: Option<u32>,
        /// keyset 游标，**不透明字符串**。
        ///
        /// 上游的排序键是 `(coalesce(last_message_at, created_at) DESC, id DESC)`，游标
        /// 编码的是 `{recency, id}` 二元组。transport **不解释**它：一旦 transport 开始
        /// 解析游标，游标格式就变成了公开契约，之后换排序键会变成破坏性变更。
        /// 它由 application 铸造、由 application 解析，中间任何一层原样搬运。
        cursor: Option<String>,
    },

    /// 返回当前已验证 actor 的公开资料。
    GetCurrentUser,

    /// 管理员 gate 探针；非 admin 由 application 返回 403。
    AdminStatus,

    /// 管理员 people keyset 页。
    ListPeople {
        /// email/name 的大小写不敏感子串；空白等价未设置。
        search: Option<String>,
        /// opaque keyset cursor。
        cursor: Option<String>,
        /// 页大小；application 钳制到 1..=200。
        limit: Option<u32>,
    },

    /// 修改一个人的角色。
    ChangePersonRole {
        /// 被管理者 id。
        user_id: crate::ids::ActorId,
        /// 目标角色。
        role: Role,
    },

    /// 移除或恢复一个人的访问。
    ChangePersonAccess {
        /// 被管理者 id。
        user_id: crate::ids::ActorId,
        /// `true`=移除，`false`=恢复。
        revoked: bool,
    },

    /// 由 Rust Agent gateway 铸造的一次工具调用；仍须在 application 里走完整 §8.1 管线。
    InvokeTool(ToolInvocation),
}

/// 应用层应答。封闭 enum，与 [`AppCommand`] 一一对应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppReply {
    /// [`AppCommand::Health`] 的应答。
    Health(HealthReport),
    /// [`AppCommand::ListVisibleChannels`] 的应答。
    Channels(ChannelPage),
    /// [`AppCommand::GetCurrentUser`] 应答。
    CurrentUser(CurrentUser),
    /// [`AppCommand::AdminStatus`] 应答。
    AdminStatus(AdminStatus),
    /// [`AppCommand::ListPeople`] 应答。
    People(PeoplePage),
    /// role/access 变更后的最新 person。
    Person(Person),
    /// [`AppCommand::InvokeTool`] 的已持久化、已脱敏结果。
    Tool(ToolResult),
}

/// 探活结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthReport {
    /// 进程是否可服务。
    ///
    /// 刻意只有一个布尔：readiness 的细节（数据库池、sidecar 版本、engine 状态）属于
    /// §16.4 的 metrics 与 `openbot-server` 的 `/readyz`，不是跨边界 DTO 的内容。把依赖
    /// 明细放进公开应答会顺带泄漏部署拓扑。
    pub ok: bool,
}

/// 一页 channel。
///
/// **空列表是合法值**（§15.3 末条：「空、新 thread history 200 + empty list」）：
/// `items` 为空时序列化成 `[]` 而不是 `null`，`next_cursor` 为 `None` 时序列化成 `null`
/// 而不是被省略。上游缺陷 #72「空 history 500」正是把「空」当成错误的结果（§2.4），
/// 本类型在序列化形状上就把这条堵死，并由 `empty_page_serializes_as_empty_list` 钉住。
///
/// # 字段名为什么是 camelCase
///
/// v3 §15.1 把现有 `/api/channels` 的 **input/output schema** 纳入 canonical inventory 的
/// parity 面，所以线上字段名是契约的一部分，不是风格问题。上游 `channelSummaryDto` 发出的
/// 是 `channels` / `nextCursor` / `agentIds` / `threadId` / `lastMessageAt` 这一组 camelCase
/// 名字。这里用 `rename_all` 让**同一个类型**既是内部 typed 边界又是线上形状 —— 另建一层
/// HTTP DTO 做改名同样能对齐，但那是两份必须手工保持同步的真源，改一处忘另一处不会有人发现。
/// 由 `channel_page_json_keys_match_upstream_wire` 与 `channel_summary_json_keys_match_upstream_wire`
/// 两条测试逐键钉死。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelPage {
    /// 本页条目，可能为空。上游键名是 `channels`（不是 `items`）。
    pub channels: Vec<ChannelSummary>,
    /// 下一页游标；`None` 表示已到末页。不透明，见
    /// [`AppCommand::ListVisibleChannels::cursor`]。
    pub next_cursor: Option<String>,
}

/// channel 列表项。
///
/// 这是 **DTO，不是行结构**：真实 `channels` 表有 12 列，这里只投影上游 channel `list`
/// 路由实际返回的那几项。两条刻意的排除：
///
/// - **`allowed_groups` 绝不进 DTO**。§6.5 条 5 定死「group 只负责 provision channel
///   membership，所有运行时 channel route 仍检查 materialized membership」。把它发给
///   transport 会诱导下游拿它做访问判定 —— 那正是上游 `allowed_groups` 长期是 no-op 的
///   同一枚硬币的反面（§2.4）。可见性判定已经在服务端做完了，客户端拿到这一行就等于有权看。
/// - `package_id` / `override` / `description` / `suggested_prompts` / `updated_at` 属于
///   channel 详情或 provisioning 面，不属于列表投影；需要时随各自的 ledger 条目单独加。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelSummary {
    /// channel 身份。
    pub id: ChannelId,
    /// 展示名。它是**数据**不是文案：不进本地化表，原样来自数据库。
    pub name: String,
    /// 该 channel 上挂载的 bot。
    pub agent_ids: Vec<BotId>,
    /// 最近一条消息的文本预览；从未有过消息时为 `None`。
    pub last_message: Option<String>,
    /// 最近一条消息的时间；从未有过消息时为 `None`。
    ///
    /// 它同时是 keyset 排序键的第一项（`coalesce(last_message_at, created_at) DESC`）。
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_message_at: Option<OffsetDateTime>,
    /// 发出最近一条消息的 bot；人类发出或从未有过消息时为 `None`。
    pub last_message_agent_id: Option<BotId>,
    /// 创建时间。`last_message_at` 缺失时它是排序键的回落项。
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// 该 channel 关联的 thread；尚未开出 thread 时为 `None`。
    ///
    /// # 为什么是 `Option`，而上游那一项非空
    ///
    /// 上游 `channels/routes.ts` 的 `channelDto` 里 `threadId` 是必填 `string`，但那个
    /// 非空**是 join 造出来的假象**：`list` 与 `get` 都对 `intelligence_channel_mappings`
    /// 做 INNER JOIN，没有 mapping 行的 channel 根本不会出现在结果里 —— 于是"每个可见
    /// channel 都有 thread"在上游恒真。
    ///
    /// 那个 join 必须删（§28.1 R22）：Intelligence 已按 §4.1 退役、该表按 §14.2 降级为
    /// 只读 legacy provenance，继续 join 会把 §6.5 刚补上 membership 的包 channel 原样
    /// 过滤回不可达。join 一删，"可见但还没有 thread"就成了合法状态（例如刚 provision、
    /// 还没有人打开过的包 channel），所以这里必须是 `Option`。
    ///
    /// 它的数据源随之改为 §4.3 的 native `threads`，**不是** `intelligence_channel_mappings`。
    /// G1 还没有 native thread 表，本字段恒 `None`；G3 接上真源。
    pub thread_id: Option<ThreadId>,
    /// 该 channel 当前是否可用。
    pub active: bool,
}

/// 订阅请求。封闭 enum，理由同 [`AppCommand`]。
///
/// G1 只有探活一项：真正的 thread 订阅是 G3 的工作，提前定义会得到一个没有消费者、
/// 也没有 ledger 条目背书的形状。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubscriptionRequest {
    /// 订阅心跳流。
    Health,
}

/// 订阅流上的事件。封闭 enum。
///
/// 注意 `AppEventStream` 本身**不在**本 crate：它是 `Stream<Item = AppEvent>` 的别名，
/// 需要 `futures`/`async` 机制，属于 `openbot-application`。本 crate 只承载帧的内容。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppEvent {
    /// 心跳。`seq` 单调递增，供 viewer 判断是否丢帧。
    Heartbeat {
        /// 序号。
        seq: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_summary() -> ChannelSummary {
        ChannelSummary {
            id: ChannelId::new("legacy-channel-42"),
            name: "General".to_owned(),
            agent_ids: vec![BotId::new("bot-1")],
            // 有 thread 的常态；与下面 channel_summary_without_messages_round_trips 里的
            // None 构成两向对照 —— 只测一侧的话，Option 序列化成 null 还是被整键省略
            // 这个差别照不出来。
            thread_id: Some(ThreadId::new("thread-1")),
            last_message: Some("hi".to_owned()),
            last_message_at: Some(datetime!(2026-08-22 04:05:06 UTC)),
            last_message_agent_id: Some(BotId::new("bot-1")),
            created_at: datetime!(2026-08-01 00:00:00 UTC),
            active: true,
        }
    }

    /// §15.3 末条的机械兑现：空页必须是 `{"items":[],"next_cursor":null}`。
    ///
    /// `[]` 与 `null` 在客户端是两种东西 —— 后者会让「没有 channel」和「字段缺失」不可
    /// 区分，那正是上游 #72 空 history 崩掉的同一类形状问题。
    #[test]
    fn empty_page_serializes_as_empty_list_not_null() {
        let page = ChannelPage::default();
        let json = serde_json::to_string(&page).unwrap();
        assert_eq!(json, r#"{"channels":[],"nextCursor":null}"#);

        // 反向也必须成立：这段 JSON 读回来是一个合法的空页，不是错误。
        let back: ChannelPage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, page);
        assert!(back.channels.is_empty());
        assert!(back.next_cursor.is_none());
    }

    /// 线上字段名与上游逐键相等（v3 §15.1 把 `/api/channels` 的 output schema 纳入 parity）。
    ///
    /// 期望值不是我抄来的，是上游 `channels/routes.ts` 里 `channelDto` 与
    /// `channelSummaryDto` 两个函数返回对象的字面键，合起来九个。改名会当场判红。
    #[test]
    fn channel_summary_json_keys_match_upstream_wire() {
        let json = serde_json::to_string(&sample_summary()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut got: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        got.sort_unstable();

        // 上游 channelDto: id / name / agentIds / threadId / active
        // 上游 channelSummaryDto 追加: lastMessage / lastMessageAt / lastMessageAgentId / createdAt
        let mut want = [
            "id",
            "name",
            "agentIds",
            "threadId",
            "active",
            "lastMessage",
            "lastMessageAt",
            "lastMessageAgentId",
            "createdAt",
        ];
        want.sort_unstable();
        assert_eq!(got, want, "线上字段集与上游不一致：{json}");
    }

    /// 同上，页信封那一层。上游 `list` 返回 `{ channels, nextCursor }`。
    #[test]
    fn channel_page_json_keys_match_upstream_wire() {
        let json = serde_json::to_string(&ChannelPage::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut got: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        got.sort_unstable();
        let mut want = ["channels", "nextCursor"];
        want.sort_unstable();
        assert_eq!(got, want, "页信封字段集与上游不一致：{json}");

        // 负向对照：确认这条断言不是在「随便什么键集都过」的世界里成立。
        assert_ne!(
            got,
            ["items", "next_cursor"],
            "旧的 snake_case 形状不应再通过"
        );
    }

    /// 正向对照：同一个类型在**非空**时确实序列化出条目 —— 证明上一条不是靠
    /// 「这个类型序列化出来永远是空的」蒙混过关。
    #[test]
    fn non_empty_page_actually_carries_items() {
        let page = ChannelPage {
            channels: vec![sample_summary()],
            next_cursor: Some("opaque-cursor".to_owned()),
        };
        let json = serde_json::to_string(&page).unwrap();
        assert!(json.contains(r#""id":"legacy-channel-42""#), "{json}");
        assert!(json.contains(r#""nextCursor":"opaque-cursor""#), "{json}");
        let back: ChannelPage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, page);
    }

    #[test]
    fn channel_summary_timestamps_are_rfc3339() {
        let json = serde_json::to_string(&sample_summary()).unwrap();
        assert!(
            json.contains(r#""createdAt":"2026-08-01T00:00:00Z""#),
            "{json}"
        );
        assert!(
            json.contains(r#""lastMessageAt":"2026-08-22T04:05:06Z""#),
            "{json}"
        );
    }

    #[test]
    fn channel_summary_without_messages_round_trips() {
        let summary = ChannelSummary {
            id: ChannelId::new("c-2"),
            name: "Empty".to_owned(),
            agent_ids: Vec::new(),
            // 可见但还没有 thread：join 删掉之后的合法状态（见 thread_id 字段文档）。
            thread_id: None,
            last_message: None,
            last_message_at: None,
            last_message_agent_id: None,
            created_at: datetime!(2026-08-01 00:00:00 UTC),
            active: false,
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains(r#""lastMessageAt":null"#), "{json}");
        let back: ChannelSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back, summary);
    }

    /// `allowed_groups` 不在 DTO 里（§6.5 条 5）。
    ///
    /// 负向断言配正向对照：同一手法在**确实存在**的字段上命中，证明这不是一条
    /// 「反正 JSON 里什么都没有」的空断言。
    #[test]
    fn allowed_groups_never_crosses_the_boundary() {
        let json = serde_json::to_string(&sample_summary()).unwrap();
        assert!(
            !json.contains("allowed_groups"),
            "allowed_groups 不得进 DTO：运行时可见性只认 materialized membership"
        );
        assert!(!json.contains("packageId"), "{json}");
        assert!(!json.contains("package_id"), "{json}");
        assert!(!json.contains("override"), "{json}");
        // 正向对照：确实被投影的字段都在。
        assert!(json.contains("agentIds"), "{json}");
        assert!(json.contains("active"), "{json}");
    }

    #[test]
    fn command_is_internally_tagged_and_closed() {
        let listed = AppCommand::ListVisibleChannels {
            limit: Some(50),
            cursor: None,
        };
        let json = serde_json::to_string(&listed).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"list_visible_channels","limit":50,"cursor":null}"#
        );
        assert_eq!(serde_json::from_str::<AppCommand>(&json).unwrap(), listed);

        assert_eq!(
            serde_json::to_string(&AppCommand::Health).unwrap(),
            r#"{"kind":"health"}"#
        );

        let tool = AppCommand::InvokeTool(ToolInvocation {
            call_id: crate::ids::ToolCallId::new("call-1"),
            run_id: crate::ids::RunId::new("run-1"),
            bot_id: BotId::new("bot-1"),
            call_seq: 2,
            tool_name: "computer.write".to_owned(),
            arguments: serde_json::json!({"x":1}),
        });
        let wire = serde_json::to_string(&tool).unwrap();
        assert_eq!(
            wire,
            r#"{"kind":"invoke_tool","callId":"call-1","runId":"run-1","botId":"bot-1","callSeq":2,"toolName":"computer.write","arguments":{"x":1}}"#,
        );
        assert_eq!(serde_json::from_str::<AppCommand>(&wire).unwrap(), tool);
    }

    /// 自由 method string 走不通：未知 `kind` 与未知字段都在反序列化阶段就失败，
    /// 到不了 application（§5.2 + §15.3「malformed payload 400，不产生 acting decision」）。
    #[test]
    fn unknown_command_kind_and_unknown_fields_are_rejected() {
        assert!(
            serde_json::from_str::<AppCommand>(r#"{"kind":"drop_all_tables"}"#).is_err(),
            "未知 kind 必须拒绝，而不是落进 dispatcher 的通配分支"
        );
        assert!(
            serde_json::from_str::<AppCommand>(
                r#"{"kind":"list_visible_channels","limit":1,"cursor":null,"principal":"admin"}"#
            )
            .is_err(),
            "renderer 自报 principal 必须被 deny_unknown_fields 当场拒绝（§5.2）"
        );
        // 正向对照：合法载荷确实能解析 —— 否则上面两条在「什么都解析不了」的世界里同样通过。
        assert!(
            serde_json::from_str::<AppCommand>(
                r#"{"kind":"list_visible_channels","limit":1,"cursor":null}"#
            )
            .is_ok()
        );
    }

    #[test]
    fn reply_subscription_and_event_round_trip() {
        let reply = AppReply::Health(HealthReport { ok: true });
        let json = serde_json::to_string(&reply).unwrap();
        assert_eq!(json, r#"{"kind":"health","ok":true}"#);
        assert_eq!(serde_json::from_str::<AppReply>(&json).unwrap(), reply);

        let channels = AppReply::Channels(ChannelPage::default());
        let json = serde_json::to_string(&channels).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"channels","channels":[],"nextCursor":null}"#
        );
        assert_eq!(serde_json::from_str::<AppReply>(&json).unwrap(), channels);

        let tool = AppReply::Tool(ToolResult {
            call_id: crate::ids::ToolCallId::new("call-1"),
            content: "ok".to_owned(),
            error_code: None,
            commit_state: crate::tool::ToolCommitState::Committed,
            visible_bytes: 2,
            truncated: false,
        });
        let json = serde_json::to_string(&tool).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"tool","callId":"call-1","content":"ok","errorCode":null,"commitState":"committed","visibleBytes":2,"truncated":false}"#,
        );
        assert_eq!(serde_json::from_str::<AppReply>(&json).unwrap(), tool);

        let request = SubscriptionRequest::Health;
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"kind":"health"}"#
        );

        let event = AppEvent::Heartbeat { seq: 7 };
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, r#"{"kind":"heartbeat","seq":7}"#);
        assert_eq!(serde_json::from_str::<AppEvent>(&json).unwrap(), event);
    }

    /// parity 常量固定为上游 `channels/routes.ts::MAX_CHANNEL_PAGE` 的取值。
    #[test]
    fn max_channel_page_is_the_upstream_parity_value() {
        assert_eq!(MAX_CHANNEL_PAGE, 200);
    }
}
