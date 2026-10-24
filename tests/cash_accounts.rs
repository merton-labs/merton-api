#![allow(clippy::unwrap_used)]

mod common;

use axum::http::{StatusCode, header};
use common::{Req, TestApp, TestResponse};
use merton_api::infrastructure::database::is_foreign_key_violation;
use merton_api::infrastructure::entities::cash_account;
use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait, Statement};
use serde_json::{Value, json};

struct User {
    session: String,
    csrf: String,
}

async fn user(app: &TestApp, email: &str) -> User {
    let session = app.signed_in(email).await;
    let csrf = app.csrf_token(&session).await;
    User { session, csrf }
}

fn accounts_url(portfolio: &str) -> String {
    format!("/api/v1/portfolios/{portfolio}/cash-accounts")
}

fn account_url(portfolio: &str, account: &str) -> String {
    format!("/api/v1/portfolios/{portfolio}/cash-accounts/{account}")
}

fn id_of(response: &TestResponse) -> String {
    response.body["id"].as_str().unwrap().to_owned()
}

async fn sql(app: &TestApp, statement: &str) {
    let db = app.db();
    db.execute_raw(Statement::from_string(db.get_database_backend(), statement))
        .await
        .unwrap();
}

async fn stored_accounts(app: &TestApp) -> u64 {
    cash_account::Entity::find().count(app.db()).await.unwrap()
}

impl TestApp {
    async fn new_portfolio(&self, u: &User, name: &str) -> String {
        let response = self
            .send(
                Req::post("/api/v1/portfolios")
                    .session(&u.session)
                    .header("x-csrf-token", &u.csrf)
                    .json(json!({ "name": name, "base_currency": "THB" })),
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        id_of(&response)
    }

    async fn portfolio_action(&self, u: &User, portfolio: &str, action: &str) -> TestResponse {
        self.send(
            Req::post(&format!("/api/v1/portfolios/{portfolio}/{action}"))
                .session(&u.session)
                .header("x-csrf-token", &u.csrf),
        )
        .await
    }

    async fn delete_portfolio(&self, u: &User, portfolio: &str) -> TestResponse {
        self.send(
            Req::delete(&format!("/api/v1/portfolios/{portfolio}"))
                .session(&u.session)
                .header("x-csrf-token", &u.csrf),
        )
        .await
    }

    async fn create_account(&self, u: &User, portfolio: &str, body: Value) -> TestResponse {
        self.send(
            Req::post(&accounts_url(portfolio))
                .session(&u.session)
                .header("x-csrf-token", &u.csrf)
                .json(body),
        )
        .await
    }

    async fn new_account(&self, u: &User, portfolio: &str, name: &str, currency: &str) -> String {
        let response = self
            .create_account(u, portfolio, json!({ "name": name, "currency": currency }))
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        id_of(&response)
    }

    async fn list_accounts(&self, u: &User, portfolio: &str, query: &str) -> TestResponse {
        self.send(Req::get(&format!("{}{query}", accounts_url(portfolio))).session(&u.session))
            .await
    }

    async fn get_account(&self, u: &User, portfolio: &str, account: &str) -> TestResponse {
        self.send(Req::get(&account_url(portfolio, account)).session(&u.session))
            .await
    }

    async fn rename_account(
        &self,
        u: &User,
        portfolio: &str,
        account: &str,
        body: Value,
    ) -> TestResponse {
        self.send(
            Req::patch(&account_url(portfolio, account))
                .session(&u.session)
                .header("x-csrf-token", &u.csrf)
                .json(body),
        )
        .await
    }

    async fn delete_account(&self, u: &User, portfolio: &str, account: &str) -> TestResponse {
        self.send(
            Req::delete(&account_url(portfolio, account))
                .session(&u.session)
                .header("x-csrf-token", &u.csrf),
        )
        .await
    }
}

// --- Create / read ----------------------------------------------------------

#[tokio::test]
async fn create_returns_account_and_allows_several_per_currency() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Main").await;

