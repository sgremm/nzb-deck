//! Database layer for usenet-dl
//!
//! Handles SQLite persistence for downloads, articles, passwords, and history.
//!
//! ## Submodules
//!
//! Methods on [`Database`] are organized by domain:
//! - [`migrations`] — Database lifecycle, schema migrations
//! - [`downloads`] — Download queue CRUD
//! - [`articles`] — Article-level tracking for resume support
//! - [`passwords`] — Password cache for archive extraction
//! - [`duplicates`] — Duplicate detection queries
//! - [`history`] — History management
//! - [`state`] — Runtime state (shutdown tracking, NZB processing, RSS seen)
//! - [`rss`] — RSS feed CRUD

use crate::types::{HistoryEntry, Status};
use sqlx::{FromRow, sqlite::SqlitePool};
use std::path::PathBuf;

mod articles;
mod downloads;
mod duplicates;
mod history;
mod migrations;
mod passwords;
mod rss;
mod state;

/// New download to be inserted into the database
#[derive(Debug, Clone)]
pub struct NewDownload {
    /// Display name for this download
    pub name: String,
    /// Path to the NZB file
    pub nzb_path: String,
    /// Original name from NZB metadata
    pub nzb_meta_name: Option<String>,
    /// SHA-256 hash of the NZB file for duplicate detection
    pub nzb_hash: Option<String>,
    /// Job name for post-processing scripts
    pub job_name: Option<String>,
    /// Category for organizing downloads
    pub category: Option<String>,
    /// Destination directory for extracted files
    pub destination: String,
    /// Post-processing flags (bitfield: 1=unpack, 2=verify, 4=repair, 8=delete)
    pub post_process: i32,
    /// Download priority (higher values download first)
    pub priority: i32,
    /// Current status (0=queued, 1=downloading, 2=completed, etc.)
    pub status: i32,
    /// Total size in bytes
    pub size_bytes: i64,
}

/// Download record from database
#[derive(Debug, Clone, FromRow)]
pub struct Download {
    /// Unique database ID
    pub id: i64,
    /// Display name for this download
    pub name: String,
    /// Path to the NZB file
    pub nzb_path: String,
    /// Original name from NZB metadata
    pub nzb_meta_name: Option<String>,
    /// SHA-256 hash of the NZB file for duplicate detection
    pub nzb_hash: Option<String>,
    /// Job name for post-processing scripts
    pub job_name: Option<String>,
    /// Category for organizing downloads
    pub category: Option<String>,
    /// Destination directory for extracted files
    pub destination: String,
    /// Post-processing flags (bitfield: 1=unpack, 2=verify, 4=repair, 8=delete)
    pub post_process: i32,
    /// Download priority (higher values download first)
    pub priority: i32,
    /// Current status (0=queued, 1=downloading, 2=completed, etc.)
    pub status: i32,
    /// Download progress as a fraction (0.0-1.0)
    pub progress: f32,
    /// Current download speed in bytes per second
    pub speed_bps: i64,
    /// Total size in bytes
    pub size_bytes: i64,
    /// Number of bytes downloaded so far
    pub downloaded_bytes: i64,
    /// Error message if download failed
    pub error_message: Option<String>,
    /// Unix timestamp when download was created
    pub created_at: i64,
    /// Unix timestamp when download started
    pub started_at: Option<i64>,
    /// Unix timestamp when download completed
    pub completed_at: Option<i64>,
    /// DirectUnpack state (0=NotStarted, 1=Active, 2=Completed, 3=Cancelled, 4=Failed)
    pub direct_unpack_state: i32,
    /// Number of files extracted by DirectUnpack (0 means vacuous completion)
    pub direct_unpack_extracted_count: i32,
}

/// New article to be inserted into the database
#[derive(Debug, Clone)]
pub struct NewArticle {
    /// Download this article belongs to
    pub download_id: crate::types::DownloadId,
    /// Usenet message-ID
    pub message_id: String,
    /// Segment number within the file
    pub segment_number: i32,
    /// Index of the NZB file this article belongs to (0-based)
    pub file_index: i32,
    /// Size of this segment in bytes
    pub size_bytes: i64,
}

/// Article record from database
#[derive(Debug, Clone, FromRow)]
pub struct Article {
    /// Unique database ID
    pub id: i64,
    /// Download this article belongs to
    pub download_id: i64,
    /// Usenet message-ID
    pub message_id: String,
    /// Segment number within the file
    pub segment_number: i32,
    /// Index of the NZB file this article belongs to (0-based)
    pub file_index: i32,
    /// Size of this segment in bytes
    pub size_bytes: i64,
    /// Article status (see [`article_status`])
    pub status: i32,
    /// Unix timestamp when article was downloaded
    pub downloaded_at: Option<i64>,
}

/// New download file to be inserted into the database
#[derive(Debug, Clone)]
pub struct NewDownloadFile {
    /// Download this file belongs to
    pub download_id: crate::types::DownloadId,
    /// Index of this file within the NZB (0-based)
    pub file_index: i32,
    /// Parsed filename (from NZB subject)
    pub filename: String,
    /// Original NZB subject line
    pub subject: Option<String>,
    /// Total number of segments in this file
    pub total_segments: i32,
}

/// Download file record from database
#[derive(Debug, Clone, FromRow)]
pub struct DownloadFile {
    /// Unique database ID
    pub id: i64,
    /// Download this file belongs to
    pub download_id: i64,
    /// Index of this file within the NZB (0-based)
    pub file_index: i32,
    /// Parsed filename (from NZB subject)
    pub filename: String,
    /// Original NZB subject line
    pub subject: Option<String>,
    /// Total number of segments in this file
    pub total_segments: i32,
    /// Whether this file is paused (0=no, 1=yes)
    pub paused: i32,
    /// Whether all segments of this file have been downloaded (0=no, 1=yes)
    pub completed: i32,
    /// Original filename before DirectRename (None if not renamed)
    pub original_filename: Option<String>,
}

