#![allow(clippy::unwrap_used)]

mod common;

use std::str::FromStr;

use axum::http::StatusCode;
use common::{Req, TestApp, TestResponse};
use rust_decimal::Decimal;
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

impl TestApp {
    async fn post_ok(&self, u: &User, uri: &str, body: Value) -> String {
        let response = self
            .send(
                Req::post(uri)
                    .session(&u.session)
                    .header("x-csrf-token", &u.csrf)
                    .json(body),
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::CREATED,
            "{uri}: {}",
            response.body
        );
        response.body["id"].as_str().unwrap().to_owned()
    }

    async fn portfolio(&self, u: &User) -> String {
        self.post_ok(
            u,
            "/api/v1/portfolios",
            json!({ "name": "Main", "base_currency": "USD" }),
        )
        .await
    }

    async fn stock(&self, u: &User, name: &str) -> String {
        self.post_ok(
            u,
            "/api/v1/instruments",
            json!({ "asset_class": "STOCK", "symbol": name, "name": name, "quote_currency": "USD" }),
        )
        .await
    }

    async fn record(&self, u: &User, portfolio: &str, body: Value) {
        self.post_ok(
            u,
            &format!("/api/v1/portfolios/{portfolio}/transactions"),
            body,
        )
        .await;
    }

    async fn price(&self, u: &User, instrument: &str, date: &str, value: &str) {
        self.post_ok(
            u,
            &format!("/api/v1/instruments/{instrument}/prices"),
            json!({ "price_date": date, "price": value }),
        )
        .await;
    }

    async fn returns(&self, u: &User, portfolio: &str, query: &str) -> TestResponse {
        self.send(
            Req::get(&format!("/api/v1/portfolios/{portfolio}/returns{query}")).session(&u.session),
        )
        .await
    }

    async fn returns_ok(&self, u: &User, portfolio: &str, as_of: &str) -> Value {
        let response = self.returns(u, portfolio, &format!("?as_of={as_of}")).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.body);
        response.body
    }
}

fn trade(kind: &str, instrument: &str, date: &str, qty: &str, price: &str) -> Value {
    json!({ "kind": kind, "trade_date": date, "instrument_id": instrument, "quantity": qty, "price": price })
}

fn by_name<'a>(body: &'a Value, name: &str) -> &'a Value {
    body["holdings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["name"] == name)
        .unwrap()
}

/// Compares decimal strings by value, so `170.0000000000` equals `170`.
fn assert_dec(value: &Value, expected: &str) {
    let actual = Decimal::from_str(value.as_str().unwrap()).unwrap();
    assert_eq!(actual, Decimal::from_str(expected).unwrap(), "{value}");
}

async fn mixed_history(app: &TestApp, alice: &User, portfolio: &str) -> String {
    let abc = app.stock(alice, "ABC").await;
    let mut buy = trade("BUY", &abc, "2026-01-05", "10", "10");
    buy["fee"] = json!("1");
    buy["tax"] = json!("0.5");
    app.record(alice, portfolio, buy).await;
    let mut buy = trade("BUY", &abc, "2026-02-05", "10", "12");
    buy["fee"] = json!("1");
    app.record(alice, portfolio, buy).await;
    let mut sell = trade("SELL", &abc, "2026-03-05", "5", "15");
    sell["fee"] = json!("1");
    sell["tax"] = json!("0.25");
    app.record(alice, portfolio, sell).await;
    app.record(
        alice,
        portfolio,
        json!({ "kind": "DIVIDEND", "trade_date": "2026-03-10", "instrument_id": abc, "amount": "6", "tax": "0.9", "fee": "0.1" }),
    )
    .await;
    for (kind, amount) in [
        ("FEE", "2"),
        ("OTHER_EXPENSE", "0.5"),
        ("TAX", "1"),
        ("TAX_RETURN", "0.4"),
    ] {
        app.record(
            alice,
            portfolio,
            json!({ "kind": kind, "trade_date": "2026-03-15", "instrument_id": abc, "amount": amount }),
        )
        .await;
    }
    app.price(alice, &abc, "2026-03-31", "14").await;
    abc
}

#[tokio::test]
async fn total_profit_adds_gain_realized_and_dividends_less_fees_and_taxes() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice).await;
    let abc = mixed_history(&app, &alice, &portfolio).await;

    let body = app.returns_ok(&alice, &portfolio, "2026-03-31").await;
    assert_eq!(body["as_of"], "2026-03-31");
    assert_eq!(body["partial"], false);
    let h = by_name(&body, "ABC");
    assert_eq!(h["instrument_id"], abc.as_str());
    assert_eq!(h["quote_currency"], "USD");
    assert_dec(&h["quantity"], "15");
    // 5 left of the first lot (50) plus the second lot (120).
    assert_dec(&h["cost_basis"], "170");
    assert_eq!(h["average_cost"], "11.33333333");
    assert_eq!(h["price"]["price"], "14");
    assert_dec(&h["market_value"], "210");
    assert_dec(&h["capital_gain"], "40");
    assert_eq!(h["capital_gain_percent"], "23.5294");
    // Sold 5 of the first lot: 75 − 50.
    assert_dec(&h["realized"], "25");
    assert_dec(&h["dividends"], "5");
    // 1 + 1 + 1 on trades, 2 FEE, 0.5 OTHER_EXPENSE.
    assert_dec(&h["fees"], "5.5");
    // 0.5 + 0.25 on trades, 1 TAX, −0.4 TAX_RETURN.
    assert_dec(&h["taxes"], "1.35");
    assert_dec(&h["total_bought"], "220");
    // 40 + 25 + 5 − 5.5 − 1.35
    assert_dec(&h["total_profit"], "63.15");
    assert_eq!(h["total_profit_percent"], "28.7045");
    assert_eq!(h["unvalued_reason"], Value::Null);
}

