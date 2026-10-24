mod common;

use axum::http::StatusCode;
use common::{Req, TestApp};
use serde_json::json;

#[tokio::test]
async fn health_returns_ok() {
    let app = TestApp::new().await;
    let response = app.send(Req::get("/api/v1/health")).await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body, json!({ "status": "ok" }));
}

#[tokio::test]
async fn openapi_documents_endpoints_and_cookie_auth() {
    let app = TestApp::new().await;
    let response = app.send(Req::get("/api/v1/openapi.json")).await;
    let doc = &response.body;

    assert_eq!(response.status, StatusCode::OK);
    assert!(doc["paths"]["/api/v1/health"]["get"].is_object());
    for (path, method) in [
        ("/api/v1/auth/register", "post"),
        ("/api/v1/auth/login", "post"),
        ("/api/v1/auth/logout", "post"),
        ("/api/v1/auth/me", "get"),
        ("/api/v1/auth/csrf", "get"),
    ] {
        assert!(doc["paths"][path][method].is_object(), "{method} {path}");
    }

    for (path, method) in [
        ("/api/v1/portfolios", "get"),
        ("/api/v1/portfolios", "post"),
        ("/api/v1/portfolios/{id}", "get"),
        ("/api/v1/portfolios/{id}", "patch"),
        ("/api/v1/portfolios/{id}", "delete"),
        ("/api/v1/portfolios/{id}/archive", "post"),
        ("/api/v1/portfolios/{id}/restore", "post"),
        ("/api/v1/instruments", "get"),
        ("/api/v1/instruments", "post"),
        ("/api/v1/instruments/{id}", "get"),
        ("/api/v1/instruments/{id}", "patch"),
        ("/api/v1/instruments/{id}", "delete"),
        ("/api/v1/portfolios/{id}/cash-accounts", "get"),
        ("/api/v1/portfolios/{id}/cash-accounts", "post"),
        ("/api/v1/portfolios/{id}/cash-accounts/{account_id}", "get"),
        (
            "/api/v1/portfolios/{id}/cash-accounts/{account_id}",
            "patch",
        ),
        (
            "/api/v1/portfolios/{id}/cash-accounts/{account_id}",
            "delete",
        ),
    ] {
        let operation = &doc["paths"][path][method];
        assert!(operation.is_object(), "{method} {path}");
        assert!(
            operation["security"].is_array(),
            "{method} {path} needs a session"
        );
        if method != "get" {
            let params = operation["parameters"].as_array().unwrap();
            assert!(
                params
                    .iter()
                    .any(|p| p["name"] == "X-CSRF-Token" && p["in"] == "header"),
                "{method} {path} must document the CSRF header"
            );
        }
    }

    let scheme = &doc["components"]["securitySchemes"]["session_cookie"];
    assert_eq!(scheme["type"], "apiKey");
    assert_eq!(scheme["in"], "cookie");
    assert_eq!(scheme["name"], "__Host-merton_session");

    let logout = &doc["paths"]["/api/v1/auth/logout"]["post"];
    assert_eq!(logout["parameters"][0]["name"], "X-CSRF-Token");
    assert_eq!(logout["parameters"][0]["in"], "header");
    assert!(
        logout["responses"]["403"]["content"]["application/problem+json"].is_object(),
        "{logout}"
    );
}

#[tokio::test]
async fn unknown_route_returns_problem_not_found() {
    let app = TestApp::new().await;
    let response = app.send(Req::get("/api/v1/nope")).await;

    response.assert_problem(StatusCode::NOT_FOUND, "not_found");
}
