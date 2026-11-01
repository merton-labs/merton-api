#![allow(clippy::unwrap_used)]

mod common;

use axum::http::StatusCode;
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

    async fn portfolio(&self, u: &User, base: &str) -> String {
        self.post_ok(
            u,
            "/api/v1/portfolios",
            json!({ "name": "Main", "base_currency": base }),
        )
        .await
    }

    async fn account(&self, u: &User, portfolio: &str, name: &str, currency: &str) -> String {
        self.post_ok(
            u,
            &format!("/api/v1/portfolios/{portfolio}/cash-accounts"),
            json!({ "name": name, "currency": currency }),
        )
        .await
    }

    async fn stock(&self, u: &User, symbol: &str, class: &str, currency: &str) -> String {
        self.post_ok(
            u,
            "/api/v1/instruments",
            json!({ "asset_class": class, "symbol": symbol, "name": symbol, "quote_currency": currency }),
        )
        .await
    }

    async fn record(&self, u: &User, portfolio: &str, body: Value) -> String {
        self.post_ok(
            u,
            &format!("/api/v1/portfolios/{portfolio}/transactions"),
            body,
        )
        .await
    }

    async fn rate(&self, u: &User, base: &str, quote: &str, date: &str, value: &str) {
        self.post_ok(
            u,
            "/api/v1/fx-rates",
            json!({ "base_currency": base, "quote_currency": quote, "rate_date": date, "rate": value }),
        )
        .await;
    }

    async fn dividends(&self, u: &User, portfolio: &str, query: &str) -> TestResponse {
        self.send(
            Req::get(&format!("/api/v1/portfolios/{portfolio}/dividends{query}"))
                .session(&u.session),
        )
        .await
    }

    async fn dividends_ok(&self, u: &User, portfolio: &str, query: &str) -> Value {
        let response = self.dividends(u, portfolio, query).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.body);
        response.body
    }
}

fn dividend(instrument: &str, date: &str, amount: &str, tax: &str) -> Value {
    json!({
        "kind": "DIVIDEND",
        "trade_date": date,
        "instrument_id": instrument,
        "amount": amount,
        "tax": tax,
    })
}

fn months(body: &Value) -> Vec<(String, String)> {
    body["months"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["month"].as_str().unwrap().to_owned(),
                m["received"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn dividends_and_interest_count_what_was_received_by_month_and_source() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "USD").await;
    let dime = app.account(&alice, &portfolio, "Dime! FCD", "USD").await;
    let schd = app.stock(&alice, "SCHD", "ETF", "USD").await;
    let pep = app.stock(&alice, "PEP", "STOCK", "USD").await;

    app.record(
        &alice,
        &portfolio,
        dividend(&schd, "2025-12-11", "10", "1.5"),
    )
    .await;
    let mut with_fee = dividend(&pep, "2025-12-30", "4", "0.6");
    with_fee["fee"] = json!("0.4");
    with_fee["cash_account_id"] = json!(dime);
    app.record(&alice, &portfolio, with_fee).await;
    app.record(
        &alice,
        &portfolio,
        json!({
            "kind": "INTEREST",
            "trade_date": "2026-01-31",
            "cash_account_id": dime,
            "amount": "75.20547945",
        }),
    )
    .await;
    // Neither is income: a deposit, and a tax return (an expense kind).
    app.record(
        &alice,
        &portfolio,
        json!({ "kind": "DEPOSIT", "trade_date": "2026-01-02", "cash_account_id": dime, "amount": "500" }),
    )
    .await;
    app.record(
        &alice,
        &portfolio,
        json!({ "kind": "TAX_RETURN", "trade_date": "2026-01-03", "instrument_id": schd, "amount": "1" }),
    )
    .await;

    let body = app.dividends_ok(&alice, &portfolio, "").await;
    assert_eq!(body["base_currency"], "USD");
    assert_eq!(body["from"], Value::Null);
    assert_eq!(body["partial"], false);
    // (10 − 1.5) + (4 − 0.6 − 0.4) + 75.20547945
    assert_eq!(body["total"], "86.70547945");
    assert_eq!(
        months(&body),
        [
            ("2025-12".to_owned(), "11.50000000".to_owned()),
            ("2026-01".to_owned(), "75.20547945".to_owned()),
        ]
    );
    let sources = body["sources"].as_array().unwrap();
    let names: Vec<_> = sources
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Dime! FCD", "SCHD", "PEP"]);
    assert_eq!(sources[0]["kind"], "CASH_ACCOUNT");
    assert_eq!(sources[0]["asset_class"], Value::Null);
    assert_eq!(sources[1]["kind"], "INSTRUMENT");
    assert_eq!(sources[1]["symbol"], "SCHD");
    assert_eq!(sources[1]["asset_class"], "ETF");
    assert_eq!(sources[1]["received"], "8.50000000");
    assert_eq!(sources[2]["received"], "3.00000000");
    let classes: Vec<_> = body["classes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["asset_class"].as_str().unwrap(),
                c["received"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        classes,
        [
            ("STOCK", "3.00000000"),
            ("ETF", "8.50000000"),
            ("CASH", "75.20547945")
        ]
    );
    assert_eq!(body["currencies"][0]["currency"], "USD");
    assert_eq!(body["currencies"][0]["received"], "86.70547945");
}

#[tokio::test]
async fn from_and_to_are_inclusive_and_validated() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "USD").await;
    let schd = app.stock(&alice, "SCHD", "ETF", "USD").await;
    for date in ["2025-11-30", "2025-12-01", "2026-11-01", "2026-11-02"] {
        app.record(&alice, &portfolio, dividend(&schd, date, "1", "0"))
            .await;
    }

    let body = app
        .dividends_ok(&alice, &portfolio, "?from=2025-12-01&to=2026-11-01")
        .await;
    assert_eq!(body["from"], "2025-12-01");
    assert_eq!(body["to"], "2026-11-01");
    assert_eq!(body["total"], "2.00000000");
    let only_from = app
        .dividends_ok(&alice, &portfolio, "?from=2026-11-01")
        .await;
    assert_eq!(only_from["total"], "2.00000000");

    for (query, field) in [
        ("?from=2026-02-30", "from"),
        ("?to=01-11-2026", "to"),
        ("?from=2026-11-02&to=2026-11-01", "to"),
    ] {
        let response = app.dividends(&alice, &portfolio, query).await;
        response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(response.body["errors"][0]["field"], field, "{query}");
    }
}

