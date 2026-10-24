//! Process-local, bounded login rate limiting.
//!
//! Limits are kept in memory, so they reset on restart and are not shared
//! between instances. This fits a single-instance self-hosted deployment.
//!
//! Every limit is enforced by [`FixedWindowLimiter::acquire`], which checks
//! and consumes quota under one lock, so concurrent requests can never be
//! accepted beyond the limit.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(15 * 60);
/// Authentication attempts (login and registration) per client network per window.
const MAX_ATTEMPTS_PER_IP: u32 = 20;
/// Unsuccessful login attempts per account per window.
const MAX_FAILURES_PER_EMAIL: u32 = 5;
/// Upper bound on tracked keys per limiter.
const MAX_TRACKED_KEYS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryAfter(pub Duration);

pub struct LoginRateLimiter {
    per_ip: FixedWindowLimiter<IpAddr>,
    per_email: FixedWindowLimiter<String>,
}

impl Default for LoginRateLimiter {
    fn default() -> Self {
        Self {
            per_ip: FixedWindowLimiter::new(MAX_ATTEMPTS_PER_IP, WINDOW, MAX_TRACKED_KEYS),
            per_email: FixedWindowLimiter::new(MAX_FAILURES_PER_EMAIL, WINDOW, MAX_TRACKED_KEYS),
        }
    }
}

impl LoginRateLimiter {
    /// Consumes one attempt from the client's budget. Called first for every
    /// login and registration, so rejected and locked-out attempts still count.
    pub fn acquire_client_attempt(&self, ip: IpAddr) -> Result<(), RetryAfter> {
        self.per_ip.acquire(network_key(ip), Instant::now())
    }

    /// Reserves one of the account's failure slots before the password is
    /// checked. The slot stays consumed unless the login succeeds, so
    /// concurrent in-flight guesses count against the limit immediately.
    pub fn acquire_account_attempt(&self, normalized_email: &str) -> Result<(), RetryAfter> {
        self.per_email
            .acquire(normalized_email.to_owned(), Instant::now())
    }

    /// Clears the account's failure count after a successful login.
    pub fn record_success(&self, normalized_email: &str) {
        self.per_email.reset(normalized_email);
    }
}

/// Groups IPv6 clients by /64, the smallest block typically assigned to one
/// subscriber, so a single client cannot rotate addresses to evade limits.
fn network_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => {
            let mut segments = v6.segments();
            segments[4..].fill(0);
            IpAddr::from(segments)
        }
    }
}

struct Window {
    started_at: Instant,
    count: u32,
}

struct FixedWindowLimiter<K> {
    max_events: u32,
    window: Duration,
    max_keys: usize,
    windows: Mutex<HashMap<K, Window>>,
}

