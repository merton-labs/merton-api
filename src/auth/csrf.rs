//! Stateless, session-bound CSRF tokens.
//!
//! A token is `HMAC-SHA256(secret, label || session_id)`. It cannot be forged
//! without the server secret and is only valid for the session it was issued to.

use base64ct::{Base64UrlUnpadded, Encoding};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use uuid::Uuid;

const LABEL: &[u8] = b"merton/csrf/v1:";

#[derive(Clone)]
pub struct CsrfKey(Hmac<Sha256>);

#[derive(Debug, thiserror::Error)]
#[error("invalid CSRF key length")]
pub struct InvalidKey;

impl CsrfKey {
    pub fn new(secret: &[u8]) -> Result<Self, InvalidKey> {
        Hmac::<Sha256>::new_from_slice(secret)
            .map(Self)
            .map_err(|_| InvalidKey)
    }

    pub fn token_for(&self, session_id: Uuid) -> String {
        Base64UrlUnpadded::encode_string(&self.mac(session_id).finalize().into_bytes())
    }

    /// Constant-time check that `token` was issued for `session_id`.
    pub fn verify(&self, session_id: Uuid, token: &str) -> bool {
        Base64UrlUnpadded::decode_vec(token)
            .is_ok_and(|tag| self.mac(session_id).verify_slice(&tag).is_ok())
    }

    fn mac(&self, session_id: Uuid) -> Hmac<Sha256> {
        let mut mac = self.0.clone();
        mac.update(LABEL);
        mac.update(session_id.as_bytes());
        mac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_verifies_only_for_its_session() {
        let key = CsrfKey::new(b"0123456789abcdef0123456789abcdef").unwrap();
        let session = Uuid::now_v7();
        let other = Uuid::now_v7();
        let token = key.token_for(session);

        assert!(key.verify(session, &token));
        assert!(!key.verify(other, &token));
    }

    #[test]
    fn token_depends_on_secret() {
        let session = Uuid::now_v7();
        let token = CsrfKey::new(b"first secret first secret first!")
            .unwrap()
            .token_for(session);

        assert!(
            !CsrfKey::new(b"other secret other secret other!")
                .unwrap()
                .verify(session, &token)
        );
    }

    #[test]
    fn rejects_malformed_tokens() {
        let key = CsrfKey::new(b"0123456789abcdef0123456789abcdef").unwrap();
        let session = Uuid::now_v7();
        let token = key.token_for(session);

        assert!(!key.verify(session, ""));
        assert!(!key.verify(session, "not base64!"));
        assert!(!key.verify(session, &token[..token.len() - 2]));
    }
}
