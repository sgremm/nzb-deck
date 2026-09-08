//! Configuration types for usenet-dl

use crate::types::Priority;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, path::PathBuf, time::Duration};
use utoipa::ToSchema;

/// Download behavior configuration (directories, concurrency, post-processing)
///
/// Groups settings related to how downloads are fetched, stored, and processed.
/// Used as a nested sub-config within [`Config`].
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DownloadConfig {
    /// Download directory (default: "./downloads")
    #[serde(default = "default_download_dir")]
    pub download_dir: PathBuf,

    /// Temporary directory (default: "./temp")
    #[serde(default = "default_temp_dir")]
    pub temp_dir: PathBuf,

    /// Maximum concurrent downloads (default: 3)
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_downloads: usize,

    /// Speed limit in bytes per second (None = unlimited)
    #[serde(default)]
    pub speed_limit_bps: Option<u64>,

    /// Default post-processing mode
    #[serde(default)]
    pub default_post_process: PostProcess,

    /// Delete sample files/folders
    #[serde(default = "default_true")]
    pub delete_samples: bool,

    /// File collision handling
    #[serde(default)]
    pub file_collision: FileCollisionAction,

    /// Maximum article failure ratio before considering a download failed (default: 0.5 = 50%)
    ///
    /// When the ratio of failed articles to total articles exceeds this threshold,
    /// the download is marked as failed. Otherwise, post-processing (PAR2 repair)
    /// is attempted.
    #[serde(default = "default_max_failure_ratio")]
    pub max_failure_ratio: f64,

    /// Fast-fail threshold — abort early if this fraction of a sample is missing (default: 0.8 = 80%)
    ///
    /// After `fast_fail_sample_size` articles have been attempted, if the failure ratio
    /// meets or exceeds this threshold, the download is cancelled immediately to avoid
    /// wasting bandwidth on mostly-expired NZBs.
    #[serde(default = "default_fast_fail_threshold")]
    pub fast_fail_threshold: f64,

    /// Number of articles to sample before evaluating the fast-fail heuristic (default: 10)
    #[serde(default = "default_fast_fail_sample_size")]
    pub fast_fail_sample_size: usize,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            download_dir: default_download_dir(),
            temp_dir: default_temp_dir(),
            max_concurrent_downloads: default_max_concurrent(),
            speed_limit_bps: None,
            default_post_process: PostProcess::default(),
            delete_samples: true,
            file_collision: FileCollisionAction::default(),
            max_failure_ratio: default_max_failure_ratio(),
            fast_fail_threshold: default_fast_fail_threshold(),
            fast_fail_sample_size: default_fast_fail_sample_size(),
        }
    }
}

/// External tool paths (unrar, 7z, par2) and password configuration
///
/// Groups settings for external binaries and password handling.
/// Used as a nested sub-config within [`Config`].
///
/// ## Custom parity handler
///
/// Callers can inject their own [`ParityHandler`](crate::parity::ParityHandler)
/// via [`parity_handler`](Self::parity_handler). When set, it takes priority
/// over `par2_path` and PATH discovery. When `None` (the default), the
/// existing auto-detection logic applies.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolsConfig {
    /// Path to global password file (one password per line)
    #[serde(default)]
    pub password_file: Option<PathBuf>,

    /// Try empty password as fallback
    #[serde(default = "default_true")]
    pub try_empty_password: bool,

    /// Path to unrar executable (auto-detected if None)
    #[serde(default)]
    pub unrar_path: Option<PathBuf>,

    /// Path to 7z executable (auto-detected if None)
    #[serde(default)]
    pub sevenzip_path: Option<PathBuf>,

    /// Path to par2 executable (auto-detected if None)
    #[serde(default)]
    pub par2_path: Option<PathBuf>,

    /// Whether to search PATH for external binaries if explicit paths not set (default: true)
    #[serde(default = "default_true")]
    pub search_path: bool,

    /// Optional pre-built parity handler to use instead of auto-detection.
    ///
    /// When `Some`, this handler is used directly and `par2_path` / PATH
    /// discovery are skipped. When `None`, the downloader falls back to its
    /// existing auto-detection logic.
    #[serde(skip)]
    #[schema(ignore)]
    pub parity_handler: Option<std::sync::Arc<dyn crate::parity::ParityHandler>>,
}

impl std::fmt::Debug for ToolsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolsConfig")
            .field("password_file", &self.password_file)
            .field("try_empty_password", &self.try_empty_password)
            .field("unrar_path", &self.unrar_path)
            .field("sevenzip_path", &self.sevenzip_path)
            .field("par2_path", &self.par2_path)
            .field("search_path", &self.search_path)
            .field(
                "parity_handler",
                &self
                    .parity_handler
                    .as_ref()
                    .map(|h| h.name()),
            )
            .finish()
    }
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            password_file: None,
            try_empty_password: true,
            unrar_path: None,
            sevenzip_path: None,
            par2_path: None,
            search_path: true,
            parity_handler: None,
        }
    }
}

