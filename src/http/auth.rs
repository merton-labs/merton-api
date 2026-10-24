//! Authentication endpoints.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use utoipa::ToSchema;
use uuid::Uuid;

use super::error::{ApiError, ProblemDetails};
use super::extract::{
    ApiJson, ClientIp, CsrfProtectedSession, CurrentSession, TrustedOrigin, session_cookie_name,
};
use crate::app::{AppState, Security};
use crate::auth::{CurrentUser, service};

/// Email and password. Not `Debug`, so the password cannot be logged by accident.
#[derive(Deserialize, ToSchema)]
pub struct CredentialsRequest {
    #[schema(example = "user@example.com")]
    pub email: String,
    #[schema(example = "ExampleSecurePassword123!", format = Password, min_length = 12, max_length = 128)]
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserResponse {
    pub id: Uuid,
    #[schema(example = "user@example.com")]
    pub email: String,
}

impl From<CurrentUser> for UserResponse {
    fn from(user: CurrentUser) -> Self {
        Self {
            id: user.id.as_uuid(),
            email: user.email,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CsrfTokenResponse {
    /// Send this value in the `X-CSRF-Token` header on authenticated mutations.
    pub csrf_token: String,
}

/// Create an account. Does not log the new user in.
#[utoipa::path(
    post,
    path = "/api/v1/auth/register",
    tag = "auth",
    request_body = CredentialsRequest,
    responses(
        (status = CREATED, description = "Account created", body = UserResponse),
        (status = BAD_REQUEST, description = "Malformed request body", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Untrusted request origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = CONFLICT, description = "Email already registered", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNPROCESSABLE_ENTITY, description = "Invalid email or password policy violation", body = ProblemDetails, content_type = "application/problem+json"),
        (status = TOO_MANY_REQUESTS, description = "Too many attempts from this client", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn register(
    State(state): State<AppState>,
    _origin: TrustedOrigin,
    ClientIp(client_ip): ClientIp,
    ApiJson(body): ApiJson<CredentialsRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    let user = service::register(
        &state.db,
        &state.security.login_limiter,
        client_ip,
        &body.email,
        body.password,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(user.into())))
}

/// Log in and receive a session cookie.
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "auth",
    request_body = CredentialsRequest,
    responses(
        (status = OK, description = "Logged in; sets the HttpOnly session cookie", body = UserResponse,
            headers(("Set-Cookie" = String, description = "Session cookie (`__Host-merton_session`)"))),
        (status = BAD_REQUEST, description = "Malformed request body", body = ProblemDetails, content_type = "application/problem+json"),
        (status = UNAUTHORIZED, description = "Invalid email or password", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Untrusted request origin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = TOO_MANY_REQUESTS, description = "Too many attempts; see Retry-After", body = ProblemDetails, content_type = "application/problem+json",
            headers(("Retry-After" = u64, description = "Seconds until the next attempt is allowed"))),
    ),
)]
pub async fn login(
    State(state): State<AppState>,
    _origin: TrustedOrigin,
    ClientIp(client_ip): ClientIp,
    jar: CookieJar,
    ApiJson(body): ApiJson<CredentialsRequest>,
) -> Result<(CookieJar, Json<UserResponse>), ApiError> {
    let cookie_name = session_cookie_name(&state.security);
    let previous_token = jar.get(cookie_name).map(|c| c.value().to_owned());
    let outcome = service::login(
        &state.db,
        &state.security.login_limiter,
        client_ip,
        &body.email,
        body.password,
        previous_token.as_deref(),
    )
    .await?;

    let max_age = outcome.lifetime.absolute_expires_at - OffsetDateTime::now_utc();
    let cookie = session_cookie(&state.security, outcome.token.as_str().to_owned(), max_age);
    Ok((jar.add(cookie), Json(outcome.user.into())))
}

/// Revoke the current session and clear its cookie.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    security(("session_cookie" = [])),
    params(("X-CSRF-Token" = String, Header, description = "Token from `GET /api/v1/auth/csrf`")),
    responses(
        (status = NO_CONTENT, description = "Session revoked and cookie cleared"),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
        (status = FORBIDDEN, description = "Missing or invalid CSRF token, or untrusted origin", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn logout(
    State(state): State<AppState>,
    CsrfProtectedSession(session): CsrfProtectedSession,
    jar: CookieJar,
) -> Result<(StatusCode, CookieJar), ApiError> {
    service::logout(&state.db, session.session_id).await?;
    let removal = session_cookie(&state.security, String::new(), Duration::ZERO);
    Ok((StatusCode::NO_CONTENT, jar.remove(removal)))
}

/// The authenticated user.
#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    tag = "auth",
    security(("session_cookie" = [])),
    responses(
        (status = OK, description = "Current user", body = UserResponse),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn me(CurrentSession(session): CurrentSession) -> Json<UserResponse> {
    Json(session.user.into())
}

/// A CSRF token bound to the current session.
#[utoipa::path(
    get,
    path = "/api/v1/auth/csrf",
    tag = "auth",
    security(("session_cookie" = [])),
    responses(
        (status = OK, description = "CSRF token for the current session", body = CsrfTokenResponse),
        (status = UNAUTHORIZED, description = "No valid session", body = ProblemDetails, content_type = "application/problem+json"),
    ),
)]
pub async fn csrf(
    State(state): State<AppState>,
    CurrentSession(session): CurrentSession,
) -> Json<CsrfTokenResponse> {
    Json(CsrfTokenResponse {
        csrf_token: state.security.csrf.token_for(session.session_id),
    })
}

fn session_cookie(security: &Security, value: String, max_age: Duration) -> Cookie<'static> {
    Cookie::build((session_cookie_name(security), value))
        .http_only(true)
        .secure(security.cookie_secure)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(max_age)
        .build()
}
