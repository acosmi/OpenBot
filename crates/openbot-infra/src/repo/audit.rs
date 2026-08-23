//! append-only audit event / checkpoint repository（v3 §8.6）。
//!
//! 全部署用 transaction advisory lock 串行分配链前驱与 checkpoint sequence；event、hash 以及
//! 首条 genesis checkpoint 同事务提交。没有 checkpoint key、已有链却没有 genesis checkpoint、
//! 或数据库 hash 无法解析时全部 fail-closed。这里没有 UPDATE/DELETE API，数据库 trigger 再挡
//! 一层绕过。

use deadpool_postgres::Pool;
use openbot_domain::audit::chain::{PrevRowHash, StoredAuditRow};
use openbot_domain::audit::checkpoint::{
    AuditCheckpoint, AuditCheckpointKind, CheckpointSignature,
};
use openbot_domain::audit::event::AuditEvent;
use openbot_domain::audit::hash::Sha256Digest;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::InfraError;
use crate::db::tables::{audit_checkpoints, audit_events};

const AUDIT_CHAIN_LOCK_KEY: i64 = 0x4f50_454e_4155_4431; // ASCII `OPENAUD1`

/// audit event 与 checkpoint 的唯一写入面。
#[derive(Clone)]
pub struct AuditEventRepo {
    pool: Pool,
}

impl AuditEventRepo {
    /// 用调用方提供的池构造。
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// 追加一条审计事件；首条 Rust 链上行与 genesis checkpoint 同事务落库。
    pub async fn append(
        &self,
        event: &AuditEvent,
        checkpoint_key: &[u8],
    ) -> Result<StoredAuditRow, InfraError> {
        let mut client = self.pool.get().await.map_err(|source| {
            InfraError::connect("为 AuditEventRepo 获取 append 事务连接", source)
        })?;
        let transaction = client
            .transaction()
            .await
            .map_err(|source| InfraError::query("开始 audit append 事务", source))?;
        let linked = append_event_in_transaction(&transaction, event, checkpoint_key).await?;
        transaction
            .commit()
            .await
            .map_err(|source| InfraError::query("提交 audit append", source))?;
        Ok(linked)
    }

    /// 追加 periodic/closure checkpoint；sequence 在同一把锁下无空洞分配。
    pub async fn append_checkpoint(
        &self,
        kind: AuditCheckpointKind,
        created_at: OffsetDateTime,
        checkpoint_key: &[u8],
    ) -> Result<AuditCheckpoint, InfraError> {
        if checkpoint_key.is_empty() {
            return Err(InfraError::repository_invariant(
                "audit_checkpoint_key_empty",
            ));
        }
        if matches!(kind, AuditCheckpointKind::Genesis { .. }) {
            return Err(InfraError::repository_invariant(
                "genesis_checkpoint_only_created_with_first_row",
            ));
        }
        let mut client = self.pool.get().await.map_err(|source| {
            InfraError::connect("为 AuditEventRepo 获取 checkpoint 事务连接", source)
        })?;
        let transaction = client
            .transaction()
            .await
            .map_err(|source| InfraError::query("开始 audit checkpoint 事务", source))?;
        transaction
            .query_one("SELECT pg_advisory_xact_lock($1)", &[&AUDIT_CHAIN_LOCK_KEY])
            .await
            .map_err(|source| InfraError::query("获取 audit checkpoint 锁", source))?;
        verify_checkpoint_segment(&transaction, &kind).await?;
        let next: i64 = transaction
            .query_one(
                "SELECT coalesce(max(sequence), -1)::bigint + 1 FROM public.audit_checkpoints",
                &[],
            )
            .await
            .map_err(|source| InfraError::query("分配 audit checkpoint sequence", source))?
            .try_get(0)
            .map_err(|source| {
                crate::db::RowDecodeError::column("audit_checkpoints", "sequence", source)
            })?;
        let checkpoint = AuditCheckpoint {
            sequence: u64::try_from(next).map_err(|_| {
                InfraError::repository_invariant("audit_checkpoint_sequence_negative")
            })?,
            created_at,
            kind,
        };
        insert_checkpoint(&transaction, &checkpoint, checkpoint_key).await?;
        transaction
            .commit()
            .await
            .map_err(|source| InfraError::query("提交 audit checkpoint", source))?;
        Ok(checkpoint)
    }

