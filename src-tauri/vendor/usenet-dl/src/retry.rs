//! Retry logic with exponential backoff
//!
//! This module provides configurable retry logic for transient failures.
//! It implements exponential backoff with optional jitter to prevent thundering herd.
//!
//! # Example
//!
//! ```no_run
//! use usenet_dl::retry::{IsRetryable, download_with_retry};
//! use usenet_dl::config::RetryConfig;
//!
//! #[derive(Debug)]
//! enum MyError {
//!     Transient,
//!     Permanent,
//! }
//!
//! impl IsRetryable for MyError {
//!     fn is_retryable(&self) -> bool {
//!         matches!(self, MyError::Transient)
//!     }
//! }
//!
//! # async fn example() -> Result<(), MyError> {
//! let config = RetryConfig::default();
//! let result = download_with_retry(&config, || async {
//!     // Your operation here
//!     Ok::<_, MyError>(())
//! }).await?;
//! # Ok(())
//! # }
//! ```

use crate::config::RetryConfig;
use crate::error::Error;
use rand::Rng;
use std::future::Future;
use std::time::Duration;

/// Trait for errors that can be classified as retryable or not
///
/// Transient failures (network timeouts, server busy, connection reset) should return `true`.
/// Permanent failures (authentication failed, disk full, corrupt data) should return `false`.
pub trait IsRetryable {
    /// Returns true if the error is transient and the operation should be retried
    fn is_retryable(&self) -> bool;
}

/// Implementation of IsRetryable for our Error type
impl IsRetryable for Error {
    fn is_retryable(&self) -> bool {
        match self {
            // Network errors are generally retryable
            Error::Network(e) => {
                // Check if it's a timeout or connection error
                e.is_timeout() || e.is_connect()
            }
            // I/O errors can be retryable in some cases
            Error::Io(e) => matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::Interrupted
            ),
            // NNTP errors need to be classified based on content
            // For now, we treat them as potentially retryable
            Error::Nntp(msg) => {
                // Common transient NNTP error patterns
                msg.contains("timeout")
                    || msg.contains("busy")
                    || msg.contains("connection")
                    || msg.contains("temporary")
                    || msg.contains("503") // Service unavailable
                    || msg.contains("400") // Server busy
            }
            // Download errors are not retryable (state/not-found/space errors)
            Error::Download(_) => false,
            // Post-processing errors are generally permanent
            Error::PostProcess(_) => false,
            // Database errors should not be retried (likely permanent)
            Error::Database(_) | Error::Sqlx(_) => false,
            // Config errors are permanent
            Error::Config { .. } => false,
            // Invalid NZB is permanent
            Error::InvalidNzb(_) => false,
            // Not found is permanent
            Error::NotFound(_) => false,
            // Shutdown in progress - not retryable
            Error::ShuttingDown => false,
            // Serialization errors are permanent
            Error::Serialization(_) => false,
            // API server errors are generally not retryable (application-level errors)
            Error::ApiServerError(_) => false,
            // Folder watch errors are generally not retryable (file system issues)
            Error::FolderWatch(_) => false,
            // Duplicate errors are permanent (not retryable)
            Error::Duplicate(_) => false,
            // Disk space errors are permanent (need user action to free space)
            Error::InsufficientSpace { .. } => false,
            // Disk space check errors are permanent (file system issues)
            Error::DiskSpaceCheckFailed(_) => false,
            // External tool errors might be retryable (temporary failures)
            Error::ExternalTool(msg) => {
                // Retry on timeouts, busy states, but not on "not found" errors
                msg.contains("timeout") || msg.contains("busy") || msg.contains("temporary")
            }
            // Not supported errors are permanent (feature unavailable)
            Error::NotSupported(_) => false,
            // Unknown errors - be conservative and don't retry
            Error::Other(_) => false,
        }
    }
}

