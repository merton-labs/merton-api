//! Cash transfers (P4) name a second cash account. The column is added with
//! an inline `REFERENCES … ON DELETE RESTRICT`, because SQLite cannot add a
//! foreign key to an existing table through `ALTER TABLE … ADD CONSTRAINT`;
//! the inline form is also valid PostgreSQL.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Transactions::Table)
                    .add_column(
                        ColumnDef::new(Transactions::DestinationCashAccountId)
                            .uuid()
                            .null()
                            .extra(r#"REFERENCES "cash_accounts" ("id") ON DELETE RESTRICT"#),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_transactions_destination_cash_account_id")
                    .table(Transactions::Table)
                    .col(Transactions::DestinationCashAccountId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx_transactions_destination_cash_account_id")
                    .table(Transactions::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Transactions::Table)
                    .drop_column(Transactions::DestinationCashAccountId)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Transactions {
    Table,
    DestinationCashAccountId,
}