    /// 按链顺序读取全部当前行（旧行 hash 双 NULL 也保留）。
    pub async fn list_all(&self) -> Result<Vec<audit_events::ChainedRow>, InfraError> {
        let client = self.pool.get().await.map_err(|source| {
            InfraError::connect("为 AuditEventRepo 读取 chain 获取连接", source)
        })?;
        let rows = client
            .query(
                "SELECT id,actor_user_id,event_type,target_type,target_id,payload,created_at, \
                        prev_hash,row_hash \
                 FROM public.audit_events ORDER BY created_at, id",
                &[],
            )
            .await
            .map_err(|source| InfraError::query("列出 audit chain", source))?;
        rows.iter()
            .map(audit_events::ChainedRow::try_from)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }

    /// 按 sequence 列出 checkpoint 行。
    pub async fn list_checkpoints(&self) -> Result<Vec<audit_checkpoints::Row>, InfraError> {
        let client = self.pool.get().await.map_err(|source| {
            InfraError::connect("为 AuditEventRepo 读取 checkpoint 获取连接", source)
        })?;
        let rows = client
            .query(
                "SELECT sequence,checkpoint_kind,first_event_id,first_row_hash,last_event_id, \
                        last_row_hash,event_count,unlinked_rows_before,retention_days,signature,created_at \
                 FROM public.audit_checkpoints ORDER BY sequence",
                &[],
            )
            .await
            .map_err(|source| InfraError::query("列出 audit checkpoints", source))?;
        rows.iter()
            .map(audit_checkpoints::Row::try_from)
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }
}

