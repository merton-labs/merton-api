//! Fees and taxes leave cost basis (ADR-LEDG-11, superseding ADR-LEDG-10).
//!
//! The position movement of every `BUY` and `SELL` is rewritten to its
//! transaction's quantity × price. Before, a buy stored quantity × price +
//! fee + tax and a sell stored quantity × price − fee − tax. Cash movements
//! are unchanged. `down` restores the old amounts from the same inputs.
//!
//! The product uses the ledger's exact multiplication, so a migrated row
//! holds the same text a new write would.

use std::str::FromStr;

use rust_decimal::Decimal;
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::ConnectionTrait;
use sea_orm_migration::sea_orm::prelude::Uuid;

use crate::ledger::decimal::{exact_add, exact_mul, exact_sub};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        rewrite(manager, |gross, _charges, _buy| Some(gross)).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        rewrite(manager, |gross, charges, buy| {
            if buy {
                exact_add(gross, charges)
            } else {
                exact_sub(gross, charges)
            }
        })
        .await
    }
}

/// Sets each trade movement's amount to `amount(gross, fee + tax, is_buy)`.
async fn rewrite(
    manager: &SchemaManager<'_>,
    amount: impl Fn(Decimal, Decimal, bool) -> Option<Decimal>,
) -> Result<(), DbErr> {
    let db = manager.get_connection();
    let select = Query::select()
        .expr_as(
            Expr::col((PositionMovements::Table, PositionMovements::Id)),
            Alias::new("id"),
        )
        .columns([
            (Transactions::Table, Transactions::Kind),
            (Transactions::Table, Transactions::Quantity),
            (Transactions::Table, Transactions::Price),
            (Transactions::Table, Transactions::Fee),
            (Transactions::Table, Transactions::Tax),
        ])
        .from(PositionMovements::Table)
        .inner_join(
            Transactions::Table,
            Expr::col((Transactions::Table, Transactions::Id))
                .equals((PositionMovements::Table, PositionMovements::TransactionId)),
        )
        .and_where(Expr::col((Transactions::Table, Transactions::Kind)).is_in(["BUY", "SELL"]))
        .to_owned();
    let rows = db.query_all(&select).await?;
    for row in rows {
        let id: Uuid = row.try_get("", "id")?;
        let kind: String = row.try_get("", "kind")?;
        let decimal = |column: &str| -> Result<Decimal, DbErr> {
            let text: Option<String> = row.try_get("", column)?;
            Decimal::from_str(text.as_deref().unwrap_or("0"))
                .map_err(|_| DbErr::Migration(format!("invalid decimal in transactions.{column}")))
        };
        let out_of_range = || DbErr::Migration("trade amount out of range".to_owned());
        let gross = exact_mul(decimal("quantity")?, decimal("price")?).ok_or_else(out_of_range)?;
        let charges = exact_add(decimal("fee")?, decimal("tax")?).ok_or_else(out_of_range)?;
        let value = amount(gross, charges, kind == "BUY").ok_or_else(out_of_range)?;
        manager
            .exec_stmt(
                Query::update()
                    .table(PositionMovements::Table)
                    .value(PositionMovements::Amount, value.to_string())
                    .and_where(Expr::col(PositionMovements::Id).eq(id))
                    .to_owned(),
            )
            .await?;
    }
    Ok(())
}

#[derive(DeriveIden, Clone, Copy)]
enum PositionMovements {
    Table,
    Id,
    TransactionId,
    Amount,
}

#[derive(DeriveIden, Clone, Copy)]
enum Transactions {
    Table,
    Id,
    Kind,
    Quantity,
    Price,
    Fee,
    Tax,
}
