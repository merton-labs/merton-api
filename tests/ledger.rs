#![allow(clippy::unwrap_used)]

//! Financial invariants of the ledger, checked against the stored rows.

mod common;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{Req, TestApp, TestResponse};
use merton_api::infrastructure::entities::{cash_movement, position_movement, transaction};
use rust_decimal::Decimal;
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

    async fn cash_account(&self, u: &User, portfolio: &str, name: &str, currency: &str) -> String {
        self.created(
            u,
            &format!("/api/v1/portfolios/{portfolio}/cash-accounts"),
            json!({ "name": name, "currency": currency }),
        )
        .await
    }

    /// An account's balance, summed in Rust from its cash movements.
    async fn balance(&self, cash_account_id: &str) -> Decimal {
        cash_movement::Entity::find()
            .filter(cash_movement::Column::CashAccountId.eq(uuid(cash_account_id)))
            .all(self.db())
            .await
            .unwrap()
            .iter()
            .map(|row| row.amount_delta.parse::<Decimal>().unwrap())
            .sum()
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

// --- Cash transactions (P4) -------------------------------------------------

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn cash_flow(kind: &str, account: &str, amount: &str) -> Value {
    json!({
        "kind": kind,
        "trade_date": "2026-03-01",
        "cash_account_id": account,
        "amount": amount,
    })
}

fn transfer(from: &str, to: &str, amount: &str) -> Value {
    json!({
        "kind": "CASH_TRANSFER",
        "trade_date": "2026-03-02",
        "cash_account_id": from,
        "destination_cash_account_id": to,
        "amount": amount,
    })
}

#[tokio::test]
async fn cash_transactions_post_exact_signed_movements() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;

    let postings = [
        (cash_flow("DEPOSIT", &book.cash, "1000.50"), "1000.50"),
        (cash_flow("WITHDRAWAL", &book.cash, "200"), "-200"),
        (cash_flow("FEE", &book.cash, "10.25"), "-10.25"),
        (cash_flow("TAX", &book.cash, "5.00"), "-5.00"),
        // Interest: 3.17 gross, 0.47 withheld, so 2.70 net.
        (
            json!({
                "kind": "INTEREST", "trade_date": "2026-03-31", "cash_account_id": book.cash,
                "amount": "3.17", "tax": "0.47",
            }),
            "2.70",
        ),
        // Dividend: 125.00 gross − 1.25 fee − 12.50 tax = 111.25 net.
        (
            json!({
                "kind": "DIVIDEND", "trade_date": "2026-04-20", "instrument_id": book.instrument,
                "cash_account_id": book.cash, "amount": "125.00", "fee": "1.25", "tax": "12.50",
            }),
            "111.25",
        ),
    ];
    for (body, delta) in postings {
        let response = app.record(&alice, &book, body.clone()).await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        let id = id_of(&response);
        assert!(app.position_rows(&id).await.is_empty(), "{body}");
        let rows = app.cash_rows(&id).await;
        assert_eq!(rows.len(), 1, "{body}");
        assert_eq!(rows[0].cash_account_id, uuid(&book.cash));
        assert_eq!(rows[0].amount_delta, delta, "{body}");
        // The input is kept as entered; the net amount is derived.
        assert_eq!(response.body["amount"], body["amount"]);
    }
    // 1000.50 − 200 − 10.25 − 5.00 + 2.70 + 111.25
    assert_eq!(app.balance(&book.cash).await, d("899.20"));
}

#[tokio::test]
async fn income_is_rejected_when_fee_and_tax_exceed_the_gross_amount() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;

    let response = app
        .record(
            &alice,
            &book,
            json!({
                "kind": "DIVIDEND", "trade_date": "2026-04-20", "instrument_id": book.instrument,
                "cash_account_id": book.cash, "amount": "10", "fee": "5", "tax": "5.01",
            }),
        )
        .await;
    response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(response.body["errors"][0]["field"], "amount");
    assert_eq!(app.ledger_row_counts().await, (0, 0, 0));
}

