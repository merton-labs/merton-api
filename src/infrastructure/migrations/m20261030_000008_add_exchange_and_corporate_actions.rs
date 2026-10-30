//! FX exchanges and corporate actions (P4) record three more inputs on a
//! transaction, all nullable decimal text (ADR-NUM-02):
//!
//! - `destination_amount`: the amount an FX exchange receives;
//! - `ratio`: a split's `new:old` ratio;
//! - `cash_in_lieu_quantity`: fractional shares paid out in cash.
//!
//! SQLite allows one column per `ALTER TABLE`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const COLUMNS: [Transactions; 3] = [
    Transactions::DestinationAmount,
    Transactions::Ratio,
    Transactions::CashInLieuQuantity,
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in COLUMNS {
            manager
                .alter_table(
                    Table::alter()
                        .table(Transactions::Table)
                        .add_column(ColumnDef::new(column).string_len(64).null())
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in COLUMNS {
            manager
                .alter_table(
                    Table::alter()
                        .table(Transactions::Table)
                        .drop_column(column)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden, Clone, Copy)]
enum Transactions {
    Table,
    DestinationAmount,
    Ratio,
    CashInLieuQuantity,
}
