//! Speed limiting using token bucket algorithm
//!
//! The SpeedLimiter provides global bandwidth limiting across all concurrent downloads
//! using an efficient lock-free token bucket implementation.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Global speed limiter shared across all downloads
///
/// Uses a token bucket algorithm for efficient, lock-free bandwidth limiting.
/// All concurrent downloads share the same bucket, naturally distributing
/// bandwidth based on demand.
///
/// # Algorithm
///
/// - Tokens represent bytes that can be transferred
/// - Tokens refill at a constant rate (limit_bps)
/// - Downloads acquire tokens before transferring data
/// - If insufficient tokens, download waits until refill
///
/// # Implementation
///
/// Uses AtomicU64 for lock-free token tracking:
/// - `limit_bps`: Speed limit in bytes per second (0 = unlimited)
/// - `tokens`: Available tokens (bytes that can be transferred now)
/// - `last_refill`: Timestamp of last token refill (nanoseconds since epoch)
#[derive(Clone)]
pub struct SpeedLimiter {
    /// Speed limit in bytes per second (0 = unlimited)
    limit_bps: Arc<AtomicU64>,
    /// Available tokens (current bucket capacity in bytes)
    tokens: Arc<AtomicU64>,
    /// Last refill timestamp (nanoseconds since arbitrary epoch)
    last_refill: Arc<AtomicU64>,
}

impl SpeedLimiter {
    /// Create a new SpeedLimiter with the specified limit
    ///
    /// # Arguments
    ///
    /// * `limit_bps` - Speed limit in bytes per second (None = unlimited)
    ///
    /// # Examples
    ///
    /// ```
    /// use usenet_dl::speed_limiter::SpeedLimiter;
    ///
    /// // 10 MB/s limit
    /// let limiter = SpeedLimiter::new(Some(10 * 1024 * 1024));
    ///
    /// // Unlimited
    /// let unlimited = SpeedLimiter::new(None);
    /// ```
    #[must_use]
    pub fn new(limit_bps: Option<u64>) -> Self {
        let limit = limit_bps.unwrap_or(0);
        let now = Self::now_nanos();

        Self {
            limit_bps: Arc::new(AtomicU64::new(limit)),
            tokens: Arc::new(AtomicU64::new(limit)),
            last_refill: Arc::new(AtomicU64::new(now)),
        }
    }

    /// Set a new speed limit
    ///
    /// This takes effect immediately. If increasing the limit, tokens are
    /// refilled to the new capacity. If decreasing, excess tokens remain
    /// until consumed.
    ///
    /// # Arguments
    ///
    /// * `limit_bps` - New speed limit in bytes per second (None = unlimited)
    ///
    /// # Examples
    ///
    /// ```
    /// use usenet_dl::speed_limiter::SpeedLimiter;
    ///
    /// let limiter = SpeedLimiter::new(Some(5_000_000)); // 5 MB/s
    ///
    /// // Increase to 10 MB/s
    /// limiter.set_limit(Some(10_000_000));
    ///
    /// // Remove limit
    /// limiter.set_limit(None);
    /// ```
    pub fn set_limit(&self, limit_bps: Option<u64>) {
        let new_limit = limit_bps.unwrap_or(0);
        let old_limit = self.limit_bps.swap(new_limit, Ordering::SeqCst);

        // If increasing limit, add extra tokens to bucket
        if new_limit > old_limit {
            let extra_tokens = new_limit - old_limit;
            self.tokens.fetch_add(extra_tokens, Ordering::SeqCst);
        }
    }

    /// Get the current speed limit
    ///
    /// Returns None if unlimited, otherwise the limit in bytes per second.
    pub fn get_limit(&self) -> Option<u64> {
        let limit = self.limit_bps.load(Ordering::Relaxed);
        if limit == 0 { None } else { Some(limit) }
    }

