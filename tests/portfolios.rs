#![allow(clippy::unwrap_used)]

mod common;

use axum::http::{StatusCode, header};
use common::{Req, TestApp, TestResponse};
use merton_api::infrastructure::database::is_foreign_key_violation;
use merton_api::infrastructure::entities::portfolio;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Statement};
use serde_json::{Value, json};

const BASE: &str = "/api/v1/portfolios";

struct User {
    session: String,
    csrf: String,
}

async fn user(app: &TestApp, email: &str) -> User {
    let session = app.signed_in(email).await;
    let csrf = app.csrf_token(&session).await;
    User { session, csrf }
}

fn url(id: &str) -> String {
    format!("{BASE}/{id}")
}

fn id_of(response: &TestResponse) -> String {
    response.body["id"].as_str().unwrap().to_owned()
}

impl TestApp {
    async fn create_portfolio(&self, u: &User, body: Value) -> TestResponse {
        self.send(
            Req::post(BASE)
                .session(&u.session)
                .header("x-csrf-token", &u.csrf)
                .json(body),
        )
        .await
    }

    async fn new_portfolio(&self, u: &User, name: &str) -> String {
        let response = self
            .create_portfolio(u, json!({ "name": name, "base_currency": "THB" }))
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        id_of(&response)
    }

    async fn get_portfolio(&self, u: &User, id: &str) -> TestResponse {
        self.send(Req::get(&url(id)).session(&u.session)).await
    }

    async fn list_portfolios(&self, u: &User, query: &str) -> TestResponse {
        self.send(Req::get(&format!("{BASE}{query}")).session(&u.session))
            .await
    }

    async fn patch_portfolio(&self, u: &User, id: &str, body: Value) -> TestResponse {
        self.send(
            Req::patch(&url(id))
                .session(&u.session)
                .header("x-csrf-token", &u.csrf)
                .json(body),
        )
        .await
    }

    async fn lifecycle(&self, u: &User, id: &str, action: &str) -> TestResponse {
        self.send(
            Req::post(&format!("{BASE}/{id}/{action}"))
                .session(&u.session)
                .header("x-csrf-token", &u.csrf),
        )
        .await
    }

    async fn delete_portfolio(&self, u: &User, id: &str) -> TestResponse {
        self.send(
            Req::delete(&url(id))
                .session(&u.session)
                .header("x-csrf-token", &u.csrf),
        )
        .await
    }
}

// --- Create -----------------------------------------------------------------

#[tokio::test]
async fn create_returns_active_portfolio_with_normalized_fields() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;

    let response = app
        .create_portfolio(
            &alice,
            json!({ "name": "  Retirement ", "base_currency": "thb" }),
        )
        .await;

    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    let body = &response.body;
    assert_eq!(body["name"], "Retirement");
    assert_eq!(body["base_currency"], "THB");
    assert_eq!(body["status"], "ACTIVE");
    assert_eq!(body["created_at"], body["updated_at"]);
    assert!(body.get("owner_user_id").is_none());
    assert_eq!(
        response.headers[header::LOCATION],
        url(body["id"].as_str().unwrap()).as_str()
    );
    assert_eq!(response.headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn create_validates_name_and_currency() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;

    let both = app
        .create_portfolio(&alice, json!({ "name": "   ", "base_currency": "US" }))
        .await;
    both.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    let fields: Vec<&str> = both.body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert_eq!(fields, ["name", "base_currency"]);

    let long_name = "x".repeat(101);
    for body in [
        json!({ "name": long_name, "base_currency": "USD" }),
        json!({ "name": "ok", "base_currency": "U$D" }),
        json!({ "name": "ok", "base_currency": "USDX" }),
    ] {
        app.create_portfolio(&alice, body)
            .await
            .assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    }
}

#[tokio::test]
async fn create_rejects_client_supplied_owner_and_unknown_fields() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let bob_id = app.me(&bob.session).await.body["id"].clone();

    let spoofed = app
        .create_portfolio(
            &alice,
            json!({ "name": "Mine", "base_currency": "THB", "owner_user_id": bob_id }),
        )
        .await;
    spoofed.assert_problem(StatusCode::BAD_REQUEST, "malformed_request");

    let missing = app
        .create_portfolio(&alice, json!({ "name": "Mine" }))
        .await;
    missing.assert_problem(StatusCode::BAD_REQUEST, "malformed_request");

    let list = app.list_portfolios(&bob, "").await;
    assert_eq!(list.body["total"], 0, "nothing was created for bob");
}

// --- List -------------------------------------------------------------------

