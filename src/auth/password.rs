//! Password policy and Argon2id hashing.
//!
//! Hashing is CPU-bound; callers on the async runtime should run these
//! functions on a blocking thread.

use argon2::{Argon2, PasswordHasher, PasswordVerifier};

pub const MIN_PASSWORD_CHARS: usize = 12;
pub const MAX_PASSWORD_CHARS: usize = 128;

/// Argon2id hash (default parameters) of a random password that was
/// discarded. Verifying against it makes unknown-account logins cost the same
/// as real ones.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$yUazLkyiKEnvzjUwiH5LJA$+IQ6okNYGVrwlf+YC7jOiwvK0NhPmQExXYvtgoq/dcs";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidPassword {
    #[error("must be at least {MIN_PASSWORD_CHARS} characters")]
    TooShort,
    #[error("must be at most {MAX_PASSWORD_CHARS} characters")]
    TooLong,
}

#[derive(Debug, thiserror::Error)]
#[error("password hashing failed: {0}")]
pub struct HashError(argon2::password_hash::Error);

/// Checks a new password against the length policy.
pub fn validate(password: &str) -> Result<(), InvalidPassword> {
    let chars = password.chars().count();
    if chars < MIN_PASSWORD_CHARS {
        Err(InvalidPassword::TooShort)
    } else if chars > MAX_PASSWORD_CHARS {
        Err(InvalidPassword::TooLong)
    } else {
        Ok(())
    }
}

/// Hashes a password with Argon2id and a fresh random salt, returning a PHC string.
pub fn hash(password: &str) -> Result<String, HashError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(HashError)
}

/// Verifies `password` against a stored PHC hash, or against a dummy hash
/// when there is no account so both paths take comparable time.
pub fn verify(password: &str, stored_hash: Option<&str>) -> bool {
    let matches = Argon2::default()
        .verify_password(password.as_bytes(), stored_hash.unwrap_or(DUMMY_HASH))
        .is_ok();
    matches && stored_hash.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::Params;
    use argon2::password_hash::phc::PasswordHash;

    #[test]
    fn enforces_length_limits() {
        assert_eq!(validate("short"), Err(InvalidPassword::TooShort));
        assert_eq!(validate(&"x".repeat(129)), Err(InvalidPassword::TooLong));
        assert_eq!(validate("twelve chars"), Ok(()));
        // Counted in characters, not bytes.
        assert_eq!(validate(&"é".repeat(12)), Ok(()));
    }

    #[test]
    fn hashes_with_argon2id_and_unique_salt() {
        let first = hash("correct horse battery").unwrap();
        let second = hash("correct horse battery").unwrap();

        assert!(first.starts_with("$argon2id$"));
        assert_ne!(first, second);
        assert!(verify("correct horse battery", Some(&first)));
        assert!(!verify("wrong horse battery", Some(&first)));
    }

    #[test]
    fn missing_account_never_verifies() {
        assert!(!verify("anything at all", None));
    }

    #[test]
    fn dummy_hash_uses_default_cost() {
        let dummy = PasswordHash::new(DUMMY_HASH).unwrap();
        let real = PasswordHash::new(&hash("some password here").unwrap()).unwrap();

        assert_eq!(dummy.algorithm, real.algorithm);
        assert_eq!(dummy.params, real.params);

        let parsed = Params::try_from(&dummy).unwrap();
        let default = Params::default();
        assert_eq!(
            (parsed.m_cost(), parsed.t_cost(), parsed.p_cost()),
            (default.m_cost(), default.t_cost(), default.p_cost())
        );
        assert_eq!(parsed.output_len(), Some(Params::DEFAULT_OUTPUT_LEN));
    }
}
