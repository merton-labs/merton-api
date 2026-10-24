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