    /// Acquire permission to transfer the specified number of bytes
    ///
    /// This method blocks until sufficient tokens are available. For unlimited
    /// speed (limit = 0), this returns immediately.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Number of bytes to transfer
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use usenet_dl::speed_limiter::SpeedLimiter;
    ///
    /// # async fn example() {
    /// let limiter = SpeedLimiter::new(Some(10_000_000)); // 10 MB/s
    ///
    /// // Before downloading each chunk
    /// let chunk_size = 8192;
    /// limiter.acquire(chunk_size).await;
    /// // ... perform download ...
    /// # }
    /// ```
    pub async fn acquire(&self, bytes: u64) {
        // Fast path: nothing to acquire
        if bytes == 0 {
            return;
        }

        // Fast path: unlimited speed
        if self.limit_bps.load(Ordering::Relaxed) == 0 {
            return;
        }

        let mut remaining = bytes;

        loop {
            // Re-read the limit each iteration so dynamic changes take effect
            let limit = self.limit_bps.load(Ordering::Relaxed);
            if limit == 0 {
                // Limit was removed while we were waiting — no throttle needed
                return;
            }

            // Refill tokens based on elapsed time
            self.refill_tokens();

            // Try to consume available tokens (partial consumption allowed)
            let current_tokens = self.tokens.load(Ordering::SeqCst);
            let to_consume = remaining.min(current_tokens);

            if to_consume > 0 {
                if self
                    .tokens
                    .compare_exchange(
                        current_tokens,
                        current_tokens - to_consume,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .is_ok()
                {
                    remaining -= to_consume;
                    if remaining == 0 {
                        return;
                    }
                }
                // CAS failed or still have remaining — retry immediately
                continue;
            }

            // No tokens available — wait for refill.
            // Cap sleep at 100ms so we re-check the limit frequently,
            // allowing dynamic limit changes to take effect promptly.
            let wait_ms = (remaining as f64 / limit as f64 * 1000.0) as u64;
            tokio::time::sleep(Duration::from_millis(wait_ms.clamp(10, 100))).await;
        }
    }

    /// Refill tokens based on elapsed time since last refill
    ///
    /// This is called automatically by acquire(), but can be called manually
    /// for testing or monitoring purposes.
    fn refill_tokens(&self) {
        let limit = self.limit_bps.load(Ordering::Relaxed);
        if limit == 0 {
            return; // Unlimited
        }

        let now = Self::now_nanos();
        let last = self.last_refill.load(Ordering::SeqCst);

        // Calculate elapsed time in seconds
        let elapsed_nanos = now.saturating_sub(last);
        let elapsed_secs = elapsed_nanos as f64 / 1_000_000_000.0;

        // Calculate tokens to add (bytes per second * seconds elapsed)
        let tokens_to_add = (limit as f64 * elapsed_secs) as u64;

        if tokens_to_add > 0 {
            // Try to update last_refill timestamp atomically
            if self
                .last_refill
                .compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                // Add tokens, but cap at limit (bucket capacity)
                let current_tokens = self.tokens.load(Ordering::SeqCst);
                let new_tokens = (current_tokens + tokens_to_add).min(limit);
                self.tokens.store(new_tokens, Ordering::SeqCst);
            }
        }
    }

    /// Get current monotonic time in nanoseconds
    ///
    /// Uses a monotonic clock that is not affected by system time changes.
    /// The epoch is arbitrary but consistent within a process lifetime.
    fn now_nanos() -> u64 {
        // Use Instant for monotonic time measurement
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let start = START.get_or_init(Instant::now);
        start.elapsed().as_nanos() as u64
    }
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_limit_increase() {
        let limiter = SpeedLimiter::new(Some(5_000_000)); // 5 MB/s
        let old_tokens = limiter.tokens.load(Ordering::Relaxed);

        limiter.set_limit(Some(10_000_000)); // 10 MB/s

        assert_eq!(limiter.get_limit(), Some(10_000_000));
        // Tokens should increase by 5 MB
        let new_tokens = limiter.tokens.load(Ordering::Relaxed);
        assert_eq!(new_tokens, old_tokens + 5_000_000);
    }

    #[test]
    fn test_set_limit_decrease() {
        let limiter = SpeedLimiter::new(Some(10_000_000)); // 10 MB/s
        let old_tokens = limiter.tokens.load(Ordering::Relaxed);

        limiter.set_limit(Some(5_000_000)); // 5 MB/s

        assert_eq!(limiter.get_limit(), Some(5_000_000));
        // Tokens should remain the same (not decreased)
        let new_tokens = limiter.tokens.load(Ordering::Relaxed);
        assert_eq!(new_tokens, old_tokens);
    }

    #[tokio::test]
    async fn test_acquire_unlimited() {
        let limiter = SpeedLimiter::new(None);

        // Should return immediately for unlimited speed
        let start = Instant::now();
        limiter.acquire(1_000_000).await;
        let elapsed = start.elapsed();

        // Should complete in under 10ms
        assert!(elapsed < Duration::from_millis(10));
    }

    #[tokio::test]
    async fn test_acquire_multiple_small_chunks() {
        let limiter = SpeedLimiter::new(Some(10_000_000)); // 10 MB/s

        // Acquire multiple small chunks
        for _ in 0..10 {
            limiter.acquire(100_000).await; // 100 KB each
        }

        // Total: 1 MB consumed
        let remaining = limiter.tokens.load(Ordering::Relaxed);
        assert!(
            (8_999_000..=9_001_000).contains(&remaining),
            "expected ~9_000_000 tokens remaining, got {remaining}"
        );
    }

