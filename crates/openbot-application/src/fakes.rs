//! 测试替身。**只在 `cfg(test)` 下编译**，不进任何发行物。
//!
//! 本 crate 的全部测试都跑在内存里，一行 SQL、一个数据库连接都不需要 —— 这不是图省事，
//! 而是端口方向正确的可观察后果（见 crate 文档〈端口在这里定义〉）。
//!
//! [`FakeChannelReader`] 刻意**模拟数据库该有的行为**（按排序键定序、按 keyset 判据裁剪、
//! 按 `limit` 截断），而不是「返回我准备好的那几行」。区别很实在：后者会让分页测试变成
//! 「断言 fake 返回了我塞进去的东西」，那种测试在翻页逻辑写反的时候照样绿。

use std::sync::Mutex;

use async_trait::async_trait;
use openbot_contracts::auth::{AuthContext, Role};
use openbot_contracts::command::ChannelSummary;
use openbot_contracts::ids::{ActorId, ChannelId, DeploymentId, TenantId};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::cursor::{ChannelCursor, channel_recency};
use crate::ports::{ChannelReader, PortError};

/// 解析测试里用的 RFC 3339 字面量。解析不了就是测试写错了，直接 panic。
pub(crate) fn ts(raw: &str) -> OffsetDateTime {
    OffsetDateTime::parse(raw, &Rfc3339).expect("测试里的时间戳字面量必须是合法 RFC 3339")
}

/// 造一行「有过消息」的 channel。
///
/// `created_at` 刻意比 `last_message_at` 早一天：两个时间戳相同的话，
/// `channel_recency` 取错字段也测不出来。
pub(crate) fn summary_at(id: &str, last_message_at: &str) -> ChannelSummary {
    let last = ts(last_message_at);
    ChannelSummary {
        id: ChannelId::new(id),
        name: format!("channel {id}"),
        agent_ids: Vec::new(),
        last_message: Some("hi".to_owned()),
        last_message_at: Some(last),
        last_message_agent_id: None,
        created_at: last - time::Duration::days(1),
        // G1 还没有 native `threads` 表，本字段恒 None（contracts 里 `thread_id` 的文档
        // 写明了理由：上游那个非空是 `intelligence_channel_mappings` 的 INNER JOIN 造出来
        // 的假象，join 一删「可见但还没有 thread」就是合法状态）。
        thread_id: None,
        active: true,
    }
}

/// 造一行「从未有过消息」的 channel —— 排序键回落到 `created_at`。
pub(crate) fn summary_without_messages(id: &str, created_at: &str) -> ChannelSummary {
    ChannelSummary {
        id: ChannelId::new(id),
        name: format!("channel {id}"),
        agent_ids: Vec::new(),
        last_message: None,
        last_message_at: None,
        last_message_agent_id: None,
        created_at: ts(created_at),
        thread_id: None,
        active: true,
    }
}

/// 造一个已认证上下文。
///
/// `auth_generation` 用一个显眼的哨兵值：tracing 测试要断言它**没有**出现在 span 里，
/// 哨兵值让那条断言不会被别的数字偶然满足。
pub(crate) const SENTINEL_AUTH_GENERATION: u64 = 424_242;

/// 造一个已认证上下文（`AuthContext::for_test` 只在 contracts 的 `testkit` feature 下存在）。
pub(crate) fn auth_for(actor: &str) -> AuthContext {
    AuthContext::for_test(
        DeploymentId::new("dep-g1"),
        TenantId::new("tenant-g1"),
        ActorId::new(actor),
        [Role::Admin, Role::User],
        SENTINEL_AUTH_GENERATION,
        false,
    )
}

/// 一次落到端口上的调用。测试用它断言 application 传下去的到底是什么。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PortCall {
    pub actor: ActorId,
    pub limit: u32,
    pub cursor: Option<ChannelCursor>,
}

/// 内存版 [`ChannelReader`]。
///
/// 「可见性」由构造时给定的 `(actor, row)` 关联表承载 —— 它就是 materialized membership
/// 在测试里的替身。**没有任何别的过滤维度**，这正是被测契约：可见性只有一个判据。
pub(crate) struct FakeChannelReader {
    rows: Vec<(ActorId, ChannelSummary)>,
    failure: Option<PortError>,
    calls: Mutex<Vec<PortCall>>,
}

impl FakeChannelReader {
    /// 空库。
    pub(crate) fn empty() -> Self {
        Self {
            rows: Vec::new(),
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// 给某个 actor 挂上若干可见行（= 给他建 membership）。
    pub(crate) fn with_visible(
        mut self,
        actor: &str,
        rows: impl IntoIterator<Item = ChannelSummary>,
    ) -> Self {
        let actor = ActorId::new(actor);
        self.rows
            .extend(rows.into_iter().map(|row| (actor.clone(), row)));
        self
    }

    /// 让端口恒定失败。
    pub(crate) fn failing(failure: PortError) -> Self {
        Self {
            rows: Vec::new(),
            failure: Some(failure),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// 迄今为止收到的全部调用。
    pub(crate) fn calls(&self) -> Vec<PortCall> {
        self.calls.lock().expect("fake 的互斥锁不会中毒").clone()
    }

    /// 调用次数。
    pub(crate) fn call_count(&self) -> usize {
        self.calls.lock().expect("fake 的互斥锁不会中毒").len()
    }
}

#[async_trait]
impl ChannelReader for FakeChannelReader {
    async fn list_visible_channels(
        &self,
        actor: &ActorId,
        limit: u32,
        cursor: Option<ChannelCursor>,
    ) -> Result<Vec<ChannelSummary>, PortError> {
        self.calls
            .lock()
            .expect("fake 的互斥锁不会中毒")
            .push(PortCall {
                actor: actor.clone(),
                limit,
                cursor: cursor.clone(),
            });

        if let Some(failure) = self.failure {
            return Err(failure);
        }

        // 一、可见性：只认关联表（= materialized membership）。
        let mut visible: Vec<ChannelSummary> = self
            .rows
            .iter()
            .filter(|(owner, _)| owner == actor)
            .map(|(_, row)| row.clone())
            .collect();

        // 二、定序：coalesce(last_message_at, created_at) DESC, id DESC。
        visible.sort_by(|a, b| (channel_recency(b), &b.id).cmp(&(channel_recency(a), &a.id)));

        // 三、keyset 裁剪：(recency, id) < (cursor.recency, cursor.id)。
        if let Some(cursor) = cursor {
            visible.retain(|row| (channel_recency(row), &row.id) < (cursor.recency, &cursor.id));
        }

        // 四、截断到调用方要的行数（调用方已经把探测用的 +1 算进来了）。
        visible.truncate(limit as usize);
        Ok(visible)
    }
}
