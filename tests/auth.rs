#![allow(clippy::unwrap_used)]

mod common;

use axum::http::{StatusCode, header};
use common::{ORIGIN, PASSWORD, Req, SESSION_COOKIE, TestApp, TestResponse, security_config};
use merton_api::auth::service;
use merton_api::auth::session::{PURGE_GRACE, hash_presented_token};
use merton_api::infrastructure::entities::{user, user_session};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, SqlErr};
use serde_json::json;
use time::macros::datetime;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

async fn session_row(app: &TestApp, token: &str) -> user_session::Model {
    user_session::Entity::find()
        .filter(user_session::Column::TokenHash.eq(hash_presented_token(token).unwrap()))
        .one(app.db())
        .await
        .unwrap()
        .unwrap()
}

async fn update_session(
    app: &TestApp,
    token: &str,
    edit: impl FnOnce(&mut user_session::ActiveModel),
) {
    let mut model: user_session::ActiveModel = session_row(app, token).await.into();
    edit(&mut model);
    model.update(app.db()).await.unwrap();
}

async fn user_row(app: &TestApp, normalized_email: &str) -> user::Model {
    user::Entity::find()
        .filter(user::Column::NormalizedEmail.eq(normalized_email))
        .one(app.db())
        .await
        .unwrap()
        .unwrap()
}

fn assert_close(actual: OffsetDateTime, expected: OffsetDateTime) {
    assert!(
        (actual - expected).abs() < Duration::seconds(5),
        "{actual} is not close to {expected}"
    );
}

// --- Registration ---------------------------------------------------------

