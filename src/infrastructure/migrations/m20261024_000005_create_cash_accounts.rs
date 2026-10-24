use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(CashAccounts::Table)
                    .col(
                        ColumnDef::new(CashAccounts::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(CashAccounts::PortfolioId).uuid().not_null())
                    .col(
                        ColumnDef::new(CashAccounts::Name)
                            .string_len(100)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CashAccounts::Currency)
                            .string_len(3)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CashAccounts::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(CashAccounts::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    // Portfolio deletion removes unused cash accounts explicitly;
                    // the database never cascades (ADR-PORT-04).
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_cash_accounts_portfolio_id")
                            .from(CashAccounts::Table, CashAccounts::PortfolioId)
                            .to(Portfolios::Table, Portfolios::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_cash_accounts_portfolio_created")
                    .table(CashAccounts::Table)
                    .col(CashAccounts::PortfolioId)
                    .col(CashAccounts::CreatedAt)
                    .col(CashAccounts::Id)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(CashAccounts::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum CashAccounts {
    Table,
    Id,
    PortfolioId,
    Name,
    Currency,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Portfolios {
    Table,
    Id,
}