/// Notification configuration (webhooks and scripts)
///
/// Groups settings for external notifications triggered by download events.
/// Used as a nested sub-config within [`Config`].
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct NotificationConfig {
    /// Webhook configurations
    #[serde(default)]
    pub webhooks: Vec<WebhookConfig>,

    /// Script configurations
    #[serde(default)]
    pub scripts: Vec<ScriptConfig>,
}

/// Main configuration for UsenetDownloader
///
/// Fields are organized into logical sub-configs for maintainability:
/// - [`download`](DownloadConfig) — directories, concurrency, post-processing
/// - [`tools`](ToolsConfig) — external binary paths, password handling
/// - [`notifications`](NotificationConfig) — webhooks and scripts
///
/// All sub-config fields are flattened for backward-compatible serialization,
/// meaning the JSON/TOML format remains unchanged (no nesting).
/// Individual fields are also accessible directly on `Config` via `Deref`-style
/// accessor methods for convenience.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct Config {
    /// NNTP server configurations (at least one required)
    pub servers: Vec<ServerConfig>,

    /// Download behavior settings (directories, concurrency, post-processing)
    #[serde(flatten)]
    pub download: DownloadConfig,

    /// External tool paths and password handling
    #[serde(flatten)]
    pub tools: ToolsConfig,

    /// Notification settings (webhooks and scripts)
    #[serde(flatten)]
    pub notifications: NotificationConfig,

    /// Content pipeline processing (extraction, cleanup, validation)
    #[serde(flatten)]
    pub processing: ProcessingConfig,

    /// Data storage and state management
    pub persistence: PersistenceConfig,

    /// Automated content discovery and ingestion
    #[serde(flatten)]
    pub automation: AutomationConfig,

    /// API and external server integration
    #[serde(flatten)]
    pub server: ServerIntegrationConfig,
}

// Convenience accessors — allow existing code to use `config.download_dir` etc.
// without changing call sites. These delegate to the sub-config structs.
impl Config {
    /// Download directory
    pub fn download_dir(&self) -> &PathBuf {
        &self.download.download_dir
    }

    /// Temporary directory
    pub fn temp_dir(&self) -> &PathBuf {
        &self.download.temp_dir
    }
}

/// NNTP server configuration
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ServerConfig {
    /// Server hostname
    pub host: String,

    /// Server port (typically 119 for unencrypted, 563 for TLS)
    pub port: u16,

    /// Use TLS (implicit TLS, not STARTTLS)
    pub tls: bool,

    /// Username for authentication
    pub username: Option<String>,

    /// Password for authentication
    pub password: Option<String>,

    /// Number of connections to maintain
    #[serde(default = "default_connections")]
    pub connections: usize,

    /// Server priority (lower = tried first, for backup servers)
    #[serde(default)]
    pub priority: i32,

    /// Number of ARTICLE commands to pipeline per connection (default: 10)
    ///
    /// Pipelining sends multiple ARTICLE commands before reading responses,
    /// reducing round-trip latency. Higher values can improve throughput but
    /// may increase memory usage. Set to 1 to disable pipelining (sequential mode).
    ///
    /// Recommended values: 5-20 depending on network latency and server capabilities.
    #[serde(default = "default_pipeline_depth")]
    pub pipeline_depth: usize,
}

/// Retry configuration for transient failures
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (default: 5)
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,

    /// Initial delay before first retry (default: 1 second)
    #[serde(default = "default_initial_delay", with = "duration_serde")]
    pub initial_delay: Duration,

    /// Maximum delay between retries (default: 60 seconds)
    #[serde(default = "default_max_delay", with = "duration_serde")]
    pub max_delay: Duration,

    /// Multiplier for exponential backoff (default: 2.0)
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,

    /// Add random jitter to delays (default: true)
    #[serde(default = "default_true")]
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            jitter: true,
        }
    }
}

/// Post-processing mode
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PostProcess {
    /// Just download, no post-processing
    None,
    /// Download + PAR2 verify
    Verify,
    /// Download + PAR2 verify/repair
    Repair,
    /// Above + extract archives
    Unpack,
    /// Above + remove intermediate files (default)
    #[default]
    UnpackAndCleanup,
}

impl PostProcess {
    /// Convert PostProcess enum to integer for database storage
    pub fn to_i32(&self) -> i32 {
        match self {
            PostProcess::None => 0,
            PostProcess::Verify => 1,
            PostProcess::Repair => 2,
            PostProcess::Unpack => 3,
            PostProcess::UnpackAndCleanup => 4,
        }
    }