#[tokio::test]
async fn register_creates_account_without_logging_in() {
    let app = TestApp::new().await;
    let response = app.register("new.user@example.com", PASSWORD).await;

    assert_eq!(response.status, StatusCode::CREATED);
    assert_eq!(response.body["email"], "new.user@example.com");
    assert!(Uuid::parse_str(response.body["id"].as_str().unwrap()).is_ok());
    assert_eq!(
        response.body.as_object().unwrap().len(),
        2,
        "only id and email"
    );
    assert!(response.session_set_cookie().is_none());
    assert_eq!(response.headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn register_rejects_duplicate_normalized_email() {
    let app = TestApp::new().await;
    assert_eq!(
        app.register("dup@example.com", PASSWORD).await.status,
        StatusCode::CREATED
    );

    let response = app.register("  DUP@Example.com ", PASSWORD).await;

    response.assert_problem(StatusCode::CONFLICT, "email_taken");
}

#[tokio::test]
async fn register_normalizes_email_but_keeps_display_form() {
    let app = TestApp::new().await;
    let response = app.register("  Alice.Smith@Example.COM ", PASSWORD).await;
    assert_eq!(response.status, StatusCode::CREATED);
    assert_eq!(response.body["email"], "Alice.Smith@Example.COM");

    let row = user_row(&app, "alice.smith@example.com").await;
    assert_eq!(row.email, "Alice.Smith@Example.COM");

    let login = app.login("alice.smith@EXAMPLE.com", PASSWORD).await;
    assert_eq!(login.status, StatusCode::OK);
}

#[tokio::test]
async fn register_rejects_invalid_email() {
    let app = TestApp::new().await;
    let response = app.register("not-an-email", PASSWORD).await;

    response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(response.body["errors"][0]["field"], "email");
}

#[tokio::test]
async fn register_rejects_password_outside_length_limits() {
    let app = TestApp::new().await;

    let short = app.register("short@example.com", "too short").await;
    short.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(short.body["errors"][0]["field"], "password");

    let long = app.register("long@example.com", &"x".repeat(129)).await;
    long.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
}

#[tokio::test]
async fn register_reports_all_invalid_fields() {
    let app = TestApp::new().await;
    let response = app.register("bad", "short").await;

    response.assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(response.body["errors"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn password_is_stored_as_argon2id_hash() {
    let app = TestApp::new().await;
    app.register("hash@example.com", PASSWORD).await;

    let row = user_row(&app, "hash@example.com").await;
    assert!(row.password_hash.starts_with("$argon2id$v=19$"));
    assert!(!row.password_hash.contains(PASSWORD));
    assert!(!format!("{row:?}").contains(&row.password_hash));
}

#[tokio::test]
async fn malformed_requests_are_problem_details() {
    let app = TestApp::new().await;

    let bad_json = app
        .send(
            Req::post("/api/v1/auth/register")
                .header("content-type", "application/json")
                .json(json!("not an object")),
        )
        .await;
    bad_json.assert_problem(StatusCode::BAD_REQUEST, "malformed_request");

    let missing_field = app
        .send(Req::post("/api/v1/auth/login").json(json!({ "email": "a@example.com" })))
        .await;
    missing_field.assert_problem(StatusCode::BAD_REQUEST, "malformed_request");

    let no_content_type = app.send(Req::post("/api/v1/auth/login")).await;
    no_content_type.assert_problem(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type");
}

// --- Login ----------------------------------------------------------------

#[tokio::test]
async fn login_sets_hardened_session_cookie() {
    let app = TestApp::new().await;
    app.register("login@example.com", PASSWORD).await;

    let response = app.login("login@example.com", PASSWORD).await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["email"], "login@example.com");
    assert!(response.body.get("password_hash").is_none());

    let cookie = response.session_set_cookie().unwrap();
    for attribute in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/"] {
        assert!(cookie.contains(attribute), "{cookie} lacks {attribute}");
    }
    assert!(
        !cookie.contains("Domain="),
        "__Host- cookies must not set Domain"
    );
    let max_age: i64 = cookie
        .split("; ")
        .find_map(|part| part.strip_prefix("Max-Age="))
        .unwrap()
        .parse()
        .unwrap();
    assert!((Duration::days(30) - Duration::seconds(max_age)).abs() < Duration::seconds(5));

    let token = response.session_token().unwrap();
    assert_eq!(token.len(), 43, "base64url of 32 random bytes");
    let row = session_row(&app, &token).await;
    assert_eq!(row.token_hash.len(), 32);
    assert_ne!(row.token_hash, token.as_bytes());
}

#[tokio::test]
async fn login_rejects_wrong_password_and_unknown_email_identically() {
    let app = TestApp::new().await;
    app.register("known@example.com", PASSWORD).await;

    let wrong_password = app
        .login("known@example.com", "not the right password")
        .await;
    let unknown_email = app.login("nobody@example.com", PASSWORD).await;

    wrong_password.assert_problem(StatusCode::UNAUTHORIZED, "invalid_credentials");
    unknown_email.assert_problem(StatusCode::UNAUTHORIZED, "invalid_credentials");
    assert_eq!(wrong_password.body, unknown_email.body);
    assert!(wrong_password.session_set_cookie().is_none());
}

#[tokio::test]
async fn login_is_rate_limited_per_email() {
    let app = TestApp::new().await;
    app.register("victim@example.com", PASSWORD).await;

    for _ in 0..5 {
        let response = app
            .login("victim@example.com", "wrong password guess")
            .await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    }
    let limited = app.login("Victim@Example.com", PASSWORD).await;

    limited.assert_problem(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    let retry_after: u64 = limited.headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(retry_after > 0 && retry_after <= 15 * 60);
    assert!(limited.session_set_cookie().is_none());
}

#[tokio::test]
async fn login_is_rate_limited_per_client_ip() {
    let app = TestApp::new().await;

    for i in 0..20 {
        let email = format!("probe{i}@example.com");
        let response = app
            .send(
                Req::post("/api/v1/auth/login")
                    .peer("198.51.100.9:4000")
                    .json(json!({ "email": email, "password": PASSWORD })),
            )
            .await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    }
    let limited = app
        .send(
            Req::post("/api/v1/auth/login")
                .peer("198.51.100.9:4001")
                .json(json!({ "email": "fresh@example.com", "password": PASSWORD })),
        )
        .await;
    limited.assert_problem(StatusCode::TOO_MANY_REQUESTS, "rate_limited");

    // Forwarded headers from an untrusted peer do not reset the limit.
    let spoofed = app
        .send(
            Req::post("/api/v1/auth/login")
                .peer("198.51.100.9:4002")
                .header("x-forwarded-for", "203.0.113.77")
                .json(json!({ "email": "fresh@example.com", "password": PASSWORD })),
        )
        .await;
    spoofed.assert_problem(StatusCode::TOO_MANY_REQUESTS, "rate_limited");

    let other_client = app
        .send(
            Req::post("/api/v1/auth/login")
                .peer("192.0.2.44:4000")
                .json(json!({ "email": "fresh@example.com", "password": PASSWORD })),
        )
        .await;
    assert_eq!(other_client.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_revokes_previously_presented_session() {
    let app = TestApp::new().await;
    let first = app.signed_in("relogin@example.com").await;

    let second_login = app
        .send(
            Req::post("/api/v1/auth/login")
                .session(&first)
                .json(json!({ "email": "relogin@example.com", "password": PASSWORD })),
        )
        .await;
    let second = second_login.session_token().unwrap();

    assert_ne!(first, second);
    app.me(&first)
        .await
        .assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
    assert_eq!(app.me(&second).await.status, StatusCode::OK);
}

// --- Sessions -------------------------------------------------------------

#[tokio::test]
async fn me_returns_current_user_for_valid_session() {
    let app = TestApp::new().await;
    let token = app.signed_in("me@example.com").await;

    let response = app.me(&token).await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["email"], "me@example.com");
    assert_eq!(response.headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn me_requires_a_session() {
    let app = TestApp::new().await;

    let response = app.send(Req::get("/api/v1/auth/me")).await;

    response.assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
}

#[tokio::test]
async fn idle_expired_session_is_rejected() {
    let app = TestApp::new().await;
    let token = app.signed_in("idle@example.com").await;
    update_session(&app, &token, |s| {
        s.idle_expires_at = Set(OffsetDateTime::now_utc() - Duration::seconds(1));
    })
    .await;

    app.me(&token)
        .await
        .assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
}

#[tokio::test]
async fn absolute_expired_session_is_rejected_even_if_recently_active() {
    let app = TestApp::new().await;
    let token = app.signed_in("absolute@example.com").await;
    let now = OffsetDateTime::now_utc();
    update_session(&app, &token, |s| {
        s.last_activity_at = Set(now);
        s.idle_expires_at = Set(now + Duration::days(7));
        s.absolute_expires_at = Set(now - Duration::seconds(1));
    })
    .await;

    app.me(&token)
        .await
        .assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
}

#[tokio::test]
async fn revoked_session_is_rejected() {
    let app = TestApp::new().await;
    let token = app.signed_in("revoked@example.com").await;
    update_session(&app, &token, |s| {
        s.revoked_at = Set(Some(OffsetDateTime::now_utc()));
    })
    .await;

    app.me(&token)
        .await
        .assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
}

#[tokio::test]
async fn activity_slides_idle_expiry_after_update_interval() {
    let app = TestApp::new().await;
    let token = app.signed_in("sliding@example.com").await;
    let now = OffsetDateTime::now_utc();
    update_session(&app, &token, |s| {
        s.last_activity_at = Set(now - Duration::hours(1));
        s.idle_expires_at = Set(now + Duration::hours(1));
    })
    .await;

    assert_eq!(app.me(&token).await.status, StatusCode::OK);

    let row = session_row(&app, &token).await;
    assert_close(row.last_activity_at, now);
    assert_close(row.idle_expires_at, now + Duration::days(7));
}

#[tokio::test]
async fn recent_activity_does_not_write_to_database() {
    let app = TestApp::new().await;
    let token = app.signed_in("quiet@example.com").await;
    let before = session_row(&app, &token).await;

    assert_eq!(app.me(&token).await.status, StatusCode::OK);
    assert_eq!(app.me(&token).await.status, StatusCode::OK);

    let after = session_row(&app, &token).await;
    assert_eq!(after.last_activity_at, before.last_activity_at);
    assert_eq!(after.idle_expires_at, before.idle_expires_at);
}

#[tokio::test]
async fn sliding_expiry_never_exceeds_absolute_expiry() {
    let app = TestApp::new().await;
    let token = app.signed_in("capped@example.com").await;
    let now = OffsetDateTime::now_utc();
    let absolute = now + Duration::days(1);
    update_session(&app, &token, |s| {
        s.last_activity_at = Set(now - Duration::hours(1));
        s.idle_expires_at = Set(now + Duration::hours(1));
        s.absolute_expires_at = Set(absolute);
    })
    .await;

    assert_eq!(app.me(&token).await.status, StatusCode::OK);

    assert_eq!(session_row(&app, &token).await.idle_expires_at, absolute);
}

#[tokio::test]
async fn logout_revokes_session_and_clears_cookie() {
    let app = TestApp::new().await;
    let token = app.signed_in("logout@example.com").await;
    let csrf = app.csrf_token(&token).await;

    let response = app
        .send(
            Req::post("/api/v1/auth/logout")
                .session(&token)
                .header("x-csrf-token", &csrf),
        )
        .await;

    assert_eq!(response.status, StatusCode::NO_CONTENT);
    let cleared = response.session_set_cookie().unwrap();
    assert!(
        cleared.starts_with(&format!("{SESSION_COOKIE}=;")),
        "{cleared}"
    );
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    assert!(
        cleared.contains("Secure") && cleared.contains("Path=/"),
        "{cleared}"
    );

    assert!(session_row(&app, &token).await.revoked_at.is_some());
    app.me(&token)
        .await
        .assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");

    let again = app
        .send(
            Req::post("/api/v1/auth/logout")
                .session(&token)
                .header("x-csrf-token", &csrf),
        )
        .await;
    again.assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
}

// --- Security -------------------------------------------------------------

#[tokio::test]
async fn logout_requires_valid_csrf_token() {
    let app = TestApp::new().await;
    let token = app.signed_in("csrf@example.com").await;
    let other = app.signed_in("csrf-other@example.com").await;
    let others_csrf = app.csrf_token(&other).await;

    let missing = app
        .send(Req::post("/api/v1/auth/logout").session(&token))
        .await;
    missing.assert_problem(StatusCode::FORBIDDEN, "csrf_invalid");

    let forged = app
        .send(
            Req::post("/api/v1/auth/logout")
                .session(&token)
                .header("x-csrf-token", "forged-token-value"),
        )
        .await;
    forged.assert_problem(StatusCode::FORBIDDEN, "csrf_invalid");

    let wrong_session = app
        .send(
            Req::post("/api/v1/auth/logout")
                .session(&token)
                .header("x-csrf-token", &others_csrf),
        )
        .await;
    wrong_session.assert_problem(StatusCode::FORBIDDEN, "csrf_invalid");

    assert_eq!(
        app.me(&token).await.status,
        StatusCode::OK,
        "session survives"
    );
}

#[tokio::test]
async fn csrf_token_requires_a_session() {
    let app = TestApp::new().await;

    let response = app.send(Req::get("/api/v1/auth/csrf")).await;

    response.assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
}

#[tokio::test]
async fn csrf_token_changes_with_each_session() {
    let app = TestApp::new().await;
    let first = app.signed_in("rotate@example.com").await;
    let second = app
        .login("rotate@example.com", PASSWORD)
        .await
        .session_token()
        .unwrap();

    assert_ne!(app.csrf_token(&first).await, app.csrf_token(&second).await);
}

#[tokio::test]
async fn mutations_from_untrusted_origins_are_rejected() {
    let app = TestApp::new().await;
    let token = app.signed_in("origin@example.com").await;
    let csrf = app.csrf_token(&token).await;
    let body = json!({ "email": "origin@example.com", "password": PASSWORD });

    for uri in ["/api/v1/auth/register", "/api/v1/auth/login"] {
        let cross_site = app
            .send(
                Req::post(uri)
                    .header("origin", "https://evil.example")
                    .json(body.clone()),
            )
            .await;
        cross_site.assert_problem(StatusCode::FORBIDDEN, "untrusted_origin");
    }

    let sec_fetch = app
        .send(
            Req::post("/api/v1/auth/login")
                .without_header("origin")
                .header("sec-fetch-site", "cross-site")
                .json(body.clone()),
        )
        .await;
    sec_fetch.assert_problem(StatusCode::FORBIDDEN, "untrusted_origin");

    let logout = app
        .send(
            Req::post("/api/v1/auth/logout")
                .header("origin", "null")
                .session(&token)
                .header("x-csrf-token", &csrf),
        )
        .await;
    logout.assert_problem(StatusCode::FORBIDDEN, "untrusted_origin");

    // Non-browser clients send no Origin and are allowed.
    let cli = app
        .send(
            Req::post("/api/v1/auth/login")
                .without_header("origin")
                .json(body),
        )
        .await;
    assert_eq!(cli.status, StatusCode::OK);
    assert_eq!(ORIGIN, "http://localhost:8080");
}

#[tokio::test]
async fn invalid_session_tokens_are_rejected() {
    let app = TestApp::new().await;
    app.signed_in("tokens@example.com").await;
    let well_formed_but_unknown = "A".repeat(43);

    for token in ["garbage", "", well_formed_but_unknown.as_str()] {
        app.me(token)
            .await
            .assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
    }
}

#[tokio::test]
async fn client_supplied_identity_is_ignored() {
    let app = TestApp::new().await;
    let alice = app.signed_in("alice@example.com").await;
    let bob_id = app.me(&app.signed_in("bob@example.com").await).await.body["id"].clone();
    let bob_id = bob_id.as_str().unwrap();

    let uri = format!("/api/v1/auth/me?user_id={bob_id}");
    let response = app
        .send(
            Req::get(&uri)
                .session(&alice)
                .header("x-user-id", bob_id)
                .json(json!({ "user_id": bob_id })),
        )
        .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.body["email"], "alice@example.com");
    assert_ne!(response.body["id"], bob_id);
}

#[tokio::test]
async fn sessions_are_isolated_between_users() {
    let app = TestApp::new().await;
    let alice = app.signed_in("iso-alice@example.com").await;
    let bob = app.signed_in("iso-bob@example.com").await;

    let alice_me = app.me(&alice).await;
    let bob_me = app.me(&bob).await;
    assert_eq!(alice_me.body["email"], "iso-alice@example.com");
    assert_eq!(bob_me.body["email"], "iso-bob@example.com");
    assert_ne!(alice_me.body["id"], bob_me.body["id"]);

    // Logging out Alice leaves Bob's session intact.
    let csrf = app.csrf_token(&alice).await;
    let logout = app
        .send(
            Req::post("/api/v1/auth/logout")
                .session(&alice)
                .header("x-csrf-token", &csrf),
        )
        .await;
    assert_eq!(logout.status, StatusCode::NO_CONTENT);
    assert_eq!(app.me(&bob).await.status, StatusCode::OK);
}

#[tokio::test]
async fn database_enforces_unique_email_and_token_hash() {
    let app = TestApp::new().await;
    let token = app.signed_in("unique@example.com").await;
    let existing_user = user_row(&app, "unique@example.com").await;
    let existing_session = session_row(&app, &token).await;
    let now = OffsetDateTime::now_utc();

    let duplicate_user = user::ActiveModel {
        id: Set(Uuid::now_v7()),
        email: Set("UNIQUE@example.com".to_owned()),
        normalized_email: Set("unique@example.com".to_owned()),
        password_hash: Set("x".to_owned()),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let err = user::Entity::insert(duplicate_user)
        .exec_without_returning(app.db())
        .await
        .unwrap_err();
    assert!(matches!(
        err.sql_err(),
        Some(SqlErr::UniqueConstraintViolation(_))
    ));

    let duplicate_session = user_session::ActiveModel {
        id: Set(Uuid::now_v7()),
        user_id: Set(existing_user.id),
        token_hash: Set(existing_session.token_hash.clone()),
        created_at: Set(now),
        last_activity_at: Set(now),
        idle_expires_at: Set(now),
        absolute_expires_at: Set(now),
        revoked_at: Set(None),
    };
    let err = user_session::Entity::insert(duplicate_session)
        .exec_without_returning(app.db())
        .await
        .unwrap_err();
    assert!(matches!(
        err.sql_err(),
        Some(SqlErr::UniqueConstraintViolation(_))
    ));
}

#[tokio::test]
async fn sessions_require_an_existing_user() {
    let app = TestApp::new().await;
    let now = OffsetDateTime::now_utc();
    let orphan = user_session::ActiveModel {
        id: Set(Uuid::now_v7()),
        user_id: Set(Uuid::now_v7()),
        token_hash: Set(vec![7; 32]),
        created_at: Set(now),
        last_activity_at: Set(now),
        idle_expires_at: Set(now),
        absolute_expires_at: Set(now),
        revoked_at: Set(None),
    };

    let err = user_session::Entity::insert(orphan)
        .exec_without_returning(app.db())
        .await
        .unwrap_err();

    assert!(matches!(
        err.sql_err(),
        Some(SqlErr::ForeignKeyConstraintViolation(_))
    ));
}

// --- Concurrency and overload ---------------------------------------------

/// Sends all requests at once and returns the response statuses.
async fn concurrently(
    app: &TestApp,
    requests: Vec<(String, serde_json::Value)>,
) -> Vec<StatusCode> {
    let mut tasks = tokio::task::JoinSet::new();
    for (peer, body) in requests {
        let app = app.clone();
        tasks.spawn(async move {
            app.send(Req::post("/api/v1/auth/login").peer(&peer).json(body))
                .await
                .status
        });
    }
    tasks.join_all().await
}

fn count(statuses: &[StatusCode], status: StatusCode) -> usize {
    statuses.iter().filter(|s| **s == status).count()
}

fn roomy_hashing() -> merton_api::config::SecurityConfig {
    merton_api::config::SecurityConfig {
        max_concurrent_hashes: 64,
        ..security_config()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_guesses_cannot_exceed_account_failure_limit() {
    let app = TestApp::with_config(&roomy_hashing()).await;
    app.register("target@example.com", PASSWORD).await;

    // Twelve wrong guesses in flight at once, each from a different client.
    let requests = (0..12)
        .map(|i| {
            (
                format!("192.0.2.{}:5000", 10 + i),
                json!({ "email": "target@example.com", "password": "wrong password guess" }),
            )
        })
        .collect();
    let statuses = concurrently(&app, requests).await;

    assert_eq!(
        count(&statuses, StatusCode::UNAUTHORIZED),
        5,
        "{statuses:?}"
    );
    assert_eq!(
        count(&statuses, StatusCode::TOO_MANY_REQUESTS),
        7,
        "{statuses:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_attempts_from_one_client_cannot_exceed_ip_limit() {
    let app = TestApp::with_config(&roomy_hashing()).await;

    let requests = (0..30)
        .map(|i| {
            (
                "198.51.100.20:6000".to_owned(),
                json!({ "email": format!("spray{i}@example.com"), "password": PASSWORD }),
            )
        })
        .collect();
    let statuses = concurrently(&app, requests).await;

    assert_eq!(
        count(&statuses, StatusCode::UNAUTHORIZED),
        20,
        "{statuses:?}"
    );
    assert_eq!(
        count(&statuses, StatusCode::TOO_MANY_REQUESTS),
        10,
        "{statuses:?}"
    );
}

fn assert_overloaded(response: &TestResponse) {
    response.assert_problem(StatusCode::SERVICE_UNAVAILABLE, "service_overloaded");
    assert_eq!(response.headers[header::RETRY_AFTER], "1");
    assert!(response.session_set_cookie().is_none());
}

#[tokio::test]
async fn saturated_hashing_rejects_all_password_work_uniformly() {
    let app = TestApp::new().await;
    app.register("busy@example.com", PASSWORD).await;
    let hashes = &app.state.security.hash_limiter;
    let held: Vec<_> = (0..4).map(|_| hashes.try_reserve().unwrap()).collect();

    let known = app.login("busy@example.com", PASSWORD).await;
    let unknown = app.login("nobody@example.com", PASSWORD).await;
    let malformed_email = app.login("not-an-email", PASSWORD).await;
    let register = app.register("new@example.com", PASSWORD).await;

    for response in [&known, &unknown, &malformed_email, &register] {
        assert_overloaded(response);
    }
    assert_eq!(
        known.body, unknown.body,
        "overload must not reveal accounts"
    );

    // Cheap validation still runs first; nothing is queued while saturated.
    app.register("bad", "short")
        .await
        .assert_problem(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed");
    assert_eq!(hashes.available(), 0);

    drop(held);
    assert_eq!(hashes.available(), 4);
    assert_eq!(
        app.login("busy@example.com", PASSWORD).await.status,
        StatusCode::OK
    );
    assert_eq!(
        app.register("new@example.com", PASSWORD).await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        hashes.available(),
        4,
        "permits returned after each operation"
    );
}

#[tokio::test]
async fn overload_does_not_consume_account_failure_budget() {
    let app = TestApp::new().await;
    app.register("patient@example.com", PASSWORD).await;
    let hashes = &app.state.security.hash_limiter;

    let held: Vec<_> = (0..4).map(|_| hashes.try_reserve().unwrap()).collect();
    for _ in 0..8 {
        assert_overloaded(
            &app.login("patient@example.com", "wrong password guess")
                .await,
        );
    }
    drop(held);

    assert_eq!(
        app.login("patient@example.com", PASSWORD).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn permits_are_returned_after_failed_logins() {
    let app = TestApp::new().await;
    app.register("again@example.com", PASSWORD).await;

    for _ in 0..3 {
        let response = app.login("again@example.com", "wrong password guess").await;
        response.assert_problem(StatusCode::UNAUTHORIZED, "invalid_credentials");
    }

    assert_eq!(app.state.security.hash_limiter.available(), 4);
}

#[tokio::test]
async fn overlong_login_password_fails_without_hashing() {
    let app = TestApp::new().await;
    app.register("long@example.com", PASSWORD).await;
    let hashes = &app.state.security.hash_limiter;
    let held: Vec<_> = (0..4).map(|_| hashes.try_reserve().unwrap()).collect();

    // Even with no hashing capacity left, an impossible password is a plain 401.
    let response = app.login("long@example.com", &"x".repeat(129)).await;

    response.assert_problem(StatusCode::UNAUTHORIZED, "invalid_credentials");
    drop(held);
}

#[tokio::test]
async fn registration_attempts_share_the_client_budget() {
    let app = TestApp::new().await;

    // Invalid registrations are cheap but still count, so they cannot be
    // used to probe without limit.
    for _ in 0..20 {
        let response = app.register("bad", "short").await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    let limited = app.register("valid@example.com", PASSWORD).await;
    limited.assert_problem(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    let login = app.login("valid@example.com", PASSWORD).await;
    login.assert_problem(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    assert_eq!(app.state.security.hash_limiter.available(), 4);
}

async fn session_exists(app: &TestApp, token: &str) -> bool {
    user_session::Entity::find()
        .filter(user_session::Column::TokenHash.eq(hash_presented_token(token).unwrap()))
        .one(app.db())
        .await
        .unwrap()
        .is_some()
}

#[tokio::test]
async fn purge_deletes_only_sessions_ended_beyond_the_grace_period() {
    let app = TestApp::new().await;
    let now = OffsetDateTime::now_utc();
    let long_ago = now - PURGE_GRACE - Duration::minutes(1);
    let recently = now - Duration::hours(1);

    let active = app.signed_in("active@example.com").await;
    let revoked_long_ago = app.signed_in("revoked-old@example.com").await;
    update_session(&app, &revoked_long_ago, |s| {
        s.revoked_at = Set(Some(long_ago))
    })
    .await;
    let revoked_recently = app.signed_in("revoked-new@example.com").await;
    update_session(&app, &revoked_recently, |s| {
        s.revoked_at = Set(Some(recently))
    })
    .await;
    let idle_long_ago = app.signed_in("idle-old@example.com").await;
    update_session(&app, &idle_long_ago, |s| s.idle_expires_at = Set(long_ago)).await;
    let idle_recently = app.signed_in("idle-new@example.com").await;
    update_session(&app, &idle_recently, |s| s.idle_expires_at = Set(recently)).await;
    let absolute_long_ago = app.signed_in("absolute-old@example.com").await;
    update_session(&app, &absolute_long_ago, |s| {
        s.idle_expires_at = Set(now + Duration::days(7));
        s.absolute_expires_at = Set(long_ago);
    })
    .await;

    let deleted = service::purge_ended_sessions(app.db(), now).await.unwrap();

    assert_eq!(deleted, 3);
    for gone in [&revoked_long_ago, &idle_long_ago, &absolute_long_ago] {
        assert!(!session_exists(&app, gone).await);
    }
    for kept in [&active, &revoked_recently, &idle_recently] {
        assert!(session_exists(&app, kept).await);
    }
    assert_eq!(app.me(&active).await.status, StatusCode::OK);
    app.me(&revoked_recently)
        .await
        .assert_problem(StatusCode::UNAUTHORIZED, "unauthenticated");
    assert_eq!(
        service::purge_ended_sessions(app.db(), now).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn purge_keeps_a_session_that_ended_exactly_at_the_cutoff() {
    let app = TestApp::new().await;
    let now = datetime!(2026-06-01 12:00:00.5 UTC);
    let cutoff = now - PURGE_GRACE;
    let at_cutoff = app.signed_in("at-cutoff@example.com").await;
    update_session(&app, &at_cutoff, |s| s.idle_expires_at = Set(cutoff)).await;
    let before_cutoff = app.signed_in("before-cutoff@example.com").await;
    update_session(&app, &before_cutoff, |s| {
        s.idle_expires_at = Set(cutoff - Duration::seconds(1));
    })
    .await;

    let deleted = service::purge_ended_sessions(app.db(), now).await.unwrap();

    assert_eq!(deleted, 1);
    assert!(session_exists(&app, &at_cutoff).await);
    assert!(!session_exists(&app, &before_cutoff).await);
}