#[tokio::test]
async fn list_returns_only_own_portfolios_in_creation_order() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let first = app.new_portfolio(&alice, "First").await;
    app.new_portfolio(&bob, "Bob's").await;
    let second = app.new_portfolio(&alice, "Second").await;

    let response = app.list_portfolios(&alice, "").await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["total"], 2);
    assert_eq!(response.body["limit"], 50);
    assert_eq!(response.body["offset"], 0);
    let ids: Vec<&str> = response.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [first.as_str(), second.as_str()]);
}

#[tokio::test]
async fn list_paginates_stably_without_overlap() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let mut created = Vec::new();
    for i in 0..5 {
        created.push(app.new_portfolio(&alice, &format!("P{i}")).await);
    }

    let mut seen = Vec::new();
    for offset in [0, 2, 4] {
        let page = app
            .list_portfolios(&alice, &format!("?limit=2&offset={offset}"))
            .await;
        assert_eq!(page.body["total"], 5);
        for item in page.body["items"].as_array().unwrap() {
            seen.push(item["id"].as_str().unwrap().to_owned());
        }
    }

    assert_eq!(seen, created);
}

#[tokio::test]
async fn list_filters_by_status() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let kept = app.new_portfolio(&alice, "Kept").await;
    let shelved = app.new_portfolio(&alice, "Shelved").await;
    app.lifecycle(&alice, &shelved, "archive").await;

    let active = app.list_portfolios(&alice, "?status=ACTIVE").await;
    let archived = app.list_portfolios(&alice, "?status=ARCHIVED").await;

    assert_eq!(active.body["total"], 1);
    assert_eq!(active.body["items"][0]["id"], kept.as_str());
    assert_eq!(archived.body["total"], 1);
    assert_eq!(archived.body["items"][0]["id"], shelved.as_str());
}

#[tokio::test]
async fn list_rejects_bad_paging_and_filters() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;

    for query in ["?limit=0", "?limit=201", "?offset=9223372036854775808"] {
        app.list_portfolios(&alice, query)
            .await
            .assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    }
    for query in ["?limit=ten", "?offset=-1", "?status=DELETED"] {
        app.list_portfolios(&alice, query)
            .await
            .assert_problem(StatusCode::BAD_REQUEST, "malformed_request");
    }
    assert_eq!(
        app.list_portfolios(&alice, "?limit=200").await.status,
        StatusCode::OK
    );
}

// --- Get / update -----------------------------------------------------------

#[tokio::test]
async fn get_returns_own_portfolio_and_404_otherwise() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_portfolio(&alice, "Mine").await;

    let found = app.get_portfolio(&alice, &id).await;
    assert_eq!(found.status, StatusCode::OK);
    assert_eq!(found.body["name"], "Mine");

    for missing in [uuid::Uuid::now_v7().to_string(), "not-a-uuid".to_owned()] {
        app.get_portfolio(&alice, &missing)
            .await
            .assert_problem(StatusCode::NOT_FOUND, "not_found");
    }
}

#[tokio::test]
async fn update_renames_and_changes_base_currency() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_portfolio(&alice, "Old").await;
    let before = app.get_portfolio(&alice, &id).await.body;

    let renamed = app
        .patch_portfolio(&alice, &id, json!({ "name": " New " }))
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    assert_eq!(renamed.body["name"], "New");
    assert_eq!(renamed.body["base_currency"], "THB");

    let recurrencied = app
        .patch_portfolio(&alice, &id, json!({ "base_currency": "usd" }))
        .await;
    assert_eq!(recurrencied.body["name"], "New");
    assert_eq!(recurrencied.body["base_currency"], "USD");
    assert_eq!(recurrencied.body["created_at"], before["created_at"]);
    assert!(recurrencied.body["updated_at"].as_str() >= before["updated_at"].as_str());
}

#[tokio::test]
async fn update_validates_input() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_portfolio(&alice, "Mine").await;

    let empty = app.patch_portfolio(&alice, &id, json!({})).await;
    empty.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(empty.body["errors"][0]["field"], "body");

    app.patch_portfolio(&alice, &id, json!({ "base_currency": "dollars" }))
        .await
        .assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    app.patch_portfolio(&alice, &id, json!({ "status": "ARCHIVED" }))
        .await
        .assert_problem(StatusCode::BAD_REQUEST, "malformed_request");

    assert_eq!(app.get_portfolio(&alice, &id).await.body["name"], "Mine");
}

// --- Lifecycle --------------------------------------------------------------

