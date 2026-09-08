//! Core types for usenet-dl

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use utoipa::ToSchema;

use crate::config::{DuplicateMethod, PostProcess};

/// Unique identifier for a download
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(transparent)]
pub struct DownloadId(pub i64);

impl DownloadId {
    /// Create a new DownloadId
    pub fn new(id: i64) -> Self {
        Self(id)
    }

    /// Get the inner i64 value
    pub fn get(&self) -> i64 {
        self.0
    }
}

impl From<i64> for DownloadId {
    fn from(id: i64) -> Self {
        Self(id)
    }
}

impl From<DownloadId> for i64 {
    fn from(id: DownloadId) -> Self {
        id.0
    }
}

impl PartialEq<i64> for DownloadId {
    fn eq(&self, other: &i64) -> bool {
        self.0 == *other
    }
}

impl PartialEq<DownloadId> for i64 {
    fn eq(&self, other: &DownloadId) -> bool {
        *self == other.0
    }
}

impl std::fmt::Display for DownloadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for DownloadId {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

// Implement sqlx Type, Encode, and Decode for database operations
impl sqlx::Type<sqlx::Sqlite> for DownloadId {
    fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
        <i64 as sqlx::Type<sqlx::Sqlite>>::type_info()
    }

    fn compatible(ty: &sqlx::sqlite::SqliteTypeInfo) -> bool {
        <i64 as sqlx::Type<sqlx::Sqlite>>::compatible(ty)
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for DownloadId {
    fn encode_by_ref(
        &self,
        buf: &mut Vec<sqlx::sqlite::SqliteArgumentValue<'q>>,
    ) -> Result<sqlx::encode::IsNull, Box<dyn std::error::Error + Send + Sync>> {
        sqlx::Encode::<sqlx::Sqlite>::encode_by_ref(&self.0, buf)
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for DownloadId {
    fn decode(value: sqlx::sqlite::SqliteValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let id = <i64 as sqlx::Decode<sqlx::Sqlite>>::decode(value)?;
        Ok(Self(id))
    }
}

/// Download status
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Queued and waiting to start
    Queued,
    /// Currently downloading
    Downloading,
    /// Paused by user
    Paused,
    /// Post-processing (verify/repair/extract)
    Processing,
    /// Successfully completed
    Complete,
    /// Failed with error
    Failed,
}

impl Status {
    /// Convert integer status code to Status enum
    pub fn from_i32(status: i32) -> Self {
        match status {
            0 => Status::Queued,
            1 => Status::Downloading,
            2 => Status::Paused,
            3 => Status::Processing,
            4 => Status::Complete,
            5 => Status::Failed,
            _ => Status::Failed, // Default to Failed for unknown status
        }
    }

    /// Convert Status enum to integer status code
    pub fn to_i32(&self) -> i32 {
        match self {
            Status::Queued => 0,
            Status::Downloading => 1,
            Status::Paused => 2,
            Status::Processing => 3,
            Status::Complete => 4,
            Status::Failed => 5,
        }
    }
}

/// Download priority
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Low priority (-1)
    Low = -1,
    /// Normal priority (0)
    #[default]
    Normal = 0,
    /// High priority (1)
    High = 1,
    /// Force start immediately (2)
    Force = 2,
}

impl Priority {
    /// Convert integer priority code to Priority enum
    pub fn from_i32(priority: i32) -> Self {
        match priority {
            -1 => Priority::Low,
            0 => Priority::Normal,
            1 => Priority::High,
            2 => Priority::Force,
            _ => Priority::Normal, // Default to Normal for unknown priority
        }
    }
}

/// Post-processing stage
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    /// Download stage
    Download,
    /// PAR2 verification
    Verify,
    /// PAR2 repair
    Repair,
    /// Archive extraction
    Extract,
    /// Move to final destination
    Move,
    /// Cleanup intermediate files
    Cleanup,
    /// DirectUnpack (extraction during download)
    DirectUnpack,
}

/// Archive type detected by file extension
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveType {
    /// RAR archive (.rar, .r00, .r01, etc.)
    Rar,
    /// 7-Zip archive (.7z)
    SevenZip,
    /// ZIP archive (.zip)
    Zip,
}

/// Event emitted during download lifecycle
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Download added to queue
    Queued {
        /// Download ID
        id: DownloadId,
        /// Download name
        name: String,
    },