/// 在调用方已有事务里追加事件；people/tool 等 acting 事务用它保证业务写与 audit 同 commit。
pub(crate) async fn append_event_in_transaction(
    transaction: &tokio_postgres::Transaction<'_>,
    event: &AuditEvent,
    checkpoint_key: &[u8],
) -> Result<StoredAuditRow, InfraError> {
    if checkpoint_key.is_empty() {
        return Err(InfraError::repository_invariant(
            "audit_checkpoint_key_empty",
        ));
    }
    let event_uuid = Uuid::parse_str(event.id.as_str())
        .map_err(|_| InfraError::repository_invariant("audit_event_id_not_uuid"))?;
    transaction
        .query_one("SELECT pg_advisory_xact_lock($1)", &[&AUDIT_CHAIN_LOCK_KEY])
        .await
        .map_err(|source| InfraError::query("获取 audit chain 锁", source))?;

    let latest = transaction
        .query_opt(
            "SELECT id,created_at,row_hash FROM public.audit_events \
             ORDER BY created_at DESC, id DESC LIMIT 1 FOR UPDATE",
            &[],
        )
        .await
        .map_err(|source| InfraError::query("读取 audit 表尾", source))?;
    let previous = match latest {
        Some(row) => {
            let latest_id: Uuid = row.try_get("id").map_err(|source| {
                crate::db::RowDecodeError::column("audit_events", "id", source)
            })?;
            let latest_at: OffsetDateTime = row.try_get("created_at").map_err(|source| {
                crate::db::RowDecodeError::column("audit_events", "created_at", source)
            })?;
            if (event.created_at, event_uuid) <= (latest_at, latest_id) {
                return Err(InfraError::repository_invariant(
                    "audit_event_order_not_monotonic",
                ));
            }
            let text: Option<String> = row.try_get("row_hash").map_err(|source| {
                crate::db::RowDecodeError::column("audit_events", "row_hash", source)
            })?;
            match text {
                Some(text) => PrevRowHash::Linked(
                    Sha256Digest::parse_hex(&text)
                        .map_err(|_| InfraError::repository_invariant("audit_row_hash_invalid"))?,
                ),
                None => {
                    let linked_exists: bool = transaction
                        .query_one(
                            "SELECT EXISTS(SELECT 1 FROM public.audit_events \
                             WHERE row_hash IS NOT NULL)",
                            &[],
                        )
                        .await
                        .map_err(|source| InfraError::query("检查 audit chain 后置旧行", source))?
                        .try_get(0)
                        .map_err(|source| {
                            crate::db::RowDecodeError::column("audit_events", "exists", source)
                        })?;
                    if linked_exists {
                        return Err(InfraError::repository_invariant(
                            "unlinked_audit_row_after_chain_start",
                        ));
                    }
                    PrevRowHash::Genesis
                }
            }
        }
        None => PrevRowHash::Genesis,
    };
    let linked = StoredAuditRow::link(event.clone(), previous);
    let prev_hash = linked.prev_hash.map(|hash| hash.to_hex());
    let row_hash = linked
        .row_hash
        .expect("StoredAuditRow::link 必有 row_hash")
        .to_hex();
    let actor = event.actor.as_ref().map(|actor| actor.as_str());
    let target_id = event.target_id.as_ref().map(|id| id.as_str());
    let payload = event.payload.to_json();
    transaction
        .execute(
            "INSERT INTO public.audit_events \
             (id, actor_user_id, event_type, target_type, target_id, payload, created_at, \
              prev_hash, row_hash) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            &[
                &event_uuid,
                &actor,
                &event.event_type.as_str(),
                &event.target_kind.as_str(),
                &target_id,
                &payload,
                &event.created_at,
                &prev_hash,
                &row_hash,
            ],
        )
        .await
        .map_err(|source| InfraError::query("追加 audit event", source))?;

    if matches!(previous, PrevRowHash::Genesis) {
        let existing_checkpoints: i64 = transaction
            .query_one("SELECT count(*)::bigint FROM public.audit_checkpoints", &[])
            .await
            .map_err(|source| InfraError::query("检查 genesis checkpoint 前提", source))?
            .try_get(0)
            .map_err(|source| {
                crate::db::RowDecodeError::column("audit_checkpoints", "count", source)
            })?;
        if existing_checkpoints != 0 {
            return Err(InfraError::repository_invariant(
                "checkpoint_exists_without_audit_chain",
            ));
        }
        let unlinked_rows_before: i64 = transaction
            .query_one(
                "SELECT count(*)::bigint FROM public.audit_events \
                 WHERE row_hash IS NULL AND prev_hash IS NULL",
                &[],
            )
            .await
            .map_err(|source| InfraError::query("统计 chain 前旧审计行", source))?
            .try_get(0)
            .map_err(|source| crate::db::RowDecodeError::column("audit_events", "count", source))?;
        let checkpoint = AuditCheckpoint {
            sequence: 0,
            created_at: event.created_at,
            kind: AuditCheckpointKind::Genesis {
                genesis_event: event.id.clone(),
                genesis_row_hash: linked.row_hash.expect("link 必有 hash"),
                unlinked_rows_before: u64::try_from(unlinked_rows_before).map_err(|_| {
                    InfraError::repository_invariant("audit_unlinked_count_negative")
                })?,
            },
        };
        insert_checkpoint(transaction, &checkpoint, checkpoint_key).await?;
    } else {
        let checkpoints: i64 = transaction
            .query_one("SELECT count(*)::bigint FROM public.audit_checkpoints", &[])
            .await
            .map_err(|source| InfraError::query("确认 audit genesis checkpoint", source))?
            .try_get(0)
            .map_err(|source| {
                crate::db::RowDecodeError::column("audit_checkpoints", "count", source)
            })?;
        if checkpoints == 0 {
            return Err(InfraError::repository_invariant(
                "audit_chain_without_genesis_checkpoint",
            ));
        }
    }
    Ok(linked)
}