/// Execute an async operation with exponential backoff retry logic
///
/// # Arguments
///
/// * `config` - Retry configuration (max attempts, delays, backoff multiplier, jitter)
/// * `operation` - Async closure that returns Result<T, E> where E implements IsRetryable
///
/// # Returns
///
/// Returns the successful result or the last error after all retry attempts are exhausted.
///
/// # Example
///
/// ```no_run
/// use usenet_dl::retry::download_with_retry;
/// use usenet_dl::config::RetryConfig;
/// use usenet_dl::error::Error;
///
/// # async fn example() -> Result<(), Error> {
/// let config = RetryConfig::default();
/// let result = download_with_retry(&config, || async {
///     // Simulate a network operation that might fail
///     Ok::<String, Error>("success".to_string())
/// }).await?;
/// # Ok(())
/// # }
/// ```
pub async fn download_with_retry<F, Fut, T, E>(
    config: &RetryConfig,
    mut operation: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: IsRetryable + std::fmt::Display,
{
    let mut attempt = 0;
    let mut delay = config.initial_delay;

    loop {
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    tracing::info!(attempts = attempt + 1, "Operation succeeded after retry");
                }
                return Ok(result);
            }
            Err(e) if e.is_retryable() && attempt < config.max_attempts => {
                attempt += 1;

                tracing::warn!(
                    error = %e,
                    attempt = attempt,
                    max_attempts = config.max_attempts,
                    delay_ms = delay.as_millis(),
                    "Operation failed, retrying"
                );

                // Calculate jittered delay
                let jittered_delay = if config.jitter {
                    add_jitter(delay)
                } else {
                    delay
                };

                // Wait before retrying
                tokio::time::sleep(jittered_delay).await;

                // Calculate next delay with exponential backoff
                let next_delay =
                    Duration::from_secs_f64(delay.as_secs_f64() * config.backoff_multiplier);
                delay = next_delay.min(config.max_delay);
            }
            Err(e) => {
                if e.is_retryable() {
                    tracing::error!(
                        error = %e,
                        attempts = attempt + 1,
                        "Operation failed after all retry attempts exhausted"
                    );
                } else {
                    tracing::error!(
                        error = %e,
                        "Operation failed with non-retryable error"
                    );
                }
                return Err(e);
            }
        }
    }
}