/// Article status constants
pub mod article_status {
    /// Article is queued and not yet downloaded
    pub const PENDING: i32 = 0;
    /// Article has been successfully downloaded
    pub const DOWNLOADED: i32 = 1;
    /// Article download failed
    pub const FAILED: i32 = 2;
}

/// New history entry to be inserted into the database
#[derive(Debug, Clone)]
pub struct NewHistoryEntry {
    /// Download name
    pub name: String,
    /// Category label
    pub category: Option<String>,
    /// Destination directory on disk
    pub destination: Option<PathBuf>,
    /// Completion status code
    pub status: i32,
    /// Total size in bytes
    pub size_bytes: u64,
    /// Total download duration in seconds
    pub download_time_secs: i64,
    /// Unix timestamp when download completed
    pub completed_at: i64,
}

/// History record from database (raw from SQLite)
#[derive(Debug, Clone, FromRow)]
pub struct HistoryRow {
    /// Unique database ID
    pub id: i64,
    /// Download name
    pub name: String,
    /// Category label
    pub category: Option<String>,
    /// Destination directory on disk
    pub destination: Option<String>,
    /// Completion status code
    pub status: i32,
    /// Total size in bytes
    pub size_bytes: i64,
    /// Total download duration in seconds
    pub download_time_secs: i64,
    /// Unix timestamp when download completed
    pub completed_at: i64,
}

impl From<HistoryRow> for HistoryEntry {
    fn from(row: HistoryRow) -> Self {
        use chrono::{TimeZone, Utc};
        use std::time::Duration;

        HistoryEntry {
            id: row.id,
            name: row.name,
            category: row.category,
            destination: row.destination.map(PathBuf::from),
            status: Status::from_i32(row.status),
            size_bytes: row.size_bytes as u64,
            download_time: Duration::from_secs(row.download_time_secs as u64),
            completed_at: Utc
                .timestamp_opt(row.completed_at, 0)
                .single()
                .unwrap_or_else(Utc::now),
        }
    }
}

/// RSS feed record from database
#[derive(Debug, Clone, FromRow)]
pub struct RssFeed {
    /// Unique database ID
    pub id: i64,
    /// Feed display name
    pub name: String,
    /// Feed URL
    pub url: String,
    /// Interval between checks in seconds
    pub check_interval_secs: i64,
    /// Category label for matched downloads
    pub category: Option<String>,
    /// Whether to automatically download matched items (0 = no, 1 = yes)
    pub auto_download: i32,
    /// Download priority
    pub priority: i32,
    /// Whether the feed is enabled (0 = disabled, 1 = enabled)
    pub enabled: i32,
    /// Unix timestamp of last feed check
    pub last_check: Option<i64>,
    /// Last error message from checking the feed
    pub last_error: Option<String>,
    /// Unix timestamp when feed was created
    pub created_at: i64,
}

/// RSS filter record from database
#[derive(Debug, Clone, FromRow)]
pub struct RssFilterRow {
    /// Unique database ID
    pub id: i64,
    /// ID of the parent RSS feed
    pub feed_id: i64,
    /// Filter display name
    pub name: String,
    /// Comma-separated include patterns
    pub include_patterns: Option<String>,
    /// Comma-separated exclude patterns
    pub exclude_patterns: Option<String>,
    /// Minimum file size in bytes
    pub min_size: Option<i64>,
    /// Maximum file size in bytes
    pub max_size: Option<i64>,
    /// Maximum age of items in seconds
    pub max_age_secs: Option<i64>,
}

/// Parameters for inserting a new RSS feed
pub struct InsertRssFeedParams<'a> {
    /// Feed name
    pub name: &'a str,
    /// Feed URL
    pub url: &'a str,
    /// Check interval in seconds
    pub check_interval_secs: i64,
    /// Optional category
    pub category: Option<&'a str>,
    /// Whether to auto-download matching items
    pub auto_download: bool,
    /// Download priority
    pub priority: i32,
    /// Whether the feed is enabled
    pub enabled: bool,
}

/// Parameters for updating an existing RSS feed
pub struct UpdateRssFeedParams<'a> {
    /// Feed ID
    pub id: i64,
    /// Feed name
    pub name: &'a str,
    /// Feed URL
    pub url: &'a str,
    /// Check interval in seconds
    pub check_interval_secs: i64,
    /// Optional category
    pub category: Option<&'a str>,
    /// Whether to auto-download matching items
    pub auto_download: bool,
    /// Download priority
    pub priority: i32,
    /// Whether the feed is enabled
    pub enabled: bool,
}

/// Parameters for inserting a new RSS filter
pub struct InsertRssFilterParams<'a> {
    /// Feed ID this filter belongs to
    pub feed_id: i64,
    /// Filter name
    pub name: &'a str,
    /// Include patterns (comma-separated regex)
    pub include_patterns: Option<&'a str>,
    /// Exclude patterns (comma-separated regex)
    pub exclude_patterns: Option<&'a str>,
    /// Minimum file size in bytes
    pub min_size: Option<i64>,
    /// Maximum file size in bytes
    pub max_size: Option<i64>,
    /// Maximum age in seconds
    pub max_age_secs: Option<i64>,
}

/// Database handle for usenet-dl
pub struct Database {
    pool: SqlitePool,
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests;