    #[test]
    fn test_set_limit_none_returns_unlimited() {
        let limiter = SpeedLimiter::new(Some(5_000_000));
        assert_eq!(limiter.get_limit(), Some(5_000_000), "should start limited");

        limiter.set_limit(None);

        assert_eq!(
            limiter.get_limit(),
            None,
            "set_limit(None) should make get_limit() return None (unlimited)"
        );
        // Internal representation: 0 means unlimited
        assert_eq!(
            limiter.limit_bps.load(Ordering::Relaxed),
            0,
            "internal limit_bps should be 0 for unlimited"
        );
    }

    #[test]
    fn test_new_none_is_unlimited() {
        let limiter = SpeedLimiter::new(None);

        assert_eq!(
            limiter.get_limit(),
            None,
            "new(None) should create an unlimited limiter"
        );
        assert_eq!(
            limiter.limit_bps.load(Ordering::Relaxed),
            0,
            "internal limit_bps should be 0 for unlimited"
        );
        // Tokens should also be 0 (no bucket needed for unlimited)
        assert_eq!(
            limiter.tokens.load(Ordering::Relaxed),
            0,
            "tokens should be 0 for unlimited limiter (no bucket needed)"
        );
    }

    #[test]
    fn test_new_with_limit_returns_that_limit() {
        let limiter = SpeedLimiter::new(Some(42_000));

        assert_eq!(
            limiter.get_limit(),
            Some(42_000),
            "new(Some(42_000)) should return Some(42_000) from get_limit()"
        );
        // Tokens should be initialized to the limit (full bucket)
        assert_eq!(
            limiter.tokens.load(Ordering::Relaxed),
            42_000,
            "initial tokens should equal the limit (full bucket)"
        );
    }

    #[test]
    fn test_transition_limited_unlimited_limited() {
        let limiter = SpeedLimiter::new(Some(1_000_000)); // 1 MB/s
        assert_eq!(limiter.get_limit(), Some(1_000_000));

        // Transition to unlimited
        limiter.set_limit(None);
        assert_eq!(
            limiter.get_limit(),
            None,
            "should be unlimited after set_limit(None)"
        );

        // Transition back to limited with a different value
        limiter.set_limit(Some(2_000_000));
        assert_eq!(
            limiter.get_limit(),
            Some(2_000_000),
            "should reflect new limit after transitioning back from unlimited"
        );

        // Verify the limiter is functional: internal limit_bps must match
        assert_eq!(limiter.limit_bps.load(Ordering::Relaxed), 2_000_000);
    }

