//! Route handlers for the REST API
//!
//! Handlers are organized by domain:
//! - [`downloads`] — Individual download management
//! - [`queue`] — Queue-wide operations
//! - [`history`] — Download history
//! - [`servers`] — Server management
//! - [`config`] — Configuration
//! - [`categories`] — Category management
//! - [`system`] — Health, events, OpenAPI, shutdown
//! - [`rss`] — RSS feed management
//! - [`scheduler`] — Schedule rule management

use serde::{Deserialize, Serialize};

mod categories;
mod config;
mod downloads;
mod history;
mod queue;
mod rss;
mod scheduler;
mod servers;
mod system;

// Re-export all handlers so `routes::function_name` continues to work
pub use categories::*;
pub use config::*;
pub use downloads::*;
pub use history::*;
pub use queue::*;
pub use rss::*;
pub use scheduler::*;
pub use servers::*;
pub use system::*;

// ============================================================================
// Query/Request Types (shared across handlers)
// ============================================================================

/// Query parameters for DELETE /downloads/:id
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct DeleteDownloadQuery {
    /// Whether to delete downloaded files (default: false)
    #[serde(default)]
    pub delete_files: bool,
}

/// Query parameters for GET /history
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct HistoryQuery {
    /// Maximum number of items to return (default: 50)
    pub limit: Option<i64>,
    /// Number of items to skip (default: 0)
    pub offset: Option<i64>,
    /// Filter by status: "complete" or "failed"
    pub status: Option<String>,
}

/// Query parameters for DELETE /history
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ClearHistoryQuery {
    /// Clear entries before this timestamp
    pub before: Option<i64>,
    /// Clear only entries with this status: "complete" or "failed"
    pub status: Option<String>,
}

/// Request body for PUT /config/speed-limit
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct SetSpeedLimitRequest {
    /// Speed limit in bytes per second. Use null for unlimited.
    pub limit_bps: Option<u64>,
}

/// Request body for POST /rss and PUT /rss/:id
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct AddRssFeedRequest {
    /// Human-readable name for the feed
    pub name: String,
    /// RSS feed configuration
    #[serde(flatten)]
    pub config: crate::config::RssFeedConfig,
}

/// Response for GET /rss - list of RSS feeds with their IDs
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RssFeedResponse {
    /// Feed ID
    pub id: i64,
    /// Feed name
    pub name: String,
    /// Feed configuration
    #[serde(flatten)]
    pub config: crate::config::RssFeedConfig,
}

/// Response for POST /rss/:id/check - number of items queued
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CheckRssFeedResponse {
    /// Number of new items queued for download
    pub queued: usize,
}

/// Response for GET /scheduler - schedule rule with ID
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ScheduleRuleResponse {
    /// Rule ID (index in the list)
    pub id: i64,
    /// Schedule rule configuration
    #[serde(flatten)]
    pub rule: crate::config::ScheduleRule,
}