impl<K: Hash + Eq + Clone> FixedWindowLimiter<K> {
    fn new(max_events: u32, window: Duration, max_keys: usize) -> Self {
        Self {
            max_events,
            window,
            max_keys: max_keys.max(1),
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Atomically checks the key's quota and, if available, consumes one unit.
    fn acquire(&self, key: K, now: Instant) -> Result<(), RetryAfter> {
        let mut windows = self.lock();
        if let Some(entry) = windows.get_mut(&key) {
            let elapsed = now.saturating_duration_since(entry.started_at);
            if elapsed >= self.window {
                *entry = Window {
                    started_at: now,
                    count: 1,
                };
            } else if entry.count >= self.max_events {
                return Err(RetryAfter(self.window - elapsed));
            } else {
                entry.count += 1;
            }
            return Ok(());
        }

        if windows.len() >= self.max_keys {
            self.evict(&mut windows, now);
        }
        windows.insert(
            key,
            Window {
                started_at: now,
                count: 1,
            },
        );
        Ok(())
    }

    fn reset<Q>(&self, key: &Q)
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.lock().remove(key);
    }

    /// Makes room for one new key: drops expired windows, or else the entry
    /// with the lowest count (oldest first). Exhausted entries go last, so
    /// flooding the table with fresh keys cannot lift an active lockout.
    fn evict(&self, windows: &mut HashMap<K, Window>, now: Instant) {
        windows.retain(|_, w| now.saturating_duration_since(w.started_at) < self.window);
        if windows.len() >= self.max_keys {
            let victim = windows
                .iter()
                .min_by_key(|(_, w)| (w.count, w.started_at))
                .map(|(k, _)| k.clone());
            if let Some(victim) = victim {
                windows.remove(&victim);
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<K, Window>> {
        // A poisoned lock only means another thread panicked mid-update; the
        // counters are still usable.
        self.windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn limiter(max: u32, max_keys: usize) -> FixedWindowLimiter<u32> {
        FixedWindowLimiter::new(max, Duration::from_secs(60), max_keys)
    }

    #[test]
    fn accepts_exactly_n_then_rejects() {
        let limiter = limiter(3, 10);
        let start = Instant::now();
        for _ in 0..3 {
            assert_eq!(limiter.acquire(1, start), Ok(()));
        }

        assert!(limiter.acquire(1, start).is_err());
        assert_eq!(limiter.lock()[&1].count, 3, "rejections consume nothing");
        assert_eq!(limiter.acquire(2, start), Ok(()), "other keys unaffected");
    }

    #[test]
    fn retry_after_is_time_left_in_window() {
        let limiter = limiter(1, 10);
        let start = Instant::now();
        limiter.acquire(1, start).unwrap();

        assert_eq!(
            limiter.acquire(1, start + Duration::from_secs(10)),
            Err(RetryAfter(Duration::from_secs(50)))
        );
        assert_eq!(
            limiter.acquire(1, start + Duration::from_millis(59_999)),
            Err(RetryAfter(Duration::from_millis(1)))
        );
    }

    #[test]
    fn window_resets_at_expiry() {
        let limiter = limiter(2, 10);
        let start = Instant::now();
        limiter.acquire(1, start).unwrap();
        limiter.acquire(1, start).unwrap();
        assert!(limiter.acquire(1, start + Duration::from_secs(59)).is_err());

        let next_window = start + Duration::from_secs(60);
        assert_eq!(limiter.acquire(1, next_window), Ok(()));
        assert_eq!(limiter.acquire(1, next_window), Ok(()));
        assert!(limiter.acquire(1, next_window).is_err());
    }

    #[test]
    fn reset_clears_key() {
        let limiter = limiter(1, 10);
        let now = Instant::now();
        limiter.acquire(1, now).unwrap();
        limiter.reset(&1);

        assert_eq!(limiter.acquire(1, now), Ok(()));
    }

    #[test]
    fn tracked_keys_are_bounded() {
        let limiter = limiter(5, 3);
        let start = Instant::now();
        for key in 0..100 {
            limiter
                .acquire(key, start + Duration::from_millis(u64::from(key)))
                .unwrap();
            assert!(limiter.lock().len() <= 3);
        }

        assert!(limiter.lock().contains_key(&99));
    }

    #[test]
    fn eviction_keeps_exhausted_keys_over_fresh_ones() {
        let limiter = limiter(2, 3);
        let start = Instant::now();
        limiter.acquire(0, start).unwrap();
        limiter.acquire(0, start).unwrap();
        assert!(limiter.acquire(0, start).is_err());

        for key in 1..50 {
            limiter.acquire(key, start).unwrap();
        }

        assert!(limiter.acquire(0, start).is_err(), "lockout survives flood");
    }

    #[test]
    fn eviction_prefers_expired_windows() {
        let limiter = limiter(2, 2);
        let start = Instant::now();
        limiter.acquire(1, start).unwrap();
        limiter.acquire(2, start + Duration::from_secs(30)).unwrap();
        limiter.acquire(2, start + Duration::from_secs(30)).unwrap();

        limiter.acquire(3, start + Duration::from_secs(61)).unwrap();

        let windows = limiter.lock();
        assert!(!windows.contains_key(&1));
        assert!(windows.contains_key(&2) && windows.contains_key(&3));
    }

    /// Releases all threads at once so they contend on the same lock.
    fn contend<T: Send>(threads: usize, attempt: impl Fn(usize) -> T + Sync) -> Vec<T> {
        let barrier = Barrier::new(threads);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|i| {
                    let barrier = &barrier;
                    let attempt = &attempt;
                    scope.spawn(move || {
                        barrier.wait();
                        attempt(i)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("thread panicked"))
                .collect()
        })
    }

    #[test]
    fn concurrent_attempts_from_one_ip_never_exceed_limit() {
        let limiter = LoginRateLimiter::default();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));

        let results = contend(64, |_| limiter.acquire_client_attempt(ip));

        let accepted = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(accepted, MAX_ATTEMPTS_PER_IP as usize);
        assert_eq!(limiter.per_ip.lock()[&ip].count, MAX_ATTEMPTS_PER_IP);
    }

    #[test]
    fn concurrent_attempts_from_different_ips_are_independent() {
        let limiter = LoginRateLimiter::default();

        let results = contend(64, |i| {
            let ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, (i % 4) as u8));
            limiter.acquire_client_attempt(ip)
        });

        // 4 IPs x 16 attempts each, all under the per-IP limit.
        assert!(results.iter().all(Result::is_ok));
        assert_eq!(limiter.per_ip.lock().len(), 4);
    }

    #[test]
    fn concurrent_guesses_for_one_account_never_exceed_limit() {
        let limiter = LoginRateLimiter::default();
        let accepted = AtomicUsize::new(0);

        contend(32, |_| {
            if limiter
                .acquire_account_attempt("victim@example.com")
                .is_ok()
            {
                accepted.fetch_add(1, Ordering::SeqCst);
            }
        });

        assert_eq!(
            accepted.load(Ordering::SeqCst),
            MAX_FAILURES_PER_EMAIL as usize
        );
    }

    #[test]
    fn concurrent_new_keys_respect_key_bound() {
        let limiter = FixedWindowLimiter::new(5, WINDOW, 10);
        let now = Instant::now();

        contend(64, |i| limiter.acquire(i, now));

        assert_eq!(limiter.lock().len(), 10);
    }

    #[test]
    fn account_slots_are_restored_by_success_only() {
        let limiter = LoginRateLimiter::default();
        for _ in 0..MAX_FAILURES_PER_EMAIL {
            limiter.acquire_account_attempt("a@example.com").unwrap();
        }
        assert!(limiter.acquire_account_attempt("a@example.com").is_err());
        assert!(limiter.acquire_account_attempt("b@example.com").is_ok());

        limiter.record_success("a@example.com");
        assert!(limiter.acquire_account_attempt("a@example.com").is_ok());
    }

    #[test]
    fn noisy_client_is_blocked_across_emails() {
        let limiter = LoginRateLimiter::default();
        let noisy = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        for _ in 0..MAX_ATTEMPTS_PER_IP {
            limiter.acquire_client_attempt(noisy).unwrap();
        }

        assert!(limiter.acquire_client_attempt(noisy).is_err());
        assert!(
            limiter
                .acquire_client_attempt(IpAddr::V4(Ipv4Addr::LOCALHOST))
                .is_ok()
        );
    }

    #[test]
    fn ipv6_clients_share_a_slash_64() {
        let a: Ipv6Addr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: Ipv6Addr = "2001:db8:1:2:bbbb::2".parse().unwrap();
        let c: Ipv6Addr = "2001:db8:1:3::1".parse().unwrap();

        assert_eq!(network_key(a.into()), network_key(b.into()));
        assert_ne!(network_key(a.into()), network_key(c.into()));

        let limiter = LoginRateLimiter::default();
        for _ in 0..MAX_ATTEMPTS_PER_IP {
            limiter.acquire_client_attempt(a.into()).unwrap();
        }
        assert!(limiter.acquire_client_attempt(b.into()).is_err());
        assert!(limiter.acquire_client_attempt(c.into()).is_ok());
    }
}