    /// Convert integer from database to PostProcess enum
    pub fn from_i32(value: i32) -> Self {
        match value {
            0 => PostProcess::None,
            1 => PostProcess::Verify,
            2 => PostProcess::Repair,
            3 => PostProcess::Unpack,
            4 => PostProcess::UnpackAndCleanup,
            _ => PostProcess::UnpackAndCleanup, // Default
        }
    }
}

/// Archive extraction configuration
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ExtractionConfig {
    /// Maximum depth for nested archive extraction (default: 2)
    #[serde(default = "default_max_recursion")]
    pub max_recursion_depth: u32,

    /// File extensions to treat as archives
    #[serde(default = "default_archive_extensions")]
    pub archive_extensions: Vec<String>,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            max_recursion_depth: 2,
            archive_extensions: default_archive_extensions(),
        }
    }
}

/// File collision handling strategy
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileCollisionAction {
    /// Append (1), (2), etc. to filename (default)
    #[default]
    Rename,
    /// Overwrite existing file
    Overwrite,
    /// Skip the file, keep existing
    Skip,
}

/// Obfuscated filename detection and renaming configuration
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DeobfuscationConfig {
    /// Enable automatic deobfuscation (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Minimum filename length to consider for deobfuscation (default: 12)
    #[serde(default = "default_min_length")]
    pub min_length: usize,
}

impl Default for DeobfuscationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_length: 12,
        }
    }
}

/// Duplicate detection configuration
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DuplicateConfig {
    /// Enable duplicate detection (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// What to do when duplicate detected
    #[serde(default)]
    pub action: DuplicateAction,

    /// Check methods (in order)
    #[serde(default = "default_duplicate_methods")]
    pub methods: Vec<DuplicateMethod>,
}

impl Default for DuplicateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            action: DuplicateAction::default(),
            methods: default_duplicate_methods(),
        }
    }
}

/// Action to take when duplicate detected
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateAction {
    /// Block the download entirely
    Block,
    /// Allow but emit warning event (default)
    #[default]
    Warn,
    /// Allow silently
    Allow,
}

/// Duplicate detection method
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateMethod {
    /// NZB content hash (most reliable)
    NzbHash,
    /// NZB filename
    NzbName,
    /// Extracted job name (deobfuscated)
    JobName,
}

/// Disk space checking configuration
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DiskSpaceConfig {
    /// Enable disk space checking (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Minimum free space to maintain (default: 1 GB)
    #[serde(default = "default_min_free_space")]
    pub min_free_space: u64,

    /// Multiplier for estimated size (default: 2.5)
    #[serde(default = "default_size_multiplier")]
    pub size_multiplier: f64,
}

impl Default for DiskSpaceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_free_space: 1024 * 1024 * 1024, // 1 GB
            size_multiplier: 2.5,
        }
    }
}

/// Cleanup configuration for intermediate files
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct CleanupConfig {
    /// Enable cleanup of intermediate files (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// File extensions to remove (.par2, .nzb, .sfv, .srr, etc.)
    #[serde(default = "default_cleanup_extensions")]
    pub target_extensions: Vec<String>,

    /// Archive extensions to remove after extraction
    #[serde(default = "default_archive_extensions")]
    pub archive_extensions: Vec<String>,

    /// Delete sample folders (default: true from Config.delete_samples)
    #[serde(default = "default_true")]
    pub delete_samples: bool,

    /// Sample folder names to detect (case-insensitive)
    #[serde(default = "default_sample_folder_names")]
    pub sample_folder_names: Vec<String>,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            target_extensions: default_cleanup_extensions(),
            archive_extensions: default_archive_extensions(),
            delete_samples: true,
            sample_folder_names: default_sample_folder_names(),
        }
    }
}

/// DirectUnpack configuration — extract archives while download is in progress
///
/// When enabled, completed RAR files are extracted as soon as all their segments
/// finish downloading, overlapping extraction with the remaining download.
/// Combined with DirectRename, which uses PAR2 metadata to fix obfuscated
/// filenames before extraction.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct DirectUnpackConfig {
    /// Enable DirectUnpack — extract archives during download (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Enable DirectRename — use PAR2 metadata to fix obfuscated filenames (default: false)
    ///
    /// Requires PAR2 files to be downloaded early. When enabled, PAR2 file articles
    /// are prioritized in the download queue.
    #[serde(default)]
    pub direct_rename: bool,

    /// How often to poll for newly completed files, in milliseconds (default: 200)
    #[serde(default = "default_direct_unpack_poll_interval")]
    pub poll_interval_ms: u64,
}

impl Default for DirectUnpackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            direct_rename: false,
            poll_interval_ms: default_direct_unpack_poll_interval(),
        }
    }
}

