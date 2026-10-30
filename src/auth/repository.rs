//! Database access for users and sessions.

use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, QueryFilter,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::infrastructure::entities::{user, user_session};

pub struct NewUser<'a> {
    pub id: Uuid,
    pub email: &'a str,
    pub normalized_email: &'a str,
    pub password_hash: &'a str,
    pub now: OffsetDateTime,
}

pub async fn insert_user(db: &impl ConnectionTrait, new: NewUser<'_>) -> Result<(), DbErr> {
    let model = user::ActiveModel {
        id: Set(new.id),
        email: Set(new.email.to_owned()),
        normalized_email: Set(new.normalized_email.to_owned()),
        password_hash: Set(new.password_hash.to_owned()),
        created_at: Set(new.now),
        updated_at: Set(new.now),
    };
    user::Entity::insert(model)
        .exec_without_returning(db)
        .await?;
    Ok(())
}

pub async fn find_user_by_normalized_email(
    db: &impl ConnectionTrait,
    normalized_email: &str,
) -> Result<Option<user::Model>, DbErr> {
    user::Entity::find()
        .filter(user::Column::NormalizedEmail.eq(normalized_email))
        .one(db)
        .await
}

pub struct NewSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: Vec<u8>,
    pub now: OffsetDateTime,
    pub idle_expires_at: OffsetDateTime,
    pub absolute_expires_at: OffsetDateTime,
}

pub async fn insert_session(db: &impl ConnectionTrait, new: NewSession) -> Result<(), DbErr> {
    let model = user_session::ActiveModel {
        id: Set(new.id),
        user_id: Set(new.user_id),
        token_hash: Set(new.token_hash),
        created_at: Set(new.now),
        last_activity_at: Set(new.now),
        idle_expires_at: Set(new.idle_expires_at),
        absolute_expires_at: Set(new.absolute_expires_at),
        revoked_at: Set(None),
    };
    user_session::Entity::insert(model)
        .exec_without_returning(db)
        .await?;
    Ok(())
}

pub async fn find_session_with_user(
    db: &impl ConnectionTrait,
    token_hash: Vec<u8>,
) -> Result<Option<(user_session::Model, user::Model)>, DbErr> {
    let found = user_session::Entity::find()
        .filter(user_session::Column::TokenHash.eq(token_hash))
        .find_also_related(user::Entity)
        .one(db)
        .await?;
    Ok(found.and_then(|(session, user)| Some((session, user?))))
}

pub async fn record_session_activity(
    db: &impl ConnectionTrait,
    session_id: Uuid,
    now: OffsetDateTime,
    idle_expires_at: OffsetDateTime,
) -> Result<(), DbErr> {
    user_session::Entity::update_many()
        .col_expr(user_session::Column::LastActivityAt, Expr::value(now))
        .col_expr(
            user_session::Column::IdleExpiresAt,
            Expr::value(idle_expires_at),
        )
        .filter(user_session::Column::Id.eq(session_id))
        .filter(user_session::Column::RevokedAt.is_null())
        .exec(db)
        .await?;
    Ok(())
}

/// Deletes every session, of any user, that was revoked or expired before
/// `cutoff`. Returns the number of rows deleted.
pub async fn delete_sessions_ended_before(
    db: &impl ConnectionTrait,
    cutoff: OffsetDateTime,
) -> Result<u64, DbErr> {
    let ended = Condition::any()
        .add(user_session::Column::RevokedAt.lt(cutoff))
        .add(user_session::Column::IdleExpiresAt.lt(cutoff))
        .add(user_session::Column::AbsoluteExpiresAt.lt(cutoff));
    let result = user_session::Entity::delete_many()
        .filter(ended)
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

pub async fn revoke_session(
    db: &impl ConnectionTrait,
    column: user_session::Column,
    value: impl Into<sea_orm::Value>,
    now: OffsetDateTime,
) -> Result<(), DbErr> {
    user_session::Entity::update_many()
        .col_expr(user_session::Column::RevokedAt, Expr::value(now))
        .filter(column.eq(value))
        .filter(user_session::Column::RevokedAt.is_null())
        .exec(db)
        .await?;
    Ok(())
}
