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

fn id_of(response: &TestResponse) -> String {
    response.body["id"].as_str().unwrap().to_owned()
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
        id_of(&response)
    }

    async fn portfolio(&self, u: &User, base: &str) -> String {
        self.post_ok(
            u,
            "/api/v1/portfolios",
            json!({ "name": "Main", "base_currency": base }),
        )
        .await
    }

    async fn account(&self, u: &User, portfolio: &str, currency: &str) -> String {
        self.post_ok(
            u,
            &format!("/api/v1/portfolios/{portfolio}/cash-accounts"),
            json!({ "name": format!("{currency} cash"), "currency": currency }),
        )
        .await
    }

    async fn stock(&self, u: &User, name: &str, class: &str, currency: &str) -> String {
        self.post_ok(
            u,
            "/api/v1/instruments",
            json!({ "asset_class": class, "name": name, "quote_currency": currency }),
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

    async fn price(&self, u: &User, instrument: &str, date: &str, value: &str) {
        self.post_ok(
            u,
            &format!("/api/v1/instruments/{instrument}/prices"),
            json!({ "price_date": date, "price": value }),
        )
        .await;
    }

    async fn rate(&self, u: &User, base: &str, quote: &str, date: &str, value: &str) {
        self.post_ok(
            u,
            "/api/v1/fx-rates",
            json!({ "base_currency": base, "quote_currency": quote, "rate_date": date, "rate": value }),
        )
        .await;
    }

    async fn report(&self, u: &User, portfolio: &str, what: &str, as_of: &str) -> TestResponse {
        self.send(
            Req::get(&format!(
                "/api/v1/portfolios/{portfolio}/{what}?as_of={as_of}"
            ))
            .session(&u.session),
        )
        .await
    }

    async fn report_ok(&self, u: &User, portfolio: &str, what: &str, as_of: &str) -> Value {
        let response = self.report(u, portfolio, what, as_of).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.body);
        response.body
    }
}

fn buy(instrument: &str, date: &str, quantity: &str, price: &str) -> Value {
    json!({
        "kind": "BUY",
        "trade_date": date,
        "instrument_id": instrument,
        "quantity": quantity,
        "price": price,
    })
}

fn opening_cash(account: &str, amount: &str) -> Value {
    json!({
        "kind": "OPENING_CASH",
        "trade_date": "2026-01-01",
        "cash_account_id": account,
        "amount": amount,
    })
}

fn by_instrument<'a>(body: &'a Value, instrument: &str) -> &'a Value {
    body["positions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["instrument_id"] == instrument)
        .unwrap()
}

// --- Holdings ----------------------------------------------------------------

#[tokio::test]
async fn holdings_replay_only_history_on_or_before_as_of() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "THB").await;
    let account = app.account(&alice, &portfolio, "THB").await;
    let ptt = app.stock(&alice, "PTT", "STOCK", "THB").await;

    app.record(&alice, &portfolio, opening_cash(&account, "10000"))
        .await;
    let mut purchase = buy(&ptt, "2026-01-02", "100", "35");
    purchase["cash_account_id"] = json!(account);
    app.record(&alice, &portfolio, purchase).await;
    app.record(
        &alice,
        &portfolio,
        json!({
            "kind": "SELL",
            "trade_date": "2026-01-10",
            "instrument_id": ptt,
            "quantity": "40",
            "price": "40",
            "cash_account_id": account,
        }),
    )
    .await;
    app.record(
        &alice,
        &portfolio,
        json!({
            "kind": "STOCK_SPLIT",
            "trade_date": "2026-01-12",
            "instrument_id": ptt,
            "ratio": "2:1",
        }),
    )
    .await;

    let before = app
        .report_ok(&alice, &portfolio, "holdings", "2026-01-01")
        .await;
    assert_eq!(before["positions"], json!([]));
    assert_eq!(before["cash"][0]["balance"], "10000");
    assert_eq!(before["base_currency"], "THB");
    assert_eq!(before["as_of"], "2026-01-01");

    let bought = app
        .report_ok(&alice, &portfolio, "holdings", "2026-01-09")
        .await;
    let position = by_instrument(&bought, &ptt);
    assert_eq!(position["quantity"], "100");
    assert_eq!(position["cost_basis"], "3500");
    assert_eq!(position["asset_class"], "STOCK");
    assert_eq!(position["quote_currency"], "THB");
    assert_eq!(bought["cash"][0]["balance"], "6500");

    let sold = app
        .report_ok(&alice, &portfolio, "holdings", "2026-01-10")
        .await;
    let position = by_instrument(&sold, &ptt);
    assert_eq!(position["quantity"], "60");
    assert_eq!(position["cost_basis"], "2100");
    assert_eq!(sold["cash"][0]["balance"], "8100");

    let split = app
        .report_ok(&alice, &portfolio, "holdings", "2026-12-31")
        .await;
    let position = by_instrument(&split, &ptt);
    assert_eq!(position["quantity"], "120.00000000");
    assert_eq!(position["cost_basis"], "2100", "a split keeps cost");
}

