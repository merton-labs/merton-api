//! Cash account queries. Each one requires, in the same SQL statement, that
//! the parent portfolio belongs to the owner; writes also require it to be
//! `ACTIVE`, so an archive cannot race a change.

use sea_orm::sea_query::{Expr, Query, SelectStatement};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Select,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::UserId;
use crate::infrastructure::entities::cash_account::{self, Column, Entity};
use crate::infrastructure::entities::portfolio;
use crate::portfolios::PortfolioStatus;

/// `SELECT 1 FROM portfolios WHERE id = ? AND owner_user_id = ? [AND status = 'ACTIVE']`.
fn owned_portfolio(owner: UserId, portfolio_id: Uuid, active_only: bool) -> SelectStatement {
    let mut select = Query::select();
    select
        .expr(Expr::value(1))
        .from(portfolio::Entity)
        .and_where(portfolio::Column::Id.eq(portfolio_id))
        .and_where(portfolio::Column::OwnerUserId.eq(owner.as_uuid()));
    if active_only {
        select.and_where(portfolio::Column::Status.eq(PortfolioStatus::Active.as_str()));
    }
    select
}

fn in_owned_portfolio(owner: UserId, portfolio_id: Uuid) -> Select<Entity> {
    Entity::find()
        .filter(Column::PortfolioId.eq(portfolio_id))
        .filter(Expr::exists(owned_portfolio(owner, portfolio_id, false)))
}

/// Inserts the account only if its portfolio is owned and active; returns
/// the number of rows inserted.
pub async fn insert_if_active(
    db: &impl ConnectionTrait,
    owner: UserId,
    model: cash_account::Model,
) -> Result<u64, DbErr> {
    let mut values = owned_portfolio(owner, model.portfolio_id, true);
    values.clear_selects().exprs([
        Expr::value(model.id),
        Expr::value(model.portfolio_id),
        Expr::value(model.name),
        Expr::value(model.currency),
        Expr::value(model.created_at),
        Expr::value(model.updated_at),
    ]);
    let mut insert = Query::insert();
    insert
        .into_table(Entity)
        .columns([
            Column::Id,
            Column::PortfolioId,
            Column::Name,
            Column::Currency,
            Column::CreatedAt,
            Column::UpdatedAt,
        ])
        .select_from(values)
        .map_err(|err| DbErr::Custom(err.to_string()))?;
    Ok(db.execute(&insert).await?.rows_affected())
}

pub async fn find(
    db: &impl ConnectionTrait,
    owner: UserId,
    portfolio_id: Uuid,
    id: Uuid,
) -> Result<Option<cash_account::Model>, DbErr> {
    in_owned_portfolio(owner, portfolio_id)
        .filter(Column::Id.eq(id))
        .one(db)
        .await
}

pub async fn list(
    db: &impl ConnectionTrait,
    owner: UserId,
    portfolio_id: Uuid,
    limit: u64,
    offset: u64,
) -> Result<Vec<cash_account::Model>, DbErr> {
    in_owned_portfolio(owner, portfolio_id)
        .order_by_asc(Column::CreatedAt)
        .order_by_asc(Column::Id)
        .limit(limit)
        .offset(offset)
        .all(db)
        .await
}

pub async fn count(
    db: &impl ConnectionTrait,
    owner: UserId,
    portfolio_id: Uuid,
) -> Result<u64, DbErr> {
    in_owned_portfolio(owner, portfolio_id).count(db).await
}

pub async fn rename_if_active(
    db: &impl ConnectionTrait,
    owner: UserId,
    portfolio_id: Uuid,
    id: Uuid,
    name: &str,
    now: OffsetDateTime,
) -> Result<u64, DbErr> {
    let result = Entity::update_many()
        .col_expr(Column::Name, Expr::value(name))
        .col_expr(Column::UpdatedAt, Expr::value(now))
        .filter(Column::Id.eq(id))
        .filter(Column::PortfolioId.eq(portfolio_id))
        .filter(Expr::exists(owned_portfolio(owner, portfolio_id, true)))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

pub async fn delete_if_active(
    db: &impl ConnectionTrait,
    owner: UserId,
    portfolio_id: Uuid,
    id: Uuid,
) -> Result<u64, DbErr> {
    let result = Entity::delete_many()
        .filter(Column::Id.eq(id))
        .filter(Column::PortfolioId.eq(portfolio_id))
        .filter(Expr::exists(owned_portfolio(owner, portfolio_id, true)))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// Removes every account of a portfolio, for portfolio deletion. The caller
/// must already have verified ownership inside the same transaction.
pub(crate) async fn delete_all_in_portfolio(
    db: &impl ConnectionTrait,
    portfolio_id: Uuid,
) -> Result<u64, DbErr> {
    let result = Entity::delete_many()
        .filter(Column::PortfolioId.eq(portfolio_id))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}
