//! Rate limiting middleware for the API
//!
//! Provides configurable rate limiting with support for exempt paths and IPs.

use axum::{
    Json,
    extract::{ConnectInfo, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Instant,
};
use tokio::sync::Mutex;

use crate::config::RateLimitConfig;

/// Simple token bucket rate limiter
struct TokenBucket {
    /// Available tokens
    tokens: f64,
    /// Last refill time
    last_refill: Instant,
    /// Tokens per second
    rate: f64,
    /// Maximum burst size
    capacity: u32,
}

impl TokenBucket {
    fn new(rate: f64, capacity: u32) -> Self {
        Self {
            tokens: capacity as f64,
            last_refill: Instant::now(),
            rate,
            capacity,
        }
    }

    fn try_consume(&mut self) -> Option<u64> {
        // Refill tokens based on time elapsed
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity as f64);
        self.last_refill = now;

        // Try to consume one token
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            None
        } else {
            // Calculate wait time
            let wait_secs = ((1.0 - self.tokens) / self.rate).ceil() as u64;
            Some(wait_secs)
        }
    }
}

/// Rate limiter with per-IP tracking
pub struct RateLimiter {
    /// Per-IP token buckets
    buckets: Mutex<HashMap<IpAddr, TokenBucket>>,
    /// Configuration
    config: RateLimitConfig,
}

impl RateLimiter {
    /// Create a new rate limiter from configuration
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// Check if a path is exempt from rate limiting
    fn is_path_exempt(&self, path: &str) -> bool {
        self.config.exempt_paths.iter().any(|exempt| {
            // Support both exact matches and prefix matches
            path == exempt || path.starts_with(exempt)
        })
    }

    /// Check if an IP address is exempt from rate limiting
    fn is_ip_exempt(&self, addr: &SocketAddr) -> bool {
        self.config.exempt_ips.contains(&addr.ip())
    }

    /// Check if request should be rate limited
    pub async fn check(&self, path: &str, addr: SocketAddr) -> Option<u64> {
        // Check if path is exempt
        if self.is_path_exempt(path) {
            return None;
        }

        // Check if IP is exempt
        if self.is_ip_exempt(&addr) {
            return None;
        }

        // Get or create token bucket for this IP
        // Scope the lock tightly to avoid holding it during try_consume
        let mut buckets = self.buckets.lock().await;
        let bucket = buckets.entry(addr.ip()).or_insert_with(|| {
            TokenBucket::new(
                self.config.requests_per_second as f64,
                self.config.burst_size,
            )
        });
        // try_consume is fast, so holding the lock briefly is acceptable
        // The bucket is modified in place, so we need the mutable borrow
        bucket.try_consume()
    }
}

/// Rate limiting middleware function
pub async fn rate_limit_middleware(
    axum::extract::State(limiter): axum::extract::State<Arc<RateLimiter>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: axum::middleware::Next,
) -> Response {
    match limiter.check(req.uri().path(), addr).await {
        None => next.run(req).await,
        Some(retry_after) => {
            let error = json!({
                "error": {
                    "code": "rate_limited",
                    "message": "Too many requests",
                    "details": {
                        "retry_after_seconds": retry_after
                    }
                }
            });
            (StatusCode::TOO_MANY_REQUESTS, Json(error)).into_response()
        }
    }
}