#[tokio::test]
async fn closed_positions_are_not_listed() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "THB").await;
    let ptt = app.stock(&alice, "PTT", "STOCK", "THB").await;
    app.record(&alice, &portfolio, buy(&ptt, "2026-01-02", "10", "35"))
        .await;
    app.record(
        &alice,
        &portfolio,
        json!({
            "kind": "SELL",
            "trade_date": "2026-01-03",
            "instrument_id": ptt,
            "quantity": "10",
            "price": "36",
        }),
    )
    .await;

    let holdings = app
        .report_ok(&alice, &portfolio, "holdings", "2026-01-31")
        .await;
    assert_eq!(holdings["positions"], json!([]));
    assert_eq!(holdings["cash"], json!([]));
}

// --- Valuation ---------------------------------------------------------------

#[tokio::test]
async fn valuation_converts_with_direct_and_inverse_rates_on_or_before_as_of() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "THB").await;
    let usd_cash = app.account(&alice, &portfolio, "USD").await;
    let ptt = app.stock(&alice, "PTT", "STOCK", "THB").await;
    let spy = app.stock(&alice, "SPY", "ETF", "USD").await;
    app.record(&alice, &portfolio, buy(&ptt, "2026-01-02", "100", "35"))
        .await;
    app.record(&alice, &portfolio, buy(&spy, "2026-01-02", "2.5", "500"))
        .await;
    app.record(&alice, &portfolio, opening_cash(&usd_cash, "100"))
        .await;

    app.price(&alice, &ptt, "2026-01-14", "36.50").await;
    app.price(&alice, &ptt, "2026-01-16", "99").await; // after as_of
    app.price(&alice, &spy, "2026-01-05", "510.25").await; // 10 days old
    app.rate(&alice, "USD", "THB", "2026-01-13", "36").await;
    app.rate(&alice, "THB", "USD", "2026-01-15", "0.025").await; // newer inverse
    app.rate(&alice, "USD", "THB", "2026-01-20", "40").await; // after as_of

    let v = app
        .report_ok(&alice, &portfolio, "valuation", "2026-01-15")
        .await;
    assert_eq!(v["partial"], false);
    assert_eq!(v["unvalued"], json!([]));

    let p = by_instrument(&v, &ptt);
    assert_eq!(p["price"]["price"], "36.50");
    assert_eq!(p["price"]["date"], "2026-01-14");
    assert_eq!(p["price"]["stale"], false);
    assert_eq!(p["market_value"], "3650.00");
    assert_eq!(p["fx"], Value::Null, "already in the base currency");
    assert_eq!(p["value"], "3650.00000000");

    // The inverse THB/USD rate is newer, so 1275.625 USD ÷ 0.025.
    let s = by_instrument(&v, &spy);
    assert_eq!(s["price"]["stale"], true);
    assert_eq!(s["market_value"], "1275.625");
    assert_eq!(s["fx"]["inverse"], true);
    assert_eq!(s["fx"]["rate"], "0.025");
    assert_eq!(s["fx"]["date"], "2026-01-15");
    assert_eq!(s["fx"]["stale"], false);
    assert_eq!(s["value"], "51025.00000000");

    assert_eq!(v["cash"][0]["value"], "4000.00000000");
    assert_eq!(v["positions_value"], "54675.00000000");
    assert_eq!(v["cash_value"], "4000.00000000");
    assert_eq!(v["net_value"], "58675.00000000");

    // Two days earlier only the direct rate exists (and is not stale).
    let earlier = app
        .report_ok(&alice, &portfolio, "valuation", "2026-01-13")
        .await;
    let s = by_instrument(&earlier, &spy);
    assert_eq!(s["fx"]["inverse"], false);
    assert_eq!(s["value"], "45922.50000000");
    assert_eq!(
        by_instrument(&earlier, &ptt)["unvalued_reason"],
        "MISSING_PRICE"
    );
}