    /// Download removed from queue
    Removed {
        /// Download ID
        id: DownloadId,
    },

    /// Download progress update
    Downloading {
        /// Download ID
        id: DownloadId,
        /// Progress percentage (0.0 to 100.0)
        percent: f32,
        /// Current speed in bytes per second
        speed_bps: u64,
        /// Number of articles that have failed so far
        #[serde(skip_serializing_if = "Option::is_none")]
        failed_articles: Option<u64>,
        /// Total number of articles in the download
        #[serde(skip_serializing_if = "Option::is_none")]
        total_articles: Option<u64>,
        /// Download health percentage (100.0 = no failures, 0.0 = all failed)
        #[serde(skip_serializing_if = "Option::is_none")]
        health_percent: Option<f32>,
    },

    /// Download completed (starting post-processing)
    DownloadComplete {
        /// Download ID
        id: DownloadId,
        /// Number of articles that failed during download
        #[serde(skip_serializing_if = "Option::is_none")]
        articles_failed: Option<u64>,
        /// Total number of articles in the download
        #[serde(skip_serializing_if = "Option::is_none")]
        articles_total: Option<u64>,
    },

    /// Download failed
    DownloadFailed {
        /// Download ID
        id: DownloadId,
        /// Error message
        error: String,
        /// Number of articles that succeeded before failure
        #[serde(skip_serializing_if = "Option::is_none")]
        articles_succeeded: Option<u64>,
        /// Number of articles that failed
        #[serde(skip_serializing_if = "Option::is_none")]
        articles_failed: Option<u64>,
        /// Total number of articles in the download
        #[serde(skip_serializing_if = "Option::is_none")]
        articles_total: Option<u64>,
    },

    /// PAR2 verification started
    Verifying {
        /// Download ID
        id: DownloadId,
    },

    /// PAR2 verification completed
    VerifyComplete {
        /// Download ID
        id: DownloadId,
        /// Whether files are damaged
        damaged: bool,
    },

    /// PAR2 repair started
    Repairing {
        /// Download ID
        id: DownloadId,
        /// Blocks needed for repair
        blocks_needed: u32,
        /// Blocks available
        blocks_available: u32,
    },

    /// PAR2 repair completed
    RepairComplete {
        /// Download ID
        id: DownloadId,
        /// Whether repair was successful
        success: bool,
    },

    /// PAR2 repair skipped (not supported or not needed)
    RepairSkipped {
        /// Download ID
        id: DownloadId,
        /// Reason for skipping
        reason: String,
    },

    /// Archive extraction started
    Extracting {
        /// Download ID
        id: DownloadId,
        /// Archive filename
        archive: String,
        /// Extraction progress (0.0 to 100.0)
        percent: f32,
    },

    /// Archive extraction completed
    ExtractComplete {
        /// Download ID
        id: DownloadId,
    },

    /// Moving files to destination
    Moving {
        /// Download ID
        id: DownloadId,
        /// Destination path
        destination: PathBuf,
    },

    /// Cleaning up intermediate files
    Cleaning {
        /// Download ID
        id: DownloadId,
    },

    /// Download fully complete
    Complete {
        /// Download ID
        id: DownloadId,
        /// Final path
        path: PathBuf,
    },

    /// Download failed at some stage
    Failed {
        /// Download ID
        id: DownloadId,
        /// Stage where failure occurred
        stage: Stage,
        /// Error message
        error: String,
        /// Whether files were kept
        files_kept: bool,
    },

    /// Speed limit changed
    SpeedLimitChanged {
        /// New limit in bytes per second (None = unlimited)
        limit_bps: Option<u64>,
    },

    /// Queue paused
    QueuePaused,

    /// Queue resumed
    QueueResumed,

    /// Webhook delivery failed
    WebhookFailed {
        /// Webhook URL
        url: String,
        /// Error message
        error: String,
    },

    /// Script execution failed
    ScriptFailed {
        /// Script path
        script: PathBuf,
        /// Exit code (if available)
        exit_code: Option<i32>,
    },

    /// Duplicate download detected
    DuplicateDetected {
        /// Existing download ID that matches
        id: DownloadId,
        /// Name of the new download attempt
        name: String,
        /// Detection method used
        method: DuplicateMethod,
        /// Name of the existing download
        existing_name: String,
    },

