//! Schema migrations, applied at startup.
//!
//! Add new migrations as `mYYYYMMDD_HHMMSS_<name>.rs` modules in this
//! directory and register them in [`Migrator::migrations`] in order.

use sea_orm_migration::{MigrationTrait, MigratorTrait, async_trait};

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        Vec::new()
    }
}