/// Content pipeline processing configuration
///
/// Groups settings related to post-download file processing, validation,
/// and cleanup. All settings in this config are used together during the
/// post-processing pipeline.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ProcessingConfig {
    /// Archive extraction configuration
    #[serde(default)]
    pub extraction: ExtractionConfig,

    /// Duplicate detection configuration (pre-download validation)
    #[serde(default)]
    pub duplicate: DuplicateConfig,

    /// Disk space checking configuration (pre-download validation)
    #[serde(default)]
    pub disk_space: DiskSpaceConfig,

    /// Retry configuration for transient failures
    #[serde(default)]
    pub retry: RetryConfig,

    /// Cleanup configuration for intermediate files
    #[serde(default)]
    pub cleanup: CleanupConfig,

    /// DirectUnpack — extract archives while download is still in progress
    #[serde(default)]
    pub direct_unpack: DirectUnpackConfig,
}

/// Automated content discovery and ingestion configuration
///
/// Groups settings related to automated content sources (RSS, watch folders)
/// and content naming intelligence (deobfuscation).
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct AutomationConfig {
    /// RSS feed configurations
    #[serde(default)]
    pub rss_feeds: Vec<RssFeedConfig>,

    /// Watch folders for auto-importing NZBs
    #[serde(default)]
    pub watch_folders: Vec<WatchFolderConfig>,

    /// Filename deobfuscation configuration
    #[serde(default)]
    pub deobfuscation: DeobfuscationConfig,
}

/// Data storage and state management configuration
///
/// Groups settings related to persistence, state, and runtime-mutable
/// configurations.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct PersistenceConfig {
    /// Database path (default: "./usenet-dl.db")
    #[serde(default = "default_database_path")]
    pub database_path: PathBuf,

    /// Schedule rules for time-based speed limits
    #[serde(default)]
    pub schedule_rules: Vec<ScheduleRule>,

    /// Category configurations
    #[serde(default)]
    pub categories: HashMap<String, CategoryConfig>,
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self {
            database_path: default_database_path(),
            schedule_rules: vec![],
            categories: HashMap::new(),
        }
    }
}

/// API and external server integration configuration
///
/// Groups settings for external access and control interfaces.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ServerIntegrationConfig {
    /// REST API configuration
    #[serde(default)]
    pub api: ApiConfig,
}

/// REST API configuration
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ApiConfig {
    /// Address to bind to (default: 127.0.0.1:6789)
    #[serde(default = "default_bind_address")]
    pub bind_address: SocketAddr,

    /// Optional API key for authentication
    #[serde(default)]
    pub api_key: Option<String>,

    /// Enable CORS for browser access (default: true)
    #[serde(default = "default_true")]
    pub cors_enabled: bool,

    /// Allowed CORS origins (default: ["*"])
    #[serde(default = "default_cors_origins")]
    pub cors_origins: Vec<String>,

    /// Enable Swagger UI at /swagger-ui (default: true)
    #[serde(default = "default_true")]
    pub swagger_ui: bool,

    /// Rate limiting configuration
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            api_key: None,
            cors_enabled: true,
            cors_origins: default_cors_origins(),
            swagger_ui: true,
            rate_limit: RateLimitConfig::default(),
        }
    }
}

/// Rate limiting configuration
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct RateLimitConfig {
    /// Enable rate limiting (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Requests per second per IP (default: 100)
    #[serde(default = "default_requests_per_second")]
    pub requests_per_second: u32,

    /// Burst size (default: 200)
    #[serde(default = "default_burst_size")]
    pub burst_size: u32,

    /// Endpoints exempt from rate limiting
    #[serde(default = "default_exempt_paths")]
    pub exempt_paths: Vec<String>,

    /// IPs exempt from rate limiting (e.g., localhost)
    #[serde(default = "default_exempt_ips")]
    pub exempt_ips: Vec<std::net::IpAddr>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_per_second: 100,
            burst_size: 200,
            exempt_paths: default_exempt_paths(),
            exempt_ips: default_exempt_ips(),
        }
    }
}

