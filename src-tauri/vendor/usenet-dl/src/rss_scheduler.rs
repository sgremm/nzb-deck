//! RSS feed scheduling and periodic checking
//!
//! This module provides background scheduling for RSS feed checks. The scheduler
//! manages multiple feeds with independent check intervals, automatically fetching
//! and processing new items.
//!
//! # Features
//!
//! - Independent per-feed check intervals
//! - Respects feed enable/disable state
//! - Graceful shutdown handling
//! - Last-check time tracking
//!
//! # Example
//!
//! ```no_run
//! use usenet_dl::{UsenetDownloader, config::Config};
//! use usenet_dl::rss_scheduler::RssScheduler;
//! use usenet_dl::rss_manager::RssManager;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = Config::default();
//! let downloader = Arc::new(UsenetDownloader::new(config).await?);
//! let rss_manager = Arc::new(RssManager::new(
//!     downloader.db.clone(),
//!     downloader.clone(),
//!     downloader.config.rss_feeds.clone(),
//! )?);
//!
//! let scheduler = RssScheduler::new(downloader.clone(), rss_manager);
//!
//! // Run scheduler (blocks until shutdown)
//! tokio::spawn(async move {
//!     scheduler.run().await;
//! });
//! # Ok(())
//! # }
//! ```

use crate::{UsenetDownloader, rss_manager::RssManager};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info, warn};

/// RSS feed scheduler that periodically checks configured feeds
///
/// The scheduler manages periodic checking of RSS feeds based on their
/// configured check intervals. Each feed is checked independently according
/// to its own interval setting.
pub struct RssScheduler {
    /// Reference to the RSS manager for fetching/processing feeds
    rss_manager: Arc<RssManager>,

    /// Reference to downloader for accessing config and shutdown status
    downloader: Arc<UsenetDownloader>,
}

impl RssScheduler {
    /// Creates a new RSS scheduler
    ///
    /// # Parameters
    /// - `downloader`: Reference to the UsenetDownloader for config access
    /// - `rss_manager`: Reference to the RSS manager for feed operations
    pub fn new(downloader: Arc<UsenetDownloader>, rss_manager: Arc<RssManager>) -> Self {
        Self {
            rss_manager,
            downloader,
        }
    }

