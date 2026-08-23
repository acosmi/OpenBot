//! durable tool decision / attempt repositories（v3 §8.1）。
//!
//! [`ToolCallRepo::record_first_decision`] 是持久化半边的关键入口：call 与 first attempt 在同一
//! PostgreSQL 事务写入，**commit 成功之后**才构造 `DurableDecisionReceipt`。写任一行失败都
//! 没有 receipt，类型状态机就拿不到 capability。`ToolAttemptRepo` 再以条件 UPDATE 承担
//! decision_recorded → executing → completed/reconciliation_required，状态竞争返回 `None`，不会
//! 覆盖另一执行者的结果。

use core::time::Duration;

use deadpool_postgres::Pool;
use openbot_contracts::ids::PolicyDecisionId;
use openbot_domain::tool::commit::CommitState;
use openbot_domain::tool::pipeline::{AttemptId, DurableDecisionReceipt};
use time::OffsetDateTime;

use crate::db::InfraError;
use crate::db::tables::{tool_attempts, tool_calls};
use crate::repo::common::{RepoCore, columns_sql, insert_sql};

/// 首次 decision 事务的两行输入。
#[derive(Clone, Debug, PartialEq)]
pub struct FirstDurableDecision {
    /// call/decision 行。
    pub call: tool_calls::Row,
    /// 与它同事务写入的 attempt #0。
    pub attempt: tool_attempts::Row,
}

/// 一次已经发生的执行要写入 attempt 的无内容结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistedToolOutcome {
    /// 提交状态。
    pub commit_state: CommitState,
    /// 输出字节数，不含输出内容。
    pub output_bytes: u32,
    /// 执行耗时。
    pub duration: Duration,
    /// 稳定错误码，不是错误文案。
    pub error_code: Option<&'static str>,
    /// 完成时刻。
    pub finished_at: OffsetDateTime,
}

/// durable tool call repository。
#[derive(Clone)]
pub struct ToolCallRepo {
    pool: Pool,
    core: RepoCore<tool_calls::Row>,
}

impl ToolCallRepo {
    /// 用调用方提供的池构造。
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self {
            core: RepoCore::new(pool.clone()),
            pool,
        }
    }

    /// 同事务写 call + attempt，commit 后签发领域回执。
    pub async fn record_first_decision(
        &self,
        decision: &FirstDurableDecision,
    ) -> Result<DurableDecisionReceipt, InfraError> {
        validate_first_decision(decision)?;
        let decision_id = decision.call.decision_id.clone();
        let attempt_id = decision.attempt.attempt_id.clone();

        let mut client = self.pool.get().await.map_err(|source| {
            InfraError::connect("为 ToolCallRepo 获取 decision 事务连接", source)
        })?;
        let transaction = client
            .transaction()
            .await
            .map_err(|source| InfraError::query("开始 durable tool decision 事务", source))?;

        let call_params = decision.call.as_sql_params();
        transaction
            .query_one(&insert_sql::<tool_calls::Row>(), &call_params)
            .await
            .map_err(|source| InfraError::query("写 durable tool call", source))?;
        let attempt_params = decision.attempt.as_sql_params();
        transaction
            .query_one(&insert_sql::<tool_attempts::Row>(), &attempt_params)
            .await
            .map_err(|source| InfraError::query("写 durable tool attempt", source))?;

        transaction
            .commit()
            .await
            .map_err(|source| InfraError::query("提交 durable tool decision", source))?;

        Ok(DurableDecisionReceipt::issued_by_repository(
            PolicyDecisionId::new(decision_id),
            AttemptId::new(attempt_id),
        ))
    }

    /// 按 call id 读取。
    pub async fn find_by_id(&self, id: &str) -> Result<Option<tool_calls::Row>, InfraError> {
        self.core.find("\"tool_call_id\" = $1", &[&id]).await
    }

    /// 按 `(run_id, call_seq)` 稳定列出。
    pub async fn list_all(&self) -> Result<Vec<tool_calls::Row>, InfraError> {
        self.core.list("\"run_id\", \"call_seq\"").await
    }
}

impl core::fmt::Debug for ToolCallRepo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolCallRepo").finish_non_exhaustive()
    }
}

/// tool attempt 状态机的持久化适配器。
#[derive(Clone)]
pub struct ToolAttemptRepo {
    core: RepoCore<tool_attempts::Row>,
}

