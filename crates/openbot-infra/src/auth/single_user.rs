//! 单用户模式唯一 principal 的 PostgreSQL 持久化。
//!
//! 固定 actor id 不能随重写改名：上游用它把 thread/memory 归到同一个人，换 id 会把旧数据
//! 变成孤儿。初始化只在显式启用时访问数据库；启用后以一个事务恢复 canonical user 字段并
//! 通过领域 [`plan_set_role`] 把角色集合收敛为唯一 admin。

use deadpool_postgres::Pool;
use openbot_contracts::auth::Role;
use openbot_contracts::ids::ActorId;
use openbot_domain::identity::roles::plan_set_role;

use crate::db::InfraError;
use crate::repo::people_admin::apply_role_plan;

/// 与固定上游 `DEV_ACTOR.id` 相同；既有数据兼容键。
pub const SINGLE_USER_ACTOR_ID: &str = "dev-local-user";

/// 与固定上游 `DEV_ACTOR.email` 相同。
pub const SINGLE_USER_EMAIL: &str = "dev@openbot.local";

/// 上游 actor 没有另设 name，持久化时回落到 email。
pub const SINGLE_USER_NAME: &str = SINGLE_USER_EMAIL;

/// 按显式开关初始化单用户 principal。
///
/// `enabled=false` 在取连接之前返回 `Ok(false)`；`true` 时恢复 canonical id/email/name，保留
/// 已有 `auth_generation`，并把 `user_roles` 原子收敛为 admin 一行。
///
/// # Errors
///
/// 取连接、事务、唯一约束或写入失败均返回脱敏 [`InfraError`]；canonical email 已被另一用户
/// 占用时因此响亮失败，不会接管或删除对方。
pub async fn initialize_single_user(pool: &Pool, enabled: bool) -> Result<bool, InfraError> {
    if !enabled {
        return Ok(false);
    }

    let mut client = pool
        .get()
        .await
        .map_err(|error| InfraError::connect("取单用户初始化连接", error))?;
    let transaction = client
        .transaction()
        .await
        .map_err(|error| InfraError::query("开始单用户初始化事务", error))?;
    let affected = transaction
        .execute(
            "INSERT INTO public.users \
             (id,email,name,email_verified,groups,auth_generation) \
             VALUES($1,$2,$3,false,'{}'::text[],0) \
             ON CONFLICT(id) DO UPDATE SET \
               email=EXCLUDED.email,name=EXCLUDED.name,updated_at=clock_timestamp()",
            &[&SINGLE_USER_ACTOR_ID, &SINGLE_USER_EMAIL, &SINGLE_USER_NAME],
        )
        .await
        .map_err(|error| InfraError::query("恢复单用户 canonical identity", error))?;
    if affected != 1 {
        return Err(InfraError::repository_invariant(
            "single_user_upsert_count_invalid",
        ));
    }

    let actor = ActorId::new(SINGLE_USER_ACTOR_ID);
    // §6.1 直接裁决单用户唯一 principal 是 admin；多用户新身份才由 seed_role 判 floor。
    apply_role_plan(&transaction, &plan_set_role(&actor, Role::Admin)).await?;

    transaction
        .commit()
        .await
        .map_err(|error| InfraError::query("提交单用户初始化事务", error))?;
    Ok(true)
}
