#![allow(clippy::unwrap_used)]

mod common;

use axum::http::{StatusCode, header};
use common::{Req, TestApp, TestResponse};
use merton_api::infrastructure::entities::{fx_rate, instrument_price};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

const FX: &str = "/api/v1/fx-rates";

struct User {
    session: String,
    csrf: String,
}

async fn user(app: &TestApp, email: &str) -> User {
    let session = app.signed_in(email).await;
    let csrf = app.csrf_token(&session).await;
    User { session, csrf }
}

fn prices(instrument: &str) -> String {
    format!("/api/v1/instruments/{instrument}/prices")
}

fn price_url(instrument: &str, id: &str) -> String {
    format!("{}/{id}", prices(instrument))
}

fn fx_url(id: &str) -> String {
    format!("{FX}/{id}")
}

fn id_of(response: &TestResponse) -> String {
    response.body["id"].as_str().unwrap().to_owned()
}

fn fields(response: &TestResponse) -> Vec<String> {
    response.body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap().to_owned())
        .collect()
}

fn price(date: &str, value: &str) -> Value {
    json!({ "price_date": date, "price": value })
}

fn rate(base: &str, quote: &str, date: &str, value: &str) -> Value {
    json!({
        "base_currency": base,
        "quote_currency": quote,
        "rate_date": date,
        "rate": value,
    })
}

impl TestApp {
    async fn write(&self, u: &User, req: Req<'_>) -> TestResponse {
        self.send(req.session(&u.session).header("x-csrf-token", &u.csrf))
            .await
    }

    async fn read(&self, u: &User, uri: &str) -> TestResponse {
        self.send(Req::get(uri).session(&u.session)).await
    }

    async fn instrument(&self, u: &User, currency: &str) -> String {
        let response = self
            .write(
                u,
                Req::post("/api/v1/instruments").json(json!({
                    "asset_class": "STOCK",
                    "name": "Priced",
                    "quote_currency": currency,
                })),
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        id_of(&response)
    }

    async fn add_price(&self, u: &User, instrument: &str, body: Value) -> TestResponse {
        self.write(u, Req::post(&prices(instrument)).json(body))
            .await
    }

    async fn new_price(&self, u: &User, instrument: &str, date: &str, value: &str) -> String {
        let response = self.add_price(u, instrument, price(date, value)).await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        id_of(&response)
    }

    async fn add_rate(&self, u: &User, body: Value) -> TestResponse {
        self.write(u, Req::post(FX).json(body)).await
    }

    async fn new_rate(&self, u: &User, base: &str, quote: &str, date: &str, value: &str) -> String {
        let response = self.add_rate(u, rate(base, quote, date, value)).await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
        id_of(&response)
    }
}

fn dates(list: &TestResponse, field: &str) -> Vec<String> {
    list.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item[field].as_str().unwrap().to_owned())
        .collect()
}

// --- Prices ------------------------------------------------------------------

#[tokio::test]
async fn prices_are_recorded_exactly_and_listed_latest_first() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let instrument = app.instrument(&alice, "THB").await;

    let created = app
        .add_price(&alice, &instrument, price("2026-01-15", "35.250"))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let id = id_of(&created);
    assert_eq!(
        created.headers[header::LOCATION],
        price_url(&instrument, &id)
    );
    assert_eq!(created.body["instrument_id"], instrument.as_str());
    assert_eq!(created.body["price_date"], "2026-01-15");
    assert_eq!(created.body["price"], "35.250", "scale is kept");
    assert_eq!(created.body["source"], "MANUAL");

    let fetched = app.read(&alice, &price_url(&instrument, &id)).await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.body, created.body);

    app.new_price(&alice, &instrument, "2026-01-17", "36").await;
    app.new_price(&alice, &instrument, "2026-01-16", "0").await;
    let list = app.read(&alice, &prices(&instrument)).await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.body["total"], 3);
    assert_eq!(
        dates(&list, "price_date"),
        ["2026-01-17", "2026-01-16", "2026-01-15"]
    );

    let window = app
        .read(
            &alice,
            &format!("{}?from=2026-01-16&to=2026-01-16", prices(&instrument)),
        )
        .await;
    assert_eq!(window.body["total"], 1);
    assert_eq!(window.body["items"][0]["price"], "0");

    let page = app
        .read(&alice, &format!("{}?limit=1&offset=1", prices(&instrument)))
        .await;
    assert_eq!(page.body["total"], 3);
    assert_eq!(dates(&page, "price_date"), ["2026-01-16"]);
}

