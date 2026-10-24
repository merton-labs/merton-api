#![allow(clippy::unwrap_used)]

//! Financial invariants of the ledger, checked against the stored rows.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{Req, TestApp, TestResponse};
use merton_api::infrastructure::entities::{cash_movement, position_movement, transaction};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

struct User {
    session: String,
    csrf: String,
}

struct Book {
    portfolio: String,
    instrument: String,
    cash: String,
}

async fn user(app: &TestApp, email: &str) -> User {
    let session = app.signed_in(email).await;
    let csrf = app.csrf_token(&session).await;
    User { session, csrf }
}

fn id_of(response: &TestResponse) -> String {
    response.body["id"].as_str().unwrap().to_owned()
}

fn uuid(id: &str) -> Uuid {
    Uuid::parse_str(id).unwrap()
}

impl TestApp {
    async fn post_json(&self, u: &User, uri: &str, body: Value) -> TestResponse {
        self.send(
            Req::post(uri)
                .session(&u.session)
                .header("x-csrf-token", &u.csrf)
                .json(body),
        )
        .await
    }

    async fn created(&self, u: &User, uri: &str, body: Value) -> String {
        let response = self.post_json(u, uri, body).await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        id_of(&response)
    }

    /// A portfolio with one THB stock and one THB cash account.
    async fn book(&self, u: &User) -> Book {
        let portfolio = self
            .created(
                u,
                "/api/v1/portfolios",
                json!({ "name": "Book", "base_currency": "THB" }),
            )
            .await;
        let instrument = self
            .created(
                u,
                "/api/v1/instruments",
                json!({ "asset_class": "STOCK", "name": "PTT", "quote_currency": "THB" }),
            )
            .await;
        let cash = self
            .created(
                u,
                &format!("/api/v1/portfolios/{portfolio}/cash-accounts"),
                json!({ "name": "Cash", "currency": "THB" }),
            )
            .await;
        Book {
            portfolio,
            instrument,
            cash,
        }
    }

    async fn record(&self, u: &User, book: &Book, body: Value) -> TestResponse {
        self.post_json(
            u,
            &format!("/api/v1/portfolios/{}/transactions", book.portfolio),
            body,
        )
        .await
    }

    async fn replace_txn(&self, u: &User, book: &Book, id: &str, body: Value) -> TestResponse {
        self.send(
            Req::put(&format!(
                "/api/v1/portfolios/{}/transactions/{id}",
                book.portfolio
            ))
            .session(&u.session)
            .header("x-csrf-token", &u.csrf)
            .json(body),
        )
        .await
    }

    async fn delete_txn(&self, u: &User, book: &Book, id: &str) -> TestResponse {
        self.send(
            Req::delete(&format!(
                "/api/v1/portfolios/{}/transactions/{id}",
                book.portfolio
            ))
            .session(&u.session)
            .header("x-csrf-token", &u.csrf),
        )
        .await
    }

    async fn position_rows(&self, transaction_id: &str) -> Vec<position_movement::Model> {
        position_movement::Entity::find()
            .filter(position_movement::Column::TransactionId.eq(uuid(transaction_id)))
            .all(self.db())
            .await
            .unwrap()
    }

    async fn cash_rows(&self, transaction_id: &str) -> Vec<cash_movement::Model> {
        cash_movement::Entity::find()
            .filter(cash_movement::Column::TransactionId.eq(uuid(transaction_id)))
            .all(self.db())
            .await
            .unwrap()
    }

    async fn ledger_row_counts(&self) -> (u64, u64, u64) {
        let db = self.db();
        (
            transaction::Entity::find().count(db).await.unwrap(),
            position_movement::Entity::find().count(db).await.unwrap(),
            cash_movement::Entity::find().count(db).await.unwrap(),
        )
    }
}

fn buy(book: &Book, date: &str, quantity: &str, price: &str) -> Value {
    json!({
        "kind": "BUY",
        "trade_date": date,
        "instrument_id": book.instrument,
        "quantity": quantity,
        "price": price,
    })
}

fn sell(book: &Book, date: &str, quantity: &str, price: &str) -> Value {
    json!({
        "kind": "SELL",
        "trade_date": date,
        "instrument_id": book.instrument,
        "quantity": quantity,
        "price": price,
    })
}

fn with(mut body: Value, field: &str, value: Value) -> Value {
    body[field] = value;
    body
}

// --- Posting ----------------------------------------------------------------

