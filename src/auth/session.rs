//! Session tokens and lifetime policy.
//!
//! The browser holds a random 256-bit token; the database stores only its
//! SHA-256 digest. Idle expiry slides forward with activity but never past
//! the absolute expiry.

use base64ct::{Base64UrlUnpadded, Encoding};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

pub const IDLE_TIMEOUT: Duration = Duration::days(7);
pub const ABSOLUTE_TIMEOUT: Duration = Duration::days(30);
/// Minimum time between activity writes for one session.
pub const ACTIVITY_UPDATE_INTERVAL: Duration = Duration::minutes(15);

const TOKEN_BYTES: usize = 32;

/// A session token as handed to the client. Not `Debug`, to keep it out of logs.
pub struct SessionToken(String);

#[derive(Debug, thiserror::Error)]
#[error("failed to generate session token: {0}")]
pub struct TokenGenerationError(getrandom::Error);

impl SessionToken {
    pub fn generate() -> Result<Self, TokenGenerationError> {
        let mut bytes = [0u8; TOKEN_BYTES];
        getrandom::fill(&mut bytes).map_err(TokenGenerationError)?;
        Ok(Self(Base64UrlUnpadded::encode_string(&bytes)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn hash(&self) -> Vec<u8> {
        Sha256::digest(self.0.as_bytes()).to_vec()
    }
}

/// Hashes a client-presented token, rejecting values that cannot have been
/// issued by [`SessionToken::generate`].
pub fn hash_presented_token(token: &str) -> Option<Vec<u8>> {
    let mut decoded = [0u8; TOKEN_BYTES];
    match Base64UrlUnpadded::decode(token, &mut decoded) {
        Ok(bytes) if bytes.len() == TOKEN_BYTES => Some(Sha256::digest(token.as_bytes()).to_vec()),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lifetime {
    pub idle_expires_at: OffsetDateTime,
    pub absolute_expires_at: OffsetDateTime,
}

impl Lifetime {
    pub fn starting_at(now: OffsetDateTime) -> Self {
        Self {
            idle_expires_at: now + IDLE_TIMEOUT,
            absolute_expires_at: now + ABSOLUTE_TIMEOUT,
        }
    }

    pub fn is_active_at(&self, now: OffsetDateTime) -> bool {
        now < self.idle_expires_at && now < self.absolute_expires_at
    }

    /// Idle expiry after activity at `now`, capped at the absolute expiry.
    pub fn refreshed_idle_expiry(&self, now: OffsetDateTime) -> OffsetDateTime {
        (now + IDLE_TIMEOUT).min(self.absolute_expires_at)
    }
}

pub fn activity_update_due(last_activity_at: OffsetDateTime, now: OffsetDateTime) -> bool {
    now - last_activity_at >= ACTIVITY_UPDATE_INTERVAL
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-01-01 00:00 UTC);

    #[test]
    fn tokens_have_256_bits_and_are_unique() {
        let a = SessionToken::generate().unwrap();
        let b = SessionToken::generate().unwrap();

        assert_eq!(Base64UrlUnpadded::decode_vec(a.as_str()).unwrap().len(), 32);
        assert_ne!(a.as_str(), b.as_str());
        assert_eq!(a.hash().len(), 32);
    }

    #[test]
    fn presented_token_hash_matches_issued_hash() {
        let token = SessionToken::generate().unwrap();

        assert_eq!(hash_presented_token(token.as_str()), Some(token.hash()));
    }

    #[test]
    fn rejects_malformed_presented_tokens() {
        assert_eq!(hash_presented_token(""), None);
        assert_eq!(hash_presented_token("not a token"), None);
        assert_eq!(hash_presented_token(&"A".repeat(42)), None);
        assert_eq!(hash_presented_token(&"A".repeat(64)), None);
    }

    #[test]
    fn lifetime_expires_on_idle_or_absolute_limit() {
        let lifetime = Lifetime::starting_at(NOW);

        assert!(lifetime.is_active_at(NOW + Duration::days(6)));
        assert!(!lifetime.is_active_at(NOW + IDLE_TIMEOUT));

        let long_lived = Lifetime {
            idle_expires_at: NOW + Duration::days(40),
            ..lifetime
        };
        assert!(!long_lived.is_active_at(NOW + ABSOLUTE_TIMEOUT));
    }

    #[test]
    fn idle_refresh_never_passes_absolute_expiry() {
        let lifetime = Lifetime::starting_at(NOW);

        let early = NOW + Duration::days(1);
        assert_eq!(lifetime.refreshed_idle_expiry(early), early + IDLE_TIMEOUT);

        let late = NOW + Duration::days(28);
        assert_eq!(
            lifetime.refreshed_idle_expiry(late),
            lifetime.absolute_expires_at
        );
    }

    #[test]
    fn activity_updates_are_rate_limited() {
        assert!(!activity_update_due(NOW, NOW + Duration::minutes(14)));
        assert!(activity_update_due(NOW, NOW + ACTIVITY_UPDATE_INTERVAL));
    }
}
