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
                    .table(Portfolios::Table)
                    .col(
                        ColumnDef::new(Portfolios::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Portfolios::OwnerUserId).uuid().not_null())
                    .col(ColumnDef::new(Portfolios::Name).string_len(100).not_null())
                    .col(
                        ColumnDef::new(Portfolios::BaseCurrency)
                            .string_len(3)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Portfolios::Status)
                            .string_len(16)
                            .not_null()
                            .check(Expr::col(Portfolios::Status).is_in(["ACTIVE", "ARCHIVED"])),
                    )
                    .col(
                        ColumnDef::new(Portfolios::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Portfolios::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    // Users are never deleted while they own portfolios.
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_portfolios_owner_user_id")
                            .from(Portfolios::Table, Portfolios::OwnerUserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Serves owner-scoped listing in creation order.
        manager
            .create_index(
                Index::create()
                    .name("idx_portfolios_owner_created")
                    .table(Portfolios::Table)
                    .col(Portfolios::OwnerUserId)
                    .col(Portfolios::CreatedAt)
                    .col(Portfolios::Id)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Portfolios::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Portfolios {
    Table,
    Id,
    OwnerUserId,
    Name,
    BaseCurrency,
    Status,
    CreatedAt,
    UpdatedAt,
}