#[tokio::test]
async fn trades_post_exact_position_and_cash_movements() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;

    // Buy: 10 × 35.25 = 352.50; cost = 352.50 + 15.50 + 1.09 = 369.09.
    let bought = app
        .record(
            &alice,
            &book,
            json!({
                "kind": "BUY", "trade_date": "2026-01-10", "instrument_id": book.instrument,
                "cash_account_id": book.cash, "quantity": "10", "price": "35.25",
                "fee": "15.50", "tax": "1.09",
            }),
        )
        .await;
    assert_eq!(bought.status, StatusCode::CREATED, "{}", bought.body);
    let buy_id = id_of(&bought);
    let position = &app.position_rows(&buy_id).await[0];
    assert_eq!(
        (
            position.kind.as_str(),
            position.quantity.as_str(),
            position.amount.as_str()
        ),
        ("ACQUIRE", "10", "369.09")
    );
    assert_eq!(app.cash_rows(&buy_id).await[0].amount_delta, "-369.09");

    // Sell: 4 × 40 = 160; proceeds = 160 − 2 − 0.50 = 157.50.
    let sold = app
        .record(
            &alice,
            &book,
            json!({
                "kind": "SELL", "trade_date": "2026-02-01", "instrument_id": book.instrument,
                "cash_account_id": book.cash, "quantity": "4", "price": "40",
                "fee": "2", "tax": "0.50",
            }),
        )
        .await;
    let sell_id = id_of(&sold);
    let position = &app.position_rows(&sell_id).await[0];
    assert_eq!(
        (
            position.kind.as_str(),
            position.quantity.as_str(),
            position.amount.as_str()
        ),
        ("DISPOSE", "4", "157.50")
    );
    assert_eq!(app.cash_rows(&sell_id).await[0].amount_delta, "157.50");
}

#[tokio::test]
async fn cash_movements_are_optional_and_opening_cash_is_signed() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;

    let no_cash = app
        .created(
            &alice,
            &format!("/api/v1/portfolios/{}/transactions", book.portfolio),
            buy(&book, "2026-01-10", "1", "10"),
        )
        .await;
    assert!(app.cash_rows(&no_cash).await.is_empty());

    let overdraft = app
        .created(
            &alice,
            &format!("/api/v1/portfolios/{}/transactions", book.portfolio),
            json!({
                "kind": "OPENING_CASH", "trade_date": "2026-01-01",
                "cash_account_id": book.cash, "amount": "-250.75",
            }),
        )
        .await;
    assert_eq!(app.cash_rows(&overdraft).await[0].amount_delta, "-250.75");
    assert!(app.position_rows(&overdraft).await.is_empty());
}

#[tokio::test]
async fn decimals_round_trip_exactly_as_text() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;

    let response = app
        .record(
            &alice,
            &book,
            with(
                buy(
                    &book,
                    "2026-01-10",
                    "0.00000001",
                    "999999999999999.99999999",
                ),
                "fee",
                json!("0.10"),
            ),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    assert_eq!(response.body["quantity"], "0.00000001");
    assert_eq!(response.body["price"], "999999999999999.99999999");
    assert_eq!(response.body["fee"], "0.10");
    assert_eq!(response.body["tax"], "0");

    // 0.00000001 × 999999999999999.99999999 = 9999999.9999999999999999; + 0.10.
    let position = &app.position_rows(&id_of(&response)).await[0];
    assert_eq!(position.amount, "10000000.0999999999999999");
    let db = app.db();
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            "SELECT typeof(quantity), typeof(price), typeof(fee) FROM transactions",
        ))
        .await
        .unwrap()
        .unwrap();
    for i in 0..3 {
        assert_eq!(row.try_get_by_index::<String>(i).unwrap(), "text");
    }
}

// --- FIFO validation over history -------------------------------------------

#[tokio::test]
async fn overselling_is_rejected_and_nothing_is_written() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    app.record(&alice, &book, buy(&book, "2026-01-10", "10", "1"))
        .await;
    let before = app.ledger_row_counts().await;

    let response = app
        .record(&alice, &book, sell(&book, "2026-02-01", "10.00000001", "1"))
        .await;

    response.assert_problem(StatusCode::CONFLICT, "insufficient_holdings");
    assert!(
        response.body["detail"]
            .as_str()
            .unwrap()
            .contains("2026-02-01")
    );
    assert_eq!(app.ledger_row_counts().await, before);
}

#[tokio::test]
async fn sales_are_checked_at_their_point_in_history() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    app.record(&alice, &book, buy(&book, "2026-01-10", "10", "1"))
        .await;

    // Holdings are 10 today, but none were held on 2026-01-05.
    app.record(&alice, &book, sell(&book, "2026-01-05", "1", "1"))
        .await
        .assert_problem(StatusCode::CONFLICT, "insufficient_holdings");
}

#[tokio::test]
async fn sort_order_sequences_transactions_on_the_same_day() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    app.record(&alice, &book, buy(&book, "2026-03-01", "5", "1"))
        .await;

    let before_buy = with(sell(&book, "2026-03-01", "5", "1"), "sort_order", json!(-1));
    app.record(&alice, &book, before_buy)
        .await
        .assert_problem(StatusCode::CONFLICT, "insufficient_holdings");

    // Same sort_order: entry order decides, and the buy came first.
    let after_buy = app
        .record(&alice, &book, sell(&book, "2026-03-01", "5", "1"))
        .await;
    assert_eq!(after_buy.status, StatusCode::CREATED, "{}", after_buy.body);
}

