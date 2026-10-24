//! Authentication use cases: register, log in, authenticate, log out.

use std::net::IpAddr;

use sea_orm::{DatabaseConnection, SqlErr, TransactionTrait};
use time::OffsetDateTime;
use uuid::Uuid;

use super::email::Email;
use super::password::{self, HashLimiter, HashPermit};
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

/// Creates an account. Every registration attempt, valid or not, spends the
/// per-client budget shared with login; only valid ones take a hashing permit.
pub async fn register(
    db: &DatabaseConnection,
    limiter: &LoginRateLimiter,
    hashes: &HashLimiter,
    client_ip: IpAddr,
    email: &str,
    password: String,
) -> Result<CurrentUser, AuthError> {
    limiter
        .acquire_client_attempt(client_ip)
        .map_err(AuthError::RateLimited)?;
    let email = validate_registration(email, &password)?;
    let password_hash = reserve_hash(hashes)?
        .hash(password)
        .await
        .map_err(InternalError::from)??;

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
///
/// Checks run in this order, each before the work it guards:
/// 1. the client's attempt budget (429);
/// 2. the account lookup, done before taking a permit so database latency
///    never holds scarce hashing capacity;
/// 3. a hashing permit (503), taken only if verification will run and before
///    the account budget, so overload never consumes failure slots;
/// 4. the account's failure budget (429), reserved before verification so
///    concurrent guesses cannot exceed it.
///
/// None of these depend on whether the account exists.
pub async fn login(
    db: &DatabaseConnection,
    limiter: &LoginRateLimiter,
    hashes: &HashLimiter,
    client_ip: IpAddr,
    email: &str,
    password: String,
    previous_token: Option<&str>,
) -> Result<LoginOutcome, AuthError> {
    let email = Email::parse(email).ok();
    let normalized_email = email.as_ref().map(Email::normalized);
    limiter
        .acquire_client_attempt(client_ip)
        .map_err(AuthError::RateLimited)?;

    let user = match normalized_email {
        Some(normalized) => repository::find_user_by_normalized_email(db, normalized).await?,
        None => None,
    };

    // Over-long input can never match (registration rejects it), so it fails
    // without hashing; it still counts as a failed attempt below.
    let permit = if password.chars().count() <= password::MAX_PASSWORD_CHARS {
        Some(reserve_hash(hashes)?)
    } else {
        None
    };
    if let Some(normalized) = normalized_email {
        limiter
            .acquire_account_attempt(normalized)
            .map_err(AuthError::RateLimited)?;
    }

    // Unknown accounts are verified against a dummy hash so that both cases
    // take about the same time.
    let verified = match permit {
        Some(permit) => {
            let stored_hash = user.as_ref().map(|u| u.password_hash.clone());
            permit
                .verify(password, stored_hash)
                .await
                .map_err(InternalError::from)?
        }
        None => false,
    };

    // A failed attempt keeps the account slot it reserved above.
    let user = match user {
        Some(user) if verified => user,
        _ => return Err(AuthError::InvalidCredentials),
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

fn reserve_hash(hashes: &HashLimiter) -> Result<HashPermit, AuthError> {
    hashes.try_reserve().map_err(|_| {
        tracing::warn!("password hashing capacity exhausted; rejecting request");
        AuthError::Overloaded
    })
}

impl From<password::HashError> for AuthError {
    fn from(err: password::HashError) -> Self {
        Self::Internal(err.into())
    }
}
