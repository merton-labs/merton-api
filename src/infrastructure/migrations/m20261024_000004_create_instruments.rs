use sea_orm_migration::prelude::*;

use super::m20261024_000001_create_users::Users;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Instruments::Table)
                    .col(
                        ColumnDef::new(Instruments::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Instruments::OwnerUserId).uuid().not_null())
                    .col(
                        ColumnDef::new(Instruments::AssetClass)
                            .string_len(16)
                            .not_null()
                            .check(Expr::col(Instruments::AssetClass).is_in([
                                "STOCK",
                                "ETF",
                                "MUTUAL_FUND",
                            ])),
                    )
                    .col(ColumnDef::new(Instruments::Name).string_len(200).not_null())
                    // Identity attributes are optional and deliberately not
                    // unique: tickers and listings are ambiguous (ADR-INST-01).
                    .col(ColumnDef::new(Instruments::Symbol).string_len(32))
                    .col(ColumnDef::new(Instruments::Isin).string_len(12))
                    .col(ColumnDef::new(Instruments::Exchange).string_len(32))
                    .col(
                        ColumnDef::new(Instruments::QuoteCurrency)
                            .string_len(3)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Instruments::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Instruments::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_instruments_owner_user_id")
                            .from(Instruments::Table, Instruments::OwnerUserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_instruments_owner_created")
                    .table(Instruments::Table)
                    .col(Instruments::OwnerUserId)
                    .col(Instruments::CreatedAt)
                    .col(Instruments::Id)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Instruments::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Instruments {
    Table,
    Id,
    OwnerUserId,
    AssetClass,
    Name,
    Symbol,
    Isin,
    Exchange,
    QuoteCurrency,
    CreatedAt,
    UpdatedAt,
}
