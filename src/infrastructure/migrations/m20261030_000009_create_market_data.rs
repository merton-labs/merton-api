//! Historical market data for valuation (P5): instrument prices and FX
//! rates. Both are entered by the user; nothing is fetched (ADR-VAL-02/03).
//!
//! - One price per instrument per date, in the instrument's quote currency.
//! - One rate per user, `base`/`quote` pair and date: one unit of `base` is
//!   worth `rate` units of `quote`.
//!
//! Values are decimal text (ADR-NUM-02); dates are date-only (ADR-DB-05).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Entered through the API, or imported from CSV (P6).
const SOURCES: [&str; 2] = ["MANUAL", "IMPORT"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(InstrumentPrices::Table)
                    .col(
                        ColumnDef::new(InstrumentPrices::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(InstrumentPrices::InstrumentId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(InstrumentPrices::PriceDate)
                            .date()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(InstrumentPrices::Price)
                            .string_len(64)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(InstrumentPrices::Source)
                            .string_len(16)
                            .not_null()
                            .check(Expr::col(InstrumentPrices::Source).is_in(SOURCES)),
                    )
                    .col(
                        ColumnDef::new(InstrumentPrices::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(InstrumentPrices::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    // An instrument with prices cannot be deleted; the database never
                    // cascades.
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_instrument_prices_instrument_id")
                            .from(InstrumentPrices::Table, InstrumentPrices::InstrumentId)
                            .to(Instruments::Table, Instruments::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Also serves "latest price on or before a date".
        manager
            .create_index(
                Index::create()
                    .name("idx_instrument_prices_instrument_date")
                    .table(InstrumentPrices::Table)
                    .col(InstrumentPrices::InstrumentId)
                    .col(InstrumentPrices::PriceDate)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(FxRates::Table)
                    .col(ColumnDef::new(FxRates::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(FxRates::OwnerUserId).uuid().not_null())
                    .col(
                        ColumnDef::new(FxRates::BaseCurrency)
                            .string_len(3)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(FxRates::QuoteCurrency)
                            .string_len(3)
                            .not_null(),
                    )
                    .col(ColumnDef::new(FxRates::RateDate).date().not_null())
                    .col(ColumnDef::new(FxRates::Rate).string_len(64).not_null())
                    .col(
                        ColumnDef::new(FxRates::Source)
                            .string_len(16)
                            .not_null()
                            .check(Expr::col(FxRates::Source).is_in(SOURCES)),
                    )
                    .col(
                        ColumnDef::new(FxRates::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(FxRates::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_fx_rates_owner_user_id")
                            .from(FxRates::Table, FxRates::OwnerUserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_fx_rates_owner_pair_date")
                    .table(FxRates::Table)
                    .col(FxRates::OwnerUserId)
                    .col(FxRates::BaseCurrency)
                    .col(FxRates::QuoteCurrency)
                    .col(FxRates::RateDate)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(FxRates::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(InstrumentPrices::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum InstrumentPrices {
    Table,
    Id,
    InstrumentId,
    PriceDate,
    Price,
    Source,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum FxRates {
    Table,
    Id,
    OwnerUserId,
    BaseCurrency,
    QuoteCurrency,
    RateDate,
    Rate,
    Source,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Instruments {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
