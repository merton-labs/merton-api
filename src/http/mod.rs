//! HTTP routing and OpenAPI description.

mod auth;
mod cash_accounts;
mod error;
pub mod extract;
mod health;
mod instruments;
mod market_data;
mod portfolios;
mod transactions;

use axum::http::{HeaderValue, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::app::AppState;

#[derive(OpenApi)]
#[openapi(
    info(title = "Merton API"),
    paths(
        health::health,
        auth::register,
        auth::login,
        auth::logout,
        auth::me,
        auth::csrf,
        portfolios::create,
        portfolios::list,
        portfolios::get,
        portfolios::update,
        portfolios::archive,
        portfolios::restore,
        portfolios::delete,
        instruments::create,
        instruments::list,
        instruments::get,
        instruments::update,
        instruments::delete,
        cash_accounts::create,
        cash_accounts::list,
        cash_accounts::get,
        cash_accounts::update,
        cash_accounts::delete,
        transactions::create,
        transactions::list,
        transactions::get,
        transactions::replace,
        transactions::delete,
        market_data::create_price,
        market_data::list_prices,
        market_data::get_price,
        market_data::replace_price,
        market_data::delete_price,
        market_data::create_fx_rate,
        market_data::list_fx_rates,
        market_data::get_fx_rate,
        market_data::replace_fx_rate,
        market_data::delete_fx_rate,
    ),
    modifiers(&SessionCookieScheme),
    tags(
        (name = "health", description = "Service health"),
        (name = "auth", description = "Registration, login and sessions. Errors use RFC 9457 Problem Details."),
        (name = "portfolios", description = "The current user's private portfolios. Other users' portfolios are reported as 404."),
        (name = "instruments", description = "The current user's instruments. Identity fields are optional and need not be unique."),
        (name = "cash-accounts", description = "Cash accounts inside a portfolio. Changes require an active portfolio."),
        (name = "transactions", description = "The portfolio ledger. Decimal values are JSON strings; FIFO history is validated on every change."),
        (name = "market-data", description = "The current user's historical instrument prices and FX rates, entered manually. Decimal values are JSON strings."),
    ),
)]
pub struct ApiDoc;

struct SessionCookieScheme;

impl Modify for SessionCookieScheme {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "session_cookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                extract::SECURE_SESSION_COOKIE,
                "HttpOnly session cookie set by login. Named `merton_session` when \
                 SESSION_COOKIE_SECURE=false (localhost development only).",
            ))),
        );
    }
}

pub fn router(state: AppState) -> Router {
    let auth = Router::new()
        .route("/register", post(auth::register))
        .route("/login", post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/csrf", get(auth::csrf))
        .layer(no_store());

    let portfolios = Router::new()
        .route("/", get(portfolios::list).post(portfolios::create))
        .route(
            "/{id}",
            get(portfolios::get)
                .patch(portfolios::update)
                .delete(portfolios::delete),
        )
        .route("/{id}/archive", post(portfolios::archive))
        .route("/{id}/restore", post(portfolios::restore))
        .route(
            "/{id}/cash-accounts",
            get(cash_accounts::list).post(cash_accounts::create),
        )
        .route(
            "/{id}/transactions",
            get(transactions::list).post(transactions::create),
        )
        .route(
            "/{id}/transactions/{transaction_id}",
            get(transactions::get)
                .put(transactions::replace)
                .delete(transactions::delete),
        )
        .route(
            "/{id}/cash-accounts/{account_id}",
            get(cash_accounts::get)
                .patch(cash_accounts::update)
                .delete(cash_accounts::delete),
        )
        .layer(no_store());

    let instruments = Router::new()
        .route("/", get(instruments::list).post(instruments::create))
        .route(
            "/{id}",
            get(instruments::get)
                .patch(instruments::update)
                .delete(instruments::delete),
        )
        .route(
            "/{id}/prices",
            get(market_data::list_prices).post(market_data::create_price),
        )
        .route(
            "/{id}/prices/{price_id}",
            get(market_data::get_price)
                .put(market_data::replace_price)
                .delete(market_data::delete_price),
        )
        .layer(no_store());

    let fx_rates = Router::new()
        .route(
            "/",
            get(market_data::list_fx_rates).post(market_data::create_fx_rate),
        )
        .route(
            "/{id}",
            get(market_data::get_fx_rate)
                .put(market_data::replace_fx_rate)
                .delete(market_data::delete_fx_rate),
        )
        .layer(no_store());

    let api_v1 = Router::new()
        .route("/health", get(health::health))
        .route("/openapi.json", get(openapi_json))
        .nest("/auth", auth)
        .nest("/portfolios", portfolios)
        .nest("/instruments", instruments)
        .nest("/fx-rates", fx_rates);

    Router::new()
        .nest("/api/v1", api_v1)
        .fallback(error::not_found)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .with_state(state)
}

/// Authentication and private financial data must never be cached.
fn no_store() -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