#[tokio::test]
async fn dividends_leave_holdings_and_cost_basis_alone() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    app.record(&alice, &book, buy(&book, "2026-01-10", "10", "5"))
        .await;

    // Without a cash account, a dividend is recorded but moves nothing.
    let dividend = app
        .record(
            &alice,
            &book,
            json!({
                "kind": "DIVIDEND", "trade_date": "2026-02-01",
                "instrument_id": book.instrument, "amount": "4.00",
            }),
        )
        .await;
    assert_eq!(dividend.status, StatusCode::CREATED, "{}", dividend.body);
    let id = id_of(&dividend);
    assert!(app.position_rows(&id).await.is_empty());
    assert!(app.cash_rows(&id).await.is_empty());

    // The whole position can still be sold, and only once.
    let sold = app
        .record(&alice, &book, sell(&book, "2026-03-01", "10", "6"))
        .await;
    assert_eq!(sold.status, StatusCode::CREATED, "{}", sold.body);
    app.record(&alice, &book, sell(&book, "2026-03-02", "0.00000001", "6"))
        .await
        .assert_problem(StatusCode::CONFLICT, "insufficient_holdings");
}

#[tokio::test]
async fn dividend_cash_account_must_use_the_quote_currency() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let usd = app
        .cash_account(&alice, &book.portfolio, "USD", "USD")
        .await;

    let response = app
        .record(
            &alice,
            &book,
            json!({
                "kind": "DIVIDEND", "trade_date": "2026-04-20", "instrument_id": book.instrument,
                "cash_account_id": usd, "amount": "10",
            }),
        )
        .await;
    response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(response.body["errors"][0]["field"], "cash_account_id");
    assert_eq!(app.ledger_row_counts().await, (0, 0, 0));
}

#[tokio::test]
async fn transfers_post_balanced_movements() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let savings = app
        .cash_account(&alice, &book.portfolio, "Savings", "THB")
        .await;
    app.record(&alice, &book, cash_flow("DEPOSIT", &book.cash, "1000"))
        .await;

    let response = app
        .record(&alice, &book, transfer(&book.cash, &savings, "400.10"))
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    assert_eq!(response.body["destination_cash_account_id"], savings);
    let mut rows = app.cash_rows(&id_of(&response)).await;
    rows.sort_by_key(|row| row.amount_delta.starts_with('-'));
    let moves: Vec<_> = rows
        .iter()
        .map(|row| (row.cash_account_id, row.amount_delta.as_str()))
        .collect();
    assert_eq!(
        moves,
        [(uuid(&savings), "400.10"), (uuid(&book.cash), "-400.10")]
    );
    assert_eq!(app.balance(&book.cash).await, d("599.90"));
    assert_eq!(app.balance(&savings).await, d("400.10"));
    // A transfer neither creates nor destroys money.
    assert_eq!(
        app.balance(&book.cash).await + app.balance(&savings).await,
        d("1000")
    );
}

#[tokio::test]
async fn transfers_stay_within_one_portfolio_owner_and_currency() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let book = app.book(&alice).await;
    let other_book = app.book(&alice).await;
    let bobs = app.book(&bob).await;
    let usd = app
        .cash_account(&alice, &book.portfolio, "USD", "USD")
        .await;

    for (body, field) in [
        // Another currency is an FX exchange, not a transfer.
        (
            transfer(&book.cash, &usd, "1"),
            "destination_cash_account_id",
        ),
        // Accounts in another portfolio, even one's own, are not reachable.
        (
            transfer(&book.cash, &other_book.cash, "1"),
            "destination_cash_account_id",
        ),
        (
            transfer(&book.cash, &bobs.cash, "1"),
            "destination_cash_account_id",
        ),
        (transfer(&bobs.cash, &book.cash, "1"), "cash_account_id"),
        (
            transfer(&book.cash, &book.cash, "1"),
            "destination_cash_account_id",
        ),
    ] {
        let response = app.record(&alice, &book, body.clone()).await;
        response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(response.body["errors"][0]["field"], field, "{body}");
    }
    // A foreign account is reported exactly like a missing one.
    let foreign = app
        .record(&alice, &book, transfer(&book.cash, &bobs.cash, "1"))
        .await;
    let missing = app
        .record(
            &alice,
            &book,
            transfer(&book.cash, &Uuid::now_v7().to_string(), "1"),
        )
        .await;
    assert_eq!(foreign.body["errors"], missing.body["errors"]);
    assert_eq!(app.ledger_row_counts().await, (0, 0, 0));
}

