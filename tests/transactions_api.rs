#![allow(clippy::unwrap_used)]

mod common;

use axum::http::{StatusCode, header};
use common::{Req, TestApp, TestResponse};
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

fn transactions_url(portfolio: &str) -> String {
    format!("/api/v1/portfolios/{portfolio}/transactions")
}

fn transaction_url(portfolio: &str, transaction: &str) -> String {
    format!("/api/v1/portfolios/{portfolio}/transactions/{transaction}")
}

fn id_of(response: &TestResponse) -> String {
    response.body["id"].as_str().unwrap().to_owned()
}

fn error_fields(response: &TestResponse) -> Vec<&str> {
    response.body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect()
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

    async fn archive_portfolio(&self, u: &User, portfolio: &str) {
        let response = self
            .send(
                Req::post(&format!("/api/v1/portfolios/{portfolio}/archive"))
                    .session(&u.session)
                    .header("x-csrf-token", &u.csrf),
            )
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    }

    async fn new_instrument(&self, u: &User, name: &str, currency: &str) -> String {
        let response = self
            .send(
                Req::post("/api/v1/instruments")
                    .session(&u.session)
                    .header("x-csrf-token", &u.csrf)
                    .json(json!({
                        "asset_class": "STOCK",
                        "name": name,
                        "quote_currency": currency,
                    })),
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        id_of(&response)
    }

    async fn new_cash_account(
        &self,
        u: &User,
        portfolio: &str,
        name: &str,
        currency: &str,
    ) -> String {
        let response = self
            .send(
                Req::post(&format!("/api/v1/portfolios/{portfolio}/cash-accounts"))
                    .session(&u.session)
                    .header("x-csrf-token", &u.csrf)
                    .json(json!({ "name": name, "currency": currency })),
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        id_of(&response)
    }

    async fn create_transaction(&self, u: &User, portfolio: &str, body: Value) -> TestResponse {
        self.send(
            Req::post(&transactions_url(portfolio))
                .session(&u.session)
                .header("x-csrf-token", &u.csrf)
                .json(body),
        )
        .await
    }

    async fn new_transaction(&self, u: &User, portfolio: &str, body: Value) -> String {
        let response = self.create_transaction(u, portfolio, body).await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        id_of(&response)
    }

    async fn list_transactions(&self, u: &User, portfolio: &str, query: &str) -> TestResponse {
        self.send(Req::get(&format!("{}{query}", transactions_url(portfolio))).session(&u.session))
            .await
    }

    async fn get_transaction(&self, u: &User, portfolio: &str, transaction: &str) -> TestResponse {
        self.send(Req::get(&transaction_url(portfolio, transaction)).session(&u.session))
            .await
    }

    async fn replace_transaction(
        &self,
        u: &User,
        portfolio: &str,
        transaction: &str,
        body: Value,
    ) -> TestResponse {
        self.send(
            Req::put(&transaction_url(portfolio, transaction))
                .session(&u.session)
                .header("x-csrf-token", &u.csrf)
                .json(body),
        )
        .await
    }

    async fn delete_transaction(
        &self,
        u: &User,
        portfolio: &str,
        transaction: &str,
    ) -> TestResponse {
        self.send(
            Req::delete(&transaction_url(portfolio, transaction))
                .session(&u.session)
                .header("x-csrf-token", &u.csrf),
        )
        .await
    }
}

// --- Create and validation --------------------------------------------------

#[tokio::test]
async fn create_accepts_minimal_body_for_each_kind_and_sets_defaults() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Main").await;
    let instrument = app.new_instrument(&alice, "PTT", "THB").await;
    let cash = app
        .new_cash_account(&alice, &portfolio, "Broker", "THB")
        .await;

    let opening_pos = app
        .create_transaction(
            &alice,
            &portfolio,
            json!({
                "kind": "OPENING_POSITION",
                "trade_date": "2026-01-01",
                "instrument_id": instrument,
                "quantity": "10",
                "amount": "350.00",
            }),
        )
        .await;
    assert_eq!(
        opening_pos.status,
        StatusCode::CREATED,
        "{}",
        opening_pos.body
    );
    assert_eq!(
        opening_pos.headers[header::LOCATION],
        transaction_url(&portfolio, &id_of(&opening_pos)).as_str()
    );
    assert_eq!(opening_pos.headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(opening_pos.body["portfolio_id"], portfolio);
    assert_eq!(opening_pos.body["kind"], "OPENING_POSITION");
    assert_eq!(opening_pos.body["trade_date"], "2026-01-01");
    assert_eq!(opening_pos.body["sort_order"], 0);
    assert_eq!(opening_pos.body["instrument_id"], instrument);
    assert_eq!(opening_pos.body["cash_account_id"], Value::Null);
    assert_eq!(opening_pos.body["quantity"], "10");
    assert_eq!(opening_pos.body["price"], Value::Null);
    assert_eq!(opening_pos.body["fee"], Value::Null);
    assert_eq!(opening_pos.body["tax"], Value::Null);
    assert_eq!(opening_pos.body["amount"], "350.00");
    assert_eq!(opening_pos.body["note"], Value::Null);
    assert_eq!(
        opening_pos.body["created_at"],
        opening_pos.body["updated_at"]
    );

    let opening_cash = app
        .create_transaction(
            &alice,
            &portfolio,
            json!({
                "kind": "OPENING_CASH",
                "trade_date": "2026-01-01",
                "cash_account_id": cash,
                "amount": "-250.50",
            }),
        )
        .await;
    assert_eq!(
        opening_cash.status,
        StatusCode::CREATED,
        "{}",
        opening_cash.body
    );
    assert_eq!(
        opening_cash.headers[header::LOCATION],
        transaction_url(&portfolio, &id_of(&opening_cash)).as_str()
    );
    assert_eq!(opening_cash.body["kind"], "OPENING_CASH");
    assert_eq!(opening_cash.body["sort_order"], 0);
    assert_eq!(opening_cash.body["instrument_id"], Value::Null);
    assert_eq!(opening_cash.body["cash_account_id"], cash);
    assert_eq!(opening_cash.body["quantity"], Value::Null);
    assert_eq!(opening_cash.body["price"], Value::Null);
    assert_eq!(opening_cash.body["fee"], Value::Null);
    assert_eq!(opening_cash.body["tax"], Value::Null);
    assert_eq!(opening_cash.body["amount"], "-250.50");
    assert_eq!(opening_cash.body["note"], Value::Null);

    let buy = app
        .create_transaction(
            &alice,
            &portfolio,
            json!({
                "kind": "BUY",
                "trade_date": "2026-01-02",
                "instrument_id": instrument,
                "quantity": "5",
                "price": "36.25",
            }),
        )
        .await;
    assert_eq!(buy.status, StatusCode::CREATED, "{}", buy.body);
    assert_eq!(
        buy.headers[header::LOCATION],
        transaction_url(&portfolio, &id_of(&buy)).as_str()
    );
    assert_eq!(buy.body["kind"], "BUY");
    assert_eq!(buy.body["sort_order"], 0);
    assert_eq!(buy.body["instrument_id"], instrument);
    assert_eq!(buy.body["cash_account_id"], Value::Null);
    assert_eq!(buy.body["quantity"], "5");
    assert_eq!(buy.body["price"], "36.25");
    assert_eq!(buy.body["fee"], "0");
    assert_eq!(buy.body["tax"], "0");
    assert_eq!(buy.body["amount"], Value::Null);
    assert_eq!(buy.body["note"], Value::Null);

    let sell = app
        .create_transaction(
            &alice,
            &portfolio,
            json!({
                "kind": "SELL",
                "trade_date": "2026-01-03",
                "instrument_id": instrument,
                "quantity": "4",
                "price": "38.00",
            }),
        )
        .await;
    assert_eq!(sell.status, StatusCode::CREATED, "{}", sell.body);
    assert_eq!(
        sell.headers[header::LOCATION],
        transaction_url(&portfolio, &id_of(&sell)).as_str()
    );
    assert_eq!(sell.body["kind"], "SELL");
    assert_eq!(sell.body["instrument_id"], instrument);
    assert_eq!(sell.body["cash_account_id"], Value::Null);
    assert_eq!(sell.body["quantity"], "4");
    assert_eq!(sell.body["price"], "38.00");
    assert_eq!(sell.body["fee"], "0");
    assert_eq!(sell.body["tax"], "0");
    assert_eq!(sell.body["amount"], Value::Null);
}

#[tokio::test]
async fn create_enforces_required_and_forbidden_fields_per_kind() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Main").await;
    let instrument = app.new_instrument(&alice, "PTT", "THB").await;
    let cash = app
        .new_cash_account(&alice, &portfolio, "Broker", "THB")
        .await;

    let cases: [(&str, &[&str], Value, &[&str]); 4] = [
        (
            "OPENING_POSITION",
            &["instrument_id", "quantity", "amount"],
            json!({
                "kind": "OPENING_POSITION",
                "trade_date": "2026-01-01",
                "instrument_id": instrument,
                "quantity": "10",
                "amount": "100",
                "cash_account_id": cash,
                "price": "10",
                "fee": "1",
                "tax": "1",
            }),
            &["cash_account_id", "price", "fee", "tax"],
        ),
        (
            "OPENING_CASH",
            &["cash_account_id", "amount"],
            json!({
                "kind": "OPENING_CASH",
                "trade_date": "2026-01-01",
                "cash_account_id": cash,
                "amount": "100",
                "instrument_id": instrument,
                "quantity": "1",
                "price": "10",
                "fee": "1",
                "tax": "1",
            }),
            &["instrument_id", "quantity", "price", "fee", "tax"],
        ),
        (
            "BUY",
            &["instrument_id", "quantity", "price"],
            json!({
                "kind": "BUY",
                "trade_date": "2026-01-01",
                "instrument_id": instrument,
                "quantity": "1",
                "price": "10",
                "amount": "10",
            }),
            &["amount"],
        ),
        (
            "SELL",
            &["instrument_id", "quantity", "price"],
            json!({
                "kind": "SELL",
                "trade_date": "2026-01-01",
                "instrument_id": instrument,
                "quantity": "1",
                "price": "10",
                "amount": "10",
            }),
            &["amount"],
        ),
    ];

    for (kind, required, with_forbidden, forbidden) in cases {
        let missing = app
            .create_transaction(
                &alice,
                &portfolio,
                json!({ "kind": kind, "trade_date": "2026-01-01" }),
            )
            .await;
        missing.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(error_fields(&missing), required, "missing for {kind}");
        for err in missing.body["errors"].as_array().unwrap() {
            let msg = err["message"].as_str().unwrap();
            assert!(
                msg.contains(&format!("is required for {kind}")),
                "{kind}: {msg}"
            );
        }

        let disallowed = app
            .create_transaction(&alice, &portfolio, with_forbidden)
            .await;
        disallowed.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(error_fields(&disallowed), forbidden, "forbidden for {kind}");
        for err in disallowed.body["errors"].as_array().unwrap() {
            let msg = err["message"].as_str().unwrap();
            assert!(
                msg.contains(&format!("is not allowed for {kind}")),
                "{kind}: {msg}"
            );
        }
    }

    let long_note = app
        .create_transaction(
            &alice,
            &portfolio,
            json!({
                "kind": "OPENING_CASH",
                "trade_date": "2026-01-01",
                "cash_account_id": cash,
                "amount": "100",
                "note": "x".repeat(501),
            }),
        )
        .await;
    long_note.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(error_fields(&long_note), ["note"]);

    for malformed in [
        json!({
            "kind": "DIVIDEND",
            "trade_date": "2026-01-01",
        }),
        json!({
            "kind": "OPENING_CASH",
            "trade_date": "2026-01-01",
            "cash_account_id": cash,
            "amount": "100",
            "unknown_field": true,
        }),
    ] {
        app.create_transaction(&alice, &portfolio, malformed)
            .await
            .assert_problem(StatusCode::BAD_REQUEST, "malformed_request");
    }
}

#[tokio::test]
async fn decimal_grammar_limits_and_scale_are_enforced() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Main").await;
    let instrument = app.new_instrument(&alice, "PTT", "THB").await;
    let cash = app
        .new_cash_account(&alice, &portfolio, "Broker", "THB")
        .await;

    let sixteen_digits = "9".repeat(16);
    for bad in [
        "1e3",
        "+1",
        "1_000",
        "1.123456789",
        sixteen_digits.as_str(),
        "",
        ".5",
        "5.",
        "0",
        "-1",
    ] {
        let response = app
            .create_transaction(
                &alice,
                &portfolio,
                json!({
                    "kind": "BUY",
                    "trade_date": "2026-01-01",
                    "instrument_id": instrument,
                    "quantity": bad,
                    "price": "10",
                }),
            )
            .await;
        response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(error_fields(&response), ["quantity"], "input: {bad}");
    }

    for number_body in [
        json!({
            "kind": "BUY",
            "trade_date": "2026-01-01",
            "instrument_id": instrument,
            "quantity": 10,
            "price": "35.25",
        }),
        json!({
            "kind": "BUY",
            "trade_date": "2026-01-01",
            "instrument_id": instrument,
            "quantity": "10",
            "price": 35.25,
        }),
        json!({
            "kind": "OPENING_CASH",
            "trade_date": "2026-01-01",
            "cash_account_id": cash,
            "amount": 100,
        }),
    ] {
        app.create_transaction(&alice, &portfolio, number_body)
            .await
            .assert_problem(StatusCode::BAD_REQUEST, "malformed_request");
    }

    let created = app
        .create_transaction(
            &alice,
            &portfolio,
            json!({
                "kind": "BUY",
                "trade_date": "2026-01-01",
                "instrument_id": instrument,
                "quantity": "100.50",
                "price": "007.5",
                "fee": "1.2000",
                "tax": "0.00000001",
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.body["quantity"], "100.50");
    assert_eq!(created.body["price"], "7.5");
    assert_eq!(created.body["fee"], "1.2000");
    assert_eq!(created.body["tax"], "0.00000001");

    let fetched = app
        .get_transaction(&alice, &portfolio, &id_of(&created))
        .await;
    assert_eq!(fetched.body["quantity"], "100.50");
    assert_eq!(fetched.body["price"], "7.5");
    assert_eq!(fetched.body["fee"], "1.2000");
    assert_eq!(fetched.body["tax"], "0.00000001");
}

#[tokio::test]
async fn trade_date_validates_format_calendar_and_bounds() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Main").await;
    let cash = app
        .new_cash_account(&alice, &portfolio, "Broker", "THB")
        .await;

    for bad_date in [
        "2026-02-30",
        "2026/01/01",
        "1899-12-31",
        "2101-01-01",
        "2026-1-01",
        "",
    ] {
        let response = app
            .create_transaction(
                &alice,
                &portfolio,
                json!({
                    "kind": "OPENING_CASH",
                    "trade_date": bad_date,
                    "cash_account_id": cash,
                    "amount": "100.00",
                }),
            )
            .await;
        response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(error_fields(&response), ["trade_date"], "date: {bad_date}");
    }

    for edge_date in ["1900-01-01", "2100-12-31"] {
        let ok = app
            .create_transaction(
                &alice,
                &portfolio,
                json!({
                    "kind": "OPENING_CASH",
                    "trade_date": edge_date,
                    "cash_account_id": cash,
                    "amount": "10.00",
                }),
            )
            .await;
        assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.body);
        assert_eq!(ok.body["trade_date"], edge_date);
    }
}