#[tokio::test]
async fn archive_and_restore_follow_lifecycle_rules() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_portfolio(&alice, "Cycle").await;

    app.lifecycle(&alice, &id, "restore")
        .await
        .assert_problem(StatusCode::CONFLICT, "portfolio_not_archived");

    let archived = app.lifecycle(&alice, &id, "archive").await;
    assert_eq!(archived.status, StatusCode::OK);
    assert_eq!(archived.body["status"], "ARCHIVED");

    // Archived portfolios stay readable but cannot be changed or re-archived.
    assert_eq!(
        app.get_portfolio(&alice, &id).await.body["status"],
        "ARCHIVED"
    );
    app.patch_portfolio(&alice, &id, json!({ "name": "Nope" }))
        .await
        .assert_problem(StatusCode::CONFLICT, "portfolio_archived");
    app.lifecycle(&alice, &id, "archive")
        .await
        .assert_problem(StatusCode::CONFLICT, "portfolio_archived");

    let restored = app.lifecycle(&alice, &id, "restore").await;
    assert_eq!(restored.body["status"], "ACTIVE");
    assert_eq!(restored.body["name"], "Cycle");
    let renamed = app
        .patch_portfolio(&alice, &id, json!({ "name": "Back" }))
        .await;
    assert_eq!(renamed.body["name"], "Back");
}

// --- Delete -----------------------------------------------------------------

#[tokio::test]
async fn delete_removes_portfolio_in_any_status() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let active = app.new_portfolio(&alice, "Active").await;
    let archived = app.new_portfolio(&alice, "Archived").await;
    app.lifecycle(&alice, &archived, "archive").await;

    for id in [&active, &archived] {
        let response = app.delete_portfolio(&alice, id).await;
        assert_eq!(response.status, StatusCode::NO_CONTENT);
        app.get_portfolio(&alice, id)
            .await
            .assert_problem(StatusCode::NOT_FOUND, "not_found");
    }
    app.delete_portfolio(&alice, &active)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn delete_is_refused_when_records_reference_the_portfolio() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_portfolio(&alice, "With history").await;

    // Stand-in for a future financial table: references with ON DELETE RESTRICT.
    let db = app.db();
    let backend = db.get_database_backend();
    for sql in [
        "CREATE TABLE ledger_probe (portfolio_id BLOB NOT NULL \
         REFERENCES portfolios(id) ON DELETE RESTRICT)",
        "INSERT INTO ledger_probe (portfolio_id) SELECT id FROM portfolios",
    ] {
        db.execute_raw(Statement::from_string(backend, sql))
            .await
            .unwrap();
    }

    app.delete_portfolio(&alice, &id)
        .await
        .assert_problem(StatusCode::CONFLICT, "portfolio_has_history");
    assert_eq!(app.get_portfolio(&alice, &id).await.status, StatusCode::OK);
}

// --- Ownership and request protection --------------------------------------

#[tokio::test]
async fn other_users_portfolios_are_indistinguishable_from_missing_ones() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let mallory = user(&app, "mallory@example.com").await;
    let id = app.new_portfolio(&alice, "Private").await;
    let before = app.get_portfolio(&alice, &id).await.body;
    let missing = app
        .get_portfolio(&mallory, &uuid::Uuid::now_v7().to_string())
        .await;

    let attempts = [
        app.get_portfolio(&mallory, &id).await,
        app.patch_portfolio(&mallory, &id, json!({ "name": "Pwned" }))
            .await,
        app.lifecycle(&mallory, &id, "archive").await,
        app.lifecycle(&mallory, &id, "restore").await,
        app.delete_portfolio(&mallory, &id).await,
    ];
    for response in &attempts {
        response.assert_problem(StatusCode::NOT_FOUND, "not_found");
        assert_eq!(response.body, missing.body, "same body as a missing id");
    }

    assert_eq!(app.list_portfolios(&mallory, "").await.body["total"], 0);
    assert_eq!(
        app.get_portfolio(&alice, &id).await.body,
        before,
        "unchanged"
    );
}

#[tokio::test]
async fn mutations_require_csrf_and_reads_require_a_session() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_portfolio(&alice, "Guarded").await;
    let item = url(&id);
    let archive = format!("{item}/archive");
    let restore = format!("{item}/restore");
    let create = json!({ "name": "X", "base_currency": "USD" });

    let without_csrf = [
        Req::post(BASE).json(create.clone()),
        Req::patch(&item).json(json!({ "name": "X" })),
        Req::post(&archive),
        Req::post(&restore),
        Req::delete(&item),
    ];
    for req in without_csrf {
        app.send(req.session(&alice.session))
            .await
            .assert_problem(StatusCode::FORBIDDEN, "csrf_invalid");
    }

    for req in [Req::get(BASE), Req::get(&item)] {
        app.send(req)
            .await
            .assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
    }

    let cross_site = app
        .send(
            Req::post(BASE)
                .header("origin", "https://evil.example")
                .session(&alice.session)
                .header("x-csrf-token", &alice.csrf)
                .json(create),
        )
        .await;
    cross_site.assert_problem(StatusCode::FORBIDDEN, "untrusted_origin");

    let list = app.list_portfolios(&alice, "").await;
    assert_eq!(list.body["total"], 1, "no request above changed anything");
    assert_eq!(list.body["items"][0]["status"], "ACTIVE");
}

