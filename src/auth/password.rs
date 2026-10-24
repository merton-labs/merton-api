//! Password policy and Argon2id hashing.
//!
//! Each Argon2 operation uses ~19 MiB and tens of milliseconds of CPU, so
//! they only run through a [`HashPermit`] from a shared [`HashLimiter`]. The
//! permit moves into the blocking task and is released when the computation
//! finishes, even if the request that started it was cancelled.

use std::sync::Arc;

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinError;

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

/// Bounds the number of concurrent Argon2 operations.
#[derive(Clone)]
pub struct HashLimiter {
    permits: Arc<Semaphore>,
}

/// All hashing capacity is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("password hashing capacity is exhausted")]
pub struct HashCapacityExhausted;

/// The right to run one Argon2 operation. Capacity is returned when it drops.
pub struct HashPermit {
    _held: OwnedSemaphorePermit,
}

impl HashLimiter {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    /// Takes a permit without waiting, so overload never builds a queue.
    pub fn try_reserve(&self) -> Result<HashPermit, HashCapacityExhausted> {
        Arc::clone(&self.permits)
            .try_acquire_owned()
            .map(|permit| HashPermit { _held: permit })
            .map_err(|_| HashCapacityExhausted)
    }

    pub fn available(&self) -> usize {
        self.permits.available_permits()
    }
}

impl HashPermit {
    /// Hashes a new password on a blocking thread.
    pub async fn hash(self, password: String) -> Result<Result<String, HashError>, JoinError> {
        self.run(move || hash(&password)).await
    }

    /// Verifies a password (against a dummy hash when there is no account)
    /// on a blocking thread.
    pub async fn verify(
        self,
        password: String,
        stored_hash: Option<String>,
    ) -> Result<bool, JoinError> {
        self.run(move || verify(&password, stored_hash.as_deref()))
            .await
    }

    /// Runs `f` on the blocking pool. The permit is owned by the closure, so
    /// dropping the returned future does not release it early.
    async fn run<T: Send + 'static>(
        self,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, JoinError> {
        tokio::task::spawn_blocking(move || {
            let _permit = self;
            f()
        })
        .await
    }
}

/// Hashes a password with Argon2id and a fresh random salt, returning a PHC string.
fn hash(password: &str) -> Result<String, HashError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(HashError)
}

/// Verifies `password` against a stored PHC hash, or against a dummy hash
/// when there is no account so both paths take comparable time.
fn verify(password: &str, stored_hash: Option<&str>) -> bool {
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;
    use tokio::sync::mpsc as tokio_mpsc;

    /// Waits until every permit is back, failing instead of hanging.
    async fn wait_until_idle(limiter: &HashLimiter, total: u32) {
        let all = tokio::time::timeout(
            Duration::from_secs(5),
            Arc::clone(&limiter.permits).acquire_many_owned(total),
        )
        .await;
        assert!(all.is_ok(), "permits were not released");
    }

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

    #[test]
    fn limiter_hands_out_at_most_n_permits() {
        let limiter = HashLimiter::new(2);
        let first = limiter.try_reserve().unwrap();
        let _second = limiter.try_reserve().unwrap();

        assert_eq!(limiter.try_reserve().err(), Some(HashCapacityExhausted));
        assert_eq!(limiter.available(), 0);

        drop(first);
        assert!(limiter.try_reserve().is_ok());
    }

    #[tokio::test]
    async fn concurrent_operations_never_exceed_limit() {
        let limiter = HashLimiter::new(2);
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (entered_tx, mut entered_rx) = tokio_mpsc::unbounded_channel();
        let mut releases = Vec::new();
        let mut tasks = Vec::new();

        for _ in 0..2 {
            let permit = limiter.try_reserve().unwrap();
            let (release_tx, release_rx) = mpsc::channel::<()>();
            releases.push(release_tx);
            let (running, peak, entered_tx) =
                (Arc::clone(&running), Arc::clone(&peak), entered_tx.clone());
            tasks.push(tokio::spawn(permit.run(move || {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                running.fetch_sub(1, Ordering::SeqCst);
            })));
        }
        entered_rx.recv().await.unwrap();
        entered_rx.recv().await.unwrap();

        // Both permits are busy: further work is refused, not queued.
        assert!(limiter.try_reserve().is_err());

        for release in releases {
            release.send(()).unwrap();
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(limiter.available(), 2);
    }

    #[tokio::test]
    async fn permit_is_released_after_success_and_failure() {
        let limiter = HashLimiter::new(1);

        let ok = limiter.try_reserve().unwrap().run(|| 42).await;
        assert_eq!(ok.unwrap(), 42);
        assert_eq!(limiter.available(), 1);

        let panicked = limiter
            .try_reserve()
            .unwrap()
            .run(|| panic!("simulated hashing failure"))
            .await;
        assert!(panicked.unwrap_err().is_panic());
        assert_eq!(limiter.available(), 1);

        let wrong = limiter
            .try_reserve()
            .unwrap()
            .verify("wrong password!!".to_owned(), None)
            .await;
        assert!(!wrong.unwrap());
        assert_eq!(limiter.available(), 1);
    }

    #[tokio::test]
    async fn cancelled_request_keeps_permit_until_computation_ends() {
        let limiter = HashLimiter::new(1);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();

        let permit = limiter.try_reserve().unwrap();
        let request = tokio::spawn(permit.run(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));
        entered_rx.await.unwrap();

        // The client disconnects: the request future is dropped mid-hash.
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());

        // The blocking computation is still running, so capacity stays taken.
        assert_eq!(limiter.available(), 0);
        assert!(limiter.try_reserve().is_err());

        release_tx.send(()).unwrap();
        wait_until_idle(&limiter, 1).await;
    }

    #[tokio::test]
    async fn dropping_an_unstarted_operation_releases_its_permit() {
        let limiter = HashLimiter::new(1);
        let operation = limiter
            .try_reserve()
            .unwrap()
            .verify("never runs at all".to_owned(), None);
        assert_eq!(limiter.available(), 0);

        drop(operation);

        assert_eq!(limiter.available(), 1);
    }
}