    let first = app
        .create_account(
            &alice,
            &portfolio,
            json!({ "name": " Broker ", "currency": "usd" }),
        )
        .await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);
    assert_eq!(first.body["name"], "Broker");
    assert_eq!(first.body["currency"], "USD");
    assert_eq!(first.body["portfolio_id"], portfolio.as_str());
    assert_eq!(
        first.headers[header::LOCATION],
        account_url(&portfolio, &id_of(&first)).as_str()
    );
    assert_eq!(first.headers[header::CACHE_CONTROL], "no-store");

    let second = app.new_account(&alice, &portfolio, "Savings", "USD").await;
    assert_ne!(second, id_of(&first));

    let list = app.list_accounts(&alice, &portfolio, "").await;
    assert_eq!(list.body["total"], 2);
}

#[tokio::test]
async fn create_validates_input_and_rejects_unknown_fields() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Main").await;

    let invalid = app
        .create_account(
            &alice,
            &portfolio,
            json!({ "name": " ", "currency": "dollars" }),
        )
        .await;
    invalid.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(invalid.body["errors"].as_array().unwrap().len(), 2);

    let other = app.new_portfolio(&alice, "Other").await;
    let redirected = app
        .create_account(
            &alice,
            &portfolio,
            json!({ "name": "X", "currency": "USD", "portfolio_id": other }),
        )
        .await;
    redirected.assert_problem(StatusCode::BAD_REQUEST, "malformed_request");

    assert_eq!(stored_accounts(&app).await, 0);
}

#[tokio::test]
async fn list_is_scoped_to_one_portfolio_and_paginated() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let main = app.new_portfolio(&alice, "Main").await;
    let side = app.new_portfolio(&alice, "Side").await;
    let mut created = Vec::new();
    for i in 0..3 {
        created.push(
            app.new_account(&alice, &main, &format!("A{i}"), "THB")
                .await,
        );
    }
    app.new_account(&alice, &side, "Elsewhere", "THB").await;

    let mut seen = Vec::new();
    for offset in 0..3 {
        let page = app
            .list_accounts(&alice, &main, &format!("?limit=1&offset={offset}"))
            .await;
        assert_eq!(page.body["total"], 3);
        seen.push(page.body["items"][0]["id"].as_str().unwrap().to_owned());
    }
    assert_eq!(seen, created);

    app.list_accounts(&alice, &main, "?limit=0")
        .await
        .assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    app.list_accounts(&alice, &uuid::Uuid::now_v7().to_string(), "")
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn rename_changes_only_the_name() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Main").await;
    let account = app.new_account(&alice, &portfolio, "Old", "THB").await;

    let renamed = app
        .rename_account(&alice, &portfolio, &account, json!({ "name": "New" }))
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    assert_eq!(renamed.body["name"], "New");

    let recurrencied = app
        .rename_account(
            &alice,
            &portfolio,
            &account,
            json!({ "name": "New", "currency": "USD" }),
        )
        .await;
    recurrencied.assert_problem(StatusCode::BAD_REQUEST, "malformed_request");
    app.rename_account(&alice, &portfolio, &account, json!({ "name": "" }))
        .await
        .assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");

    let stored = app.get_account(&alice, &portfolio, &account).await;
    assert_eq!(stored.body["name"], "New");
    assert_eq!(stored.body["currency"], "THB");
}

#[tokio::test]
async fn delete_removes_account() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Main").await;
    let account = app.new_account(&alice, &portfolio, "Temp", "THB").await;

    let deleted = app.delete_account(&alice, &portfolio, &account).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    app.get_account(&alice, &portfolio, &account)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    app.delete_account(&alice, &portfolio, &account)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
}

// --- Ownership --------------------------------------------------------------

#[tokio::test]
async fn accounts_are_only_reachable_through_their_own_portfolio() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let main = app.new_portfolio(&alice, "Main").await;
    let side = app.new_portfolio(&alice, "Side").await;
    let account = app.new_account(&alice, &main, "Main cash", "THB").await;

    // Same user, wrong portfolio in the path.
    app.get_account(&alice, &side, &account)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    app.rename_account(&alice, &side, &account, json!({ "name": "Moved" }))
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    app.delete_account(&alice, &side, &account)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");

    assert_eq!(
        app.get_account(&alice, &main, &account).await.body["name"],
        "Main cash"
    );
}