#[tokio::test]
async fn editing_that_breaks_later_history_is_rolled_back() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let buy_id = id_of(
        &app.record(&alice, &book, buy(&book, "2026-01-10", "10", "2"))
            .await,
    );
    app.record(&alice, &book, sell(&book, "2026-02-01", "8", "3"))
        .await;

    app.replace_txn(&alice, &book, &buy_id, buy(&book, "2026-01-10", "5", "2"))
        .await
        .assert_problem(StatusCode::CONFLICT, "insufficient_holdings");
    // Moving the buy after the sale breaks history too.
    app.replace_txn(&alice, &book, &buy_id, buy(&book, "2026-03-01", "10", "2"))
        .await
        .assert_problem(StatusCode::CONFLICT, "insufficient_holdings");

    let stored = &app.position_rows(&buy_id).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(
        (stored[0].quantity.as_str(), stored[0].amount.as_str()),
        ("10", "20")
    );

    let ok = app
        .replace_txn(&alice, &book, &buy_id, buy(&book, "2026-01-10", "8", "2.5"))
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    assert_eq!(app.position_rows(&buy_id).await[0].amount, "20.0");
}

#[tokio::test]
async fn moving_a_trade_to_another_instrument_replays_both() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let other = app
        .created(
            &alice,
            "/api/v1/instruments",
            json!({ "asset_class": "ETF", "name": "Other", "quote_currency": "THB" }),
        )
        .await;
    let buy_id = id_of(
        &app.record(&alice, &book, buy(&book, "2026-01-10", "10", "1"))
            .await,
    );
    app.record(&alice, &book, sell(&book, "2026-02-01", "4", "1"))
        .await;

    // The old instrument would be left with a sale and no purchase.
    let moved = with(
        buy(&book, "2026-01-10", "10", "1"),
        "instrument_id",
        json!(other),
    );
    app.replace_txn(&alice, &book, &buy_id, moved.clone())
        .await
        .assert_problem(StatusCode::CONFLICT, "insufficient_holdings");
    assert_eq!(
        app.position_rows(&buy_id).await[0].instrument_id,
        uuid(&book.instrument)
    );
}

#[tokio::test]
async fn replacing_a_transaction_reposts_its_movements() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let id = id_of(
        &app.record(
            &alice,
            &book,
            with(
                buy(&book, "2026-01-10", "2", "5"),
                "cash_account_id",
                json!(book.cash),
            ),
        )
        .await,
    );
    assert_eq!(app.position_rows(&id).await.len(), 1);
    assert_eq!(app.cash_rows(&id).await[0].amount_delta, "-10");

    // BUY -> OPENING_CASH: the position movement goes, the cash one changes.
    let cash_only = json!({
        "kind": "OPENING_CASH", "trade_date": "2026-01-01",
        "cash_account_id": book.cash, "amount": "-3.5",
    });
    let replaced = app.replace_txn(&alice, &book, &id, cash_only).await;
    assert_eq!(replaced.status, StatusCode::OK, "{}", replaced.body);
    assert!(app.position_rows(&id).await.is_empty());
    let cash = app.cash_rows(&id).await;
    assert_eq!(cash.len(), 1);
    assert_eq!(cash[0].amount_delta, "-3.5");

    // OPENING_CASH -> OPENING_POSITION: the cash movement goes.
    let opening = json!({
        "kind": "OPENING_POSITION", "trade_date": "2026-01-01",
        "instrument_id": book.instrument, "quantity": "7", "amount": "70.25",
    });
    let replaced = app.replace_txn(&alice, &book, &id, opening).await;
    assert_eq!(replaced.status, StatusCode::OK, "{}", replaced.body);
    assert!(app.cash_rows(&id).await.is_empty());
    let position = app.position_rows(&id).await;
    assert_eq!(position.len(), 1);
    assert_eq!(
        (
            position[0].kind.as_str(),
            position[0].quantity.as_str(),
            position[0].amount.as_str()
        ),
        ("ACQUIRE", "7", "70.25")
    );
    assert_eq!(app.ledger_row_counts().await, (1, 1, 0));
}

#[tokio::test]
async fn replacing_with_another_users_references_is_rejected() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let book = app.book(&alice).await;
    let bobs = app.book(&bob).await;
    let id = id_of(
        &app.record(&alice, &book, buy(&book, "2026-01-10", "2", "5"))
            .await,
    );

    for (field, value) in [
        ("instrument_id", bobs.instrument.clone()),
        ("cash_account_id", bobs.cash.clone()),
    ] {
        let body = with(buy(&book, "2026-01-10", "2", "5"), field, json!(value));
        let response = app.replace_txn(&alice, &book, &id, body).await;
        response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(response.body["errors"][0]["field"], field);
    }
    let stored = app.position_rows(&id).await;
    assert_eq!(stored[0].instrument_id, uuid(&book.instrument));
    assert!(app.cash_rows(&id).await.is_empty());
}

