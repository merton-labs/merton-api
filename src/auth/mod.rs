//! Local email/password authentication with server-managed sessions.
//!
//! This module is independent of HTTP; `crate::http` adapts it to Axum.

pub mod csrf;
pub mod email;
pub mod password;
pub mod rate_limit;
mod repository;
pub mod service;
pub mod session;

use sea_orm::DbErr;
use time::OffsetDateTime;
use uuid::Uuid;

/// Identifier of an authenticated user.
///
/// Only constructed from verified server-side session data, so code that
/// receives a `UserId` can use it to scope queries to the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserId(Uuid);

impl UserId {
    pub(crate) fn from_verified(id: Uuid) -> Self {
        Self(id)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentUser {
    pub id: UserId,
    pub email: String,
}

/// A valid, unexpired, unrevoked session and its owner.
#[derive(Debug, Clone)]
pub struct AuthenticatedSession {
    pub session_id: Uuid,
    pub user: CurrentUser,
    pub absolute_expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub field: &'static str,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    #[error("email is already registered")]
    EmailTaken,
    #[error("invalid email or password")]
    InvalidCredentials,
    #[error("too many login attempts")]
    RateLimited(rate_limit::RetryAfter),
    #[error("no valid session")]
    Unauthenticated,
    #[error(transparent)]
    Internal(#[from] InternalError),
}

#[derive(Debug, thiserror::Error)]
pub enum InternalError {
    #[error("database error: {0}")]
    Database(#[from] DbErr),
    #[error(transparent)]
    PasswordHash(#[from] password::HashError),
    #[error(transparent)]
    Token(#[from] session::TokenGenerationError),
    #[error("blocking task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

impl From<DbErr> for AuthError {
    fn from(err: DbErr) -> Self {
        Self::Internal(err.into())
    }
}
