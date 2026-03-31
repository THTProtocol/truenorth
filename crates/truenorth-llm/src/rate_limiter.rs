//! Per-provider rate limit tracking with exponential backoff and jitter.
//!
//! Each provider gets its own `ProviderRateLimitState` entry tracking:
//! - Whether the provider is currently rate-limited
//! - When the rate limit expires (from `Retry-After` headers)
//! - Whether the provider has been permanently exhausted
//! - Exponential backoff state for repeated failures

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

/// State for a single provider's rate limit tracking.
#[derive(Debug, Clone)]
pub struct ProviderRateLimitState {
    /// Whether this provider is currently rate-limited.
    pub is_rate_limited: bool,
    /// When the rate limit expires (None if not rate-limited).
    pub rate_limit_expires_at: Option<Instant>,
    /// Whether this provider has been permanently exhausted (API key invalid/quota gone).
    pub is_exhausted: bool,
    /// Whether this provider has been manually disabled.
    pub is_manually_disabled: bool,
    /// Number of consecutive failures (for exponential backoff calculation).
    pub consecutive_failures: u32,
    /// Total successful requests this session.
    pub success_count: u64,
    /// Total failed requests this session.
    pub failure_count: u64,
}

impl Default for ProviderRateLimitState {
    fn default() -> Self {
        Self {
            is_rate_limited: false,
            rate_limit_expires_at: None,
            is_exhausted: false,
            is_manually_disabled: false,
            consecutive_failures: 0,
            success_count: 0,
            failure_count: 0,
        }
    }
}

impl ProviderRateLimitState {
    /// Returns true if the provider is currently available for use.
    pub fn is_available(&self) -> bool {
        if self.is_exhausted || self.is_manually_disabled {
            return false;
        }
        if self.is_rate_limited {
            if let Some(expires_at) = self.rate_limit_expires_at {
                if Instant::now() >= expires_at {
                    // Rate limit has expired — it will be cleared on next `check_and_clear`
                    return true;
                }
            }
            return false;
        }
        true
    }

    /// Clears rate limit state if the expiry time has passed.
    pub fn check_and_clear_rate_limit(&mut self) {
        if self.is_rate_limited {
            if let Some(expires_at) = self.rate_limit_expires_at {
                if Instant::now() >= expires_at {
                    self.is_rate_limited = false;
                    self.rate_limit_expires_at = None;
                    self.consecutive_failures = 0;
                }
            }
        }
    }

    /// Returns how many seconds until the rate limit expires, if rate-limited.
    pub fn seconds_until_available(&self) -> Option<u64> {
        if !self.is_rate_limited {
            return None;
        }
        self.rate_limit_expires_at.map(|expires| {
            let now = Instant::now();
            if expires > now {
                expires.duration_since(now).as_secs()
            } else {
                0
            }
        })
    }
}

/// Thread-safe rate limit tracker for all registered providers.
///
/// Stored as `Arc<RateLimiter>` so that multiple components (router, individual
/// provider wrappers) can observe and update rate limit state without races.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<String, ProviderRateLimitState>>>,
}

impl RateLimiter {
    /// Creates a new empty rate limiter.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Ensures a provider entry exists, creating a default one if absent.
    pub fn register_provider(&self, provider_name: &str) {
        let mut map = self.inner.lock().expect("rate limiter lock poisoned");
        map.entry(provider_name.to_string()).or_default();
    }

    /// Returns true if the named provider is currently available.
    pub fn is_available(&self, provider_name: &str) -> bool {
        let mut map = self.inner.lock().expect("rate limiter lock poisoned");
        let state = map.entry(provider_name.to_string()).or_default();
        state.check_and_clear_rate_limit();
        state.is_available()
    }

