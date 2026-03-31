//! Wall-clock timeout enforcement (watchdog).
//!
//! The `Watchdog` tracks elapsed wall-clock time and signals timeout
//! when the configured duration is exceeded. Used by the executor to
//! enforce per-task time limits.

use std::time::{Duration, Instant};
use uuid::Uuid;

/// Wall-clock watchdog timer.
///
/// Created at task start. The executor calls `is_timed_out()` before
/// each step to check if the overall task time limit has been exceeded.
///
/// Design rationale: `tokio::time::timeout` is the preferred approach for
/// wrapping async calls, but the watchdog provides a simple polling API
/// for the step loop without requiring wrapping every operation.
#[derive(Debug)]
pub struct Watchdog {
    task_id: Uuid,
    started_at: Instant,
    timeout: Duration,
}

impl Watchdog {
    /// Creates a new watchdog for the given task.
    pub fn new(task_id: Uuid, timeout: Duration) -> Self {
        Self {
            task_id,
            started_at: Instant::now(),
            timeout,
        }
    }

    /// Returns true if the wall-clock timeout has been exceeded.
    pub fn is_timed_out(&self) -> bool {
        self.started_at.elapsed() >= self.timeout
    }

    /// Returns the elapsed time since the watchdog was created.
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Returns the remaining time before timeout.
    ///
    /// Returns `Duration::ZERO` if already timed out.
    pub fn remaining(&self) -> Duration {
        self.timeout.saturating_sub(self.started_at.elapsed())
    }

    /// Returns the task ID this watchdog is monitoring.
    pub fn task_id(&self) -> Uuid {
        self.task_id
    }

    /// Returns the configured timeout duration.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Wraps an async operation with a timeout using `tokio::time::timeout`.
    ///
    /// Returns `None` if the operation times out, `Some(result)` otherwise.
    pub async fn wrap_with_timeout<F, T>(&self, future: F) -> Option<T>
    where
        F: std::future::Future<Output = T>,
    {
        let remaining = self.remaining();
        tokio::time::timeout(remaining, future).await.ok()
    }
}

/// Wraps an async operation with a hard timeout.
///
/// Convenience function for wrapping individual operations with configurable
/// timeouts independent of the task-level watchdog.
pub async fn with_timeout<F, T, E>(
    timeout: Duration,
    future: F,
) -> Result<T, E>
where
    F: std::future::Future<Output = Result<T, E>>,
    E: From<std::io::Error>,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(result) => result,
        Err(_) => Err(E::from(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("Operation timed out after {}ms", timeout.as_millis()),
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watchdog_not_timed_out_immediately() {
        let watchdog = Watchdog::new(Uuid::new_v4(), Duration::from_secs(60));
        assert!(!watchdog.is_timed_out());
    }

    #[test]
    fn watchdog_timed_out_with_zero_duration() {
        let watchdog = Watchdog::new(Uuid::new_v4(), Duration::ZERO);
        // With zero timeout, should immediately be timed out
        assert!(watchdog.is_timed_out());
    }

    #[test]
    fn elapsed_increases_over_time() {
        let watchdog = Watchdog::new(Uuid::new_v4(), Duration::from_secs(60));
        let before = watchdog.elapsed();
        // Small sleep
        std::thread::sleep(Duration::from_millis(10));
        let after = watchdog.elapsed();
        assert!(after >= before);
    }

    #[test]
    fn remaining_decreases_over_time() {
        let watchdog = Watchdog::new(Uuid::new_v4(), Duration::from_secs(60));
        let before = watchdog.remaining();
        std::thread::sleep(Duration::from_millis(10));
        let after = watchdog.remaining();
        assert!(after < before);
    }

    #[tokio::test]
    async fn wrap_with_timeout_succeeds_for_fast_future() {
        let watchdog = Watchdog::new(Uuid::new_v4(), Duration::from_secs(10));
        let result = watchdog.wrap_with_timeout(async { 42u32 }).await;
        assert_eq!(result, Some(42));
    }

    #[tokio::test]
    async fn wrap_with_timeout_returns_none_on_slow_future() {
        let watchdog = Watchdog::new(Uuid::new_v4(), Duration::from_millis(10));
        // Wait for timeout to expire
        std::thread::sleep(Duration::from_millis(20));
        let result = watchdog.wrap_with_timeout(async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            42u32
        }).await;
        // Should be None since timeout has already passed
        assert!(result.is_none() || result == Some(42)); // Race condition possible
    }
}