/// Schedule rule for time-based actions
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ScheduleRule {
    /// Human-readable name
    pub name: String,

    /// Days this rule applies (empty = all days)
    #[serde(default)]
    pub days: Vec<Weekday>,

    /// Start time (HH:MM)
    pub start_time: String,

    /// End time (HH:MM)
    pub end_time: String,

    /// Action to take during this window
    pub action: ScheduleAction,

    /// Whether rule is active
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Day of week for schedule rules
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum Weekday {
    /// Monday
    Monday,
    /// Tuesday
    Tuesday,
    /// Wednesday
    Wednesday,
    /// Thursday
    Thursday,
    /// Friday
    Friday,
    /// Saturday
    Saturday,
    /// Sunday
    Sunday,
}

/// Action to take during scheduled time window
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScheduleAction {
    /// Set speed limit (bytes per second)
    SpeedLimit {
        /// Speed limit in bytes per second
        limit_bps: u64,
    },
    /// Unlimited speed
    Unlimited,
    /// Pause all downloads
    Pause,
}

/// Watch folder configuration
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct WatchFolderConfig {
    /// Directory to watch for NZB files
    pub path: PathBuf,

    /// What to do with NZB after adding to queue
    #[serde(default)]
    pub after_import: WatchFolderAction,

    /// Category to assign (None = use default)
    #[serde(default)]
    pub category: Option<String>,

    /// Scan interval (default: 5 seconds)
    #[serde(default = "default_scan_interval", with = "duration_serde")]
    pub scan_interval: Duration,
}

/// Action to take with NZB file after import
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WatchFolderAction {
    /// Delete NZB file
    Delete,
    /// Move to a 'processed' subfolder (default)
    #[default]
    MoveToProcessed,
    /// Keep in place
    Keep,
}

/// Webhook configuration
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct WebhookConfig {
    /// URL to POST to
    pub url: String,

    /// Events that trigger this webhook
    pub events: Vec<WebhookEvent>,

    /// Optional authentication header value
    #[serde(default)]
    pub auth_header: Option<String>,

    /// Timeout for webhook requests (default: 30 seconds)
    #[serde(default = "default_webhook_timeout", with = "duration_serde")]
    pub timeout: Duration,
}

/// Webhook trigger event
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum WebhookEvent {
    /// Triggered when a download completes successfully
    OnComplete,
    /// Triggered when a download fails
    OnFailed,
    /// Triggered when a download is queued
    OnQueued,
}

/// Script execution configuration
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ScriptConfig {
    /// Path to script/executable
    pub path: PathBuf,

    /// Events that trigger this script
    pub events: Vec<ScriptEvent>,

    /// Timeout for script execution (default: 5 minutes)
    #[serde(default = "default_script_timeout", with = "duration_serde")]
    pub timeout: Duration,
}

/// Script trigger event
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum ScriptEvent {
    /// Triggered when a download completes successfully
    OnComplete,
    /// Triggered when a download fails
    OnFailed,
    /// Triggered when post-processing completes
    OnPostProcessComplete,
}

/// RSS feed configuration
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct RssFeedConfig {
    /// Feed URL (RSS or Atom)
    pub url: String,

    /// How often to check the feed (default: 15 minutes)
    #[serde(default = "default_rss_check_interval", with = "duration_serde")]
    pub check_interval: Duration,

    /// Category to assign to downloads
    #[serde(default)]
    pub category: Option<String>,

    /// Only download items matching these filters
    #[serde(default)]
    pub filters: Vec<RssFilter>,

    /// Automatically download matches (vs just notify)
    #[serde(default = "default_true")]
    pub auto_download: bool,

    /// Priority for auto-downloaded items
    #[serde(default)]
    pub priority: Priority,

    /// Whether feed is active
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// RSS feed filter
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct RssFilter {
    /// Filter name (for UI)
    pub name: String,

    /// Patterns to include (regex)
    #[serde(default)]
    pub include: Vec<String>,

    /// Patterns to exclude (regex)
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Minimum size (bytes)
    #[serde(default)]
    pub min_size: Option<u64>,

    /// Maximum size (bytes)
    #[serde(default)]
    pub max_size: Option<u64>,

    /// Maximum age from publish date (seconds)
    #[serde(default, with = "optional_duration_serde")]
    pub max_age: Option<Duration>,
}

/// Category configuration
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct CategoryConfig {
    /// Destination directory for this category
    pub destination: PathBuf,

    /// Override default post-processing
    #[serde(default)]
    pub post_process: Option<PostProcess>,

    /// Category-specific scripts
    #[serde(default)]
    pub scripts: Vec<ScriptConfig>,
}

// Default value functions
fn default_download_dir() -> PathBuf {
    PathBuf::from("downloads")
}

fn default_temp_dir() -> PathBuf {
    PathBuf::from("temp")
}

fn default_max_concurrent() -> usize {
    3
}

fn default_database_path() -> PathBuf {
    PathBuf::from("usenet-dl.db")
}

fn default_connections() -> usize {
    10
}

fn default_pipeline_depth() -> usize {
    10
}

fn default_true() -> bool {
    true
}

fn default_max_failure_ratio() -> f64 {
    0.5
}

fn default_fast_fail_threshold() -> f64 {
    0.8
}

fn default_fast_fail_sample_size() -> usize {
    10
}

fn default_max_attempts() -> u32 {
    5
}

fn default_initial_delay() -> Duration {
    Duration::from_secs(1)
}

fn default_max_delay() -> Duration {
    Duration::from_secs(60)
}

fn default_backoff_multiplier() -> f64 {
    2.0
}

fn default_max_recursion() -> u32 {
    2
}

fn default_archive_extensions() -> Vec<String> {
    vec![
        "rar".into(),
        "zip".into(),
        "7z".into(),
        "tar".into(),
        "gz".into(),
        "bz2".into(),
    ]
}

fn default_min_length() -> usize {
    12
}

fn default_duplicate_methods() -> Vec<DuplicateMethod> {
    vec![DuplicateMethod::NzbHash, DuplicateMethod::JobName]
}

fn default_min_free_space() -> u64 {
    1024 * 1024 * 1024 // 1 GB
}

fn default_size_multiplier() -> f64 {
    2.5
}

fn default_bind_address() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 6789))
}