#[tokio::test]
async fn other_users_accounts_are_indistinguishable_from_missing_ones() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let mallory = user(&app, "mallory@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Private").await;
    let account = app.new_account(&alice, &portfolio, "Secret", "THB").await;
    let own_portfolio = app.new_portfolio(&mallory, "Mine").await;
    let before = app.get_account(&alice, &portfolio, &account).await.body;
    let missing = app
        .get_account(&mallory, &own_portfolio, &uuid::Uuid::now_v7().to_string())
        .await;

    let attempts = [
        app.list_accounts(&mallory, &portfolio, "").await,
        app.create_account(
            &mallory,
            &portfolio,
            json!({ "name": "X", "currency": "THB" }),
        )
        .await,
        app.get_account(&mallory, &portfolio, &account).await,
        app.rename_account(&mallory, &portfolio, &account, json!({ "name": "Pwned" }))
            .await,
        app.delete_account(&mallory, &portfolio, &account).await,
        // Her own portfolio in the path, Alice's account id.
        app.get_account(&mallory, &own_portfolio, &account).await,
        app.rename_account(
            &mallory,
            &own_portfolio,
            &account,
            json!({ "name": "Pwned" }),
        )
        .await,
        app.delete_account(&mallory, &own_portfolio, &account).await,
    ];
    for response in &attempts {
        response.assert_problem(StatusCode::NOT_FOUND, "not_found");
        assert_eq!(response.body, missing.body, "same body as a missing id");
    }

    assert_eq!(
        app.get_account(&alice, &portfolio, &account).await.body,
        before
    );
    assert_eq!(stored_accounts(&app).await, 1, "nothing was created");
}

// --- Archived portfolios ----------------------------------------------------

#[tokio::test]
async fn archived_portfolios_allow_reads_but_not_changes() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Shelved").await;
    let account = app.new_account(&alice, &portfolio, "Cash", "THB").await;
    app.portfolio_action(&alice, &portfolio, "archive").await;

    assert_eq!(
        app.list_accounts(&alice, &portfolio, "").await.body["total"],
        1
    );
    assert_eq!(
        app.get_account(&alice, &portfolio, &account).await.status,
        StatusCode::OK
    );

    for response in [
        app.create_account(
            &alice,
            &portfolio,
            json!({ "name": "New", "currency": "THB" }),
        )
        .await,
        app.rename_account(&alice, &portfolio, &account, json!({ "name": "Renamed" }))
            .await,
        app.delete_account(&alice, &portfolio, &account).await,
    ] {
        response.assert_problem(StatusCode::CONFLICT, "portfolio_archived");
    }
    assert_eq!(stored_accounts(&app).await, 1);

    app.portfolio_action(&alice, &portfolio, "restore").await;
    let renamed = app
        .rename_account(&alice, &portfolio, &account, json!({ "name": "Renamed" }))
        .await;
    assert_eq!(renamed.body["name"], "Renamed");
}

// --- Deletion and history ---------------------------------------------------

