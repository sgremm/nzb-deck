//! Core downloader implementation split into focused submodules.
//!
//! The `UsenetDownloader` struct and its methods are organized by domain:
//! - [`queue`] - Priority queue management
//! - [`control`] - Download lifecycle control (pause/resume/cancel)
//! - [`config_ops`] - Runtime configuration updates
//! - [`rss`] - RSS feed management
//! - [`server`] - Server connectivity testing
//! - [`lifecycle`] - Startup and shutdown coordination
//! - [`nzb`] - NZB file parsing and ingestion
//! - [`webhooks`] - Webhook and script notifications
//! - [`tasks`] - Legacy download task spawning
//! - [`queue_processor`] - Queue processing and orchestration
//! - [`download_task`] - Core download execution
//! - [`background_tasks`] - Progress reporting and batch updates
//! - [`services`] - Background service starters
//! - [`post_process`] - Post-processing pipeline entry

mod background_tasks;
mod config_ops;
mod control;
pub(crate) mod direct_unpack;
mod download_task;
mod lifecycle;
mod nzb;
mod post_process;
mod queue;
mod queue_processor;
mod rss;
mod server;
mod services;
mod tasks;
mod webhooks;

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
pub(crate) mod test_helpers;
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests;

// Re-export parameter structs for testing
pub use webhooks::{TriggerScriptsParams, TriggerWebhooksParams};

use crate::config::Config;
use crate::db::Database;
use crate::error::{Error, Result};
use crate::parity::{CliParityHandler, NoOpParityHandler, ParityHandler};
use crate::post_processing;
use crate::speed_limiter;
use crate::types::{DownloadId, Priority};

/// Queue and download state management
#[derive(Clone)]
pub(crate) struct QueueState {
    /// Priority queue for managing download order (protected by Mutex)
    pub(crate) queue:
        std::sync::Arc<tokio::sync::Mutex<std::collections::BinaryHeap<QueuedDownload>>>,
    /// Semaphore to limit concurrent downloads (respects max_concurrent_downloads config)
    pub(crate) concurrent_limit: std::sync::Arc<tokio::sync::Semaphore>,
    /// Map of active downloads to their cancellation tokens (for pause/cancel operations)
    pub(crate) active_downloads: std::sync::Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<DownloadId, tokio_util::sync::CancellationToken>,
        >,
    >,
    /// Flag to indicate whether new downloads are accepted (set to false during shutdown)
    pub(crate) accepting_new: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Runtime-mutable configuration (separate from static config)
#[derive(Clone)]
pub(crate) struct RuntimeConfig {
    /// Runtime-mutable categories (separate from config for dynamic updates)
    pub(crate) categories: std::sync::Arc<
        tokio::sync::RwLock<std::collections::HashMap<String, crate::config::CategoryConfig>>,
    >,
    /// Runtime-mutable schedule rules (separate from config for dynamic updates)
    pub(crate) schedule_rules:
        std::sync::Arc<tokio::sync::RwLock<Vec<crate::config::ScheduleRule>>>,
    /// Next schedule rule ID counter
    pub(crate) next_schedule_rule_id: std::sync::Arc<std::sync::atomic::AtomicI64>,
}

/// Post-processing and parity handling
#[derive(Clone)]
pub(crate) struct ProcessingPipeline {
    /// Post-processing pipeline executor
    pub(crate) post_processor: std::sync::Arc<post_processing::PostProcessor>,
    /// Parity handler for PAR2 verification and repair (trait object for pluggable implementations)
    pub(crate) parity_handler: std::sync::Arc<dyn ParityHandler>,
}

/// Main downloader instance (cloneable - all fields are Arc-wrapped)
#[derive(Clone)]
pub struct UsenetDownloader {
    /// Database instance for persistence (wrapped in Arc for sharing across tasks)
    /// Public for integration tests to query download status
    pub db: std::sync::Arc<Database>,
    /// Event broadcast channel sender (multiple subscribers supported)
    pub(crate) event_tx: tokio::sync::broadcast::Sender<crate::types::Event>,
    /// Configuration (wrapped in Arc for sharing across tasks)
    pub(crate) config: std::sync::Arc<Config>,
    /// NNTP connection pools (one per server, wrapped in Arc for sharing across tasks)
    pub(crate) nntp_pools: std::sync::Arc<Vec<nntp_rs::NntpPool>>,
    /// Global speed limiter shared across all downloads (token bucket algorithm)
    pub(crate) speed_limiter: speed_limiter::SpeedLimiter,
    /// Queue and download state management
    pub(crate) queue_state: QueueState,
    /// Runtime-mutable configuration
    pub(crate) runtime_config: RuntimeConfig,
    /// Post-processing and parity handling
    pub(crate) processing: ProcessingPipeline,
}

/// Internal struct representing a download in the priority queue
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct QueuedDownload {
    pub(crate) id: DownloadId,
    pub(crate) priority: Priority,
    pub(crate) created_at: i64, // Unix timestamp for tie-breaking
}

// Implement Ord for BinaryHeap (max-heap by default)
impl Ord for QueuedDownload {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // First compare by priority (higher priority wins)
        match self.priority.cmp(&other.priority) {
            std::cmp::Ordering::Equal => {
                // If priorities are equal, older downloads come first (FIFO)
                // Note: Reversed because we want older (lower timestamp) to have higher priority
                other.created_at.cmp(&self.created_at)
            }
            ordering => ordering,
        }
    }
}