#[tokio::test]
async fn missing_prices_and_rates_give_a_partial_valuation() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "THB").await;
    let eur_cash = app.account(&alice, &portfolio, "EUR").await;
    let priced = app.stock(&alice, "Priced", "STOCK", "THB").await;
    let unpriced = app.stock(&alice, "Unpriced", "STOCK", "THB").await;
    app.record(&alice, &portfolio, buy(&priced, "2026-01-02", "10", "1"))
        .await;
    app.record(&alice, &portfolio, buy(&unpriced, "2026-01-02", "10", "1"))
        .await;
    app.record(&alice, &portfolio, opening_cash(&eur_cash, "50"))
        .await;
    app.price(&alice, &priced, "2026-01-02", "2").await;
    // No triangulation: EUR→USD→THB is never used.
    app.rate(&alice, "EUR", "USD", "2026-01-02", "1.08").await;
    app.rate(&alice, "USD", "THB", "2026-01-02", "36").await;

    let v = app
        .report_ok(&alice, &portfolio, "valuation", "2026-01-02")
        .await;
    assert_eq!(v["partial"], true);
    assert_eq!(
        v["unvalued"],
        json!([
            { "kind": "POSITION", "id": unpriced, "reason": "MISSING_PRICE" },
            { "kind": "CASH", "id": eur_cash, "reason": "MISSING_FX_RATE" },
        ])
    );
    let missing = by_instrument(&v, &unpriced);
    assert_eq!(missing["value"], Value::Null, "never valued at zero");
    assert_eq!(v["cash"][0]["value"], Value::Null);
    assert_eq!(v["net_value"], "20.00000000");

    let a = app
        .report_ok(&alice, &portfolio, "allocation", "2026-01-02")
        .await;
    assert_eq!(a["partial"], true);
}

#[tokio::test]
async fn another_users_rates_and_portfolios_are_never_used() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let portfolio = app.portfolio(&alice, "THB").await;
    let usd = app.account(&alice, &portfolio, "USD").await;
    app.record(&alice, &portfolio, opening_cash(&usd, "10"))
        .await;
    app.rate(&bob, "USD", "THB", "2026-01-01", "36").await;

    let v = app
        .report_ok(&alice, &portfolio, "valuation", "2026-01-01")
        .await;
    assert_eq!(v["cash"][0]["unvalued_reason"], "MISSING_FX_RATE");

    for what in ["holdings", "valuation", "allocation"] {
        app.report(&bob, &portfolio, what, "2026-01-01")
            .await
            .assert_problem(StatusCode::NOT_FOUND, "not_found");
        app.send(Req::get(&format!(
            "/api/v1/portfolios/{portfolio}/{what}?as_of=2026-01-01"
        )))
        .await
        .assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
    }
}

#[tokio::test]
async fn as_of_is_required_and_validated() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "THB").await;

    for what in ["holdings", "valuation", "allocation"] {
        let missing = app
            .send(
                Req::get(&format!("/api/v1/portfolios/{portfolio}/{what}")).session(&alice.session),
            )
            .await;
        missing.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(missing.body["errors"][0]["field"], "as_of");
        for bad in ["2026-02-30", "15-01-2026", "1800-01-01"] {
            app.report(&alice, &portfolio, what, bad)
                .await
                .assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        }
    }
}

#[tokio::test]
async fn changing_the_base_currency_revalues_without_touching_transactions() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "THB").await;
    let spy = app.stock(&alice, "SPY", "ETF", "USD").await;
    app.record(&alice, &portfolio, buy(&spy, "2026-01-02", "2", "500"))
        .await;
    app.price(&alice, &spy, "2026-01-02", "500").await;
    app.rate(&alice, "USD", "THB", "2026-01-02", "36").await;
    let transactions = format!("/api/v1/portfolios/{portfolio}/transactions");
    let before = app
        .send(Req::get(&transactions).session(&alice.session))
        .await;

    let thb = app
        .report_ok(&alice, &portfolio, "valuation", "2026-01-02")
        .await;
    assert_eq!(thb["net_value"], "36000.00000000");

    let changed = app
        .send(
            Req::patch(&format!("/api/v1/portfolios/{portfolio}"))
                .session(&alice.session)
                .header("x-csrf-token", &alice.csrf)
                .json(json!({ "base_currency": "USD" })),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);

    let usd = app
        .report_ok(&alice, &portfolio, "valuation", "2026-01-02")
        .await;
    assert_eq!(usd["base_currency"], "USD");
    assert_eq!(usd["net_value"], "1000.00000000");
    assert_eq!(by_instrument(&usd, &spy)["fx"], Value::Null);
    let after = app
        .send(Req::get(&transactions).session(&alice.session))
        .await;
    assert_eq!(after.body, before.body, "transactions are unchanged");
}