    /// Starts the RSS feed checking scheduler
    ///
    /// This runs in a loop checking each feed according to its check_interval.
    /// The scheduler will:
    /// 1. Check if shutdown was requested (via downloader.queue_state.accepting_new flag)
    /// 2. For each enabled feed:
    ///    - Fetch and parse the feed
    ///    - Process new items (filter, mark as seen, auto-download)
    ///    - Log results
    /// 3. Sleep for a brief interval (30 seconds) before next check
    ///
    /// Each feed tracks its last check time independently. Feeds are checked
    /// when current_time - last_check >= check_interval.
    pub async fn run(self) {
        info!("RSS scheduler started");

        // Track last check time for each feed (indexed by URL for simplicity)
        let mut last_check_times: std::collections::HashMap<String, SystemTime> =
            std::collections::HashMap::new();

        loop {
            // Check for shutdown signal via downloader's accepting_new flag
            if !self
                .downloader
                .queue_state
                .accepting_new
                .load(Ordering::SeqCst)
            {
                info!("RSS scheduler shutting down");
                break;
            }

            // Read feeds from the database so API-added/modified feeds are picked up
            let db_feeds = match self.downloader.db.get_all_rss_feeds().await {
                Ok(f) => f,
                Err(e) => {
                    error!(error = %e, "Failed to load RSS feeds from database, falling back to config");
                    // Fall back to empty -- will retry next cycle
                    vec![]
                }
            };

            // Also include config-file feeds that may not be in the database
            let config_feeds = &self.downloader.config.automation.rss_feeds;

            if db_feeds.is_empty() && config_feeds.is_empty() {
                debug!("No RSS feeds configured, scheduler idle");
                sleep(Duration::from_secs(30)).await;
                continue;
            }

            let now = SystemTime::now();

            // Process database feeds (these have proper IDs)
            for feed_row in &db_feeds {
                if feed_row.enabled == 0 {
                    debug!(url = %feed_row.url, "RSS feed disabled, skipping");
                    continue;
                }

                let feed_config = crate::config::RssFeedConfig {
                    url: feed_row.url.clone(),
                    check_interval: Duration::from_secs(feed_row.check_interval_secs as u64),
                    category: feed_row.category.clone(),
                    filters: vec![], // Filters are loaded during process_feed_items via the DB
                    auto_download: feed_row.auto_download != 0,
                    priority: crate::types::Priority::from_i32(feed_row.priority),
                    enabled: true,
                };

                // Check if it's time to check this feed
                let should_check = match last_check_times.get(&feed_row.url) {
                    Some(last_check) => match now.duration_since(*last_check) {
                        Ok(elapsed) => elapsed >= feed_config.check_interval,
                        Err(_) => {
                            warn!(url = %feed_row.url, "System time went backwards, checking feed");
                            true
                        }
                    },
                    None => true,
                };

                if !should_check {
                    continue;
                }

                debug!(
                    url = %feed_row.url,
                    feed_id = feed_row.id,
                    interval = ?feed_config.check_interval,
                    "Checking RSS feed"
                );

                // Load filters from DB for this feed
                let db_filters = match self.downloader.db.get_rss_filters(feed_row.id).await {
                    Ok(f) => f,
                    Err(e) => {
                        error!(feed_id = feed_row.id, error = %e, "Failed to load RSS filters");
                        vec![]
                    }
                };
                let filters: Vec<crate::config::RssFilter> = db_filters
                    .into_iter()
                    .map(|row| crate::config::RssFilter {
                        name: row.name,
                        include: row
                            .include_patterns
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or_default(),
                        exclude: row
                            .exclude_patterns
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or_default(),
                        min_size: row.min_size.map(|s| s as u64),
                        max_size: row.max_size.map(|s| s as u64),
                        max_age: row.max_age_secs.map(|s| Duration::from_secs(s as u64)),
                    })
                    .collect();

                let feed_with_filters = crate::config::RssFeedConfig {
                    filters,
                    ..feed_config.clone()
                };

                match self.rss_manager.check_feed(&feed_with_filters).await {
                    Ok(items) => {
                        info!(
                            url = %feed_row.url,
                            item_count = items.len(),
                            "Successfully fetched RSS feed"
                        );

                        // Use the actual database feed_id instead of hardcoded 0
                        match self
                            .rss_manager
                            .process_feed_items(feed_row.id, &feed_with_filters, items)
                            .await
                        {
                            Ok(downloaded_count) => {
                                if downloaded_count > 0 {
                                    info!(
                                        url = %feed_row.url,
                                        count = downloaded_count,
                                        "Auto-downloaded items from RSS feed"
                                    );
                                }
                            }
                            Err(e) => {
                                error!(
                                    url = %feed_row.url,
                                    error = %e,
                                    "Failed to process RSS feed items"
                                );
                            }
                        }

                        // Update check status in DB
                        if let Err(e) = self
                            .downloader
                            .db
                            .update_rss_feed_check_status(feed_row.id, None)
                            .await
                        {
                            warn!(feed_id = feed_row.id, error = %e, "Failed to update feed check status");
                        }
                    }
                    Err(e) => {
                        error!(
                            url = %feed_row.url,
                            error = %e,
                            "Failed to fetch RSS feed"
                        );
                        // Record the error in DB
                        let _ = self
                            .downloader
                            .db
                            .update_rss_feed_check_status(feed_row.id, Some(&e.to_string()))
                            .await;
                    }
                }

                last_check_times.insert(feed_row.url.clone(), now);
            }

            // Sleep before next check cycle (1 second)
            // This prevents tight loops while remaining responsive to shutdown
            sleep(Duration::from_secs(1)).await;
        }

        info!("RSS scheduler stopped");
    }
}