#[tokio::test]
async fn totals_beyond_the_numeric_range_are_rejected() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    // Each cost (5 × 10^28) fits a Decimal; their sum does not.
    let huge = buy(&book, "2026-01-10", "500000000000000", "100000000000000");
    let first = app.record(&alice, &book, huge.clone()).await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);

    let second = app.record(&alice, &book, huge).await;
    second.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(second.body["errors"][0]["field"], "quantity");
    assert_eq!(app.ledger_row_counts().await, (1, 1, 0));
}

#[tokio::test]
async fn deleting_a_purchase_that_a_sale_needs_is_rejected() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let buy_id = id_of(
        &app.record(&alice, &book, buy(&book, "2026-01-10", "10", "1"))
            .await,
    );
    let sell_id = id_of(
        &app.record(&alice, &book, sell(&book, "2026-02-01", "10", "1"))
            .await,
    );

    app.delete_txn(&alice, &book, &buy_id)
        .await
        .assert_problem(StatusCode::CONFLICT, "insufficient_holdings");
    assert_eq!(app.ledger_row_counts().await, (2, 2, 0));

    assert_eq!(
        app.delete_txn(&alice, &book, &sell_id).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.delete_txn(&alice, &book, &buy_id).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(app.ledger_row_counts().await, (0, 0, 0));
}

#[tokio::test]
async fn opening_lots_join_fifo_in_date_order() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    for (date, qty, cost) in [("2024-06-01", "3", "100"), ("2025-01-15", "2", "90")] {
        let lot = json!({
            "kind": "OPENING_POSITION", "trade_date": date,
            "instrument_id": book.instrument, "quantity": qty, "amount": cost,
        });
        assert_eq!(
            app.record(&alice, &book, lot).await.status,
            StatusCode::CREATED
        );
    }

    let response = app
        .record(&alice, &book, sell(&book, "2026-01-01", "5", "50"))
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    app.record(&alice, &book, sell(&book, "2026-01-02", "0.00000001", "50"))
        .await
        .assert_problem(StatusCode::CONFLICT, "insufficient_holdings");
}

// --- Concurrency ------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_sales_cannot_oversell() {
    let app = TestApp::new().await;
    let alice = Arc::new(user(&app, "alice@example.com").await);
    let book = Arc::new(app.book(&alice).await);
    app.record(&alice, &book, buy(&book, "2026-01-10", "10", "1"))
        .await;

    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let (app, alice, book) = (app.clone(), alice.clone(), book.clone());
        tasks.spawn(async move {
            app.record(&alice, &book, sell(&book, "2026-02-01", "3", "1"))
                .await
                .status
        });
    }
    let statuses = tasks.join_all().await;

    let created = statuses
        .iter()
        .filter(|s| **s == StatusCode::CREATED)
        .count();
    assert_eq!(
        created, 3,
        "10 units allow exactly three sales of 3: {statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::CONFLICT)
            .count(),
        5
    );
}

// --- History protects reference data ----------------------------------------

#[tokio::test]
async fn history_blocks_deleting_portfolio_instrument_and_cash_account() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let txn = id_of(
        &app.record(
            &alice,
            &book,
            with(
                buy(&book, "2026-01-10", "1", "1"),
                "cash_account_id",
                json!(book.cash),
            ),
        )
        .await,
    );
    let delete = |uri: String| {
        let (app, alice) = (&app, &alice);
        async move {
            app.send(
                Req::delete(&uri)
                    .session(&alice.session)
                    .header("x-csrf-token", &alice.csrf),
            )
            .await
        }
    };

    delete(format!("/api/v1/portfolios/{}", book.portfolio))
        .await
        .assert_problem(StatusCode::CONFLICT, "portfolio_has_history");
    delete(format!("/api/v1/instruments/{}", book.instrument))
        .await
        .assert_problem(StatusCode::CONFLICT, "instrument_in_use");
    delete(format!(
        "/api/v1/portfolios/{}/cash-accounts/{}",
        book.portfolio, book.cash
    ))
    .await
    .assert_problem(StatusCode::CONFLICT, "cash_account_in_use");
    assert_eq!(app.ledger_row_counts().await, (1, 1, 1));

    // Once the history is gone, everything can be removed.
    app.delete_txn(&alice, &book, &txn).await;
    assert_eq!(
        delete(format!("/api/v1/portfolios/{}", book.portfolio))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        delete(format!("/api/v1/instruments/{}", book.instrument))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
}
