//! Ledger queries. Reads require, in SQL, that the portfolio belongs to the
//! caller. Writes run inside a transaction that has already locked the
//! owned, active portfolio (see `service`).

use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Select,
};
use uuid::Uuid;

use crate::auth::UserId;
use crate::infrastructure::entities::{cash_movement, position_movement, transaction};
use crate::portfolios::repository::owned_portfolio;

fn in_owned_portfolio(
    owner: UserId,
    portfolio_id: Uuid,
    instrument_id: Option<Uuid>,
) -> Select<transaction::Entity> {
    let query = transaction::Entity::find()
        .filter(transaction::Column::PortfolioId.eq(portfolio_id))
        .filter(Expr::exists(owned_portfolio(owner, portfolio_id, false)));
    match instrument_id {
        Some(id) => query.filter(transaction::Column::InstrumentId.eq(id)),
        None => query,
    }
}

/// Ledger order: trade date, then `sort_order`, then the time-ordered id.
fn in_ledger_order(query: Select<transaction::Entity>) -> Select<transaction::Entity> {
    query
        .order_by_asc(transaction::Column::TradeDate)
        .order_by_asc(transaction::Column::SortOrder)
        .order_by_asc(transaction::Column::Id)
}

pub async fn find(
    db: &impl ConnectionTrait,
    owner: UserId,
    portfolio_id: Uuid,
    id: Uuid,
) -> Result<Option<transaction::Model>, DbErr> {
    in_owned_portfolio(owner, portfolio_id, None)
        .filter(transaction::Column::Id.eq(id))
        .one(db)
        .await
}

pub async fn list(
    db: &impl ConnectionTrait,
    owner: UserId,
    portfolio_id: Uuid,
    instrument_id: Option<Uuid>,
    limit: u64,
    offset: u64,
) -> Result<Vec<transaction::Model>, DbErr> {
    in_ledger_order(in_owned_portfolio(owner, portfolio_id, instrument_id))
        .limit(limit)
        .offset(offset)
        .all(db)
        .await
}

pub async fn count(
    db: &impl ConnectionTrait,
    owner: UserId,
    portfolio_id: Uuid,
    instrument_id: Option<Uuid>,
) -> Result<u64, DbErr> {
    in_owned_portfolio(owner, portfolio_id, instrument_id)
        .count(db)
        .await
}

pub async fn any_in_portfolio(
    db: &impl ConnectionTrait,
    portfolio_id: Uuid,
) -> Result<bool, DbErr> {
    let count = transaction::Entity::find()
        .filter(transaction::Column::PortfolioId.eq(portfolio_id))
        .limit(1)
        .count(db)
        .await?;
    Ok(count > 0)
}

pub async fn insert(db: &impl ConnectionTrait, model: transaction::Model) -> Result<(), DbErr> {
    transaction::Entity::insert(active(model))
        .exec_without_returning(db)
        .await?;
    Ok(())
}

/// Replaces every user-editable column of an existing transaction.
pub async fn replace(db: &impl ConnectionTrait, model: transaction::Model) -> Result<(), DbErr> {
    let mut row = active(model);
    row.created_at = sea_orm::ActiveValue::NotSet;
    row.portfolio_id = sea_orm::ActiveValue::NotSet;
    transaction::Entity::update(row).exec(db).await?;
    Ok(())
}

pub async fn delete(db: &impl ConnectionTrait, id: Uuid) -> Result<(), DbErr> {
    transaction::Entity::delete_by_id(id).exec(db).await?;
    Ok(())
}

fn active(model: transaction::Model) -> transaction::ActiveModel {
    transaction::ActiveModel {
        id: Set(model.id),
        portfolio_id: Set(model.portfolio_id),
        kind: Set(model.kind),
        trade_date: Set(model.trade_date),
        sort_order: Set(model.sort_order),
        instrument_id: Set(model.instrument_id),
        cash_account_id: Set(model.cash_account_id),
        destination_cash_account_id: Set(model.destination_cash_account_id),
        quantity: Set(model.quantity),
        ratio: Set(model.ratio),
        price: Set(model.price),
        fee: Set(model.fee),
        tax: Set(model.tax),
        amount: Set(model.amount),
        destination_amount: Set(model.destination_amount),
        cash_in_lieu_quantity: Set(model.cash_in_lieu_quantity),
        note: Set(model.note),
        created_at: Set(model.created_at),
        updated_at: Set(model.updated_at),
    }
}

pub async fn insert_position_movement(
    db: &impl ConnectionTrait,
    model: position_movement::Model,
) -> Result<(), DbErr> {
    let row = position_movement::ActiveModel {
        id: Set(model.id),
        transaction_id: Set(model.transaction_id),
        portfolio_id: Set(model.portfolio_id),
        instrument_id: Set(model.instrument_id),
        kind: Set(model.kind),
        quantity: Set(model.quantity),
        amount: Set(model.amount),
    };
    position_movement::Entity::insert(row)
        .exec_without_returning(db)
        .await?;
    Ok(())
}

pub async fn insert_cash_movement(
    db: &impl ConnectionTrait,
    model: cash_movement::Model,
) -> Result<(), DbErr> {
    let row = cash_movement::ActiveModel {
        id: Set(model.id),
        transaction_id: Set(model.transaction_id),
        portfolio_id: Set(model.portfolio_id),
        cash_account_id: Set(model.cash_account_id),
        amount_delta: Set(model.amount_delta),
    };
    cash_movement::Entity::insert(row)
        .exec_without_returning(db)
        .await?;
    Ok(())
}

pub async fn delete_movements(
    db: &impl ConnectionTrait,
    transaction_id: Uuid,
) -> Result<(), DbErr> {
    position_movement::Entity::delete_many()
        .filter(position_movement::Column::TransactionId.eq(transaction_id))
        .exec(db)
        .await?;
    cash_movement::Entity::delete_many()
        .filter(cash_movement::Column::TransactionId.eq(transaction_id))
        .exec(db)
        .await?;
    Ok(())
}

/// Every position movement of one instrument in one portfolio, in ledger
/// order, with the transaction it belongs to.
pub async fn position_history(
    db: &impl ConnectionTrait,
    portfolio_id: Uuid,
    instrument_id: Uuid,
) -> Result<Vec<(position_movement::Model, transaction::Model)>, DbErr> {
    let rows = position_movement::Entity::find()
        .filter(position_movement::Column::PortfolioId.eq(portfolio_id))
        .filter(position_movement::Column::InstrumentId.eq(instrument_id))
        .find_also_related(transaction::Entity)
        .order_by_asc(transaction::Column::TradeDate)
        .order_by_asc(transaction::Column::SortOrder)
        .order_by_asc(transaction::Column::Id)
        .order_by_asc(position_movement::Column::Id)
        .all(db)
        .await?;
    rows.into_iter()
        .map(|(movement, transaction)| {
            transaction
                .map(|t| (movement, t))
                .ok_or_else(|| DbErr::RecordNotFound("movement without transaction".to_owned()))
        })
        .collect()
}