#[tokio::test]
async fn replacing_a_transfer_reposts_both_movements() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let savings = app
        .cash_account(&alice, &book.portfolio, "Savings", "THB")
        .await;
    let id = id_of(
        &app.record(&alice, &book, transfer(&book.cash, &savings, "50"))
            .await,
    );

    let replaced = app
        .replace_txn(&alice, &book, &id, cash_flow("DEPOSIT", &savings, "70"))
        .await;
    assert_eq!(replaced.status, StatusCode::OK, "{}", replaced.body);
    assert!(replaced.body["destination_cash_account_id"].is_null());
    assert_eq!(app.cash_rows(&id).await.len(), 1);
    assert_eq!(app.balance(&book.cash).await, d("0"));
    assert_eq!(app.balance(&savings).await, d("70"));

    // Deleting removes every movement.
    assert_eq!(
        app.delete_txn(&alice, &book, &id).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(app.ledger_row_counts().await, (0, 0, 0));
}

#[tokio::test]
async fn transfer_history_protects_the_destination_account() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let savings = app
        .cash_account(&alice, &book.portfolio, "Savings", "THB")
        .await;
    app.record(&alice, &book, transfer(&book.cash, &savings, "1"))
        .await;

    app.send(
        Req::delete(&format!(
            "/api/v1/portfolios/{}/cash-accounts/{savings}",
            book.portfolio
        ))
        .session(&alice.session)
        .header("x-csrf-token", &alice.csrf),
    )
    .await
    .assert_problem(StatusCode::CONFLICT, "cash_account_in_use");
    assert_eq!(app.ledger_row_counts().await, (1, 0, 2));
}

#[tokio::test]
async fn transfer_destination_column_restricts_deletes() {
    let app = TestApp::new().await;
    let db = app.db();
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            "SELECT \"table\", \"to\", on_delete FROM pragma_foreign_key_list('transactions') \
             WHERE \"from\" = 'destination_cash_account_id'",
        ))
        .await
        .unwrap()
        .unwrap();
    let target: String = row.try_get("", "table").unwrap();
    let column: String = row.try_get("", "to").unwrap();
    let on_delete: String = row.try_get("", "on_delete").unwrap();
    assert_eq!(
        (target.as_str(), column.as_str(), on_delete.as_str()),
        ("cash_accounts", "id", "RESTRICT")
    );
}

// --- Tax returns, FX exchanges and corporate actions (P4) -------------------

fn fx(from: &str, to: &str, spent: &str, received: &str) -> Value {
    json!({
        "kind": "FX_EXCHANGE",
        "trade_date": "2026-03-03",
        "cash_account_id": from,
        "destination_cash_account_id": to,
        "amount": spent,
        "destination_amount": received,
    })
}

fn split(book: &Book, date: &str, ratio: &str) -> Value {
    json!({
        "kind": "STOCK_SPLIT",
        "trade_date": date,
        "instrument_id": book.instrument,
        "ratio": ratio,
    })
}

impl TestApp {
    /// Records `body`, asserting it was accepted.
    async fn ok(&self, u: &User, book: &Book, body: Value) -> String {
        let response = self.record(u, book, body).await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        id_of(&response)
    }