#[tokio::test]
async fn one_price_per_instrument_and_date() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let instrument = app.instrument(&alice, "THB").await;
    let other = app.instrument(&alice, "THB").await;
    app.new_price(&alice, &instrument, "2026-01-15", "35").await;
    let later = app.new_price(&alice, &instrument, "2026-01-16", "36").await;

    app.add_price(&alice, &instrument, price("2026-01-15", "99"))
        .await
        .assert_problem(StatusCode::CONFLICT, "price_exists");
    app.write(
        &alice,
        Req::put(&price_url(&instrument, &later)).json(price("2026-01-15", "36")),
    )
    .await
    .assert_problem(StatusCode::CONFLICT, "price_exists");

    // Another instrument may use the same date.
    app.new_price(&alice, &other, "2026-01-15", "10").await;

    let unchanged = app.read(&alice, &price_url(&instrument, &later)).await;
    assert_eq!(unchanged.body["price_date"], "2026-01-16");
}

#[tokio::test]
async fn concurrent_prices_for_one_date_admit_exactly_one() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let instrument = app.instrument(&alice, "THB").await;

    let (a, b) = tokio::join!(
        app.add_price(&alice, &instrument, price("2026-01-15", "1")),
        app.add_price(&alice, &instrument, price("2026-01-15", "2")),
    );
    let mut statuses = [a.status, b.status];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::CREATED, StatusCode::CONFLICT]);
    let list = app.read(&alice, &prices(&instrument)).await;
    assert_eq!(list.body["total"], 1);
}

#[tokio::test]
async fn prices_can_be_replaced_and_deleted() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let instrument = app.instrument(&alice, "USD").await;
    let id = app
        .new_price(&alice, &instrument, "2026-01-15", "100")
        .await;
    let item = price_url(&instrument, &id);

    let replaced = app
        .write(&alice, Req::put(&item).json(price("2026-01-14", "99.5")))
        .await;
    assert_eq!(replaced.status, StatusCode::OK, "{}", replaced.body);
    assert_eq!(replaced.body["id"], id.as_str());
    assert_eq!(replaced.body["price_date"], "2026-01-14");
    assert_eq!(replaced.body["price"], "99.5");

    let deleted = app.write(&alice, Req::delete(&item)).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    app.read(&alice, &item)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    app.write(&alice, Req::delete(&item))
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn invalid_prices_are_rejected_field_by_field() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let instrument = app.instrument(&alice, "THB").await;

    for (body, expected) in [
        (price("2026-02-30", "1"), vec!["price_date"]),
        (price("1899-12-31", "1"), vec!["price_date"]),
        (price("2026-01-15", "-1"), vec!["price"]),
        (price("2026-01-15", "1.123456789"), vec!["price"]),
        (price("2026-01-15", "1e3"), vec!["price"]),
        (price("15/01/2026", "x"), vec!["price_date", "price"]),
    ] {
        let response = app.add_price(&alice, &instrument, body.clone()).await;
        response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(fields(&response), expected, "{body}");
    }

    for body in [
        json!({ "price_date": "2026-01-15" }),
        json!({ "price_date": "2026-01-15", "price": 35.25 }),
        json!({ "price_date": "2026-01-15", "price": "1", "source": "IMPORT" }),
    ] {
        app.add_price(&alice, &instrument, body)
            .await
            .assert_problem(StatusCode::BAD_REQUEST, "malformed_request");
    }

    let backwards = app
        .read(
            &alice,
            &format!("{}?from=2026-02-01&to=2026-01-01", prices(&instrument)),
        )
        .await;
    backwards.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(fields(&backwards), ["to"]);

    let list = app.read(&alice, &prices(&instrument)).await;
    assert_eq!(list.body["total"], 0);
}