#[tokio::test]
async fn archived_portfolios_can_still_be_valued() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "THB").await;
    let account = app.account(&alice, &portfolio, "THB").await;
    app.record(&alice, &portfolio, opening_cash(&account, "5"))
        .await;
    let archived = app
        .send(
            Req::post(&format!("/api/v1/portfolios/{portfolio}/archive"))
                .session(&alice.session)
                .header("x-csrf-token", &alice.csrf),
        )
        .await;
    assert!(archived.status.is_success(), "{}", archived.body);

    let v = app
        .report_ok(&alice, &portfolio, "valuation", "2026-01-01")
        .await;
    assert_eq!(v["net_value"], "5.00000000");
}

// --- Allocation --------------------------------------------------------------

#[tokio::test]
async fn allocation_is_signed_on_net_portfolio_value() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "USD").await;
    let account = app.account(&alice, &portfolio, "USD").await;
    let stock = app.stock(&alice, "ACME", "STOCK", "USD").await;
    app.record(&alice, &portfolio, opening_cash(&account, "8000"))
        .await;
    let mut purchase = buy(&stock, "2026-01-02", "100", "100");
    purchase["cash_account_id"] = json!(account);
    app.record(&alice, &portfolio, purchase).await;
    app.price(&alice, &stock, "2026-01-02", "100").await;

    // Stocks 10,000 and cash −2,000: NPV 8,000, so 125% and −25%.
    let a = app
        .report_ok(&alice, &portfolio, "allocation", "2026-01-02")
        .await;
    assert_eq!(a["partial"], false);
    assert_eq!(a["negative_cash"], true);
    assert_eq!(a["net_value"], "8000.00000000");
    assert_eq!(
        a["classes"],
        json!([
            { "asset_class": "STOCK", "value": "10000.00000000", "percent": "125.0000" },
            { "asset_class": "CASH", "value": "-2000.00000000", "percent": "-25.0000" },
        ])
    );

    // Before the purchase the stock is not held: all cash.
    let early = app
        .report_ok(&alice, &portfolio, "allocation", "2026-01-01")
        .await;
    assert_eq!(early["negative_cash"], false);
    assert_eq!(
        early["classes"],
        json!([{ "asset_class": "CASH", "value": "8000.00000000", "percent": "100.0000" }])
    );
}

#[tokio::test]
async fn allocation_percentages_are_null_when_net_value_is_not_positive() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let portfolio = app.portfolio(&alice, "USD").await;
    let account = app.account(&alice, &portfolio, "USD").await;
    let stock = app.stock(&alice, "ACME", "STOCK", "USD").await;
    let mut purchase = buy(&stock, "2026-01-02", "10", "100");
    purchase["cash_account_id"] = json!(account);
    app.record(&alice, &portfolio, purchase).await;
    app.price(&alice, &stock, "2026-01-02", "100").await;

    // Stocks 1,000 and cash −1,000: NPV 0.
    let a = app
        .report_ok(&alice, &portfolio, "allocation", "2026-01-02")
        .await;
    assert_eq!(a["net_value"], "0.00000000");
    for class in a["classes"].as_array().unwrap() {
        assert_eq!(class["percent"], Value::Null, "{class}");
    }
}

#[tokio::test]
async fn openapi_documents_valuation_operations() {
    let app = TestApp::new().await;
    let response = app.send(Req::get("/api/v1/openapi.json")).await;
    let paths = &response.body["paths"];
    for what in ["holdings", "valuation", "allocation"] {
        let op = &paths[format!("/api/v1/portfolios/{{id}}/{what}")]["get"];
        assert_eq!(op["tags"][0], "valuation", "{what}");
        assert!(op["security"].is_array(), "{what}");
        let has_as_of = op["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "as_of" && p["in"] == "query");
        assert!(has_as_of, "{what}");
    }
    let schemas = &response.body["components"]["schemas"];
    assert_eq!(
        schemas["UnvaluedReasonDto"]["enum"],
        json!(["MISSING_PRICE", "MISSING_FX_RATE", "OUT_OF_RANGE"])
    );
    assert_eq!(
        schemas["AllocationClassDto"]["enum"],
        json!(["STOCK", "ETF", "MUTUAL_FUND", "CASH"])
    );
}