#[tokio::test]
async fn each_payment_converts_at_the_rate_on_its_own_date() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "THB").await;
    let schd = app.stock(&alice, "SCHD", "ETF", "USD").await;
    app.record(&alice, &portfolio, dividend(&schd, "2026-01-10", "10", "1"))
        .await;
    app.record(&alice, &portfolio, dividend(&schd, "2026-02-10", "10", "1"))
        .await;
    app.record(&alice, &portfolio, dividend(&schd, "2026-03-10", "10", "1"))
        .await;
    app.rate(&alice, "USD", "THB", "2026-01-09", "36").await;
    // A later inverse rate wins over an older direct one: 9 / 0.03 = 300.
    app.rate(&alice, "THB", "USD", "2026-02-01", "0.03").await;
    // On the payment date itself, a direct rate: 9 × 32.5 = 292.5.
    app.rate(&alice, "USD", "THB", "2026-03-10", "32.5").await;
    // After every payment: never used.
    app.rate(&alice, "USD", "THB", "2026-04-01", "99").await;

    let body = app.dividends_ok(&alice, &portfolio, "").await;
    assert_eq!(body["partial"], false);
    assert_eq!(
        months(&body),
        [
            ("2026-01".to_owned(), "324.00000000".to_owned()),
            ("2026-02".to_owned(), "300.00000000".to_owned()),
            ("2026-03".to_owned(), "292.50000000".to_owned()),
        ]
    );
    assert_eq!(body["total"], "916.50000000");
    assert_eq!(body["sources"][0]["currency"], "USD");
    assert_eq!(body["currencies"][0]["currency"], "USD");
    assert_eq!(body["currencies"][0]["received"], "916.50000000");
}

#[tokio::test]
async fn a_payment_before_any_rate_is_listed_not_counted() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "THB").await;
    let schd = app.stock(&alice, "SCHD", "ETF", "USD").await;
    let ptt = app.stock(&alice, "PTT", "STOCK", "THB").await;
    let early = app
        .record(&alice, &portfolio, dividend(&schd, "2026-01-05", "10", "1"))
        .await;
    app.record(
        &alice,
        &portfolio,
        dividend(&ptt, "2026-01-06", "100", "10"),
    )
    .await;
    app.rate(&alice, "USD", "THB", "2026-01-09", "36").await;

    let body = app.dividends_ok(&alice, &portfolio, "").await;
    assert_eq!(body["partial"], true);
    assert_eq!(body["total"], "90.00000000");
    let unconverted = body["unconverted"].as_array().unwrap();
    assert_eq!(unconverted.len(), 1);
    assert_eq!(unconverted[0]["transaction_id"], early.as_str());
    assert_eq!(unconverted[0]["kind"], "DIVIDEND");
    assert_eq!(unconverted[0]["trade_date"], "2026-01-05");
    assert_eq!(unconverted[0]["name"], "SCHD");
    assert_eq!(unconverted[0]["currency"], "USD");
    assert_eq!(unconverted[0]["received"], "9");
    assert_eq!(unconverted[0]["reason"], "MISSING_FX_RATE");
    assert_eq!(body["sources"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn another_users_portfolio_and_rates_are_never_used() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let portfolio = app.portfolio(&alice, "THB").await;
    let schd = app.stock(&alice, "SCHD", "ETF", "USD").await;
    app.record(&alice, &portfolio, dividend(&schd, "2026-01-10", "10", "0"))
        .await;
    app.rate(&bob, "USD", "THB", "2026-01-01", "36").await;

    let body = app.dividends_ok(&alice, &portfolio, "").await;
    assert_eq!(body["unconverted"][0]["reason"], "MISSING_FX_RATE");

    app.dividends(&bob, &portfolio, "")
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    app.send(Req::get(&format!(
        "/api/v1/portfolios/{portfolio}/dividends"
    )))
    .await
    .assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
}

#[tokio::test]
async fn archived_portfolios_still_report_dividends() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "USD").await;
    let schd = app.stock(&alice, "SCHD", "ETF", "USD").await;
    app.record(
        &alice,
        &portfolio,
        dividend(&schd, "2026-01-10", "2", "0.3"),
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

    let body = app.dividends_ok(&alice, &portfolio, "").await;
    assert_eq!(body["total"], "1.70000000");
}

#[tokio::test]
async fn openapi_documents_the_dividends_operation() {
    let app = TestApp::new().await;
    let response = app.send(Req::get("/api/v1/openapi.json")).await;
    let op = &response.body["paths"]["/api/v1/portfolios/{id}/dividends"]["get"];
    assert_eq!(op["tags"][0], "dividends");
    assert!(op["security"].is_array());
    let params: Vec<_> = op["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["in"] == "query")
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(params, ["from", "to"]);
    assert_eq!(
        response.body["components"]["schemas"]["DividendSourceKindDto"]["enum"],
        json!(["INSTRUMENT", "CASH_ACCOUNT"])
    );
}