#[tokio::test]
async fn other_users_prices_are_not_found() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let instrument = app.instrument(&alice, "THB").await;
    let id = app.new_price(&alice, &instrument, "2026-01-15", "35").await;
    let item = price_url(&instrument, &id);

    app.add_price(&bob, &instrument, price("2026-01-16", "1"))
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    for uri in [prices(&instrument), item.clone()] {
        app.read(&bob, &uri)
            .await
            .assert_problem(StatusCode::NOT_FOUND, "not_found");
    }
    app.write(&bob, Req::put(&item).json(price("2026-01-15", "1")))
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    app.write(&bob, Req::delete(&item))
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");

    // A price is only reachable through its own instrument, even for Bob's
    // own instrument path.
    let bobs = app.instrument(&bob, "THB").await;
    app.read(&bob, &price_url(&bobs, &id))
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    app.write(&bob, Req::delete(&price_url(&bobs, &id)))
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");

    let intact = app.read(&alice, &item).await;
    assert_eq!(intact.status, StatusCode::OK);
    assert_eq!(intact.body["price"], "35");
}

#[tokio::test]
async fn an_instrument_with_prices_cannot_be_deleted() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let instrument = app.instrument(&alice, "THB").await;
    let id = app.new_price(&alice, &instrument, "2026-01-15", "35").await;
    let instrument_url = format!("/api/v1/instruments/{instrument}");

    app.write(&alice, Req::delete(&instrument_url))
        .await
        .assert_problem(StatusCode::CONFLICT, "instrument_in_use");
    let deleted = app
        .write(&alice, Req::delete(&price_url(&instrument, &id)))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let gone = app.write(&alice, Req::delete(&instrument_url)).await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT);
}

// --- FX rates ----------------------------------------------------------------

#[tokio::test]
async fn fx_rates_are_recorded_normalized_and_filtered() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;

    let created = app
        .add_rate(&alice, rate(" usd ", "thb", "2026-01-15", "36.2500"))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let id = id_of(&created);
    assert_eq!(created.headers[header::LOCATION], fx_url(&id));
    assert_eq!(created.body["base_currency"], "USD");
    assert_eq!(created.body["quote_currency"], "THB");
    assert_eq!(created.body["rate_date"], "2026-01-15");
    assert_eq!(created.body["rate"], "36.2500");
    assert_eq!(created.body["source"], "MANUAL");
    assert_eq!(app.read(&alice, &fx_url(&id)).await.body, created.body);

    app.new_rate(&alice, "USD", "THB", "2026-01-16", "36.3")
        .await;
    // The inverse pair is a different pair and may share a date.
    app.new_rate(&alice, "THB", "USD", "2026-01-15", "0.0276")
        .await;
    app.new_rate(&alice, "EUR", "THB", "2026-01-14", "39.1")
        .await;

    let all = app.read(&alice, FX).await;
    assert_eq!(all.body["total"], 4);
    assert_eq!(
        dates(&all, "rate_date"),
        ["2026-01-16", "2026-01-15", "2026-01-15", "2026-01-14"]
    );
    assert_eq!(
        all.body["items"][1]["base_currency"], "THB",
        "pairs sort by base"
    );

    let usd_thb = app
        .read(
            &alice,
            &format!("{FX}?base_currency=usd&quote_currency=THB"),
        )
        .await;
    assert_eq!(usd_thb.body["total"], 2);
    let into_thb = app
        .read(&alice, &format!("{FX}?quote_currency=THB&to=2026-01-15"))
        .await;
    assert_eq!(dates(&into_thb, "rate_date"), ["2026-01-15", "2026-01-14"]);

    let bad = app
        .read(&alice, &format!("{FX}?base_currency=US&from=2026-13-01"))
        .await;
    bad.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(fields(&bad), ["base_currency", "from"]);
}