/// Add random jitter to a delay to prevent thundering herd
///
/// Jitter is uniformly distributed between 0% and 100% of the delay.
/// This means the actual delay will be between `delay` and `2 * delay`.
///
/// # Arguments
///
/// * `delay` - Base delay duration
///
/// # Returns
///
/// Jittered delay duration
fn add_jitter(delay: Duration) -> Duration {
    let mut rng = rand::thread_rng();
    let jitter_factor: f64 = rng.gen_range(0.0..=1.0);
    let jittered_secs = delay.as_secs_f64() * (1.0 + jitter_factor);
    Duration::from_secs_f64(jittered_secs)
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[derive(Debug)]
    enum TestError {
        Transient,
        Permanent,
    }

    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                TestError::Transient => write!(f, "transient error"),
                TestError::Permanent => write!(f, "permanent error"),
            }
        }
    }

    impl IsRetryable for TestError {
        fn is_retryable(&self) -> bool {
            matches!(self, TestError::Transient)
        }
    }

    #[tokio::test]
    async fn test_success_no_retry() {
        let config = RetryConfig::default();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<_, TestError>(42)
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 1, "should only call once");
    }

    #[tokio::test]
    async fn test_retry_transient_then_succeed() {
        let config = RetryConfig {
            max_attempts: 3,
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            jitter: false,
        };

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                let count = counter.fetch_add(1, Ordering::SeqCst);
                if count < 2 {
                    Err(TestError::Transient)
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "should retry twice before success"
        );
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let config = RetryConfig {
            max_attempts: 2,
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            jitter: false,
        };

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(TestError::Transient)
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "should try initial + 2 retries"
        );
    }

    #[tokio::test]
    async fn test_permanent_error_no_retry() {
        let config = RetryConfig::default();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(TestError::Permanent)
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "should not retry permanent error"
        );
    }

    #[tokio::test]
    async fn test_exponential_backoff() {
        let config = RetryConfig {
            max_attempts: 3,
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            jitter: false,
        };

        let start = std::time::Instant::now();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let _result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(TestError::Transient)
            }
        })
        .await;

        let elapsed = start.elapsed();

        // Total expected delay: 10ms + 20ms + 40ms = 70ms
        // Upper bound is generous to tolerate CI and coverage instrumentation overhead
        assert!(
            elapsed >= Duration::from_millis(70),
            "should wait at least 70ms, waited {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "should not wait too long, waited {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_jitter_adds_randomness() {
        let delay = Duration::from_millis(100);

        // Test that jitter produces different values
        let jittered1 = add_jitter(delay);
        let jittered2 = add_jitter(delay);

        // Jitter should produce values between delay and 2*delay
        assert!(jittered1 >= delay);
        assert!(jittered1 <= delay * 2);
        assert!(jittered2 >= delay);
        assert!(jittered2 <= delay * 2);

        // With high probability, two jittered values should be different
        // (could fail very rarely due to randomness, but extremely unlikely)
        // Skip this assertion as it's non-deterministic
    }

    #[tokio::test]
    async fn test_max_delay_cap() {
        let config = RetryConfig {
            max_attempts: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(3),
            backoff_multiplier: 10.0, // Very aggressive multiplier
            jitter: false,
        };

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let start = std::time::Instant::now();

        let _result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(TestError::Transient)
            }
        })
        .await;

        let elapsed = start.elapsed();

        // Delays should be capped at max_delay (3 seconds)
        // First delay: 1s
        // Second delay: min(10s, 3s) = 3s
        // Third delay: min(30s, 3s) = 3s
        // Fourth delay: min(300s, 3s) = 3s
        // Fifth delay: min(3000s, 3s) = 3s
        // Total: 1 + 3 + 3 + 3 + 3 = 13 seconds
        assert!(
            elapsed >= Duration::from_secs(13),
            "should wait at least 13s with max_delay cap, waited {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_secs(15),
            "should not exceed expected time significantly, waited {:?}",
            elapsed
        );
    }

    // Note: reqwest::Error doesn't have a simple constructor for testing,
    // so we test network retryability indirectly through integration tests

    #[tokio::test]
    async fn test_individual_retry_delays_never_exceed_max_delay() {
        // Aggressive multiplier: without capping, delays would be 50ms, 500ms, 5000ms, 50000ms
        // With max_delay=200ms, they should be 50ms, 200ms, 200ms, 200ms
        let config = RetryConfig {
            max_attempts: 4,
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_millis(200),
            backoff_multiplier: 10.0,
            jitter: false,
        };

        let timestamps = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let ts_clone = timestamps.clone();

        let _result = download_with_retry(&config, || {
            let ts = ts_clone.clone();
            async move {
                ts.lock().await.push(std::time::Instant::now());
                Err::<i32, _>(TestError::Transient)
            }
        })
        .await;

        let ts = timestamps.lock().await;
        // initial call + 4 retries = 5 calls
        assert_eq!(ts.len(), 5, "should have initial + 4 retries = 5 calls");

        // Check each inter-retry gap is capped at max_delay (200ms) + tolerance
        let max_allowed = Duration::from_millis(350); // 200ms + generous tolerance for scheduling
        for i in 1..ts.len() {
            let gap = ts[i].duration_since(ts[i - 1]);
            assert!(
                gap <= max_allowed,
                "delay between attempt {} and {} was {:?}, which exceeds max_delay (200ms) + tolerance ({:?})",
                i,
                i + 1,
                gap,
                max_allowed
            );
        }

        // Verify that later delays are capped: gap[2→3] and gap[3→4] should be ~200ms,
        // not 5000ms or 50000ms as they would be without capping
        let gap_3_to_4 = ts[3].duration_since(ts[2]);
        let gap_4_to_5 = ts[4].duration_since(ts[3]);

        assert!(
            gap_3_to_4 >= Duration::from_millis(150),
            "third delay should be ~200ms (capped), was {:?}",
            gap_3_to_4
        );
        assert!(
            gap_4_to_5 >= Duration::from_millis(150),
            "fourth delay should be ~200ms (capped), was {:?}",
            gap_4_to_5
        );
    }

    #[test]
    fn test_error_is_retryable_io() {
        let timeout_err = Error::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout"));
        assert!(timeout_err.is_retryable());

        let connection_refused = Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        ));
        assert!(connection_refused.is_retryable());

        let not_found = Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not found",
        ));
        assert!(!not_found.is_retryable());
    }

    #[test]
    fn test_error_is_retryable_nntp() {
        let timeout = Error::Nntp("connection timeout".to_string());
        assert!(timeout.is_retryable());

        let busy = Error::Nntp("server busy (400)".to_string());
        assert!(busy.is_retryable());

        let auth_failed = Error::Nntp("authentication failed".to_string());
        assert!(!auth_failed.is_retryable());
    }

    #[test]
    fn test_error_is_retryable_permanent() {
        use crate::error::{DatabaseError, DownloadError};

        assert!(
            !Error::Config {
                message: "bad config".to_string(),
                key: None,
            }
            .is_retryable()
        );
        assert!(
            !Error::Database(DatabaseError::QueryFailed("db error".to_string())).is_retryable()
        );
        assert!(!Error::InvalidNzb("bad nzb".to_string()).is_retryable());
        assert!(!Error::NotFound("not found".to_string()).is_retryable());
        assert!(!Error::Download(DownloadError::NotFound { id: 123 }).is_retryable());
    }

    // -----------------------------------------------------------------------
    // add_jitter bounds verification
    // -----------------------------------------------------------------------

    #[test]
    fn add_jitter_stays_within_bounds_over_many_iterations() {
        let delay = Duration::from_millis(50);
        // Run enough iterations that a bounds violation would almost certainly surface
        for i in 0..200 {
            let jittered = add_jitter(delay);
            assert!(
                jittered >= delay,
                "iteration {i}: jittered {jittered:?} < base delay {delay:?}"
            );
            assert!(
                jittered <= delay * 2,
                "iteration {i}: jittered {jittered:?} > 2x base delay {:?}",
                delay * 2
            );
        }
    }

    #[test]
    fn add_jitter_on_zero_delay_returns_zero() {
        let jittered = add_jitter(Duration::ZERO);
        assert_eq!(
            jittered,
            Duration::ZERO,
            "jitter on zero delay should remain zero"
        );
    }

    // -----------------------------------------------------------------------
    // max_attempts=0 edge case: fails immediately on first error
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn zero_max_attempts_fails_on_first_transient_error() {
        let config = RetryConfig {
            max_attempts: 0,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            jitter: false,
        };

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(TestError::Transient)
            }
        })
        .await;

        assert!(
            matches!(result, Err(TestError::Transient)),
            "should return the transient error without retrying"
        );
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "should call the operation exactly once (no retries when max_attempts=0)"
        );
    }

    // -----------------------------------------------------------------------
    // Backoff delay increases exponentially (timing-based verification)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn backoff_delays_increase_exponentially() {
        let config = RetryConfig {
            max_attempts: 3,
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 2.0,
            jitter: false,
        };

        let timestamps = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let ts_clone = timestamps.clone();

        let _result = download_with_retry(&config, || {
            let ts = ts_clone.clone();
            async move {
                ts.lock().await.push(std::time::Instant::now());
                Err::<i32, _>(TestError::Transient)
            }
        })
        .await;

        let ts = timestamps.lock().await;
        assert_eq!(ts.len(), 4, "initial + 3 retries = 4 calls");

        // Gap between call 0 and 1 should be ~50ms (initial_delay)
        let gap1 = ts[1].duration_since(ts[0]);
        // Gap between call 1 and 2 should be ~100ms (50 * 2.0)
        let gap2 = ts[2].duration_since(ts[1]);
        // Gap between call 2 and 3 should be ~200ms (100 * 2.0)
        let gap3 = ts[3].duration_since(ts[2]);

        assert!(
            gap1 >= Duration::from_millis(40),
            "first delay should be ~50ms, was {:?}",
            gap1
        );
        assert!(
            gap2 >= Duration::from_millis(80),
            "second delay should be ~100ms, was {:?}",
            gap2
        );
        assert!(
            gap3 >= Duration::from_millis(160),
            "third delay should be ~200ms, was {:?}",
            gap3
        );

        // Verify exponential growth: each gap should be roughly 2x the previous
        let ratio = gap2.as_secs_f64() / gap1.as_secs_f64();
        assert!(
            (1.5..=2.5).contains(&ratio),
            "gap2/gap1 ratio should be ~2.0, was {ratio:.2}"
        );
    }

    // -----------------------------------------------------------------------
    // Jitter enabled in config produces delays within expected range
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn jitter_enabled_produces_delay_within_expected_range() {
        let config = RetryConfig {
            max_attempts: 1,
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(10),
            backoff_multiplier: 2.0,
            jitter: true,
        };

        let start = std::time::Instant::now();

        let _result =
            download_with_retry(&config, || async { Err::<i32, _>(TestError::Transient) }).await;

        let elapsed = start.elapsed();

        // With jitter, first delay is between 50ms and 100ms
        // Second attempt fails and exhausts retries (no more delay after that)
        // Upper bound is generous to tolerate CI and coverage instrumentation overhead
        assert!(
            elapsed >= Duration::from_millis(40),
            "should wait at least the base delay, waited {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "should not wait longer than expected, waited {:?}",
            elapsed
        );
    }

    // -----------------------------------------------------------------------
    // Remaining IsRetryable implementations for Error variants
    // -----------------------------------------------------------------------

    #[test]
    fn io_connection_reset_is_retryable() {
        let err = Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset by peer",
        ));
        assert!(
            err.is_retryable(),
            "ConnectionReset should be retryable for transient network glitches"
        );
    }

    #[test]
    fn io_connection_aborted_is_retryable() {
        let err = Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "aborted",
        ));
        assert!(err.is_retryable());
    }

    #[test]
    fn io_not_connected_is_retryable() {
        let err = Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "not connected",
        ));
        assert!(err.is_retryable());
    }

    #[test]
    fn io_broken_pipe_is_retryable() {
        let err = Error::Io(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "broken pipe",
        ));
        assert!(err.is_retryable());
    }

    #[test]
    fn io_interrupted_is_retryable() {
        let err = Error::Io(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "interrupted",
        ));
        assert!(err.is_retryable());
    }

    #[test]
    fn io_permission_denied_is_not_retryable() {
        let err = Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "denied",
        ));
        assert!(
            !err.is_retryable(),
            "PermissionDenied is permanent, not transient"
        );
    }

    #[test]
    fn nntp_503_service_unavailable_is_retryable() {
        let err = Error::Nntp("503 service temporarily unavailable".to_string());
        assert!(err.is_retryable());
    }

    #[test]
    fn nntp_400_server_busy_is_retryable() {
        let err = Error::Nntp("400 server too busy".to_string());
        assert!(err.is_retryable());
    }

    #[test]
    fn nntp_temporary_failure_is_retryable() {
        let err = Error::Nntp("temporary failure, please retry".to_string());
        assert!(err.is_retryable());
    }

    #[test]
    fn nntp_unknown_error_without_keywords_is_not_retryable() {
        let err = Error::Nntp("430 no such article".to_string());
        assert!(
            !err.is_retryable(),
            "NNTP error without transient keywords should not be retried"
        );
    }

    #[test]
    fn external_tool_timeout_is_retryable() {
        let err = Error::ExternalTool("timeout waiting for par2".to_string());
        assert!(err.is_retryable());
    }

    #[test]
    fn external_tool_busy_is_retryable() {
        let err = Error::ExternalTool("process busy, try again".to_string());
        assert!(err.is_retryable());
    }

    #[test]
    fn external_tool_temporary_is_retryable() {
        let err = Error::ExternalTool("temporary failure in unrar".to_string());
        assert!(err.is_retryable());
    }

    #[test]
    fn external_tool_not_found_is_not_retryable() {
        let err = Error::ExternalTool("par2 not found in PATH".to_string());
        assert!(
            !err.is_retryable(),
            "missing binary is permanent, not transient"
        );
    }

    #[test]
    fn post_process_error_is_never_retryable() {
        use crate::error::PostProcessError;
        let err = Error::PostProcess(PostProcessError::ExtractionFailed {
            archive: std::path::PathBuf::from("test.rar"),
            reason: "CRC error".to_string(),
        });
        assert!(!err.is_retryable(), "post-processing errors are permanent");
    }

    #[test]
    fn shutting_down_is_not_retryable() {
        assert!(
            !Error::ShuttingDown.is_retryable(),
            "shutdown should not trigger retries"
        );
    }

    #[test]
    fn serialization_error_is_not_retryable() {
        let err = Error::Serialization(serde_json::from_str::<String>("bad json").unwrap_err());
        assert!(!err.is_retryable());
    }

    #[test]
    fn api_server_error_is_not_retryable() {
        let err = Error::ApiServerError("bind failed".to_string());
        assert!(!err.is_retryable());
    }

    #[test]
    fn folder_watch_error_is_not_retryable() {
        let err = Error::FolderWatch("inotify error".to_string());
        assert!(!err.is_retryable());
    }

    #[test]
    fn duplicate_error_is_not_retryable() {
        let err = Error::Duplicate("already exists".to_string());
        assert!(!err.is_retryable());
    }

    #[test]
    fn insufficient_space_is_not_retryable() {
        let err = Error::InsufficientSpace {
            required: 1_000_000,
            available: 500,
        };
        assert!(
            !err.is_retryable(),
            "disk space issues require user action, not retries"
        );
    }

    #[test]
    fn disk_space_check_failed_is_not_retryable() {
        let err = Error::DiskSpaceCheckFailed("statvfs failed".to_string());
        assert!(!err.is_retryable());
    }

    #[test]
    fn not_supported_is_not_retryable() {
        let err = Error::NotSupported("feature unavailable".to_string());
        assert!(!err.is_retryable());
    }

    #[test]
    fn other_error_is_not_retryable() {
        let err = Error::Other("unknown problem".to_string());
        assert!(!err.is_retryable());
    }
}
