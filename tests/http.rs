mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use std::error::Error;

use tower::ServiceExt;

async fn get_json(uri: &str) -> Result<(StatusCode, Value), Box<dyn Error>> {
    let state = common::test_state().await?;
    let response = merton_api::http::router(state)
        .oneshot(Request::get(uri).body(Body::empty())?)
        .await?;

    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&body)?))
}

#[tokio::test]
async fn health_returns_ok() {
    let (status, body) = get_json("/api/v1/health").await.unwrap();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "status": "ok" }));
}

#[tokio::test]
async fn openapi_documents_health_endpoint() {
    let (status, body) = get_json("/api/v1/openapi.json").await.unwrap();

    assert_eq!(status, StatusCode::OK);
    assert!(body["paths"]["/api/v1/health"]["get"].is_object());
}

#[tokio::test]
async fn unknown_route_returns_not_found() {
    let state = common::test_state().await.unwrap();
    let response = merton_api::http::router(state)
        .oneshot(Request::get("/api/v1/nope").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
