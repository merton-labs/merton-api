#![allow(clippy::unwrap_used)]

mod common;

use axum::http::{StatusCode, header};
use common::{Req, TestApp, TestResponse};
use merton_api::infrastructure::database::is_foreign_key_violation;
use merton_api::infrastructure::entities::instrument;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Statement};
use serde_json::{Value, json};

const BASE: &str = "/api/v1/instruments";

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
    async fn create_instrument(&self, u: &User, body: Value) -> TestResponse {
        self.send(
            Req::post(BASE)
                .session(&u.session)
                .header("x-csrf-token", &u.csrf)
                .json(body),
        )
        .await
    }

    async fn new_instrument(&self, u: &User, name: &str) -> String {
        let response = self
            .create_instrument(
                u,
                json!({
                    "asset_class": "STOCK",
                    "name": name,
                    "symbol": "PTT",
                    "quote_currency": "THB",
                }),
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        id_of(&response)
    }

    async fn get_instrument(&self, u: &User, id: &str) -> TestResponse {
        self.send(Req::get(&url(id)).session(&u.session)).await
    }

    async fn list_instruments(&self, u: &User, query: &str) -> TestResponse {
        self.send(Req::get(&format!("{BASE}{query}")).session(&u.session))
            .await
    }

    async fn patch_instrument(&self, u: &User, id: &str, body: Value) -> TestResponse {
        self.send(
            Req::patch(&url(id))
                .session(&u.session)
                .header("x-csrf-token", &u.csrf)
                .json(body),
        )
        .await
    }

    async fn delete_instrument(&self, u: &User, id: &str) -> TestResponse {
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
async fn create_returns_normalized_fields_and_headers() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;

    let response = app
        .create_instrument(
            &alice,
            json!({
                "asset_class": "STOCK",
                "name": "  Apple Inc. ",
                "symbol": " aapl.OQ ",
                "isin": " us0378331005 ",
                "exchange": "   ",
                "quote_currency": "usd",
            }),
        )
        .await;

    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    let body = &response.body;
    assert_eq!(body["asset_class"], "STOCK");
    assert_eq!(body["name"], "Apple Inc.");
    assert_eq!(body["symbol"], "aapl.OQ");
    assert_eq!(body["isin"], "US0378331005");
    assert_eq!(body["exchange"], Value::Null);
    assert_eq!(body["quote_currency"], "USD");
    assert_eq!(body["created_at"], body["updated_at"]);
    assert!(body.get("owner_user_id").is_none());
    assert_eq!(
        response.headers[header::LOCATION],
        url(body["id"].as_str().unwrap()).as_str()
    );
    assert_eq!(response.headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn create_validates_fields_and_identifiers() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;

    let all_invalid = app
        .create_instrument(
            &alice,
            json!({
                "asset_class": "STOCK",
                "name": "   ",
                "symbol": "X".repeat(33),
                "isin": "US0378331006",
                "exchange": "Y".repeat(33),
                "quote_currency": "US",
            }),
        )
        .await;
    all_invalid.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    let fields: Vec<&str> = all_invalid.body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert_eq!(
        fields,
        ["name", "symbol", "isin", "exchange", "quote_currency"]
    );

    for body in [
        json!({
            "asset_class": "STOCK",
            "name": "Apple",
            "isin": "US0378331006",
            "quote_currency": "USD",
        }),
        json!({
            "asset_class": "STOCK",
            "name": "Apple",
            "isin": "NOT-AN-ISIN",
            "quote_currency": "USD",
        }),
        json!({
            "asset_class": "STOCK",
            "name": "Apple",
            "symbol": "S".repeat(33),
            "quote_currency": "USD",
        }),
        json!({
            "asset_class": "STOCK",
            "name": "Apple",
            "quote_currency": "U$D",
        }),
    ] {
        app.create_instrument(&alice, body)
            .await
            .assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    }
}

#[tokio::test]
async fn create_rejects_unknown_asset_class_unknown_fields_and_missing_fields() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let bob_id = app.me(&bob.session).await.body["id"].clone();

    for body in [
        json!({
            "asset_class": "BOND",
            "name": "Treasury",
            "quote_currency": "USD",
        }),
        json!({
            "asset_class": "STOCK",
            "name": "Mine",
            "quote_currency": "USD",
            "owner_user_id": bob_id,
        }),
        json!({
            "asset_class": "STOCK",
            "name": "Mine",
        }),
    ] {
        app.create_instrument(&alice, body)
            .await
            .assert_problem(StatusCode::BAD_REQUEST, "malformed_request");
    }

    let list = app.list_instruments(&bob, "").await;
    assert_eq!(list.body["total"], 0, "nothing was created for bob");
}

#[tokio::test]
async fn duplicate_identities_are_allowed() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let payload = json!({
        "asset_class": "STOCK",
        "name": "Apple Inc.",
        "symbol": "AAPL",
        "isin": "US0378331005",
        "exchange": "NASDAQ",
        "quote_currency": "USD",
    });

    let first = app.create_instrument(&alice, payload.clone()).await;
    let second = app.create_instrument(&alice, payload).await;

    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);
    assert_eq!(second.status, StatusCode::CREATED, "{}", second.body);
    assert_ne!(first.body["id"], second.body["id"]);
    assert_eq!(app.list_instruments(&alice, "").await.body["total"], 2);
}