// --- List, replace, delete --------------------------------------------------

#[tokio::test]
async fn list_orders_by_ledger_sequence_paginates_and_filters_by_instrument() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Main").await;
    let inst_a = app.new_instrument(&alice, "Alpha", "THB").await;
    let inst_b = app.new_instrument(&alice, "Beta", "THB").await;
    let cash = app
        .new_cash_account(&alice, &portfolio, "Broker", "THB")
        .await;

    let t_late = app
        .new_transaction(
            &alice,
            &portfolio,
            json!({
                "kind": "BUY",
                "trade_date": "2026-01-03",
                "sort_order": 0,
                "instrument_id": inst_a,
                "quantity": "5",
                "price": "12",
            }),
        )
        .await;
    let t_mid_sort2 = app
        .new_transaction(
            &alice,
            &portfolio,
            json!({
                "kind": "BUY",
                "trade_date": "2026-01-02",
                "sort_order": 2,
                "instrument_id": inst_b,
                "quantity": "3",
                "price": "20",
            }),
        )
        .await;
    let t_mid_tie_first = app
        .new_transaction(
            &alice,
            &portfolio,
            json!({
                "kind": "BUY",
                "trade_date": "2026-01-02",
                "sort_order": 1,
                "instrument_id": inst_a,
                "quantity": "2",
                "price": "11",
            }),
        )
        .await;
    let t_mid_tie_second = app
        .new_transaction(
            &alice,
            &portfolio,
            json!({
                "kind": "OPENING_CASH",
                "trade_date": "2026-01-02",
                "sort_order": 1,
                "cash_account_id": cash,
                "amount": "500.00",
            }),
        )
        .await;
    let t_early = app
        .new_transaction(
            &alice,
            &portfolio,
            json!({
                "kind": "OPENING_POSITION",
                "trade_date": "2026-01-01",
                "sort_order": 9,
                "instrument_id": inst_a,
                "quantity": "10",
                "amount": "100.00",
            }),
        )
        .await;

    let expected = vec![
        t_early.clone(),
        t_mid_tie_first.clone(),
        t_mid_tie_second,
        t_mid_sort2,
        t_late.clone(),
    ];

    let all = app.list_transactions(&alice, &portfolio, "").await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.body);
    assert_eq!(all.body["total"], 5);
    assert_eq!(all.body["limit"], 50);
    assert_eq!(all.body["offset"], 0);
    let ids: Vec<String> = all.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, expected);

    let slice = app
        .list_transactions(&alice, &portfolio, "?limit=2&offset=1")
        .await;
    assert_eq!(slice.status, StatusCode::OK);
    assert_eq!(slice.body["total"], 5);
    assert_eq!(slice.body["limit"], 2);
    assert_eq!(slice.body["offset"], 1);
    let slice_ids: Vec<String> = slice.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(slice_ids, expected[1..3]);

    let filtered = app
        .list_transactions(&alice, &portfolio, &format!("?instrument_id={inst_a}"))
        .await;
    assert_eq!(filtered.status, StatusCode::OK);
    assert_eq!(filtered.body["total"], 3);
    let filtered_ids: Vec<String> = filtered.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(filtered_ids, vec![t_early, t_mid_tie_first, t_late]);

    for bad_query in ["?limit=0", "?limit=201"] {
        app.list_transactions(&alice, &portfolio, bad_query)
            .await
            .assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    }
}