#[tokio::test]
async fn query_supplied_identity_is_ignored() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    app.new_portfolio(&bob, "Bob's").await;
    let bob_id = app.me(&bob.session).await.body["id"].clone();

    let response = app
        .list_portfolios(
            &alice,
            &format!("?owner_user_id={}", bob_id.as_str().unwrap()),
        )
        .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["total"], 0);
}

// --- Schema -----------------------------------------------------------------

#[tokio::test]
async fn users_with_portfolios_cannot_be_deleted() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    app.new_portfolio(&alice, "Anchor").await;
    let db = app.db();

    let err = db
        .execute_raw(Statement::from_string(
            db.get_database_backend(),
            "DELETE FROM users",
        ))
        .await
        .unwrap_err();

    assert!(is_foreign_key_violation(&err), "{err}");
}

#[tokio::test]
async fn owner_listing_index_exists() {
    let app = TestApp::new().await;
    let db = app.db();

    let rows = db
        .query_all_raw(Statement::from_string(
            db.get_database_backend(),
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'portfolios'",
        ))
        .await
        .unwrap();
    let names: Vec<String> = rows
        .iter()
        .map(|r| r.try_get_by_index::<String>(0).unwrap())
        .collect();

    assert!(
        names.iter().any(|n| n == "idx_portfolios_owner_created"),
        "{names:?}"
    );
}

#[tokio::test]
async fn equal_creation_times_are_ordered_by_id() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let mut ids = Vec::new();
    for name in ["A", "B", "C"] {
        ids.push(app.new_portfolio(&alice, name).await);
    }
    let same_instant = time::OffsetDateTime::now_utc();
    portfolio::Entity::update_many()
        .col_expr(portfolio::Column::CreatedAt, Expr::value(same_instant))
        .exec(app.db())
        .await
        .unwrap();
    ids.sort();

    let mut seen = Vec::new();
    for offset in 0..3 {
        let page = app
            .list_portfolios(&alice, &format!("?limit=1&offset={offset}"))
            .await;
        seen.push(page.body["items"][0]["id"].as_str().unwrap().to_owned());
    }

    assert_eq!(seen, ids);
}

#[tokio::test]
async fn deleting_another_users_portfolio_with_history_is_still_404() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let mallory = user(&app, "mallory@example.com").await;
    let id = app.new_portfolio(&alice, "With history").await;
    let db = app.db();
    let backend = db.get_database_backend();
    for sql in [
        "CREATE TABLE ledger_probe (portfolio_id BLOB NOT NULL \
         REFERENCES portfolios(id) ON DELETE RESTRICT)",
        "INSERT INTO ledger_probe (portfolio_id) SELECT id FROM portfolios",
    ] {
        db.execute_raw(Statement::from_string(backend, sql))
            .await
            .unwrap();
    }

    // A 409 here would reveal that the portfolio exists.
    app.delete_portfolio(&mallory, &id)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn database_rejects_unknown_status_values() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_portfolio(&alice, "Checked").await;

    let result = portfolio::Entity::update_many()
        .col_expr(portfolio::Column::Status, Expr::value("DELETED"))
        .filter(portfolio::Column::Id.eq(uuid::Uuid::parse_str(&id).unwrap()))
        .exec(app.db())
        .await;

    assert!(result.is_err(), "CHECK constraint must reject DELETED");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_lifecycle_changes_apply_exactly_once() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_portfolio(&alice, "Contended").await;
    let alice = std::sync::Arc::new(alice);

    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let (app, alice, id) = (app.clone(), alice.clone(), id.clone());
        tasks.spawn(async move { app.lifecycle(&alice, &id, "archive").await.status });
    }
    let statuses = tasks.join_all().await;

    let ok = statuses.iter().filter(|s| **s == StatusCode::OK).count();
    let conflict = statuses
        .iter()
        .filter(|s| **s == StatusCode::CONFLICT)
        .count();
    assert_eq!((ok, conflict), (1, 7), "{statuses:?}");
    assert_eq!(
        app.get_portfolio(&alice, &id).await.body["status"],
        "ARCHIVED"
    );
}