impl ToolAttemptRepo {
    /// 用调用方提供的池构造。
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self {
            core: RepoCore::new(pool),
        }
    }

    /// 记录一次重试 attempt；必须仍是未铸 capability 的 `decision_recorded` 初态。
    pub async fn insert_retry(
        &self,
        attempt: &tool_attempts::Row,
    ) -> Result<tool_attempts::Row, InfraError> {
        validate_pristine_attempt(attempt)?;
        self.core.insert(attempt).await
    }

    /// 按复合主键读取。
    pub async fn find_by_key(
        &self,
        tool_call_id: &str,
        attempt_seq: i64,
    ) -> Result<Option<tool_attempts::Row>, InfraError> {
        self.core
            .find(
                "\"tool_call_id\" = $1 AND \"attempt_seq\" = $2",
                &[&tool_call_id, &attempt_seq],
            )
            .await
    }

    /// 按 call/sequence 稳定列出。
    pub async fn list_all(&self) -> Result<Vec<tool_attempts::Row>, InfraError> {
        self.core.list("\"tool_call_id\", \"attempt_seq\"").await
    }

    /// 把单次 capability 绑定到已 durable attempt，并进入 executing。
    pub async fn attach_capability(
        &self,
        tool_call_id: &str,
        attempt_seq: i64,
        capability_id: &str,
        started_at: OffsetDateTime,
    ) -> Result<Option<tool_attempts::Row>, InfraError> {
        if capability_id.is_empty() {
            return Err(InfraError::repository_invariant("capability_id_empty"));
        }
        let sql = format!(
            "UPDATE public.tool_attempts \
             SET capability_id=$3, status='executing', started_at=$4 \
             WHERE tool_call_id=$1 AND attempt_seq=$2 \
               AND status='decision_recorded' AND capability_id IS NULL \
             RETURNING {}",
            columns_sql::<tool_attempts::Row>(),
        );
        let client = self.core.pool().get().await.map_err(|source| {
            InfraError::connect("为 ToolAttemptRepo 绑定 capability 获取连接", source)
        })?;
        let row = client
            .query_opt(
                &sql,
                &[&tool_call_id, &attempt_seq, &capability_id, &started_at],
            )
            .await
            .map_err(|source| InfraError::query("绑定 tool capability", source))?;
        row.as_ref()
            .map(tool_attempts::Row::try_from)
            .transpose()
            .map_err(Into::into)
    }

    /// 记录执行 outcome；capability 必须与 executing 行逐字相等。
    pub async fn record_outcome(
        &self,
        tool_call_id: &str,
        attempt_seq: i64,
        capability_id: &str,
        outcome: &PersistedToolOutcome,
    ) -> Result<Option<tool_attempts::Row>, InfraError> {
        let duration_ms = i64::try_from(outcome.duration.as_millis())
            .map_err(|_| InfraError::repository_invariant("tool_duration_overflow"))?;
        let output_bytes = i64::from(outcome.output_bytes);
        let status = if outcome.commit_state.requires_reconciliation() {
            "reconciliation_required"
        } else {
            "completed"
        };
        let commit_state = outcome.commit_state.as_str();
        let sql = format!(
            "UPDATE public.tool_attempts \
             SET status=$4, commit_state=$5, output_bytes=$6, duration_ms=$7, \
                 error_code=$8, finished_at=$9 \
             WHERE tool_call_id=$1 AND attempt_seq=$2 \
               AND capability_id=$3 AND status='executing' \
             RETURNING {}",
            columns_sql::<tool_attempts::Row>(),
        );
        let client = self.core.pool().get().await.map_err(|source| {
            InfraError::connect("为 ToolAttemptRepo 写 outcome 获取连接", source)
        })?;
        let row = client
            .query_opt(
                &sql,
                &[
                    &tool_call_id,
                    &attempt_seq,
                    &capability_id,
                    &status,
                    &commit_state,
                    &output_bytes,
                    &duration_ms,
                    &outcome.error_code,
                    &outcome.finished_at,
                ],
            )
            .await
            .map_err(|source| InfraError::query("记录 tool outcome", source))?;
        row.as_ref()
            .map(tool_attempts::Row::try_from)
            .transpose()
            .map_err(Into::into)
    }
}

impl core::fmt::Debug for ToolAttemptRepo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolAttemptRepo").finish_non_exhaustive()
    }
}

fn validate_first_decision(decision: &FirstDurableDecision) -> Result<(), InfraError> {
    if decision.attempt.tool_call_id != decision.call.tool_call_id {
        return Err(InfraError::repository_invariant("attempt_call_id_mismatch"));
    }
    if decision.attempt.attempt_seq != 0 {
        return Err(InfraError::repository_invariant(
            "first_attempt_sequence_not_zero",
        ));
    }
    validate_pristine_attempt(&decision.attempt)
}

fn validate_pristine_attempt(attempt: &tool_attempts::Row) -> Result<(), InfraError> {
    if attempt.status != "decision_recorded"
        || attempt.capability_id.is_some()
        || attempt.commit_state.is_some()
        || attempt.output_bytes.is_some()
        || attempt.duration_ms.is_some()
        || attempt.error_code.is_some()
        || attempt.started_at.is_some()
        || attempt.finished_at.is_some()
    {
        return Err(InfraError::repository_invariant(
            "attempt_not_pristine_decision_record",
        ));
    }
    Ok(())
}