    /// Asserts that exactly `held` units can be sold on `date`, without
    /// changing the ledger.
    async fn assert_holding(&self, u: &User, book: &Book, date: &str, held: &str) {
        let probe = |quantity: &str| sell(book, date, quantity, "1");
        let over = format!("{}", d(held) + d("0.00000001"));
        self.record(u, book, with(probe(&over), "sort_order", json!(1000)))
            .await
            .assert_problem(StatusCode::CONFLICT, "insufficient_holdings");
        let id = self
            .ok(u, book, with(probe(held), "sort_order", json!(1000)))
            .await;
        assert_eq!(
            self.delete_txn(u, book, &id).await.status,
            StatusCode::NO_CONTENT
        );
    }
}

#[tokio::test]
async fn tax_returns_credit_the_account() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;

    let id = app
        .ok(&alice, &book, cash_flow("TAX_RETURN", &book.cash, "52.40"))
        .await;
    assert_eq!(app.cash_rows(&id).await[0].amount_delta, "52.40");
    assert!(app.position_rows(&id).await.is_empty());
}

#[tokio::test]
async fn transfer_fees_are_paid_by_the_source() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let savings = app
        .cash_account(&alice, &book.portfolio, "Savings", "THB")
        .await;

    app.ok(
        &alice,
        &book,
        with(transfer(&book.cash, &savings, "100"), "fee", json!("15")),
    )
    .await;
    assert_eq!(app.balance(&book.cash).await, d("-115"));
    assert_eq!(app.balance(&savings).await, d("100"));
}

#[tokio::test]
async fn fx_exchanges_post_both_currencies_and_the_fee_on_the_source() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let usd = app
        .cash_account(&alice, &book.portfolio, "USD", "USD")
        .await;

    // 3,300 THB plus a 25 THB fee buys 100.00 USD.
    let response = app
        .record(
            &alice,
            &book,
            with(fx(&book.cash, &usd, "3300", "100.00"), "fee", json!("25")),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    assert_eq!(response.body["amount"], "3300");
    assert_eq!(response.body["destination_amount"], "100.00");
    assert_eq!(response.body["fee"], "25");
    assert_eq!(app.balance(&book.cash).await, d("-3325"));
    assert_eq!(app.balance(&usd).await, d("100.00"));
    assert!(app.position_rows(&id_of(&response)).await.is_empty());
}

#[tokio::test]
async fn fx_exchanges_need_two_currencies_in_one_portfolio() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let book = app.book(&alice).await;
    let bobs = app.book(&bob).await;
    let savings = app
        .cash_account(&alice, &book.portfolio, "Savings", "THB")
        .await;
    let bobs_usd = app.cash_account(&bob, &bobs.portfolio, "USD", "USD").await;

    for (body, field) in [
        // Same currency: that is a transfer.
        (
            fx(&book.cash, &savings, "1", "1"),
            "destination_cash_account_id",
        ),
        (
            fx(&book.cash, &bobs_usd, "1", "1"),
            "destination_cash_account_id",
        ),
        (
            with(
                fx(&book.cash, &savings, "1", "1"),
                "destination_amount",
                Value::Null,
            ),
            "destination_amount",
        ),
    ] {
        let response = app.record(&alice, &book, body.clone()).await;
        response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(response.body["errors"][0]["field"], field, "{body}");
    }
    assert_eq!(app.ledger_row_counts().await, (0, 0, 0));
}

#[tokio::test]
async fn splits_scale_holdings_and_replay_after_edits() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let bought = app
        .ok(&alice, &book, buy(&book, "2026-01-10", "10", "40"))
        .await;
    let split_id = app
        .ok(&alice, &book, split(&book, "2026-02-01", "4:1"))
        .await;

    let rows = app.position_rows(&split_id).await;
    assert_eq!(
        (
            rows[0].kind.as_str(),
            rows[0].quantity.as_str(),
            rows[0].amount.as_str()
        ),
        ("SPLIT", "4", "1")
    );
    app.assert_holding(&alice, &book, "2026-02-01", "40").await;

    // A sale of all 40 depends on the purchase made before the split.
    app.ok(&alice, &book, sell(&book, "2026-03-01", "40", "11"))
        .await;
    app.replace_txn(&alice, &book, &bought, buy(&book, "2026-01-10", "9", "40"))
        .await
        .assert_problem(StatusCode::CONFLICT, "insufficient_holdings");
    // Removing the split leaves the sale uncovered too.
    app.delete_txn(&alice, &book, &split_id)
        .await
        .assert_problem(StatusCode::CONFLICT, "insufficient_holdings");
}