fn default_cors_origins() -> Vec<String> {
    vec!["*".into()]
}

fn default_requests_per_second() -> u32 {
    100
}

fn default_burst_size() -> u32 {
    200
}

fn default_exempt_paths() -> Vec<String> {
    vec![
        "/api/v1/events".to_string(), // SSE is long-lived
        "/api/v1/health".to_string(), // Health checks should always work
    ]
}

fn default_exempt_ips() -> Vec<std::net::IpAddr> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    vec![
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ]
}

fn default_scan_interval() -> Duration {
    Duration::from_secs(5)
}

fn default_webhook_timeout() -> Duration {
    Duration::from_secs(30)
}

fn default_script_timeout() -> Duration {
    Duration::from_secs(300) // 5 minutes
}

fn default_cleanup_extensions() -> Vec<String> {
    vec![
        "par2".into(),
        "PAR2".into(),
        "nzb".into(),
        "NZB".into(),
        "sfv".into(),
        "SFV".into(),
        "srr".into(),
        "SRR".into(),
        "nfo".into(),
        "NFO".into(),
    ]
}

fn default_sample_folder_names() -> Vec<String> {
    vec![
        "sample".into(),
        "Sample".into(),
        "SAMPLE".into(),
        "samples".into(),
        "Samples".into(),
        "SAMPLES".into(),
    ]
}

fn default_rss_check_interval() -> Duration {
    Duration::from_secs(15 * 60) // 15 minutes
}

fn default_direct_unpack_poll_interval() -> u64 {
    200
}

// Duration serialization helper
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

// Optional Duration serialization helper
mod optional_duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match duration {
            Some(d) => serializer.serialize_some(&d.as_secs()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = Option::<u64>::deserialize(deserializer)?;
        Ok(secs.map(Duration::from_secs))
    }
}

// Conversion from our ServerConfig to nntp-rs's ServerConfig
impl From<ServerConfig> for nntp_rs::ServerConfig {
    fn from(config: ServerConfig) -> Self {
        nntp_rs::ServerConfig {
            host: config.host,
            port: config.port,
            tls: config.tls,
            allow_insecure_tls: false,
            username: config.username.unwrap_or_default(),
            password: config.password.unwrap_or_default(),
        }
    }
}