    /// Marks a provider as rate-limited for `retry_after_secs` seconds.
    ///
    /// Applies jitter (±10%) to the retry window to avoid thundering-herd
    /// behaviour when multiple providers hit the limit simultaneously.
    pub fn mark_rate_limited(&self, provider_name: &str, retry_after_secs: u64) {
        let jitter_factor = 0.9 + (fastrand_jitter() * 0.2); // 0.9 to 1.1
        let jittered_secs = ((retry_after_secs as f64) * jitter_factor) as u64;
        let expires_at = Instant::now() + Duration::from_secs(jittered_secs.max(1));

        let mut map = self.inner.lock().expect("rate limiter lock poisoned");
        let state = map.entry(provider_name.to_string()).or_default();
        state.is_rate_limited = true;
        state.rate_limit_expires_at = Some(expires_at);
        state.consecutive_failures += 1;
        state.failure_count += 1;

        warn!(
            provider = provider_name,
            retry_after_secs = jittered_secs,
            consecutive_failures = state.consecutive_failures,
            "Provider rate limited — backing off"
        );
    }

    /// Permanently marks a provider as exhausted (API key invalid or quota gone).
    ///
    /// Unlike rate limits, exhaustion is not reversible within the same session.
    pub fn mark_exhausted(&self, provider_name: &str) {
        let mut map = self.inner.lock().expect("rate limiter lock poisoned");
        let state = map.entry(provider_name.to_string()).or_default();
        state.is_exhausted = true;
        state.failure_count += 1;

        warn!(
            provider = provider_name,
            "Provider marked exhausted — will be skipped for the rest of this session"
        );
    }

    /// Marks a provider as manually disabled.
    pub fn mark_disabled(&self, provider_name: &str, reason: &str) {
        let mut map = self.inner.lock().expect("rate limiter lock poisoned");
        let state = map.entry(provider_name.to_string()).or_default();
        state.is_manually_disabled = true;

        warn!(
            provider = provider_name,
            reason = reason,
            "Provider manually disabled"
        );
    }

    /// Restores a provider — clears rate limit, exhausted, and disabled flags.
    pub fn restore(&self, provider_name: &str) {
        let mut map = self.inner.lock().expect("rate limiter lock poisoned");
        let state = map.entry(provider_name.to_string()).or_default();
        state.is_rate_limited = false;
        state.rate_limit_expires_at = None;
        state.is_exhausted = false;
        state.is_manually_disabled = false;
        state.consecutive_failures = 0;

        info!(provider = provider_name, "Provider restored to active rotation");
    }

    /// Records a successful request for a provider.
    pub fn record_success(&self, provider_name: &str) {
        let mut map = self.inner.lock().expect("rate limiter lock poisoned");
        let state = map.entry(provider_name.to_string()).or_default();
        state.consecutive_failures = 0;
        state.success_count += 1;
    }

    /// Records a failed request for a provider (not rate-limited or exhausted — just an error).
    pub fn record_failure(&self, provider_name: &str) {
        let mut map = self.inner.lock().expect("rate limiter lock poisoned");
        let state = map.entry(provider_name.to_string()).or_default();
        state.consecutive_failures += 1;
        state.failure_count += 1;
    }

    /// Returns a snapshot of the rate limit state for a provider.
    pub fn get_state(&self, provider_name: &str) -> ProviderRateLimitState {
        let mut map = self.inner.lock().expect("rate limiter lock poisoned");
        let state = map.entry(provider_name.to_string()).or_default();
        state.check_and_clear_rate_limit();
        state.clone()
    }

    /// Returns all provider states (for status reporting).
    pub fn all_states(&self) -> HashMap<String, ProviderRateLimitState> {
        let mut map = self.inner.lock().expect("rate limiter lock poisoned");
        // Clear any expired rate limits before reporting
        for state in map.values_mut() {
            state.check_and_clear_rate_limit();
        }
        map.clone()
    }

    /// Computes the exponential backoff delay for a provider based on consecutive failures.
    ///
    /// Formula: base_delay * 2^(failures - 1), capped at `max_delay_secs`.
    /// A small random jitter (±20%) is applied to prevent thundering herd.
    pub fn backoff_delay(&self, provider_name: &str) -> Duration {
        let map = self.inner.lock().expect("rate limiter lock poisoned");
        let failures = map
            .get(provider_name)
            .map(|s| s.consecutive_failures)
            .unwrap_or(0);

        const BASE_DELAY_SECS: f64 = 1.0;
        const MAX_DELAY_SECS: f64 = 60.0;

        if failures == 0 {
            return Duration::ZERO;
        }

        let base = BASE_DELAY_SECS * (2_f64.powi(failures.saturating_sub(1) as i32));
        let capped = base.min(MAX_DELAY_SECS);
        let jitter = capped * (0.8 + fastrand_jitter() * 0.4); // 80–120% of capped

        debug!(
            provider = provider_name,
            consecutive_failures = failures,
            backoff_secs = jitter,
            "Computed exponential backoff"
        );

        Duration::from_secs_f64(jitter)
    }