/// 在 audit 锁内铸造严格晚于当前表尾的 `(id, created_at)`；跨事务业务写用。
pub(crate) async fn next_event_coordinates(
    transaction: &tokio_postgres::Transaction<'_>,
) -> Result<(openbot_contracts::ids::AuditEventId, OffsetDateTime), InfraError> {
    transaction
        .query_one("SELECT pg_advisory_xact_lock($1)", &[&AUDIT_CHAIN_LOCK_KEY])
        .await
        .map_err(|source| InfraError::query("获取 audit coordinate 锁", source))?;
    let row = transaction
        .query_one(
            "SELECT gen_random_uuid() AS id, \
                    CASE WHEN max(created_at) >= clock_timestamp() \
                         THEN max(created_at) + interval '1 microsecond' \
                         ELSE clock_timestamp() END AS created_at \
             FROM public.audit_events",
            &[],
        )
        .await
        .map_err(|source| InfraError::query("铸造 audit event coordinate", source))?;
    let id: Uuid = row
        .try_get("id")
        .map_err(|source| crate::db::RowDecodeError::column("audit_events", "id", source))?;
    let created_at: OffsetDateTime = row.try_get("created_at").map_err(|source| {
        crate::db::RowDecodeError::column("audit_events", "created_at", source)
    })?;
    Ok((
        openbot_contracts::ids::AuditEventId::new(id.to_string()),
        created_at,
    ))
}

impl core::fmt::Debug for AuditEventRepo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AuditEventRepo").finish_non_exhaustive()
    }
}

async fn insert_checkpoint(
    transaction: &tokio_postgres::Transaction<'_>,
    checkpoint: &AuditCheckpoint,
    key: &[u8],
) -> Result<(), InfraError> {
    let signature = checkpoint
        .sign(key)
        .map_err(|_| InfraError::repository_invariant("audit_checkpoint_key_empty"))?;
    let columns = checkpoint_columns(&checkpoint.kind)?;
    let sequence = i64::try_from(checkpoint.sequence)
        .map_err(|_| InfraError::repository_invariant("audit_checkpoint_sequence_overflow"))?;
    let event_count = i64::try_from(columns.event_count)
        .map_err(|_| InfraError::repository_invariant("audit_checkpoint_count_overflow"))?;
    let unlinked = columns
        .unlinked
        .map(i64::try_from)
        .transpose()
        .map_err(|_| InfraError::repository_invariant("audit_unlinked_count_overflow"))?;
    let signature = signature_hex(&signature);
    transaction
        .execute(
            "INSERT INTO public.audit_checkpoints \
             (sequence,checkpoint_kind,first_event_id,first_row_hash,last_event_id,last_row_hash, \
              event_count,unlinked_rows_before,retention_days,signature,created_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            &[
                &sequence,
                &columns.kind,
                &columns.first_event,
                &columns.first_hash,
                &columns.last_event,
                &columns.last_hash,
                &event_count,
                &unlinked,
                &columns.retention,
                &signature,
                &checkpoint.created_at,
            ],
        )
        .await
        .map_err(|source| InfraError::query("追加 audit checkpoint", source))?;
    Ok(())
}

struct CheckpointColumns {
    kind: &'static str,
    first_event: String,
    first_hash: String,
    last_event: String,
    last_hash: String,
    event_count: u64,
    unlinked: Option<u64>,
    retention: Option<i32>,
}

