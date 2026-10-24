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
                    .table(UserSessions::Table)
                    .col(
                        ColumnDef::new(UserSessions::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(UserSessions::UserId).uuid().not_null())
                    .col(
                        ColumnDef::new(UserSessions::TokenHash)
                            .binary_len(32)
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(UserSessions::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserSessions::LastActivityAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserSessions::IdleExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(UserSessions::AbsoluteExpiresAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(ColumnDef::new(UserSessions::RevokedAt).timestamp_with_time_zone())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_sessions_user_id")
                            .from(UserSessions::Table, UserSessions::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_sessions_user_id")
                    .table(UserSessions::Table)
                    .col(UserSessions::UserId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(UserSessions::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum UserSessions {
    Table,
    Id,
    UserId,
    TokenHash,
    CreatedAt,
    LastActivityAt,
    IdleExpiresAt,
    AbsoluteExpiresAt,
    RevokedAt,
}
