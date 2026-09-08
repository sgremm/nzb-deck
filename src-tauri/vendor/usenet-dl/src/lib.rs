//! # usenet-dl
//!
//! Highly configurable backend library for Usenet download applications.
//!
//! ## Design Philosophy
//!
//! usenet-dl is designed to be:
//! - **Highly configurable** - Almost every behavior can be customized
//! - **Sensible defaults** - Works out of the box with zero configuration
//! - **Library-first** - No CLI or UI, purely a Rust crate for embedding
//! - **Event-driven** - Consumers subscribe to events, no polling required
//!
//! ## Quick Start
//!
//! ```no_run
//! use usenet_dl::{UsenetDownloader, Config, ServerConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = Config {
//!         servers: vec![
//!             ServerConfig {
//!                 host: "news.example.com".to_string(),
//!                 port: 563,
//!                 tls: true,
//!                 username: Some("user".to_string()),
//!                 password: Some("pass".to_string()),
//!                 connections: 10,
//!                 priority: 0,
//!                 pipeline_depth: 10,
//!             }
//!         ],
//!         ..Default::default()
//!     };
//!
//!     let downloader = UsenetDownloader::new(config).await?;
//!
//!     // Subscribe to events
//!     let mut events = downloader.subscribe();
//!     tokio::spawn(async move {
//!         while let Ok(event) = events.recv().await {
//!             println!("Event: {:?}", event);
//!         }
//!     });
//!
//!     Ok(())
//! }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::unwrap_used)]
#![warn(clippy::expect_used)]

/// REST API module
pub mod api;
/// Configuration types
pub mod config;
/// Database persistence layer
pub mod db;
/// Filename deobfuscation
pub mod deobfuscation;
/// Core downloader implementation (decomposed into focused submodules)
pub mod downloader;
/// Error types
pub mod error;
/// Archive extraction
pub mod extraction;
/// Folder watching for automatic NZB import
pub mod folder_watcher;
/// PAR2 parity handling
pub mod parity;
/// Post-processing pipeline
pub mod post_processing;
/// Retry logic with exponential backoff
pub mod retry;
/// RSS feed management
pub mod rss_manager;
/// RSS feed scheduler
pub mod rss_scheduler;
/// Time-based scheduling
pub mod scheduler;
/// Scheduler task execution
pub mod scheduler_task;
/// Speed limiting with token bucket
pub mod speed_limiter;
/// Core types and events
pub mod types;
/// Utility functions
pub mod utils;

// Re-export commonly used types
pub use config::{Config, DuplicateAction, ServerConfig};
pub use db::Database;
pub use downloader::UsenetDownloader;
pub use error::{
    ApiError, DatabaseError, DownloadError, Error, ErrorDetail, PostProcessError, Result,
    ToHttpStatus,
};
pub use parity::{
    CliParityHandler, NoOpParityHandler, ParityCapabilities, ParityHandler, RepairResult,
    VerifyResult,
};
pub use scheduler::{RuleId, ScheduleAction, ScheduleRule, Scheduler, Weekday};
pub use types::{
    DownloadId, DownloadInfo, DownloadOptions, DuplicateInfo, Event, HistoryEntry, Priority,
    QueueStats, ServerCapabilities, ServerTestResult, Stage, Status,
};

/// Helper function to run the downloader with graceful signal handling.
///
/// Waits for a termination signal and then calls the downloader's `shutdown()` method.
///
/// - **Unix:** listens for SIGTERM and SIGINT, with fallbacks if signal registration fails.
/// - **Windows/other:** listens for Ctrl+C via `tokio::signal::ctrl_c()`.
///
/// # Example
///
/// ```no_run
/// use usenet_dl::{UsenetDownloader, Config, run_with_shutdown};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = Config::default();
///     let downloader = UsenetDownloader::new(config).await?;
///
///     // Run with automatic signal handling
///     run_with_shutdown(downloader).await?;
///
///     Ok(())
/// }
/// ```
pub async fn run_with_shutdown(downloader: UsenetDownloader) -> Result<()> {
    wait_for_signal().await;
    downloader.shutdown().await
}

#[cfg(unix)]
async fn wait_for_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    // Set up signal handlers - these may fail in restricted environments (containers, tests)
    let sigterm_result = signal(SignalKind::terminate());
    let sigint_result = signal(SignalKind::interrupt());

    match (sigterm_result, sigint_result) {
        (Ok(mut sigterm), Ok(mut sigint)) => {
            tokio::select! {
                _ = sigterm.recv() => {
                    tracing::info!("Received SIGTERM signal");
                }
                _ = sigint.recv() => {
                    tracing::info!("Received SIGINT signal (Ctrl+C)");
                }
            }
        }
        (Err(e), _) => {
            tracing::warn!(error = %e, "Could not register SIGTERM handler, waiting for SIGINT only");
            if let Ok(mut sigint) = signal(SignalKind::interrupt()) {
                sigint.recv().await;
                tracing::info!("Received SIGINT signal (Ctrl+C)");
            } else {
                tracing::error!("Could not register any signal handlers, using ctrl_c fallback");
                tokio::signal::ctrl_c().await.ok();
            }
        }
        (_, Err(e)) => {
            tracing::warn!(error = %e, "Could not register SIGINT handler, waiting for SIGTERM only");
            if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
                sigterm.recv().await;
                tracing::info!("Received SIGTERM signal");
            } else {
                tracing::error!("Could not register any signal handlers, using ctrl_c fallback");
                tokio::signal::ctrl_c().await.ok();
            }
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() {
    match tokio::signal::ctrl_c().await {
        Ok(()) => {
            tracing::info!("Received Ctrl+C signal");
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to listen for Ctrl+C signal");
        }
    }
}
