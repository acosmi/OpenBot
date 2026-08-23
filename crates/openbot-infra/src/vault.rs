//! Vault credential repository（v3 §6.4）。
//!
//! 与普通表 CRUD 不同，本类型刻意没有物理 delete：撤销只写 `revoked_at`，轮换走
//! `(id, expected_key_id)` compare-and-swap，防止两个轮换者互相覆盖。密文本体仍由
//! `db::tables::credentials::Row` 的手写 Debug 脱敏；本层的错误只保留 SQLSTATE/标识符。

use deadpool_postgres::Pool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::tables::credentials;
use crate::db::{InfraError, tables};
use crate::repo::common::{RepoCore, columns_sql};

/// `credentials` 的唯一持久化入口。
#[derive(Clone)]
pub struct CredentialRepo {
    core: RepoCore<credentials::Row>,
}

impl CredentialRepo {
    /// 用调用方提供的池构造。
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self {
            core: RepoCore::new(pool),
        }
    }

    /// 插入一份已经由领域 Vault 封装好的密文行。
    pub async fn insert(&self, row: &credentials::Row) -> Result<credentials::Row, InfraError> {
        self.core.insert(row).await
    }

    /// 按 id 读取（含已撤销，供审计/轮换恢复）。
    pub async fn find_by_id(&self, id: &Uuid) -> Result<Option<credentials::Row>, InfraError> {
        self.core.find("\"id\" = $1", &[&id]).await
    }

    /// 只读取未撤销凭据；已撤销与不存在对普通消费方同为 `None`。
    pub async fn find_active_by_id(
        &self,
        id: &Uuid,
    ) -> Result<Option<credentials::Row>, InfraError> {
        self.core
            .find("\"id\" = $1 AND \"revoked_at\" IS NULL", &[&id])
            .await
    }

    /// 稳定列出全部凭据元数据/密文行（Debug 仍脱敏）。
    pub async fn list_all(&self) -> Result<Vec<credentials::Row>, InfraError> {
        self.core.list("\"id\"").await
    }

    /// 撤销一次；不存在或已撤销返回 `None`，不会改写首次撤销时刻。
    pub async fn revoke(
        &self,
        id: &Uuid,
        revoked_at: OffsetDateTime,
    ) -> Result<Option<credentials::Row>, InfraError> {
        let sql = format!(
            "UPDATE public.credentials SET revoked_at=$2, updated_at=$2 \
             WHERE id=$1 AND revoked_at IS NULL RETURNING {}",
            columns_sql::<credentials::Row>(),
        );
        let client = self
            .core
            .pool()
            .get()
            .await
            .map_err(|source| InfraError::connect("为 CredentialRepo 撤销获取连接", source))?;
        let row = client
            .query_opt(&sql, &[&id, &revoked_at])
            .await
            .map_err(|source| InfraError::query("撤销 credential", source))?;
        row.as_ref()
            .map(credentials::Row::try_from)
            .transpose()
            .map_err(Into::into)
    }

    /// 用旧 key id 做 compare-and-swap 轮换；竞争失败返回 `None`，调用方必须重读后裁决。
    pub async fn rotate_if_current(
        &self,
        id: &Uuid,
        expected_key_id: &str,
        encrypted_value: &str,
        new_key_id: &str,
        metadata: &serde_json::Value,
        updated_at: OffsetDateTime,
    ) -> Result<Option<credentials::Row>, InfraError> {
        let sql = format!(
            "UPDATE public.credentials \
             SET encrypted_value=$3, key_id=$4, metadata=$5, updated_at=$6 \
             WHERE id=$1 AND key_id=$2 AND revoked_at IS NULL RETURNING {}",
            columns_sql::<tables::credentials::Row>(),
        );
        let client = self
            .core
            .pool()
            .get()
            .await
            .map_err(|source| InfraError::connect("为 CredentialRepo 轮换获取连接", source))?;
        let row = client
            .query_opt(
                &sql,
                &[
                    &id,
                    &expected_key_id,
                    &encrypted_value,
                    &new_key_id,
                    &metadata,
                    &updated_at,
                ],
            )
            .await
            .map_err(|source| InfraError::query("compare-and-swap 轮换 credential", source))?;
        row.as_ref()
            .map(credentials::Row::try_from)
            .transpose()
            .map_err(Into::into)
    }
}

impl core::fmt::Debug for CredentialRepo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CredentialRepo").finish_non_exhaustive()
    }
}