// --- List -------------------------------------------------------------------

#[tokio::test]
async fn list_filters_paginates_and_enforces_bounds() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;

    let stock = id_of(
        &app.create_instrument(
            &alice,
            json!({ "asset_class": "STOCK", "name": "S1", "quote_currency": "USD" }),
        )
        .await,
    );
    app.new_instrument(&bob, "Bob's").await;
    let etf = id_of(
        &app.create_instrument(
            &alice,
            json!({ "asset_class": "ETF", "name": "E1", "quote_currency": "USD" }),
        )
        .await,
    );
    let fund = id_of(
        &app.create_instrument(
            &alice,
            json!({ "asset_class": "MUTUAL_FUND", "name": "M1", "quote_currency": "THB" }),
        )
        .await,
    );
    let expected = vec![stock, etf.clone(), fund];

    let all = app.list_instruments(&alice, "").await;
    assert_eq!(all.status, StatusCode::OK);
    assert_eq!(all.body["total"], 3);
    assert_eq!(all.body["limit"], 50);
    assert_eq!(all.body["offset"], 0);
    let ids: Vec<String> = all.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, expected);

    let etfs = app.list_instruments(&alice, "?asset_class=ETF").await;
    assert_eq!(etfs.body["total"], 1);
    assert_eq!(etfs.body["items"][0]["id"], etf);

    let mut paged = Vec::new();
    for offset in [0, 2] {
        let page = app
            .list_instruments(&alice, &format!("?limit=2&offset={offset}"))
            .await;
        assert_eq!(page.body["total"], 3);
        for item in page.body["items"].as_array().unwrap() {
            paged.push(item["id"].as_str().unwrap().to_owned());
        }
    }
    assert_eq!(paged, expected);

    for query in ["?limit=0", "?limit=201", "?offset=9223372036854775808"] {
        app.list_instruments(&alice, query)
            .await
            .assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    }
    for query in ["?limit=ten", "?offset=-1", "?asset_class=BOND"] {
        app.list_instruments(&alice, query)
            .await
            .assert_problem(StatusCode::BAD_REQUEST, "malformed_request");
    }
    assert_eq!(
        app.list_instruments(&alice, "?limit=200").await.status,
        StatusCode::OK
    );
}

// --- Get / update -----------------------------------------------------------

#[tokio::test]
async fn get_returns_own_instrument_and_404_otherwise() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let id = app.new_instrument(&alice, "Mine").await;

    let found = app.get_instrument(&alice, &id).await;
    assert_eq!(found.status, StatusCode::OK);
    assert_eq!(found.body["name"], "Mine");

    app.get_instrument(&bob, &id)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    for missing in [uuid::Uuid::now_v7().to_string(), "not-a-uuid".to_owned()] {
        app.get_instrument(&alice, &missing)
            .await
            .assert_problem(StatusCode::NOT_FOUND, "not_found");
    }
}

#[tokio::test]
async fn update_modifies_attributes_and_clears_nullable_fields() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let created = app
        .create_instrument(
            &alice,
            json!({
                "asset_class": "STOCK",
                "name": "Old Name",
                "symbol": "OLD",
                "exchange": "SET",
                "quote_currency": "THB",
            }),
        )
        .await;
    let id = id_of(&created);

    let updated = app
        .patch_instrument(
            &alice,
            &id,
            json!({
                "asset_class": "ETF",
                "name": "  New Name ",
                "symbol": null,
                "isin": " gb0002634946 ",
            }),
        )
        .await;

    assert_eq!(updated.status, StatusCode::OK, "{}", updated.body);
    assert_eq!(updated.body["asset_class"], "ETF");
    assert_eq!(updated.body["name"], "New Name");
    assert_eq!(updated.body["symbol"], Value::Null);
    assert_eq!(updated.body["isin"], "GB0002634946");
    assert_eq!(updated.body["exchange"], "SET");
    assert_eq!(updated.body["quote_currency"], "THB");
    assert_eq!(updated.body["created_at"], created.body["created_at"]);
    assert!(updated.body["updated_at"].as_str() >= created.body["updated_at"].as_str());
}