fn checkpoint_columns(kind: &AuditCheckpointKind) -> Result<CheckpointColumns, InfraError> {
    Ok(match kind {
        AuditCheckpointKind::Genesis {
            genesis_event,
            genesis_row_hash,
            unlinked_rows_before,
        } => CheckpointColumns {
            kind: "genesis",
            first_event: genesis_event.as_str().to_owned(),
            first_hash: genesis_row_hash.to_hex(),
            last_event: genesis_event.as_str().to_owned(),
            last_hash: genesis_row_hash.to_hex(),
            event_count: 1,
            unlinked: Some(*unlinked_rows_before),
            retention: None,
        },
        AuditCheckpointKind::Periodic { segment } => CheckpointColumns {
            kind: "periodic",
            first_event: segment.first_event.as_str().to_owned(),
            first_hash: segment.first_row_hash.to_hex(),
            last_event: segment.last_event.as_str().to_owned(),
            last_hash: segment.last_row_hash.to_hex(),
            event_count: segment.event_count,
            unlinked: None,
            retention: None,
        },
        AuditCheckpointKind::Closure {
            segment,
            retention_days,
        } => {
            CheckpointColumns {
                kind: "closure",
                first_event: segment.first_event.as_str().to_owned(),
                first_hash: segment.first_row_hash.to_hex(),
                last_event: segment.last_event.as_str().to_owned(),
                last_hash: segment.last_row_hash.to_hex(),
                event_count: segment.event_count,
                unlinked: None,
                retention: Some(i32::try_from(retention_days.get()).map_err(|_| {
                    InfraError::repository_invariant("audit_retention_days_overflow")
                })?),
            }
        }
    })
}

async fn verify_checkpoint_segment(
    transaction: &tokio_postgres::Transaction<'_>,
    kind: &AuditCheckpointKind,
) -> Result<(), InfraError> {
    let segment = match kind {
        AuditCheckpointKind::Genesis { .. } => return Ok(()),
        AuditCheckpointKind::Periodic { segment }
        | AuditCheckpointKind::Closure { segment, .. } => segment,
    };
    let first_id = Uuid::parse_str(segment.first_event.as_str())
        .map_err(|_| InfraError::repository_invariant("audit_checkpoint_first_id_not_uuid"))?;
    let last_id = Uuid::parse_str(segment.last_event.as_str())
        .map_err(|_| InfraError::repository_invariant("audit_checkpoint_last_id_not_uuid"))?;
    let first = transaction
        .query_opt(
            "SELECT created_at,row_hash FROM public.audit_events WHERE id=$1 AND row_hash=$2",
            &[&first_id, &segment.first_row_hash.to_hex()],
        )
        .await
        .map_err(|source| InfraError::query("验证 checkpoint 首边界", source))?
        .ok_or_else(|| {
            InfraError::repository_invariant("audit_checkpoint_first_boundary_missing")
        })?;
    let last = transaction
        .query_opt(
            "SELECT created_at,row_hash FROM public.audit_events WHERE id=$1 AND row_hash=$2",
            &[&last_id, &segment.last_row_hash.to_hex()],
        )
        .await
        .map_err(|source| InfraError::query("验证 checkpoint 尾边界", source))?
        .ok_or_else(|| {
            InfraError::repository_invariant("audit_checkpoint_last_boundary_missing")
        })?;
    let first_at: OffsetDateTime = first.try_get("created_at").map_err(|source| {
        crate::db::RowDecodeError::column("audit_events", "created_at", source)
    })?;
    let last_at: OffsetDateTime = last.try_get("created_at").map_err(|source| {
        crate::db::RowDecodeError::column("audit_events", "created_at", source)
    })?;
    let count: i64 = transaction
        .query_one(
            "SELECT count(*)::bigint FROM public.audit_events \
             WHERE row_hash IS NOT NULL \
               AND (created_at,id) >= ($1,$2) AND (created_at,id) <= ($3,$4)",
            &[&first_at, &first_id, &last_at, &last_id],
        )
        .await
        .map_err(|source| InfraError::query("验证 checkpoint 区间条数", source))?
        .try_get(0)
        .map_err(|source| crate::db::RowDecodeError::column("audit_events", "count", source))?;
    if u64::try_from(count).ok() != Some(segment.event_count) {
        return Err(InfraError::repository_invariant(
            "audit_checkpoint_event_count_mismatch",
        ));
    }
    Ok(())
}

fn signature_hex(signature: &CheckpointSignature) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in signature.as_bytes() {
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    out
}
