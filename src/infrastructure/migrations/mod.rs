//! Schema migrations, applied at startup.
//!
//! Add new migrations as `mYYYYMMDD_HHMMSS_<name>.rs` modules in this
//! directory and register them in [`Migrator::migrations`] in order.

mod m20261024_000001_create_users;
mod m20261024_000002_create_user_sessions;

use sea_orm_migration::{MigrationTrait, MigratorTrait, async_trait};

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261024_000001_create_users::Migration),
            Box::new(m20261024_000002_create_user_sessions::Migration),
        ]
    }
}