#[tokio::test]
async fn only_history_on_or_before_as_of_counts() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice).await;
    let abc = mixed_history(&app, &alice, &portfolio).await;
    app.price(&alice, &abc, "2026-02-27", "11").await;

    let body = app.returns_ok(&alice, &portfolio, "2026-02-28").await;
    let h = by_name(&body, "ABC");
    assert_dec(&h["quantity"], "20");
    assert_dec(&h["realized"], "0");
    assert_dec(&h["dividends"], "0");
    assert_dec(&h["fees"], "2");
    assert_dec(&h["capital_gain"], "0");
    // 0 + 0 + 0 − 2 − 0.5
    assert_dec(&h["total_profit"], "-2.5");
}

#[tokio::test]
async fn sold_positions_are_listed_after_open_ones_and_need_no_price() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice).await;
    let ko = app.stock(&alice, "KO").await;
    let zed = app.stock(&alice, "ZED").await;
    let mut buy = trade("BUY", &ko, "2026-01-05", "2", "50");
    buy["fee"] = json!("0.1");
    app.record(&alice, &portfolio, buy).await;
    app.record(
        &alice,
        &portfolio,
        json!({ "kind": "DIVIDEND", "trade_date": "2026-02-01", "instrument_id": ko, "amount": "1" }),
    )
    .await;
    app.record(
        &alice,
        &portfolio,
        trade("SELL", &ko, "2026-03-01", "2", "55"),
    )
    .await;
    app.record(
        &alice,
        &portfolio,
        trade("BUY", &zed, "2026-01-05", "1", "5"),
    )
    .await;

    let body = app.returns_ok(&alice, &portfolio, "2026-03-31").await;
    let names: Vec<_> = body["holdings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["ZED", "KO"]);

    let ko = by_name(&body, "KO");
    assert_dec(&ko["quantity"], "0");
    assert_eq!(ko["average_cost"], Value::Null);
    assert_eq!(ko["price"], Value::Null);
    assert_dec(&ko["market_value"], "0");
    assert_eq!(ko["capital_gain_percent"], Value::Null);
    assert_dec(&ko["realized"], "10");
    // 0 + 10 + 1 − 0.1
    assert_dec(&ko["total_profit"], "10.9");
    assert_eq!(ko["total_profit_percent"], "10.9000");

    // An open position without a price is never valued at zero.
    assert_eq!(body["partial"], true);
    let zed = by_name(&body, "ZED");
    assert_eq!(zed["unvalued_reason"], "MISSING_PRICE");
    assert_eq!(zed["market_value"], Value::Null);
    assert_eq!(zed["total_profit"], Value::Null);
    assert_eq!(zed["average_cost"], "5.00000000");
}

#[tokio::test]
async fn as_of_is_required_and_other_users_get_not_found() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let portfolio = app.portfolio(&alice).await;

    let missing = app.returns(&alice, &portfolio, "").await;
    missing.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(missing.body["errors"][0]["field"], "as_of");
    app.returns(&alice, &portfolio, "?as_of=2026-02-30")
        .await
        .assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    app.returns(&bob, &portfolio, "?as_of=2026-01-01")
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    app.send(Req::get(&format!(
        "/api/v1/portfolios/{portfolio}/returns?as_of=2026-01-01"
    )))
    .await
    .assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
}

#[tokio::test]
async fn archived_portfolios_still_report_returns() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice).await;
    let abc = app.stock(&alice, "ABC").await;
    app.record(
        &alice,
        &portfolio,
        trade("BUY", &abc, "2026-01-05", "1", "10"),
    )
    .await;
    let archived = app
        .send(
            Req::post(&format!("/api/v1/portfolios/{portfolio}/archive"))
                .session(&alice.session)
                .header("x-csrf-token", &alice.csrf),
        )
        .await;
    assert!(archived.status.is_success(), "{}", archived.body);

    let body = app.returns_ok(&alice, &portfolio, "2026-01-31").await;
    assert_dec(&by_name(&body, "ABC")["total_bought"], "10");
}

#[tokio::test]
async fn openapi_documents_the_returns_operation() {
    let app = TestApp::new().await;
    let response = app.send(Req::get("/api/v1/openapi.json")).await;
    let op = &response.body["paths"]["/api/v1/portfolios/{id}/returns"]["get"];
    assert_eq!(op["tags"][0], "valuation");
    assert!(op["security"].is_array());
    let has_as_of = op["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "as_of" && p["in"] == "query");
    assert!(has_as_of);
}
