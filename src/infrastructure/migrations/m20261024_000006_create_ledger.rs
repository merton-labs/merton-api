//! The transaction ledger (P3). Decimal values are stored as canonical text,
//! never as REAL (ADR-NUM-02). `kind` columns have no CHECK constraint
//! because their value sets grow in P4, and SQLite cannot alter a CHECK
//! without rebuilding the table; the Rust domain types validate them.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Transactions::Table)
                    .col(uuid_pk(Transactions::Id))
                    .col(ColumnDef::new(Transactions::PortfolioId).uuid().not_null())
                    .col(ColumnDef::new(Transactions::Kind).string_len(32).not_null())
                    .col(ColumnDef::new(Transactions::TradeDate).date().not_null())
                    .col(
                        ColumnDef::new(Transactions::SortOrder)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(Transactions::InstrumentId).uuid())
                    .col(ColumnDef::new(Transactions::CashAccountId).uuid())
                    .col(ColumnDef::new(Transactions::Quantity).string_len(64))
                    .col(ColumnDef::new(Transactions::Price).string_len(64))
                    .col(ColumnDef::new(Transactions::Fee).string_len(64))
                    .col(ColumnDef::new(Transactions::Tax).string_len(64))
                    .col(ColumnDef::new(Transactions::Amount).string_len(64))
                    .col(ColumnDef::new(Transactions::Note).string_len(500))
                    .col(timestamp(Transactions::CreatedAt))
                    .col(timestamp(Transactions::UpdatedAt))
                    .foreign_key(&mut restrict(
                        "fk_transactions_portfolio_id",
                        Transactions::Table,
                        Transactions::PortfolioId,
                        Portfolios::Table,
                        Portfolios::Id,
                    ))
                    .foreign_key(&mut restrict(
                        "fk_transactions_instrument_id",
                        Transactions::Table,
                        Transactions::InstrumentId,
                        Instruments::Table,
                        Instruments::Id,
                    ))
                    .foreign_key(&mut restrict(
                        "fk_transactions_cash_account_id",
                        Transactions::Table,
                        Transactions::CashAccountId,
                        CashAccounts::Table,
                        CashAccounts::Id,
                    ))
                    .to_owned(),
            )
            .await?;
        // Ledger order: date, then sort_order, then the time-ordered id.
        index(
            manager,
            "idx_transactions_ledger_order",
            Transactions::Table,
            [
                Transactions::PortfolioId,
                Transactions::TradeDate,
                Transactions::SortOrder,
                Transactions::Id,
            ],
        )
        .await?;
        index(
            manager,
            "idx_transactions_instrument_id",
            Transactions::Table,
            [Transactions::InstrumentId],
        )
        .await?;
        index(
            manager,
            "idx_transactions_cash_account_id",
            Transactions::Table,
            [Transactions::CashAccountId],
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table(PositionMovements::Table)
                    .col(uuid_pk(PositionMovements::Id))
                    .col(
                        ColumnDef::new(PositionMovements::TransactionId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PositionMovements::PortfolioId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PositionMovements::InstrumentId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PositionMovements::Kind)
                            .string_len(32)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PositionMovements::Quantity)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PositionMovements::Amount)
                            .string_len(64)
                            .not_null(),
                    )
                    .foreign_key(&mut restrict(
                        "fk_position_movements_transaction_id",
                        PositionMovements::Table,
                        PositionMovements::TransactionId,
                        Transactions::Table,
                        Transactions::Id,
                    ))
                    .foreign_key(&mut restrict(
                        "fk_position_movements_portfolio_id",
                        PositionMovements::Table,
                        PositionMovements::PortfolioId,
                        Portfolios::Table,
                        Portfolios::Id,
                    ))
                    .foreign_key(&mut restrict(
                        "fk_position_movements_instrument_id",
                        PositionMovements::Table,
                        PositionMovements::InstrumentId,
                        Instruments::Table,
                        Instruments::Id,
                    ))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "idx_position_movements_portfolio_instrument",
            PositionMovements::Table,
            [
                PositionMovements::PortfolioId,
                PositionMovements::InstrumentId,
            ],
        )
        .await?;
        index(
            manager,
            "idx_position_movements_transaction_id",
            PositionMovements::Table,
            [PositionMovements::TransactionId],
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table(CashMovements::Table)
                    .col(uuid_pk(CashMovements::Id))
                    .col(
                        ColumnDef::new(CashMovements::TransactionId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(CashMovements::PortfolioId).uuid().not_null())
                    .col(
                        ColumnDef::new(CashMovements::CashAccountId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CashMovements::AmountDelta)
                            .string_len(64)
                            .not_null(),
                    )
                    .foreign_key(&mut restrict(
                        "fk_cash_movements_transaction_id",
                        CashMovements::Table,
                        CashMovements::TransactionId,
                        Transactions::Table,
                        Transactions::Id,
                    ))
                    .foreign_key(&mut restrict(
                        "fk_cash_movements_portfolio_id",
                        CashMovements::Table,
                        CashMovements::PortfolioId,
                        Portfolios::Table,
                        Portfolios::Id,
                    ))
                    .foreign_key(&mut restrict(
                        "fk_cash_movements_cash_account_id",
                        CashMovements::Table,
                        CashMovements::CashAccountId,
                        CashAccounts::Table,
                        CashAccounts::Id,
                    ))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "idx_cash_movements_cash_account_id",
            CashMovements::Table,
            [CashMovements::CashAccountId],
        )
        .await?;
        index(
            manager,
            "idx_cash_movements_transaction_id",
            CashMovements::Table,
            [CashMovements::TransactionId],
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            CashMovements::Table.into_iden(),
            PositionMovements::Table.into_iden(),
            Transactions::Table.into_iden(),
        ] {
            manager
                .drop_table(Table::drop().table(table).to_owned())
                .await?;
        }
        Ok(())
    }
}