#[tokio::test]
async fn account_with_movements_cannot_be_deleted() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Main").await;
    let account = app.new_account(&alice, &portfolio, "Used", "THB").await;
    sql(
        &app,
        "CREATE TABLE movement_probe (cash_account_id BLOB NOT NULL \
         REFERENCES cash_accounts(id) ON DELETE RESTRICT)",
    )
    .await;
    sql(
        &app,
        "INSERT INTO movement_probe (cash_account_id) SELECT id FROM cash_accounts",
    )
    .await;

    app.delete_account(&alice, &portfolio, &account)
        .await
        .assert_problem(StatusCode::CONFLICT, "cash_account_in_use");
    assert_eq!(
        app.get_account(&alice, &portfolio, &account).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn deleting_a_portfolio_removes_its_unused_accounts() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let doomed = app.new_portfolio(&alice, "Doomed").await;
    let kept = app.new_portfolio(&alice, "Kept").await;
    app.new_account(&alice, &doomed, "A", "THB").await;
    app.new_account(&alice, &doomed, "B", "USD").await;
    let survivor = app.new_account(&alice, &kept, "C", "THB").await;

    let response = app.delete_portfolio(&alice, &doomed).await;

    assert_eq!(response.status, StatusCode::NO_CONTENT, "{}", response.body);
    assert_eq!(stored_accounts(&app).await, 1);
    assert_eq!(
        app.get_account(&alice, &kept, &survivor).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn portfolio_with_account_history_is_kept_intact() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "History").await;
    let used = app.new_account(&alice, &portfolio, "Used", "THB").await;
    let unused = app.new_account(&alice, &portfolio, "Unused", "THB").await;
    sql(
        &app,
        "CREATE TABLE movement_probe (cash_account_id BLOB NOT NULL \
         REFERENCES cash_accounts(id) ON DELETE RESTRICT)",
    )
    .await;
    sql(
        &app,
        "INSERT INTO movement_probe (cash_account_id) \
         SELECT id FROM cash_accounts WHERE name = 'Used'",
    )
    .await;

    app.delete_portfolio(&alice, &portfolio)
        .await
        .assert_problem(StatusCode::CONFLICT, "portfolio_has_history");

    // The whole delete rolled back: both accounts and the portfolio remain.
    assert_eq!(stored_accounts(&app).await, 2);
    for account in [&used, &unused] {
        assert_eq!(
            app.get_account(&alice, &portfolio, account).await.status,
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn database_refuses_orphaning_accounts() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Anchored").await;
    app.new_account(&alice, &portfolio, "Cash", "THB").await;
    let db = app.db();

    let err = db
        .execute_raw(Statement::from_string(
            db.get_database_backend(),
            "DELETE FROM portfolios",
        ))
        .await
        .unwrap_err();

    assert!(is_foreign_key_violation(&err), "{err}");
}

// --- Request protection -----------------------------------------------------

#[tokio::test]
async fn mutations_require_csrf_and_reads_require_a_session() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Guarded").await;
    let account = app.new_account(&alice, &portfolio, "Cash", "THB").await;
    let collection = accounts_url(&portfolio);
    let item = account_url(&portfolio, &account);

    for req in [
        Req::post(&collection).json(json!({ "name": "X", "currency": "THB" })),
        Req::patch(&item).json(json!({ "name": "X" })),
        Req::delete(&item),
    ] {
        app.send(req.session(&alice.session))
            .await
            .assert_problem(StatusCode::FORBIDDEN, "csrf_invalid");
    }
    for req in [Req::get(&collection), Req::get(&item)] {
        app.send(req)
            .await
            .assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
    }

    let stored = app.get_account(&alice, &portfolio, &account).await;
    assert_eq!(stored.body["name"], "Cash");
    assert_eq!(stored_accounts(&app).await, 1);
}

#[tokio::test]
async fn other_users_cannot_probe_archived_or_used_accounts() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let mallory = user(&app, "mallory@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Private").await;
    let used = app.new_account(&alice, &portfolio, "Used", "THB").await;
    let idle = app.new_account(&alice, &portfolio, "Idle", "THB").await;
    sql(
        &app,
        "CREATE TABLE movement_probe (cash_account_id BLOB NOT NULL \
         REFERENCES cash_accounts(id) ON DELETE RESTRICT)",
    )
    .await;
    sql(
        &app,
        "INSERT INTO movement_probe (cash_account_id) \
         SELECT id FROM cash_accounts WHERE name = 'Used'",
    )
    .await;

    // A 409 would reveal that the account exists and is in use.
    app.delete_account(&mallory, &portfolio, &used)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");

    // A 409 would reveal that the portfolio exists and is archived.
    app.portfolio_action(&alice, &portfolio, "archive").await;
    for response in [
        app.create_account(
            &mallory,
            &portfolio,
            json!({ "name": "X", "currency": "THB" }),
        )
        .await,
        app.rename_account(&mallory, &portfolio, &idle, json!({ "name": "X" }))
            .await,
        app.delete_account(&mallory, &portfolio, &idle).await,
    ] {
        response.assert_problem(StatusCode::NOT_FOUND, "not_found");
    }
    assert_eq!(stored_accounts(&app).await, 2);
}