#[tokio::test]
async fn update_validates_input_and_rejects_quote_currency() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_instrument(&alice, "Mine").await;

    let empty = app.patch_instrument(&alice, &id, json!({})).await;
    empty.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(empty.body["errors"][0]["field"], "body");

    let bad_isin = app
        .patch_instrument(&alice, &id, json!({ "isin": "US0378331006" }))
        .await;
    bad_isin.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(bad_isin.body["errors"][0]["field"], "isin");

    app.patch_instrument(&alice, &id, json!({ "quote_currency": "USD" }))
        .await
        .assert_problem(StatusCode::BAD_REQUEST, "malformed_request");

    let current = app.get_instrument(&alice, &id).await;
    assert_eq!(current.body["quote_currency"], "THB");
}

// --- Delete -----------------------------------------------------------------

#[tokio::test]
async fn delete_removes_unused_instrument_and_refuses_when_in_use() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let removable = app.new_instrument(&alice, "Removable").await;

    let deleted = app.delete_instrument(&alice, &removable).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    app.get_instrument(&alice, &removable)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");

    let in_use = app.new_instrument(&alice, "In use").await;
    let db = app.db();
    let backend = db.get_database_backend();
    for sql in [
        "CREATE TABLE price_probe (instrument_id BLOB NOT NULL \
         REFERENCES instruments(id) ON DELETE RESTRICT)",
        "INSERT INTO price_probe (instrument_id) SELECT id FROM instruments",
    ] {
        db.execute_raw(Statement::from_string(backend, sql))
            .await
            .unwrap();
    }

    app.delete_instrument(&alice, &in_use)
        .await
        .assert_problem(StatusCode::CONFLICT, "instrument_in_use");
    assert_eq!(
        app.get_instrument(&alice, &in_use).await.status,
        StatusCode::OK
    );

    // A 409 for another user would reveal that the instrument exists.
    app.delete_instrument(&bob, &in_use)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
}

// --- Ownership and request protection --------------------------------------

#[tokio::test]
async fn other_users_instruments_are_indistinguishable_from_missing_ones() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let id = app.new_instrument(&alice, "Private").await;
    let before = app.get_instrument(&alice, &id).await.body;
    let missing = app
        .get_instrument(&bob, &uuid::Uuid::now_v7().to_string())
        .await;

    let attempts = [
        app.get_instrument(&bob, &id).await,
        app.patch_instrument(&bob, &id, json!({ "name": "Pwned" }))
            .await,
        app.delete_instrument(&bob, &id).await,
    ];
    for response in &attempts {
        response.assert_problem(StatusCode::NOT_FOUND, "not_found");
        assert_eq!(response.body, missing.body, "same body as a missing id");
    }

    assert_eq!(app.list_instruments(&bob, "").await.body["total"], 0);
    assert_eq!(
        app.get_instrument(&alice, &id).await.body,
        before,
        "unchanged"
    );
}

#[tokio::test]
async fn mutations_require_csrf_and_reads_require_a_session() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_instrument(&alice, "Guarded").await;
    let item = url(&id);
    let create = json!({
        "asset_class": "STOCK",
        "name": "X",
        "quote_currency": "USD",
    });

    let without_csrf = [
        Req::post(BASE).json(create),
        Req::patch(&item).json(json!({ "name": "X" })),
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

    let list = app.list_instruments(&alice, "").await;
    assert_eq!(list.body["total"], 1, "no request above changed anything");
    assert_eq!(list.body["items"][0]["name"], "Guarded");
}

// --- Schema -----------------------------------------------------------------

#[tokio::test]
async fn schema_enforces_asset_class_check_and_user_foreign_key() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_instrument(&alice, "Checked").await;
    let db = app.db();

    let bad_class = instrument::Entity::update_many()
        .col_expr(instrument::Column::AssetClass, Expr::value("BOND"))
        .filter(instrument::Column::Id.eq(uuid::Uuid::parse_str(&id).unwrap()))
        .exec(db)
        .await;
    assert!(bad_class.is_err(), "CHECK constraint must reject BOND");

    let fk_err = db
        .execute_raw(Statement::from_string(
            db.get_database_backend(),
            "DELETE FROM users",
        ))
        .await
        .unwrap_err();
    assert!(is_foreign_key_violation(&fk_err), "{fk_err}");
}

#[tokio::test]
async fn update_rejects_null_for_required_fields() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_instrument(&alice, "Keeps its name").await;

    for body in [json!({ "name": null }), json!({ "asset_class": null })] {
        app.patch_instrument(&alice, &id, body)
            .await
            .assert_problem(StatusCode::BAD_REQUEST, "malformed_request");
    }

    let current = app.get_instrument(&alice, &id).await;
    assert_eq!(current.body["name"], "Keeps its name");
    assert_eq!(current.body["asset_class"], "STOCK");
}