fn uuid_pk(column: impl IntoIden) -> ColumnDef {
    ColumnDef::new(column)
        .uuid()
        .not_null()
        .primary_key()
        .to_owned()
}

fn timestamp(column: impl IntoIden) -> ColumnDef {
    ColumnDef::new(column)
        .timestamp_with_time_zone()
        .not_null()
        .to_owned()
}

fn restrict(
    name: &str,
    from_table: impl IntoIden,
    from_column: impl IntoIden,
    to_table: impl IntoIden,
    to_column: impl IntoIden,
) -> ForeignKeyCreateStatement {
    ForeignKey::create()
        .name(name)
        .from(from_table, from_column)
        .to(to_table, to_column)
        .on_delete(ForeignKeyAction::Restrict)
        .to_owned()
}

async fn index<T: IntoIden + Copy>(
    manager: &SchemaManager<'_>,
    name: &str,
    table: impl IntoIden,
    columns: impl IntoIterator<Item = T>,
) -> Result<(), DbErr> {
    let mut statement = Index::create();
    statement.name(name).table(table);
    for column in columns {
        statement.col(column);
    }
    manager.create_index(statement.to_owned()).await
}

#[derive(DeriveIden, Clone, Copy)]
enum Transactions {
    Table,
    Id,
    PortfolioId,
    Kind,
    TradeDate,
    SortOrder,
    InstrumentId,
    CashAccountId,
    Quantity,
    Price,
    Fee,
    Tax,
    Amount,
    Note,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden, Clone, Copy)]
enum PositionMovements {
    Table,
    Id,
    TransactionId,
    PortfolioId,
    InstrumentId,
    Kind,
    Quantity,
    Amount,
}

#[derive(DeriveIden, Clone, Copy)]
enum CashMovements {
    Table,
    Id,
    TransactionId,
    PortfolioId,
    CashAccountId,
    AmountDelta,
}

#[derive(DeriveIden, Clone, Copy)]
enum Portfolios {
    Table,
    Id,
}

#[derive(DeriveIden, Clone, Copy)]
enum Instruments {
    Table,
    Id,
}

#[derive(DeriveIden, Clone, Copy)]
enum CashAccounts {
    Table,
    Id,
}