#[tokio::test]
async fn invalid_fx_rates_are_rejected() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;

    for (body, expected) in [
        (
            rate("USD", "usd", "2026-01-15", "1"),
            vec!["quote_currency"],
        ),
        (rate("USD", "THB", "2026-01-15", "0"), vec!["rate"]),
        (rate("USD", "THB", "2026-01-15", "-36"), vec!["rate"]),
        (
            rate("USD", "THB", "2026-01-15", "0.000000001"),
            vec!["rate"],
        ),
        (rate("US", "THB", "2026-01-15", "36"), vec!["base_currency"]),
        (
            rate("USDX", "T1B", "2101-01-01", "x"),
            vec!["base_currency", "quote_currency", "rate_date", "rate"],
        ),
    ] {
        let response = app.add_rate(&alice, body.clone()).await;
        response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
        assert_eq!(fields(&response), expected, "{body}");
    }
    app.add_rate(
        &alice,
        json!({ "base_currency": "USD", "quote_currency": "THB", "rate_date": "2026-01-15" }),
    )
    .await
    .assert_problem(StatusCode::BAD_REQUEST, "malformed_request");

    assert_eq!(app.read(&alice, FX).await.body["total"], 0);
}

#[tokio::test]
async fn one_rate_per_pair_and_date_per_user() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    app.new_rate(&alice, "USD", "THB", "2026-01-15", "36").await;
    let other = app.new_rate(&alice, "USD", "THB", "2026-01-16", "37").await;

    app.add_rate(&alice, rate("usd", "thb", "2026-01-15", "35"))
        .await
        .assert_problem(StatusCode::CONFLICT, "fx_rate_exists");
    app.write(
        &alice,
        Req::put(&fx_url(&other)).json(rate("USD", "THB", "2026-01-15", "37")),
    )
    .await
    .assert_problem(StatusCode::CONFLICT, "fx_rate_exists");

    // Each user keeps their own rates.
    app.new_rate(&bob, "USD", "THB", "2026-01-15", "40").await;
    assert_eq!(app.read(&bob, FX).await.body["total"], 1);
    assert_eq!(app.read(&alice, FX).await.body["total"], 2);
}

#[tokio::test]
async fn fx_rates_can_be_replaced_and_deleted() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let id = app.new_rate(&alice, "USD", "THB", "2026-01-15", "36").await;
    let item = fx_url(&id);

    let replaced = app
        .write(
            &alice,
            Req::put(&item).json(rate("EUR", "USD", "2026-01-10", "1.08")),
        )
        .await;
    assert_eq!(replaced.status, StatusCode::OK, "{}", replaced.body);
    assert_eq!(replaced.body["base_currency"], "EUR");
    assert_eq!(replaced.body["quote_currency"], "USD");
    assert_eq!(replaced.body["rate_date"], "2026-01-10");
    assert_eq!(replaced.body["rate"], "1.08");

    app.write(
        &alice,
        Req::put(&item).json(rate("EUR", "EUR", "2026-01-10", "1")),
    )
    .await
    .assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");

    let deleted = app.write(&alice, Req::delete(&item)).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    app.read(&alice, &item)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn other_users_fx_rates_are_not_found() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let bob = user(&app, "bob@example.com").await;
    let id = app.new_rate(&alice, "USD", "THB", "2026-01-15", "36").await;
    let item = fx_url(&id);

    app.read(&bob, &item)
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    app.write(
        &bob,
        Req::put(&item).json(rate("USD", "THB", "2026-01-15", "1")),
    )
    .await
    .assert_problem(StatusCode::NOT_FOUND, "not_found");
    app.write(&bob, Req::delete(&item))
        .await
        .assert_problem(StatusCode::NOT_FOUND, "not_found");
    assert_eq!(app.read(&bob, FX).await.body["total"], 0);

    let intact = app.read(&alice, &item).await;
    assert_eq!(intact.body["rate"], "36");
}

// --- Guards and schema --------------------------------------------------------