#[tokio::test]
async fn reverse_splits_can_pay_the_fraction_as_cash_in_lieu() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    app.ok(&alice, &book, buy(&book, "2026-01-10", "10", "3"))
        .await;

    // 10 shares 1:3 is 3.33333333; the 0.33333333 is paid out as 4.00.
    let mut body = split(&book, "2026-02-01", "1:3");
    body["cash_in_lieu_quantity"] = json!("0.33333333");
    body["amount"] = json!("4.00");
    body["cash_account_id"] = json!(book.cash);
    let id = app.ok(&alice, &book, body).await;

    let rows = app.position_rows(&id).await;
    let kinds: Vec<_> = rows.iter().map(|r| r.kind.as_str()).collect();
    assert!(
        kinds.contains(&"SPLIT") && kinds.contains(&"DISPOSE"),
        "{kinds:?}"
    );
    assert_eq!(app.cash_rows(&id).await[0].amount_delta, "4.00");
    app.assert_holding(&alice, &book, "2026-02-01", "3").await;
}

#[tokio::test]
async fn stock_dividends_add_shares() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    app.ok(&alice, &book, buy(&book, "2026-01-10", "10", "5"))
        .await;
    app.ok(&alice, &book, buy(&book, "2026-01-20", "30", "6"))
        .await;

    let id = app
        .ok(
            &alice,
            &book,
            json!({
                "kind": "STOCK_DIVIDEND", "trade_date": "2026-02-01",
                "instrument_id": book.instrument, "quantity": "4",
            }),
        )
        .await;
    let rows = app.position_rows(&id).await;
    assert_eq!(
        (
            rows[0].kind.as_str(),
            rows[0].quantity.as_str(),
            rows[0].amount.as_str()
        ),
        ("STOCK_DIVIDEND", "4", "0")
    );
    assert!(app.cash_rows(&id).await.is_empty());
    app.assert_holding(&alice, &book, "2026-02-01", "44").await;
}

#[tokio::test]
async fn corporate_actions_need_a_holding() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;

    app.record(&alice, &book, split(&book, "2026-02-01", "2:1"))
        .await
        .assert_problem(StatusCode::CONFLICT, "no_holdings");
    assert_eq!(app.ledger_row_counts().await, (0, 0, 0));

    // Deleting the only purchase before a split is refused the same way.
    let bought = app
        .ok(&alice, &book, buy(&book, "2026-01-10", "1", "5"))
        .await;
    app.ok(&alice, &book, split(&book, "2026-02-01", "2:1"))
        .await;
    app.delete_txn(&alice, &book, &bought)
        .await
        .assert_problem(StatusCode::CONFLICT, "no_holdings");
    assert_eq!(app.ledger_row_counts().await, (2, 2, 0));
}

#[tokio::test]
async fn cash_in_lieu_goes_to_an_account_in_the_quote_currency() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let book = app.book(&alice).await;
    let usd = app
        .cash_account(&alice, &book.portfolio, "USD", "USD")
        .await;
    app.ok(&alice, &book, buy(&book, "2026-01-10", "3", "1"))
        .await;

    let mut body = split(&book, "2026-02-01", "1:2");
    body["cash_in_lieu_quantity"] = json!("0.5");
    body["amount"] = json!("1");
    body["cash_account_id"] = json!(usd);
    let response = app.record(&alice, &book, body).await;
    response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(response.body["errors"][0]["field"], "cash_account_id");
}