#[tokio::test]
async fn replace_updates_all_fields_preserving_created_at_and_delete_removes_entry() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Main").await;
    let instrument = app.new_instrument(&alice, "PTT", "THB").await;
    let cash = app
        .new_cash_account(&alice, &portfolio, "Broker", "THB")
        .await;

    app.new_transaction(
        &alice,
        &portfolio,
        json!({
            "kind": "OPENING_POSITION",
            "trade_date": "2026-01-01",
            "instrument_id": instrument,
            "quantity": "20",
            "amount": "700.00",
        }),
    )
    .await;

    let created = app
        .create_transaction(
            &alice,
            &portfolio,
            json!({
                "kind": "BUY",
                "trade_date": "2026-01-02",
                "sort_order": 1,
                "instrument_id": instrument,
                "quantity": "5",
                "price": "36.00",
                "fee": "2.00",
                "tax": "0.14",
                "note": "Initial buy",
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let tid = id_of(&created);

    let replaced = app
        .replace_transaction(
            &alice,
            &portfolio,
            &tid,
            json!({
                "kind": "SELL",
                "trade_date": "2026-01-05",
                "sort_order": 4,
                "instrument_id": instrument,
                "cash_account_id": cash,
                "quantity": "8",
                "price": "40.50",
            }),
        )
        .await;
    assert_eq!(replaced.status, StatusCode::OK, "{}", replaced.body);
    assert_eq!(replaced.body["id"], tid);
    assert_eq!(replaced.body["portfolio_id"], portfolio);
    assert_eq!(replaced.body["kind"], "SELL");
    assert_eq!(replaced.body["trade_date"], "2026-01-05");
    assert_eq!(replaced.body["sort_order"], 4);
    assert_eq!(replaced.body["instrument_id"], instrument);
    assert_eq!(replaced.body["cash_account_id"], cash);
    assert_eq!(replaced.body["quantity"], "8");
    assert_eq!(replaced.body["price"], "40.50");
    assert_eq!(replaced.body["fee"], "0");
    assert_eq!(replaced.body["tax"], "0");
    assert_eq!(replaced.body["amount"], Value::Null);
    assert_eq!(replaced.body["note"], Value::Null);
    assert_eq!(replaced.body["created_at"], created.body["created_at"]);
    assert!(replaced.body["updated_at"].as_str() >= created.body["updated_at"].as_str());

    let deleted = app.delete_transaction(&alice, &portfolio, &tid).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    app.get_transaction(&alice, &portfolio, &tid)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    app.delete_transaction(&alice, &portfolio, &tid)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
}

// --- References, isolation, lifecycle, security, OpenAPI --------------------

#[tokio::test]
async fn reference_validation_checks_ownership_portfolio_scope_and_currency() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let main = app.new_portfolio(&alice, "Main").await;
    let side = app.new_portfolio(&alice, "Side").await;
    let usd_stock = app.new_instrument(&alice, "AAPL", "USD").await;
    let thb_stock = app.new_instrument(&alice, "PTT", "THB").await;
    let bob_stock = app.new_instrument(&bob, "BobCorp", "THB").await;
    let main_thb_cash = app.new_cash_account(&alice, &main, "THB Cash", "THB").await;
    let side_thb_cash = app
        .new_cash_account(&alice, &side, "Side Cash", "THB")
        .await;

    let foreign_inst = app
        .create_transaction(
            &alice,
            &main,
            json!({
                "kind": "BUY",
                "trade_date": "2026-01-01",
                "instrument_id": bob_stock,
                "quantity": "1",
                "price": "10",
            }),
        )
        .await;
    let random_inst = app
        .create_transaction(
            &alice,
            &main,
            json!({
                "kind": "BUY",
                "trade_date": "2026-01-01",
                "instrument_id": uuid::Uuid::now_v7(),
                "quantity": "1",
                "price": "10",
            }),
        )
        .await;
    foreign_inst.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    random_inst.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(error_fields(&foreign_inst), ["instrument_id"]);
    assert_eq!(foreign_inst.body["errors"], random_inst.body["errors"]);

    let wrong_portfolio_cash = app
        .create_transaction(
            &alice,
            &main,
            json!({
                "kind": "BUY",
                "trade_date": "2026-01-01",
                "instrument_id": thb_stock,
                "cash_account_id": side_thb_cash,
                "quantity": "1",
                "price": "10",
            }),
        )
        .await;
    wrong_portfolio_cash.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(error_fields(&wrong_portfolio_cash), ["cash_account_id"]);

    let currency_mismatch = app
        .create_transaction(
            &alice,
            &main,
            json!({
                "kind": "BUY",
                "trade_date": "2026-01-01",
                "instrument_id": usd_stock,
                "cash_account_id": main_thb_cash,
                "quantity": "1",
                "price": "10",
            }),
        )
        .await;
    currency_mismatch.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(error_fields(&currency_mismatch), ["cash_account_id"]);
}

#[tokio::test]
async fn cross_user_access_returns_identical_404_and_leaves_data_intact() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let mallory = user(&app, "mallory@example.com").await;
    let alice_portfolio = app.new_portfolio(&alice, "Private").await;
    let alice_instrument = app.new_instrument(&alice, "PTT", "THB").await;
    let alice_tx = app
        .new_transaction(
            &alice,
            &alice_portfolio,
            json!({
                "kind": "OPENING_POSITION",
                "trade_date": "2026-01-01",
                "instrument_id": alice_instrument,
                "quantity": "10",
                "amount": "350.00",
            }),
        )
        .await;

    let mallory_portfolio = app.new_portfolio(&mallory, "Mine").await;
    let mallory_instrument = app.new_instrument(&mallory, "MineStock", "THB").await;
    let mallory_payload = json!({
        "kind": "BUY",
        "trade_date": "2026-01-02",
        "instrument_id": mallory_instrument,
        "quantity": "1",
        "price": "10",
    });

    let before = app
        .get_transaction(&alice, &alice_portfolio, &alice_tx)
        .await
        .body;
    let missing = app
        .get_transaction(
            &mallory,
            &mallory_portfolio,
            &uuid::Uuid::now_v7().to_string(),
        )
        .await;
    missing.assert_problem(StatusCode::NOT_FOUND, "not_found");

    let attempts = [
        app.list_transactions(&mallory, &alice_portfolio, "").await,
        app.create_transaction(&mallory, &alice_portfolio, mallory_payload.clone())
            .await,
        app.get_transaction(&mallory, &alice_portfolio, &alice_tx)
            .await,
        app.replace_transaction(
            &mallory,
            &alice_portfolio,
            &alice_tx,
            mallory_payload.clone(),
        )
        .await,
        app.delete_transaction(&mallory, &alice_portfolio, &alice_tx)
            .await,
        app.get_transaction(&mallory, &mallory_portfolio, &alice_tx)
            .await,
        app.replace_transaction(&mallory, &mallory_portfolio, &alice_tx, mallory_payload)
            .await,
        app.delete_transaction(&mallory, &mallory_portfolio, &alice_tx)
            .await,
    ];
    for response in &attempts {
        response.assert_problem(StatusCode::NOT_FOUND, "not_found");
        assert_eq!(response.body, missing.body, "same body as a random id");
    }

    for malformed in [
        app.get_transaction(&alice, "not-a-uuid", &alice_tx).await,
        app.get_transaction(&alice, &alice_portfolio, "not-a-uuid")
            .await,
    ] {
        malformed.assert_problem(StatusCode::NOT_FOUND, "not_found");
    }

    assert_eq!(
        app.get_transaction(&alice, &alice_portfolio, &alice_tx)
            .await
            .body,
        before
    );
    assert_eq!(
        app.list_transactions(&alice, &alice_portfolio, "")
            .await
            .body["total"],
        1
    );
}

