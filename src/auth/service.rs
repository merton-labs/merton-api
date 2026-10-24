//! Authentication use cases: register, log in, authenticate, log out.

use std::net::IpAddr;

use sea_orm::{DatabaseConnection, SqlErr, TransactionTrait};
use time::OffsetDateTime;
use uuid::Uuid;

use super::email::Email;
use super::password;
use super::rate_limit::LoginRateLimiter;
use super::repository::{self, NewSession, NewUser};
use super::session::{self, Lifetime, SessionToken};
use super::{AuthError, AuthenticatedSession, CurrentUser, FieldError, InternalError, UserId};
use crate::infrastructure::entities::user_session;

pub struct LoginOutcome {
    pub user: CurrentUser,
    pub token: SessionToken,
    pub lifetime: Lifetime,
}

/// Creates an account. Registration shares the per-client attempt budget
/// with login because each attempt costs an Argon2 hash.
pub async fn register(
    db: &DatabaseConnection,
    limiter: &LoginRateLimiter,
    client_ip: IpAddr,
    email: &str,
    password: String,
) -> Result<CurrentUser, AuthError> {
    limiter
        .check_attempt(client_ip, None)
        .map_err(AuthError::RateLimited)?;
    let email = validate_registration(email, &password)?;
    let password_hash = run_blocking(move || password::hash(&password)).await??;

    let id = Uuid::now_v7();
    let new_user = NewUser {
        id,
        email: email.display(),
        normalized_email: email.normalized(),
        password_hash: &password_hash,
        now: OffsetDateTime::now_utc(),
    };
    match repository::insert_user(db, new_user).await {
        Ok(()) => Ok(CurrentUser {
            id: UserId::from_verified(id),
            email: email.display().to_owned(),
        }),
        Err(err) if matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
            Err(AuthError::EmailTaken)
        }
        Err(err) => Err(err.into()),
    }
}

fn validate_registration(email: &str, password: &str) -> Result<Email, AuthError> {
    let mut errors = Vec::new();
    let email = Email::parse(email)
        .map_err(|err| {
            errors.push(FieldError {
                field: "email",
                message: err.to_string(),
            })
        })
        .ok();
    if let Err(err) = password::validate(password) {
        errors.push(FieldError {
            field: "password",
            message: err.to_string(),
        });
    }
    match email {
        Some(email) if errors.is_empty() => Ok(email),
        _ => Err(AuthError::Validation(errors)),
    }
}

/// Verifies credentials and opens a new session, revoking `previous_token`
/// if the client presented one.
pub async fn login(
    db: &DatabaseConnection,
    limiter: &LoginRateLimiter,
    client_ip: IpAddr,
    email: &str,
    password: String,
    previous_token: Option<&str>,
) -> Result<LoginOutcome, AuthError> {
    let email = Email::parse(email).ok();
    let normalized_email = email.as_ref().map(Email::normalized);
    limiter
        .check_attempt(client_ip, normalized_email)
        .map_err(AuthError::RateLimited)?;

    let user = match normalized_email {
        Some(normalized) => repository::find_user_by_normalized_email(db, normalized).await?,
        None => None,
    };

    // Unknown accounts are verified against a dummy hash so that both cases
    // take about the same time. Over-long input is rejected before hashing.
    let verified = if password.chars().count() <= password::MAX_PASSWORD_CHARS {
        let stored_hash = user.as_ref().map(|u| u.password_hash.clone());
        run_blocking(move || password::verify(&password, stored_hash.as_deref())).await?
    } else {
        false
    };

    let user = match user {
        Some(user) if verified => user,
        _ => {
            if let Some(normalized) = normalized_email {
                limiter.record_failure(normalized);
            }
            return Err(AuthError::InvalidCredentials);
        }
    };
    if let Some(normalized) = normalized_email {
        limiter.record_success(normalized);
    }

    let token = SessionToken::generate().map_err(InternalError::from)?;
    let now = OffsetDateTime::now_utc();
    let lifetime = Lifetime::starting_at(now);

    let txn = db.begin().await?;
    if let Some(previous_hash) = previous_token.and_then(session::hash_presented_token) {
        repository::revoke_session(&txn, user_session::Column::TokenHash, previous_hash, now)
            .await?;
    }
    repository::insert_session(
        &txn,
        NewSession {
            id: Uuid::now_v7(),
            user_id: user.id,
            token_hash: token.hash(),
            now,
            idle_expires_at: lifetime.idle_expires_at,
            absolute_expires_at: lifetime.absolute_expires_at,
        },
    )
    .await?;
    txn.commit().await?;

    Ok(LoginOutcome {
        user: CurrentUser {
            id: UserId::from_verified(user.id),
            email: user.email,
        },
        token,
        lifetime,
    })
}

/// Resolves a presented session token to its session and user.
///
/// Expired and revoked sessions are rejected. Idle expiry is extended at
/// most once per [`session::ACTIVITY_UPDATE_INTERVAL`].
pub async fn authenticate(
    db: &DatabaseConnection,
    presented_token: &str,
) -> Result<AuthenticatedSession, AuthError> {
    let token_hash =
        session::hash_presented_token(presented_token).ok_or(AuthError::Unauthenticated)?;
    let (session, user) = repository::find_session_with_user(db, token_hash)
        .await?
        .ok_or(AuthError::Unauthenticated)?;

    let now = OffsetDateTime::now_utc();
    let lifetime = Lifetime {
        idle_expires_at: session.idle_expires_at,
        absolute_expires_at: session.absolute_expires_at,
    };
    if session.revoked_at.is_some() || !lifetime.is_active_at(now) {
        return Err(AuthError::Unauthenticated);
    }

    if session::activity_update_due(session.last_activity_at, now) {
        let idle_expires_at = lifetime.refreshed_idle_expiry(now);
        // A failed activity write only shortens the idle window; the session
        // itself is still valid, so the request proceeds.
        if let Err(err) =
            repository::record_session_activity(db, session.id, now, idle_expires_at).await
        {
            tracing::warn!(error = %err, "failed to record session activity");
        }
    }

    Ok(AuthenticatedSession {
        session_id: session.id,
        user: CurrentUser {
            id: UserId::from_verified(user.id),
            email: user.email,
        },
        absolute_expires_at: session.absolute_expires_at,
    })
}

pub async fn logout(db: &DatabaseConnection, session_id: Uuid) -> Result<(), AuthError> {
    repository::revoke_session(
        db,
        user_session::Column::Id,
        session_id,
        OffsetDateTime::now_utc(),
    )
    .await?;
    Ok(())
}

async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, AuthError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|err| AuthError::Internal(err.into()))
}

impl From<password::HashError> for AuthError {
    fn from(err: password::HashError) -> Self {
        Self::Internal(err.into())
    }
}