    #[tokio::test]
    async fn test_acquire_zero_bytes_returns_immediately() {
        let limiter = SpeedLimiter::new(Some(100)); // Very low limit: 100 bytes/s

        // Drain all tokens first to ensure the limiter would block on any real acquire
        limiter.tokens.store(0, Ordering::SeqCst);

        let start = Instant::now();
        limiter.acquire(0).await;
        let elapsed = start.elapsed();

        // 0 bytes should return immediately even with an empty bucket
        // The loop condition: remaining starts at 0, so the loop body
        // checks `remaining == 0` and returns immediately.
        assert!(
            elapsed < Duration::from_millis(50),
            "acquire(0) should return immediately, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_acquire_blocks_when_tokens_exhausted() {
        // Use a very low rate so we can measure the wait time
        let rate_bps = 1_000; // 1000 bytes/sec
        let limiter = SpeedLimiter::new(Some(rate_bps));

        // Drain the bucket completely
        limiter.tokens.store(0, Ordering::SeqCst);
        // Reset the refill timestamp to now so refill calculation is clean
        limiter
            .last_refill
            .store(SpeedLimiter::now_nanos(), Ordering::SeqCst);

        let bytes_to_acquire = 500_u64; // 500 bytes at 1000 B/s = ~500ms

        let start = Instant::now();
        limiter.acquire(bytes_to_acquire).await;
        let elapsed = start.elapsed();

        // Expected time: 500 bytes / 1000 bytes/sec = 500ms
        // Use generous tolerance: 250ms - 1500ms (50%-300% of expected)
        let expected_ms = 500;
        let min_ms = expected_ms / 2; // 250ms
        let max_ms = expected_ms * 3; // 1500ms

        assert!(
            elapsed >= Duration::from_millis(min_ms),
            "acquire should have waited at least ~{expected_ms}ms for tokens, but only took {:?}",
            elapsed
        );
        assert!(
            elapsed <= Duration::from_millis(max_ms),
            "acquire took too long: {:?} (expected ~{expected_ms}ms, max {max_ms}ms)",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_concurrent_acquire_distributes_bandwidth() {
        // 4 tasks each acquiring 500 bytes at 2000 bytes/sec total
        // Total: 2000 bytes / 2000 B/s = ~1 second
        let rate_bps = 2_000;
        let limiter = SpeedLimiter::new(Some(rate_bps));

        // Drain bucket so all tasks must wait for refills
        limiter.tokens.store(0, Ordering::SeqCst);
        limiter
            .last_refill
            .store(SpeedLimiter::now_nanos(), Ordering::SeqCst);

        let num_tasks = 4;
        let bytes_per_task = 500_u64;
        let total_bytes = num_tasks * bytes_per_task; // 2000 bytes

        let start = Instant::now();
        let mut handles = vec![];

        for _ in 0..num_tasks {
            let limiter_clone = limiter.clone();
            handles.push(tokio::spawn(async move {
                limiter_clone.acquire(bytes_per_task).await;
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let elapsed = start.elapsed();

        // Expected: 2000 bytes / 2000 B/s = 1 second
        // Generous tolerance: 500ms - 3000ms (50% - 300%)
        let expected_ms = (total_bytes as f64 / rate_bps as f64 * 1000.0) as u64;
        let min_ms = expected_ms / 2;
        let max_ms = expected_ms * 3;

        assert!(
            elapsed >= Duration::from_millis(min_ms),
            "concurrent acquire completed too fast: {:?} (expected ~{expected_ms}ms, \
             total {total_bytes} bytes at {rate_bps} B/s)",
            elapsed
        );
        assert!(
            elapsed <= Duration::from_millis(max_ms),
            "concurrent acquire took too long: {:?} (expected ~{expected_ms}ms, max {max_ms}ms)",
            elapsed
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_set_limit_while_acquire_waiting_picks_up_new_limit() {
        // Start with a very slow limit so acquire will block for a long time
        let limiter = SpeedLimiter::new(Some(100)); // 100 B/s
        limiter.tokens.store(0, Ordering::SeqCst);
        limiter
            .last_refill
            .store(SpeedLimiter::now_nanos(), Ordering::SeqCst);

        let limiter_for_task = limiter.clone();

        let start = Instant::now();

        // Spawn a task that acquires 1000 bytes at 100 B/s (would take ~10 seconds)
        let acquire_handle = tokio::spawn(async move {
            limiter_for_task.acquire(1_000).await;
        });

        // Wait a bit, then increase the limit dramatically
        tokio::time::sleep(Duration::from_millis(500)).await;
        limiter.set_limit(Some(100_000)); // 100 KB/s — should speed things up enormously

        // The acquire should complete much faster than the original 10 seconds
        let result = tokio::time::timeout(Duration::from_secs(5), acquire_handle).await;
        let elapsed = start.elapsed();

        assert!(
            result.is_ok(),
            "acquire should have completed within 5s after limit increase, but timed out"
        );
        result.unwrap().unwrap(); // propagate any panic from the spawned task

        // Should complete well under the original 10 seconds
        assert!(
            elapsed < Duration::from_secs(5),
            "acquire took {:?}, expected much less than 10s after limit increase",
            elapsed
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_set_limit_to_unlimited_unblocks_waiting_acquire() {
        // Start with 1 byte/s — acquiring 1 MB would take ~1 million seconds
        let limiter = SpeedLimiter::new(Some(1));
        limiter.tokens.store(0, Ordering::SeqCst);
        limiter
            .last_refill
            .store(SpeedLimiter::now_nanos(), Ordering::SeqCst);

        let limiter_for_task = limiter.clone();

        let acquire_handle = tokio::spawn(async move {
            limiter_for_task.acquire(1_000_000).await;
        });

        // Let the acquire loop start spinning
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Switch to unlimited — this should cause the loop to see limit==0 and return
        limiter.set_limit(None);

        // The acquire should complete promptly (well under the original ~1M seconds)
        let result = tokio::time::timeout(Duration::from_secs(3), acquire_handle).await;

        assert!(
            result.is_ok(),
            "acquire(1_000_000) should complete quickly after limit set to unlimited, but timed out"
        );
        result.unwrap().unwrap(); // propagate any panic from the spawned task
    }

    #[test]
    fn test_clone_shares_state() {
        let original = SpeedLimiter::new(Some(1_000_000));
        let clone = original.clone();

        // Verify they start with the same limit
        assert_eq!(original.get_limit(), clone.get_limit());

        // Modify through the clone
        clone.set_limit(Some(5_000_000));

        // Original should see the change because they share Arc state
        assert_eq!(
            original.get_limit(),
            Some(5_000_000),
            "original should reflect limit change made via clone"
        );

        // Modify through the original
        original.set_limit(None);

        // Clone should see the change
        assert_eq!(
            clone.get_limit(),
            None,
            "clone should reflect limit change made via original"
        );
    }
}