#[tokio::test]
async fn archived_portfolios_allow_reads_and_reject_mutations_with_409() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Shelved").await;
    let instrument = app.new_instrument(&alice, "PTT", "THB").await;
    let body = json!({
        "kind": "BUY",
        "trade_date": "2026-01-01",
        "instrument_id": instrument,
        "quantity": "10",
        "price": "35.00",
    });
    let tx = app.new_transaction(&alice, &portfolio, body.clone()).await;
    app.archive_portfolio(&alice, &portfolio).await;

    let list = app.list_transactions(&alice, &portfolio, "").await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.body["total"], 1);
    assert_eq!(
        app.get_transaction(&alice, &portfolio, &tx).await.status,
        StatusCode::OK
    );

    for response in [
        app.create_transaction(&alice, &portfolio, body.clone())
            .await,
        app.replace_transaction(&alice, &portfolio, &tx, body).await,
        app.delete_transaction(&alice, &portfolio, &tx).await,
    ] {
        response.assert_problem(StatusCode::CONFLICT, "portfolio_archived");
    }
}

#[tokio::test]
async fn mutations_require_csrf_and_reads_require_a_session() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.new_portfolio(&alice, "Guarded").await;
    let instrument = app.new_instrument(&alice, "PTT", "THB").await;
    let payload = json!({
        "kind": "BUY",
        "trade_date": "2026-01-01",
        "instrument_id": instrument,
        "quantity": "10",
        "price": "35.00",
    });
    let tx = app
        .new_transaction(&alice, &portfolio, payload.clone())
        .await;
    let collection = transactions_url(&portfolio);
    let item = transaction_url(&portfolio, &tx);

    for req in [
        Req::post(&collection).json(payload.clone()),
        Req::put(&item).json(payload),
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

    assert_eq!(
        app.list_transactions(&alice, &portfolio, "").await.body["total"],
        1
    );
}

#[tokio::test]
async fn openapi_documents_all_transaction_operations_security_and_csrf() {
    let app = TestApp::new().await;
    let response = app.send(Req::get("/api/v1/openapi.json")).await;
    assert_eq!(response.status, StatusCode::OK);

    let paths = &response.body["paths"];
    let collection = &paths["/api/v1/portfolios/{id}/transactions"];
    let item = &paths["/api/v1/portfolios/{id}/transactions/{transaction_id}"];

    for (op, requires_csrf) in [
        (&collection["post"], true),
        (&collection["get"], false),
        (&item["get"], false),
        (&item["put"], true),
        (&item["delete"], true),
    ] {
        assert!(op.is_object(), "operation missing: {op}");
        assert_eq!(op["security"], json!([{ "session_cookie": [] }]));
        let has_csrf = op["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "X-CSRF-Token" && p["in"] == "header");
        assert_eq!(has_csrf, requires_csrf, "operation: {op}");
    }
}