    /// DirectUnpack coordinator started for a download
    DirectUnpackStarted {
        /// Download ID
        id: DownloadId,
    },

    /// An individual file within a download has completed (all segments downloaded)
    FileCompleted {
        /// Download ID
        id: DownloadId,
        /// File index within the NZB
        file_index: i32,
        /// Filename
        filename: String,
    },

    /// DirectUnpack is extracting a completed RAR archive
    DirectUnpackExtracting {
        /// Download ID
        id: DownloadId,
        /// Archive filename being extracted
        filename: String,
    },

    /// DirectUnpack successfully extracted an archive
    DirectUnpackExtracted {
        /// Download ID
        id: DownloadId,
        /// Archive filename that was extracted
        filename: String,
        /// List of extracted file paths
        extracted_files: Vec<String>,
    },

    /// DirectUnpack was cancelled (typically due to article failures)
    DirectUnpackCancelled {
        /// Download ID
        id: DownloadId,
        /// Reason for cancellation
        reason: String,
    },

    /// DirectUnpack completed successfully for a download
    DirectUnpackComplete {
        /// Download ID
        id: DownloadId,
    },

    /// DirectRename renamed a file using PAR2 metadata
    DirectRenamed {
        /// Download ID
        id: DownloadId,
        /// Original (obfuscated) filename
        old_name: String,
        /// New (correct) filename from PAR2 metadata
        new_name: String,
    },

    /// Graceful shutdown initiated
    Shutdown,
}

/// Information about a download in the queue
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DownloadInfo {
    /// Unique download identifier
    pub id: DownloadId,

    /// Download name (from NZB filename)
    pub name: String,

    /// Category (if assigned)
    pub category: Option<String>,

    /// Current status
    pub status: Status,

    /// Progress percentage (0.0 to 100.0)
    pub progress: f32,

    /// Current download speed in bytes per second
    pub speed_bps: u64,

    /// Total size in bytes
    pub size_bytes: u64,

    /// Downloaded bytes so far
    pub downloaded_bytes: u64,

    /// Estimated time to completion in seconds (None if unknown)
    pub eta_seconds: Option<u64>,

    /// Download priority
    pub priority: Priority,

    /// When the download was added to the queue
    pub created_at: DateTime<Utc>,

    /// When the download started (None if not started yet)
    pub started_at: Option<DateTime<Utc>>,
}

/// Options for adding a download to the queue
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct DownloadOptions {
    /// Category to assign (None = use default category)
    #[serde(default)]
    pub category: Option<String>,

    /// Override default destination directory
    #[serde(default)]
    pub destination: Option<PathBuf>,

    /// Override default post-processing mode
    #[serde(default)]
    pub post_process: Option<PostProcess>,

    /// Download priority
    #[serde(default)]
    pub priority: Priority,

    /// Password for this specific download (high priority)
    #[serde(default)]
    pub password: Option<String>,
}

/// Historical download record
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct HistoryEntry {
    /// Unique download identifier
    pub id: i64,

    /// Download name
    pub name: String,

    /// Category (if assigned)
    pub category: Option<String>,

    /// Final destination path (if completed successfully)
    pub destination: Option<PathBuf>,

    /// Final status (Complete or Failed)
    pub status: Status,

    /// Total size in bytes
    pub size_bytes: u64,

    /// Time spent downloading (not including queue wait time)
    pub download_time: Duration,

    /// When the download completed (successfully or failed)
    pub completed_at: DateTime<Utc>,
}

/// Queue statistics
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct QueueStats {
    /// Total number of downloads in queue
    pub total: usize,

    /// Number of queued downloads (waiting to start)
    pub queued: usize,

    /// Number of actively downloading
    pub downloading: usize,

    /// Number of paused downloads
    pub paused: usize,

    /// Number of downloads in post-processing
    pub processing: usize,

    /// Total download speed across all active downloads (bytes per second)
    pub total_speed_bps: u64,

    /// Total size of all downloads in queue (bytes)
    pub total_size_bytes: u64,

    /// Total downloaded bytes across all downloads
    pub downloaded_bytes: u64,

    /// Overall queue progress (0.0 to 100.0)
    pub overall_progress: f32,

    /// Current speed limit (None = unlimited)
    pub speed_limit_bps: Option<u64>,

    /// Whether queue is accepting new downloads
    pub accepting_new: bool,
}