/// Configuration update for runtime-changeable settings
///
/// This struct contains only fields that can be safely updated while the downloader is running.
/// Fields requiring restart (like database_path, download_dir, servers) are not included.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ConfigUpdate {
    /// Speed limit in bytes per second (None = unlimited)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_limit_bps: Option<Option<u64>>,
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rss_feed_serialization() {
        // Test JSON serialization/deserialization
        let feed = RssFeedConfig {
            url: "https://test.com/rss".to_string(),
            check_interval: Duration::from_secs(900),
            category: Some("movies".to_string()),
            filters: vec![],
            auto_download: true,
            priority: Priority::Normal,
            enabled: true,
        };

        let json = serde_json::to_string(&feed).expect("serialize failed");
        let deserialized: RssFeedConfig = serde_json::from_str(&json).expect("deserialize failed");

        assert_eq!(deserialized.url, feed.url);
        assert_eq!(deserialized.check_interval, feed.check_interval);
        assert_eq!(deserialized.category, feed.category);
        assert!(deserialized.auto_download);
        assert_eq!(deserialized.priority, feed.priority);
        assert!(deserialized.enabled);
    }

    // --- PostProcess integer encoding ---

    #[test]
    fn post_process_round_trips_through_i32_for_all_variants() {
        let cases = [
            (PostProcess::None, 0),
            (PostProcess::Verify, 1),
            (PostProcess::Repair, 2),
            (PostProcess::Unpack, 3),
            (PostProcess::UnpackAndCleanup, 4),
        ];

        for (variant, expected_int) in cases {
            assert_eq!(
                variant.to_i32(),
                expected_int,
                "{variant:?} should encode to {expected_int}"
            );
            assert_eq!(
                PostProcess::from_i32(expected_int),
                variant,
                "{expected_int} should decode to {variant:?}"
            );
        }
    }

    #[test]
    fn post_process_from_unknown_integer_defaults_to_unpack_and_cleanup() {
        assert_eq!(
            PostProcess::from_i32(99),
            PostProcess::UnpackAndCleanup,
            "unknown value must default to the safest full-pipeline mode"
        );
        assert_eq!(
            PostProcess::from_i32(-1),
            PostProcess::UnpackAndCleanup,
            "negative value must also default to UnpackAndCleanup"
        );
    }

    // --- ServerConfig → nntp_rs::ServerConfig conversion ---

    #[test]
    fn server_config_converts_with_credentials() {
        let our = ServerConfig {
            host: "news.example.com".to_string(),
            port: 563,
            tls: true,
            username: Some("user1".to_string()),
            password: Some("secret".to_string()),
            connections: 10,
            priority: 0,
            pipeline_depth: 10,
        };

        let nntp: nntp_rs::ServerConfig = our.into();

        assert_eq!(nntp.host, "news.example.com");
        assert_eq!(nntp.port, 563);
        assert!(nntp.tls, "TLS flag must be forwarded");
        assert!(
            !nntp.allow_insecure_tls,
            "insecure TLS must always be false"
        );
        assert_eq!(nntp.username, "user1");
        assert_eq!(nntp.password, "secret");
    }

    #[test]
    fn server_config_converts_without_credentials_to_empty_strings() {
        let our = ServerConfig {
            host: "news.free.example".to_string(),
            port: 119,
            tls: false,
            username: None,
            password: None,
            connections: 5,
            priority: 1,
            pipeline_depth: 10,
        };

        let nntp: nntp_rs::ServerConfig = our.into();

        assert_eq!(nntp.host, "news.free.example");
        assert_eq!(nntp.port, 119);
        assert!(!nntp.tls);
        assert_eq!(
            nntp.username, "",
            "None username must become empty string for nntp-rs"
        );
        assert_eq!(
            nntp.password, "",
            "None password must become empty string for nntp-rs"
        );
    }

    // --- Config JSON round-trip ---

    #[test]
    fn config_default_survives_json_round_trip() {
        let original = Config::default();

        let json = serde_json::to_string(&original).expect("Config must serialize to JSON");
        let restored: Config =
            serde_json::from_str(&json).expect("Config must deserialize from its own JSON");

        // Verify key fields survived — not just "it deserialized"
        assert_eq!(
            restored.download.download_dir, original.download.download_dir,
            "download_dir must survive round-trip"
        );
        assert_eq!(
            restored.download.temp_dir, original.download.temp_dir,
            "temp_dir must survive round-trip"
        );
        assert_eq!(
            restored.download.max_concurrent_downloads, original.download.max_concurrent_downloads,
            "max_concurrent_downloads must survive round-trip"
        );
        assert_eq!(
            restored.download.speed_limit_bps, original.download.speed_limit_bps,
            "speed_limit_bps must survive round-trip"
        );
        assert_eq!(
            restored.download.default_post_process, original.download.default_post_process,
            "default_post_process must survive round-trip"
        );
        assert_eq!(
            restored.persistence.database_path, original.persistence.database_path,
            "database_path must survive round-trip"
        );
        assert_eq!(
            restored.server.api.bind_address, original.server.api.bind_address,
            "api bind_address must survive round-trip"
        );
        assert_eq!(
            restored.processing.retry.max_attempts, original.processing.retry.max_attempts,
            "retry max_attempts must survive round-trip"
        );
        assert_eq!(
            restored.processing.retry.initial_delay, original.processing.retry.initial_delay,
            "retry initial_delay must survive round-trip"
        );
    }

    // --- Duration serde helpers ---

    #[test]
    fn duration_serde_serializes_as_seconds() {
        let config = RetryConfig {
            initial_delay: Duration::from_secs(5),
            max_delay: Duration::from_secs(120),
            ..RetryConfig::default()
        };

        let json = serde_json::to_value(&config).expect("serialize failed");

        assert_eq!(
            json["initial_delay"], 5,
            "duration_serde must serialize Duration as integer seconds"
        );
        assert_eq!(json["max_delay"], 120);
    }

    #[test]
    fn duration_serde_deserializes_from_seconds() {
        let json = r#"{"max_attempts":3,"initial_delay":10,"max_delay":300,"backoff_multiplier":2.0,"jitter":false}"#;

        let config: RetryConfig = serde_json::from_str(json).expect("deserialize failed");

        assert_eq!(
            config.initial_delay,
            Duration::from_secs(10),
            "integer 10 must deserialize to Duration::from_secs(10)"
        );
        assert_eq!(
            config.max_delay,
            Duration::from_secs(300),
            "integer 300 must deserialize to Duration::from_secs(300)"
        );
    }

    #[test]
    fn optional_duration_serde_round_trips_some_value() {
        let filter = RssFilter {
            name: "test".to_string(),
            include: vec![],
            exclude: vec![],
            min_size: None,
            max_size: None,
            max_age: Some(Duration::from_secs(3600)),
        };

        let json = serde_json::to_value(&filter).expect("serialize failed");
        assert_eq!(
            json["max_age"], 3600,
            "Some(Duration) must serialize as integer seconds"
        );

        let restored: RssFilter = serde_json::from_value(json).expect("deserialize failed");
        assert_eq!(restored.max_age, Some(Duration::from_secs(3600)));
    }

    #[test]
    fn optional_duration_serde_round_trips_none() {
        let filter = RssFilter {
            name: "test".to_string(),
            include: vec![],
            exclude: vec![],
            min_size: None,
            max_size: None,
            max_age: None,
        };

        let json = serde_json::to_value(&filter).expect("serialize failed");
        assert!(
            json["max_age"].is_null(),
            "None duration must serialize as null"
        );

        let restored: RssFilter = serde_json::from_value(json).expect("deserialize failed");
        assert_eq!(restored.max_age, None, "null must deserialize back to None");
    }

    // --- ConfigUpdate serialization ---

    #[test]
    fn config_update_none_omits_field_entirely() {
        let update = ConfigUpdate {
            speed_limit_bps: None,
        };

        let json = serde_json::to_value(&update).expect("serialize failed");
        assert!(
            !json.as_object().unwrap().contains_key("speed_limit_bps"),
            "None should be omitted due to skip_serializing_if"
        );
    }

    #[test]
    fn config_update_some_none_serializes_as_null() {
        // Some(None) means "set speed limit to unlimited"
        let update = ConfigUpdate {
            speed_limit_bps: Some(None),
        };

        let json = serde_json::to_value(&update).expect("serialize failed");
        assert!(
            json["speed_limit_bps"].is_null(),
            "Some(None) must serialize as null (= remove limit)"
        );
    }

    #[test]
    fn config_update_some_some_serializes_as_number() {
        // Some(Some(val)) means "set speed limit to val"
        let update = ConfigUpdate {
            speed_limit_bps: Some(Some(10_000_000)),
        };

        let json = serde_json::to_value(&update).expect("serialize failed");
        assert_eq!(
            json["speed_limit_bps"], 10_000_000,
            "Some(Some(10_000_000)) must serialize as the number 10000000"
        );
    }

    #[test]
    fn config_update_deserializes_missing_field_as_none() {
        let json = "{}";
        let update: ConfigUpdate = serde_json::from_str(json).expect("deserialize failed");
        assert!(
            update.speed_limit_bps.is_none(),
            "missing field must become None (= no change requested)"
        );
    }

    #[test]
    fn config_update_deserializes_null_as_none() {
        // Note: without a custom deserializer (e.g. serde_with::double_option),
        // serde treats both missing and null as None for Option<Option<T>>.
        // The three-way distinction only works on serialization (skip_serializing_if).
        let json = r#"{"speed_limit_bps": null}"#;
        let update: ConfigUpdate = serde_json::from_str(json).expect("deserialize failed");
        assert_eq!(
            update.speed_limit_bps, None,
            "null deserializes as None (same as missing) without a custom deserializer"
        );
    }

    #[test]
    fn config_update_deserializes_number_as_some_some() {
        let json = r#"{"speed_limit_bps": 5000000}"#;
        let update: ConfigUpdate = serde_json::from_str(json).expect("deserialize failed");
        assert_eq!(
            update.speed_limit_bps,
            Some(Some(5_000_000)),
            "number value must become Some(Some(val))"
        );
    }

    // --- Invalid duration deserialization ---

    #[test]
    fn duration_serde_rejects_string_instead_of_integer() {
        let json = r#"{"initial_delay": "not_a_number", "max_delay": 60}"#;
        let result = serde_json::from_str::<RetryConfig>(json);

        match result {
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("invalid type") || msg.contains("expected"),
                    "serde error should describe the type mismatch, got: {msg}"
                );
            }
            Ok(_) => panic!(
                "string value for a Duration field must produce a serde error, not silently succeed"
            ),
        }
    }

    #[test]
    fn duration_serde_rejects_negative_integer() {
        let json = r#"{"initial_delay": -1, "max_delay": 60}"#;
        let result = serde_json::from_str::<RetryConfig>(json);

        match result {
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("invalid value") || msg.contains("expected"),
                    "serde error should describe the negative value issue, got: {msg}"
                );
            }
            Ok(_) => panic!(
                "-1 for a Duration (u64) field must produce a serde error, not silently succeed"
            ),
        }
    }

    #[test]
    fn optional_duration_serde_rejects_string_instead_of_integer() {
        // RssFilter.max_age uses optional_duration_serde
        let json = r#"{"name": "test", "max_age": "forever"}"#;
        let result = serde_json::from_str::<RssFilter>(json);

        match result {
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("invalid type") || msg.contains("expected"),
                    "serde error should describe the type mismatch, got: {msg}"
                );
            }
            Ok(_) => {
                panic!("string value for an optional Duration field must produce a serde error")
            }
        }
    }
}