impl PartialOrd for QueuedDownload {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl UsenetDownloader {
    /// Create a new UsenetDownloader instance
    ///
    /// This initializes all core components:
    /// - Opens/creates the SQLite database
    /// - Runs migrations
    /// - Creates NNTP connection pools for each configured server
    /// - Sets up the event broadcast channel
    pub async fn new(config: Config) -> Result<Self> {
        // Ensure download and temp directories exist
        tokio::fs::create_dir_all(&config.download.download_dir)
            .await
            .map_err(|e| {
                Error::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "Failed to create download directory '{}': {}",
                        config.download.download_dir.display(),
                        e
                    ),
                ))
            })?;
        tokio::fs::create_dir_all(&config.download.temp_dir)
            .await
            .map_err(|e| {
                Error::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "Failed to create temp directory '{}': {}",
                        config.download.temp_dir.display(),
                        e
                    ),
                ))
            })?;

        // Initialize database
        let db = Database::new(&config.persistence.database_path).await?;

        // Mark that we're starting up (for unclean shutdown detection)
        db.set_clean_start().await?;

        // Create broadcast channel with buffer size of 1000 events
        // This allows multiple subscribers to receive all events independently
        let (event_tx, _rx) = tokio::sync::broadcast::channel(1000);

        // Create NNTP connection pools for each server
        let mut nntp_pools = Vec::with_capacity(config.servers.len());
        for server in &config.servers {
            let pool = nntp_rs::NntpPool::new(server.clone().into(), server.connections as u32)
                .await
                .map_err(|e| Error::Nntp(format!("Failed to create NNTP pool: {}", e)))?;
            nntp_pools.push(pool);
        }

        // Create priority queue (empty initially, will be loaded from database on startup)
        let queue =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BinaryHeap::new()));

        // Create semaphore for concurrent download limiting
        let concurrent_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(
            config.download.max_concurrent_downloads,
        ));

        // Create active downloads tracking map
        let active_downloads =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

        // Create speed limiter with configured limit (or unlimited if not set)
        let speed_limiter = speed_limiter::SpeedLimiter::new(config.download.speed_limit_bps);

        // Create config Arc early so we can share it
        let config_arc = std::sync::Arc::new(config.clone());

        // Initialize runtime-mutable categories from config
        let categories = std::sync::Arc::new(tokio::sync::RwLock::new(
            config.persistence.categories.clone(),
        ));

        // Initialize runtime-mutable schedule rules from config
        let schedule_rules = std::sync::Arc::new(tokio::sync::RwLock::new(
            config.persistence.schedule_rules.clone(),
        ));

        // Initialize the next ID counter (0 since config rules don't have IDs yet)
        let next_schedule_rule_id = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));

        // Initialize parity handler — prefer injected handler, then fall back to config
        let parity_handler: std::sync::Arc<dyn ParityHandler> =
            if let Some(ref handler) = config.tools.parity_handler {
                // Use the caller-provided handler directly
                std::sync::Arc::clone(handler)
            } else if let Some(ref par2_path) = config.tools.par2_path {
                // Use explicitly configured binary path
                std::sync::Arc::new(CliParityHandler::new(par2_path.clone()))
            } else if config.tools.search_path {
                // Search PATH for par2 binary
                CliParityHandler::from_path()
                    .map(|h| std::sync::Arc::new(h) as std::sync::Arc<dyn ParityHandler>)
                    .unwrap_or_else(|| std::sync::Arc::new(NoOpParityHandler))
            } else {
                // No binary configured and PATH search disabled
                std::sync::Arc::new(NoOpParityHandler)
            };

        // Log parity handler capabilities
        let parity_caps = parity_handler.capabilities();
        tracing::info!(
            parity_handler = parity_handler.name(),
            can_verify = parity_caps.can_verify,
            can_repair = parity_caps.can_repair,
            "Parity handler initialized"
        );

        // Create database Arc for sharing
        let db_arc = std::sync::Arc::new(db);

        // Create post-processing pipeline executor
        let post_processor = std::sync::Arc::new(post_processing::PostProcessor::new(
            event_tx.clone(),
            config_arc.clone(),
            parity_handler.clone(),
            db_arc.clone(),
        ));

        // Group queue and download state
        let queue_state = QueueState {
            queue,
            concurrent_limit,
            active_downloads,
            accepting_new: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        };

        // Group runtime configuration
        let runtime_config = RuntimeConfig {
            categories,
            schedule_rules,
            next_schedule_rule_id,
        };

        // Group post-processing pipeline
        let processing = ProcessingPipeline {
            post_processor,
            parity_handler,
        };

        let downloader = Self {
            db: db_arc,
            event_tx,
            config: config_arc,
            nntp_pools: std::sync::Arc::new(nntp_pools),
            speed_limiter,
            queue_state,
            runtime_config,
            processing,
        };

        // Restore any incomplete downloads from database (from previous session)
        let needs_post_processing = downloader.restore_queue().await?;
        for id in needs_post_processing {
            let dl = downloader.clone();
            tokio::spawn(async move {
                if let Err(e) = dl.start_post_processing(id).await {
                    tracing::error!(download_id = id.0, error = %e, "Post-processing failed during restore");
                }
            });
        }

        Ok(downloader)
    }

    /// Subscribe to download events
    ///
    /// Multiple subscribers are supported. Each subscriber receives all events independently.
    /// Events are buffered, but if a subscriber falls behind by more than 1000 events,
    /// it will receive a `RecvError::Lagged` error.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use usenet_dl::{UsenetDownloader, Config};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let downloader = UsenetDownloader::new(Config::default()).await?;
    ///
    ///     // UI subscriber
    ///     let mut ui_events = downloader.subscribe();
    ///     tokio::spawn(async move {
    ///         while let Ok(event) = ui_events.recv().await {
    ///             println!("UI: {:?}", event);
    ///         }
    ///     });
    ///
    ///     // Logging subscriber
    ///     let mut log_events = downloader.subscribe();
    ///     tokio::spawn(async move {
    ///         while let Ok(event) = log_events.recv().await {
    ///             tracing::info!(?event, "download event");
    ///         }
    ///     });
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<crate::types::Event> {
        self.event_tx.subscribe()
    }

    /// Get the current configuration
    ///
    /// Returns a reference to the current configuration. The configuration is wrapped in an Arc,
    /// so this is a cheap clone operation.
    ///
    /// # Returns
    ///
    /// A reference to the current `Config`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, config::Config};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = Config::default();
    /// let downloader = UsenetDownloader::new(config).await?;
    ///
    /// let current_config = downloader.get_config();
    /// println!("Download directory: {:?}", current_config.download_dir());
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_config(&self) -> std::sync::Arc<Config> {
        std::sync::Arc::clone(&self.config)
    }

    /// Query the current system capabilities
    ///
    /// Returns information about what post-processing features are currently available
    /// based on the configuration and available external tools.
    pub fn capabilities(&self) -> crate::types::Capabilities {
        let parity_caps = self.processing.parity_handler.capabilities();
        let handler_name = self.processing.parity_handler.name().to_string();

        crate::types::Capabilities {
            parity: crate::types::ParityCapabilitiesInfo {
                can_verify: parity_caps.can_verify,
                can_repair: parity_caps.can_repair,
                handler: handler_name,
            },
        }
    }

    /// Emit an event to all subscribers
    ///
    /// This is an internal helper method used throughout the codebase to emit events.
    /// Events are sent to all active subscribers via the broadcast channel.
    ///
    /// If there are no active subscribers, the event is silently dropped (ok() converts Err to None).
    /// This allows the download process to continue even if no one is listening to events.
    pub(crate) fn emit_event(&self, event: crate::types::Event) {
        // send() returns Err if there are no receivers, which is fine - we just drop the event
        self.event_tx.send(event).ok();
    }

    /// Spawn the REST API server in a background task
    ///
    /// This method spawns the API server as a separate async task using `tokio::spawn`.
    /// The server runs concurrently with download processing and listens on the configured
    /// bind address (default: 127.0.0.1:6789).
    pub fn spawn_api_server(self: &std::sync::Arc<Self>) -> tokio::task::JoinHandle<Result<()>> {
        let downloader = self.clone();
        let config = self.config.clone();

        tokio::spawn(async move { crate::api::start_api_server(downloader, config).await })
    }
}
