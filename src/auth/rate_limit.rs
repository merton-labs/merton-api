//! Process-local, bounded login rate limiting.
//!
//! Limits are kept in memory, so they reset on restart and are not shared
//! between instances. This fits a single-instance self-hosted deployment.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(15 * 60);
/// Login attempts (successful or not) per client network per window.
const MAX_ATTEMPTS_PER_IP: u32 = 20;
/// Failed logins per account per window.
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
    /// Records an attempt from `ip` and rejects it if either the client or
    /// the account (when the email is valid) is over its limit.
    pub fn check_attempt(
        &self,
        ip: IpAddr,
        normalized_email: Option<&str>,
    ) -> Result<(), RetryAfter> {
        let now = Instant::now();
        let ip = network_key(ip);
        if let Some(email) = normalized_email {
            self.per_email.check(email, now)?;
        }
        self.per_ip.check(&ip, now)?;
        self.per_ip.record(ip, now);
        Ok(())
    }

    pub fn record_failure(&self, normalized_email: &str) {
        self.per_email
            .record(normalized_email.to_owned(), Instant::now());
    }

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
            max_keys,
            windows: Mutex::new(HashMap::new()),
        }
    }

    fn check<Q>(&self, key: &Q, now: Instant) -> Result<(), RetryAfter>
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let windows = self.lock();
        match windows.get(key) {
            Some(entry) if entry.count >= self.max_events => {
                let elapsed = now.saturating_duration_since(entry.started_at);
                match self.window.checked_sub(elapsed) {
                    Some(remaining) if !remaining.is_zero() => Err(RetryAfter(remaining)),
                    _ => Ok(()),
                }
            }
            _ => Ok(()),
        }
    }

    fn record(&self, key: K, now: Instant) {
        let mut windows = self.lock();
        if !windows.contains_key(&key) && windows.len() >= self.max_keys {
            self.evict(&mut windows, now);
        }
        let entry = windows.entry(key).or_insert(Window {
            started_at: now,
            count: 0,
        });
        if now.saturating_duration_since(entry.started_at) >= self.window {
            *entry = Window {
                started_at: now,
                count: 0,
            };
        }
        entry.count = entry.count.saturating_add(1);
    }

    fn reset<Q>(&self, key: &Q)
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.lock().remove(key);
    }

    /// Drops expired windows, or the oldest one if none have expired.
    fn evict(&self, windows: &mut HashMap<K, Window>, now: Instant) {
        windows.retain(|_, w| now.saturating_duration_since(w.started_at) < self.window);
        if windows.len() >= self.max_keys {
            let oldest = windows
                .iter()
                .min_by_key(|(_, w)| w.started_at)
                .map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                windows.remove(&oldest);
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

    fn limiter(max: u32, max_keys: usize) -> FixedWindowLimiter<u32> {
        FixedWindowLimiter::new(max, Duration::from_secs(60), max_keys)
    }

    #[test]
    fn blocks_after_limit_until_window_ends() {
        let limiter = limiter(2, 10);
        let start = Instant::now();
        limiter.record(1, start);
        limiter.record(1, start);

        let blocked = limiter.check(&1, start + Duration::from_secs(10));
        assert_eq!(blocked, Err(RetryAfter(Duration::from_secs(50))));
        assert_eq!(limiter.check(&2, start), Ok(()));
        assert_eq!(limiter.check(&1, start + Duration::from_secs(60)), Ok(()));
    }

    #[test]
    fn window_restarts_after_expiry() {
        let limiter = limiter(1, 10);
        let start = Instant::now();
        limiter.record(1, start);
        limiter.record(1, start + Duration::from_secs(61));

        assert!(limiter.check(&1, start + Duration::from_secs(62)).is_err());
        assert_eq!(limiter.lock()[&1].count, 1);
    }

    #[test]
    fn reset_clears_key() {
        let limiter = limiter(1, 10);
        let now = Instant::now();
        limiter.record(1, now);
        limiter.reset(&1);

        assert_eq!(limiter.check(&1, now), Ok(()));
    }

    #[test]
    fn tracked_keys_are_bounded() {
        let limiter = limiter(5, 3);
        let start = Instant::now();
        for key in 0..100 {
            limiter.record(key, start + Duration::from_millis(u64::from(key)));
        }

        let windows = limiter.lock();
        assert_eq!(windows.len(), 3);
        assert!(windows.contains_key(&99));
    }

    #[test]
    fn login_limiter_blocks_repeated_failures_for_an_email() {
        let limiter = LoginRateLimiter::default();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        for _ in 0..MAX_FAILURES_PER_EMAIL {
            assert!(limiter.check_attempt(ip, Some("a@example.com")).is_ok());
            limiter.record_failure("a@example.com");
        }

        assert!(limiter.check_attempt(ip, Some("a@example.com")).is_err());
        assert!(limiter.check_attempt(ip, Some("b@example.com")).is_ok());
    }

    #[test]
    fn login_limiter_blocks_a_noisy_client_across_emails() {
        let limiter = LoginRateLimiter::default();
        let noisy = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        for i in 0..MAX_ATTEMPTS_PER_IP {
            assert!(
                limiter
                    .check_attempt(noisy, Some(&format!("u{i}@example.com")))
                    .is_ok()
            );
        }

        assert!(limiter.check_attempt(noisy, None).is_err());
        assert!(
            limiter
                .check_attempt(IpAddr::V4(Ipv4Addr::LOCALHOST), None)
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
    }
}