#[tokio::test]
async fn mutations_require_csrf_and_reads_require_a_session() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let instrument = app.instrument(&alice, "THB").await;
    let price_id = app.new_price(&alice, &instrument, "2026-01-15", "35").await;
    let rate_id = app.new_rate(&alice, "USD", "THB", "2026-01-15", "36").await;
    let price_item = price_url(&instrument, &price_id);
    let rate_item = fx_url(&rate_id);
    let price_list = prices(&instrument);

    for req in [
        Req::post(&price_list).json(price("2026-01-16", "1")),
        Req::put(&price_item).json(price("2026-01-16", "1")),
        Req::delete(&price_item),
        Req::post(FX).json(rate("USD", "THB", "2026-01-16", "1")),
        Req::put(&rate_item).json(rate("USD", "THB", "2026-01-16", "1")),
        Req::delete(&rate_item),
    ] {
        app.send(req.session(&alice.session))
            .await
            .assert_problem(StatusCode::FORBIDDEN, "csrf_invalid");
    }
    for uri in [&price_list, &price_item, FX, &rate_item] {
        let response = app.send(Req::get(uri)).await;
        response.assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
    }

    assert_eq!(app.read(&alice, &price_list).await.body["total"], 1);
    assert_eq!(app.read(&alice, FX).await.body["total"], 1);
}

#[tokio::test]
async fn schema_rejects_unknown_sources() {
    let app = TestApp::new().await;
    let alice = user(&app, "alice@example.com").await;
    let instrument = app.instrument(&alice, "THB").await;
    app.new_price(&alice, &instrument, "2026-01-15", "35").await;
    app.new_rate(&alice, "USD", "THB", "2026-01-15", "36").await;

    let bad_price = instrument_price::Entity::update_many()
        .col_expr(instrument_price::Column::Source, Expr::value("FEED"))
        .filter(instrument_price::Column::Source.eq("MANUAL"))
        .exec(app.db())
        .await;
    assert!(bad_price.is_err(), "CHECK constraint must reject FEED");
    let bad_rate = fx_rate::Entity::update_many()
        .col_expr(fx_rate::Column::Source, Expr::value("FEED"))
        .filter(fx_rate::Column::Source.eq("MANUAL"))
        .exec(app.db())
        .await;
    assert!(bad_rate.is_err(), "CHECK constraint must reject FEED");
}

#[tokio::test]
async fn openapi_documents_market_data_operations_and_csrf() {
    let app = TestApp::new().await;
    let response = app.send(Req::get("/api/v1/openapi.json")).await;
    assert_eq!(response.status, StatusCode::OK);
    let paths = &response.body["paths"];

    for (path, method, requires_csrf) in [
        ("/api/v1/instruments/{id}/prices", "post", true),
        ("/api/v1/instruments/{id}/prices", "get", false),
        ("/api/v1/instruments/{id}/prices/{price_id}", "get", false),
        ("/api/v1/instruments/{id}/prices/{price_id}", "put", true),
        ("/api/v1/instruments/{id}/prices/{price_id}", "delete", true),
        ("/api/v1/fx-rates", "post", true),
        ("/api/v1/fx-rates", "get", false),
        ("/api/v1/fx-rates/{id}", "get", false),
        ("/api/v1/fx-rates/{id}", "put", true),
        ("/api/v1/fx-rates/{id}", "delete", true),
    ] {
        let op = &paths[path][method];
        assert!(op.is_object(), "{method} {path}");
        assert_eq!(op["tags"][0], "market-data", "{method} {path}");
        assert!(op["security"].is_array(), "{method} {path}");
        let has_csrf = op["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "X-CSRF-Token" && p["in"] == "header");
        assert_eq!(has_csrf, requires_csrf, "{method} {path}");
    }

    let schemas = &response.body["components"]["schemas"];
    assert_eq!(schemas["SourceDto"]["enum"], json!(["MANUAL", "IMPORT"]));
    for (schema, field) in [
        ("PriceRequest", "price"),
        ("PriceResponse", "price"),
        ("FxRateRequest", "rate"),
        ("FxRateResponse", "rate"),
    ] {
        assert_eq!(
            schemas[schema]["properties"][field]["type"], "string",
            "{schema}.{field} is a decimal string"
        );
    }
}
