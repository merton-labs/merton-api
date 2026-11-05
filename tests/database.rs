mod common;

use sea_orm::{ConnectionTrait, Statement};

#[tokio::test]
async fn sqlite_foreign_keys_are_enabled() {
    let db = common::test_db().await.unwrap();
    if common::is_postgres(&db) {
        return; // PostgreSQL always enforces foreign keys.
    }
    let backend = db.get_database_backend();

    let row = db
        .query_one_raw(Statement::from_string(backend, "PRAGMA foreign_keys"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(row.try_get_by_index::<i32>(0).unwrap(), 1);
}

#[tokio::test]
async fn migrations_create_tracking_table() {
    let db = common::test_db().await.unwrap();
    let backend = db.get_database_backend();

    let sql = if common::is_postgres(&db) {
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'seaql_migrations'"
    } else {
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'seaql_migrations'"
    };
    let row = db
        .query_one_raw(Statement::from_string(backend, sql))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(row.try_get_by_index::<i64>(0).unwrap(), 1);
}