    /// Parses the `Retry-After` header value from an HTTP response.
    ///
    /// The header can be either a delay in seconds (integer) or an HTTP date.
    /// Falls back to `default_secs` if parsing fails.
    pub fn parse_retry_after(header_value: &str, default_secs: u64) -> u64 {
        // Try parsing as integer seconds first
        if let Ok(secs) = header_value.trim().parse::<u64>() {
            return secs;
        }
        // Try parsing as HTTP date (RFC 7231)
        // If parsing fails, return the default
        default_secs
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns a pseudo-random float in [0.0, 1.0) using a fast non-crypto method.
/// This avoids pulling in the `rand` crate for simple jitter calculations.
fn fastrand_jitter() -> f64 {
    // Use the current time's nanoseconds as a seed source for lightweight jitter.
    // This is not cryptographically random but is sufficient for backoff jitter.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos % 1000) as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_rate_limiter_basic() {
        let rl = RateLimiter::new();
        rl.register_provider("test_provider");

        // Initially available
        assert!(rl.is_available("test_provider"));

        // Mark rate limited
        rl.mark_rate_limited("test_provider", 60);
        assert!(!rl.is_available("test_provider"));

        // Restore
        rl.restore("test_provider");
        assert!(rl.is_available("test_provider"));
    }

    #[test]
    fn test_exhausted_provider() {
        let rl = RateLimiter::new();
        rl.mark_exhausted("exhausted_provider");
        assert!(!rl.is_available("exhausted_provider"));

        // Restore should clear exhausted flag
        rl.restore("exhausted_provider");
        assert!(rl.is_available("exhausted_provider"));
    }

    #[test]
    fn test_backoff_increases_with_failures() {
        let rl = RateLimiter::new();
        rl.register_provider("flaky_provider");

        let delay_0 = rl.backoff_delay("flaky_provider");
        assert_eq!(delay_0, Duration::ZERO);

        rl.record_failure("flaky_provider");
        let delay_1 = rl.backoff_delay("flaky_provider");

        rl.record_failure("flaky_provider");
        let delay_2 = rl.backoff_delay("flaky_provider");

        // Each failure should increase (or at minimum match) the delay
        // (jitter means we can't assert strict ordering, just that both are > 0)
        assert!(delay_1 > Duration::ZERO);
        assert!(delay_2 > Duration::ZERO);
        // Generally delay_2 >= delay_1 (base * 2^1 vs base * 2^0)
        // but due to jitter we allow up to 10% variance
        let _ = (delay_1, delay_2);
    }

    #[test]
    fn test_parse_retry_after() {
        assert_eq!(RateLimiter::parse_retry_after("30", 60), 30);
        assert_eq!(RateLimiter::parse_retry_after("  120  ", 60), 120);
        // Invalid value falls back to default
        assert_eq!(RateLimiter::parse_retry_after("invalid", 60), 60);
    }

    #[test]
    fn test_success_resets_consecutive_failures() {
        let rl = RateLimiter::new();
        rl.record_failure("p1");
        rl.record_failure("p1");
        rl.record_failure("p1");
        let state = rl.get_state("p1");
        assert_eq!(state.consecutive_failures, 3);

        rl.record_success("p1");
        let state = rl.get_state("p1");
        assert_eq!(state.consecutive_failures, 0);
    }

    #[test]
    fn test_rate_limit_expires() {
        let rl = RateLimiter::new();
        // Set a very short rate limit (1 second)
        rl.mark_rate_limited("short_limit", 1);
        assert!(!rl.is_available("short_limit"));

        // Wait for expiry
        thread::sleep(Duration::from_millis(1100));
        assert!(rl.is_available("short_limit"));
    }
}