/// Information about a detected duplicate download
#[derive(Clone, Debug)]
pub struct DuplicateInfo {
    /// Detection method that found the duplicate
    pub method: crate::config::DuplicateMethod,

    /// ID of the existing download
    pub existing_id: DownloadId,

    /// Name of the existing download
    pub existing_name: String,
}

/// Payload sent to webhooks
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct WebhookPayload {
    /// Event type (complete, failed, queued)
    pub event: String,

    /// Download ID
    pub download_id: DownloadId,

    /// Download name
    pub name: String,

    /// Category (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,

    /// Download status
    pub status: String,

    /// Final destination path (for complete downloads)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<PathBuf>,

    /// Error message (for failed downloads)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Timestamp of the event (Unix timestamp in seconds)
    pub timestamp: i64,
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    // --- Status integer encoding ---

    #[test]
    fn status_round_trips_through_i32_for_all_variants() {
        let cases = [
            (Status::Queued, 0),
            (Status::Downloading, 1),
            (Status::Paused, 2),
            (Status::Processing, 3),
            (Status::Complete, 4),
            (Status::Failed, 5),
        ];

        for (variant, expected_int) in cases {
            assert_eq!(
                variant.to_i32(),
                expected_int,
                "{variant:?} should encode to {expected_int}"
            );
            assert_eq!(
                Status::from_i32(expected_int),
                variant,
                "{expected_int} should decode to {variant:?}"
            );
        }
    }

    #[test]
    fn status_from_unknown_positive_integer_defaults_to_failed() {
        assert_eq!(
            Status::from_i32(99),
            Status::Failed,
            "unknown status 99 must fall back to Failed so corrupted DB rows surface visibly"
        );
    }

    #[test]
    fn status_from_negative_integer_defaults_to_failed() {
        assert_eq!(
            Status::from_i32(-1),
            Status::Failed,
            "negative status must fall back to Failed — not silently become Queued"
        );
    }

    // --- Priority integer encoding ---

    #[test]
    fn priority_round_trips_through_i32_for_all_variants() {
        let cases = [
            (Priority::Low, -1),
            (Priority::Normal, 0),
            (Priority::High, 1),
            (Priority::Force, 2),
        ];

        for (variant, expected_int) in cases {
            // from_i32 → variant
            assert_eq!(
                Priority::from_i32(expected_int),
                variant,
                "{expected_int} should decode to {variant:?}"
            );
            // variant discriminant → expected_int
            assert_eq!(
                variant as i32, expected_int,
                "{variant:?} discriminant should be {expected_int}"
            );
        }
    }

    #[test]
    fn priority_from_unknown_integer_defaults_to_normal() {
        assert_eq!(
            Priority::from_i32(99),
            Priority::Normal,
            "unknown priority must default to Normal, not High or Force"
        );
        assert_eq!(
            Priority::from_i32(-100),
            Priority::Normal,
            "large negative priority must default to Normal"
        );
    }

    // --- DownloadId conversions ---

    #[test]
    fn download_id_from_i64_and_back() {
        let id = DownloadId::from(42_i64);
        let raw: i64 = id.into();
        assert_eq!(
            raw, 42,
            "round-trip through From<i64>/Into<i64> must preserve value"
        );
    }

    #[test]
    fn download_id_from_str_parses_valid_integer() {
        let id = DownloadId::from_str("123").unwrap();
        assert_eq!(id.get(), 123);
    }

    #[test]
    fn download_id_from_str_parses_negative_integer() {
        let id = DownloadId::from_str("-7").unwrap();
        assert_eq!(
            id.get(),
            -7,
            "DownloadId wraps i64 and must accept negatives"
        );
    }

    #[test]
    fn download_id_from_str_rejects_non_numeric() {
        let result = DownloadId::from_str("abc");
        assert!(result.is_err(), "non-numeric string must fail to parse");
        // Verify the error is actually a ParseIntError (not some other error)
        let err = result.unwrap_err();
        // ParseIntError's Display always contains the failing input context
        let msg = err.to_string();
        assert!(
            !msg.is_empty(),
            "ParseIntError should have a descriptive message, got empty"
        );
    }

    #[test]
    fn download_id_from_str_rejects_empty_string() {
        assert!(
            DownloadId::from_str("").is_err(),
            "empty string must not parse to a DownloadId"
        );
    }

    #[test]
    fn download_id_from_str_rejects_float() {
        assert!(
            DownloadId::from_str("3.14").is_err(),
            "float string must not parse as DownloadId"
        );
    }

    #[test]
    fn download_id_display_matches_inner_value() {
        let id = DownloadId::new(999);
        assert_eq!(
            id.to_string(),
            "999",
            "Display should produce the raw i64 value"
        );
    }

    #[test]
    fn download_id_display_for_negative() {
        let id = DownloadId::new(-42);
        assert_eq!(
            id.to_string(),
            "-42",
            "Display must include the minus sign for negatives"
        );
    }

    #[test]
    fn download_id_partial_eq_with_i64() {
        let id = DownloadId::new(10);
        assert!(id == 10_i64, "DownloadId should equal matching i64");
        assert!(
            10_i64 == id,
            "i64 should equal matching DownloadId (symmetric)"
        );
        assert!(id != 11_i64, "DownloadId should not equal different i64");
    }

    // --- DownloadId parsing edge cases ---

    #[test]
    fn download_id_from_str_rejects_whitespace_padded_input() {
        // i64::from_str is strict and does not trim — verify DownloadId inherits this
        assert!(
            DownloadId::from_str(" 123 ").is_err(),
            "whitespace-padded string must not parse — API callers must trim before parsing"
        );
        assert!(
            DownloadId::from_str(" 123").is_err(),
            "leading whitespace must be rejected"
        );
        assert!(
            DownloadId::from_str("123 ").is_err(),
            "trailing whitespace must be rejected"
        );
    }

    #[test]
    fn download_id_from_str_parses_leading_zeros_as_decimal() {
        // i64::from_str treats leading zeros as plain decimal (not octal)
        let id = DownloadId::from_str("0000123").unwrap();
        assert_eq!(
            id.get(),
            123,
            "leading zeros should parse as decimal 123, not be rejected or treated as octal"
        );
    }

    #[test]
    fn download_id_from_str_rejects_i64_overflow_without_panic() {
        // i64::MAX = 9223372036854775807, so i64::MAX + 1 must fail gracefully
        let result = DownloadId::from_str("9223372036854775808");
        assert!(
            result.is_err(),
            "i64::MAX + 1 must produce an error, not wrap or panic"
        );
        // Verify the error is a ParseIntError with a meaningful message
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("too large") || msg.contains("overflow") || msg.contains("number"),
            "error message should indicate overflow, got: {msg}"
        );
    }

    #[test]
    fn download_id_from_str_rejects_negative_overflow_without_panic() {
        // i64::MIN = -9223372036854775808, so i64::MIN - 1 must fail gracefully
        let result = DownloadId::from_str("-9223372036854775809");
        assert!(
            result.is_err(),
            "i64::MIN - 1 must produce an error, not wrap or panic"
        );
    }
}

/// Result of a server connectivity test
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ServerTestResult {
    /// Whether the test was successful
    pub success: bool,

    /// Latency to connect and authenticate (if successful)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency: Option<Duration>,

    /// Error message (if failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Server capabilities (if successful)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ServerCapabilities>,
}

/// NNTP server capabilities discovered during testing
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ServerCapabilities {
    /// Whether posting is allowed
    pub posting_allowed: bool,

    /// Maximum number of connections (if advertised by server)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<u32>,

    /// Whether compression is supported (e.g., XZVER)
    pub compression: bool,
}

/// Overall system capabilities for post-processing features
///
/// This struct provides information about what features are available
/// based on the current configuration and available external tools.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Capabilities {
    /// PAR2 parity checking and repair capabilities
    pub parity: ParityCapabilitiesInfo,
}

/// Information about PAR2 parity capabilities
///
/// This struct wraps the core parity capabilities with additional
/// metadata for API consumers.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ParityCapabilitiesInfo {
    /// Whether PAR2 verification is available
    pub can_verify: bool,

    /// Whether PAR2 repair is available
    pub can_repair: bool,

    /// Name of the parity handler implementation in use
    pub handler: String,
}
