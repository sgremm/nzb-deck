//! RSS feed management for automatic NZB monitoring and downloading.
//!
//! This module provides functionality for monitoring RSS/Atom feeds, filtering items based on
//! configurable rules, and automatically downloading matching NZB files. It supports both RSS 2.0
//! and Atom feed formats, with regex-based filtering, size constraints, age limits, and duplicate
//! detection.

use crate::UsenetDownloader;
use crate::config::{RssFeedConfig, RssFilter};
use crate::db::Database;
use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use regex::Regex;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Represents an item from an RSS or Atom feed
#[derive(Clone, Debug)]
pub struct RssItem {
    /// Item title
    pub title: String,

    /// Item link/URL
    pub link: Option<String>,

    /// Unique identifier (GUID for RSS, id for Atom)
    pub guid: String,

    /// Publication date
    pub pub_date: Option<DateTime<Utc>>,

    /// Item description
    pub description: Option<String>,

    /// Size in bytes (from enclosure or custom tags)
    pub size: Option<u64>,

    /// NZB download URL (from enclosure or link)
    pub nzb_url: Option<String>,
}

/// Manages RSS feed monitoring and auto-downloading
///
/// The RssManager is responsible for:
/// - Periodically checking RSS/Atom feeds for new items
/// - Filtering items based on configured rules
/// - Tracking seen items to prevent duplicates
/// - Automatically downloading matching NZB files
pub struct RssManager {
    /// HTTP client for fetching RSS feeds
    http_client: reqwest::Client,

    /// Database reference for persistence
    db: Arc<Database>,

    /// Reference to the downloader for adding NZBs
    downloader: Arc<UsenetDownloader>,

    /// Configured RSS feeds
    feeds: Vec<RssFeedConfig>,
}

impl RssManager {
    /// Create a new RSS manager
    ///
    /// # Arguments
    /// * `db` - Database instance for persistence
    /// * `downloader` - Reference to the UsenetDownloader instance
    /// * `feeds` - List of RSS feed configurations to monitor
    ///
    /// # Errors
    /// Returns error if the HTTP client cannot be created
    pub fn new(
        db: Arc<Database>,
        downloader: Arc<UsenetDownloader>,
        feeds: Vec<RssFeedConfig>,
    ) -> Result<Self> {
        // Create HTTP client with reasonable timeout (30 seconds)
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("usenet-dl RSS Reader")
            .build()
            .map_err(|e| Error::Other(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            http_client,
            db,
            downloader,
            feeds,
        })
    }

    /// Start the RSS manager
    ///
    /// This method initializes the RSS manager and prepares it for checking feeds.
    /// Currently a no-op, but could be used for initialization in the future.
    pub fn start(&self) -> Result<()> {
        info!("RSS manager initialized with {} feeds", self.feeds.len());
        Ok(())
    }

    /// Stop the RSS manager
    ///
    /// This method stops all feed checking tasks.
    /// Currently a no-op, but will be used when scheduled checking is implemented.
    pub async fn stop(&self) {
        info!("RSS manager stopped");
    }

    /// Check a single RSS/Atom feed for new items
    ///
    /// This method:
    /// 1. Fetches the feed content via HTTP
    /// 2. Attempts to parse as RSS, falls back to Atom if that fails
    /// 3. Extracts items with their metadata (title, link, guid, size, etc.)
    /// 4. Returns a list of parsed items
    ///
    /// # Arguments
    /// * `feed_config` - The feed configuration containing the URL and other settings
    ///
    /// # Returns
    /// A vector of `RssItem` representing the items in the feed
    ///
    /// # Errors
    /// Returns error if:
    /// - HTTP request fails
    /// - Feed cannot be parsed as either RSS or Atom
    /// - Network timeout occurs
    pub async fn check_feed(&self, feed_config: &RssFeedConfig) -> Result<Vec<RssItem>> {
        debug!("Checking RSS feed: {}", feed_config.url);

        // Fetch feed content
        let response = self
            .http_client
            .get(&feed_config.url)
            .send()
            .await
            .map_err(|e| Error::Other(format!("Failed to fetch RSS feed: {}", e)))?;

        // Check HTTP status before trying to parse the response body
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Other(format!(
                "RSS feed returned HTTP {}: {}",
                status.as_u16(),
                feed_config.url
            )));
        }

        let content = response
            .text()
            .await
            .map_err(|e| Error::Other(format!("Failed to read RSS feed content: {}", e)))?;

        // Try parsing as RSS first, then Atom
        match self.parse_as_rss(&content) {
            Ok(items) => {
                debug!("Successfully parsed as RSS, found {} items", items.len());
                Ok(items)
            }
            Err(rss_err) => {
                debug!("Failed to parse as RSS: {}, trying Atom", rss_err);
                match self.parse_as_atom(&content) {
                    Ok(items) => {
                        debug!("Successfully parsed as Atom, found {} items", items.len());
                        Ok(items)
                    }
                    Err(atom_err) => Err(Error::Other(format!(
                        "Failed to parse feed as RSS or Atom. RSS error: {}. Atom error: {}",
                        rss_err, atom_err
                    ))),
                }
            }
        }
    }

    /// Parse feed content as RSS
    fn parse_as_rss(&self, content: &str) -> Result<Vec<RssItem>> {
        let channel = content
            .parse::<rss::Channel>()
            .map_err(|e| Error::Other(format!("RSS parse error: {}", e)))?;

        let items = channel
            .items()
            .iter()
            .map(|item| {
                // Extract GUID (prefer guid, fallback to link, then title)
                let guid = item
                    .guid()
                    .map(|g| g.value().to_string())
                    .or_else(|| item.link().map(|l| l.to_string()))
                    .unwrap_or_else(|| item.title().unwrap_or("").to_string());

                // Parse publication date
                let pub_date = item.pub_date().and_then(|date_str| {
                    chrono::DateTime::parse_from_rfc2822(date_str)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                });

                // Extract NZB URL (from enclosure or link)
                let nzb_url = item
                    .enclosure()
                    .map(|enc| enc.url().to_string())
                    .or_else(|| {
                        item.link()
                            .filter(|link| link.ends_with(".nzb"))
                            .map(|l| l.to_string())
                    });

                // Extract size from enclosure
                let size = item
                    .enclosure()
                    .and_then(|enc| enc.length().parse::<u64>().ok());

                RssItem {
                    title: item.title().unwrap_or("").to_string(),
                    link: item.link().map(|l| l.to_string()),
                    guid,
                    pub_date,
                    description: item.description().map(|d| d.to_string()),
                    size,
                    nzb_url,
                }
            })
            .collect();

        Ok(items)
    }

    /// Parse feed content as Atom
    fn parse_as_atom(&self, content: &str) -> Result<Vec<RssItem>> {
        let feed = atom_syndication::Feed::read_from(content.as_bytes())
            .map_err(|e| Error::Other(format!("Atom parse error: {}", e)))?;

        let items = feed
            .entries()
            .iter()
            .map(|entry| {
                // GUID is the entry ID
                let guid = entry.id().to_string();

                // Publication date (prefer published, fallback to updated)
                let pub_date = entry
                    .published()
                    .or_else(|| Some(entry.updated()))
                    .and_then(|dt| {
                        chrono::DateTime::parse_from_rfc3339(&dt.to_rfc3339())
                            .ok()
                            .map(|dt| dt.with_timezone(&Utc))
                    });

                // Extract NZB URL from links
                let nzb_url = entry
                    .links()
                    .iter()
                    .find(|link| {
                        link.href().ends_with(".nzb")
                            || link.mime_type() == Some("application/x-nzb")
                    })
                    .map(|link| link.href().to_string());

                // Try to get the primary link
                let link = entry.links().first().map(|link| link.href().to_string());

                // Extract size from enclosure-type links
                let size = entry
                    .links()
                    .iter()
                    .find(|link| link.rel() == "enclosure")
                    .and_then(|link| link.length().and_then(|l| l.parse::<u64>().ok()));

                // Description from summary or content
                let description = entry.summary().map(|s| s.as_str().to_string()).or_else(|| {
                    entry
                        .content()
                        .and_then(|c| c.value().map(|v| v.to_string()))
                });

                RssItem {
                    title: entry.title().as_str().to_string(),
                    link,
                    guid,
                    pub_date,
                    description,
                    size,
                    nzb_url,
                }
            })
            .collect();

        Ok(items)
    }

    /// Compile and validate a list of regex patterns, returning compiled regexes.
    /// Invalid patterns are logged and skipped.
    fn compile_patterns(patterns: &[String], kind: &str) -> Vec<Regex> {
        patterns
            .iter()
            .filter_map(|pattern| {
                // Use RegexBuilder with a size limit to prevent ReDoS via large compiled DFAs
                regex::RegexBuilder::new(pattern)
                    .size_limit(1024 * 1024) // 1MB compiled DFA limit
                    .build()
                    .map_err(|e| {
                        warn!("Invalid {} regex pattern '{}': {}", kind, pattern, e);
                    })
                    .ok()
            })
            .collect()
    }

    /// Check if an RSS item matches the configured filters
    ///
    /// This method applies filtering rules from an RssFilter to determine if an item should be accepted.
    /// Filtering logic:
    /// 1. If include patterns exist, at least one must match (OR logic)
    /// 2. If exclude patterns exist, none must match (exclude overrides include)
    /// 3. Size constraints (min_size, max_size) are checked if specified
    /// 4. Age constraint (max_age) is checked against publication date if specified
    ///
    /// # Arguments
    /// * `item` - The RSS item to check
    /// * `filter` - The filter rules to apply
    ///
    /// # Returns
    /// true if the item passes all filter rules, false otherwise
    pub fn matches_filters(&self, item: &RssItem, filter: &RssFilter) -> bool {
        // Build the search text (title + description)
        let search_text = format!(
            "{} {}",
            item.title,
            item.description.as_deref().unwrap_or("")
        );

        // Check include patterns (OR logic - at least one must match)
        if !filter.include.is_empty() {
            let compiled_includes = Self::compile_patterns(&filter.include, "include");
            let any_include_matches = compiled_includes.iter().any(|re| re.is_match(&search_text));

            if !any_include_matches {
                debug!(
                    "Item '{}' rejected: no include patterns matched",
                    item.title
                );
                return false;
            }
        }

        // Check exclude patterns (ANY exclude match = reject)
        let compiled_excludes = Self::compile_patterns(&filter.exclude, "exclude");
        for re in &compiled_excludes {
            if re.is_match(&search_text) {
                debug!(
                    "Item '{}' rejected: matched exclude pattern '{}'",
                    item.title,
                    re.as_str()
                );
                return false;
            }
        }

        // Check size constraints
        if let Some(size) = item.size {
            if let Some(min_size) = filter.min_size
                && size < min_size
            {
                debug!(
                    "Item '{}' rejected: size {} < min {}",
                    item.title, size, min_size
                );
                return false;
            }

            if let Some(max_size) = filter.max_size
                && size > max_size
            {
                debug!(
                    "Item '{}' rejected: size {} > max {}",
                    item.title, size, max_size
                );
                return false;
            }
        }

        // Check age constraint
        if let Some(max_age) = filter.max_age
            && let Some(pub_date) = item.pub_date
        {
            let age = Utc::now().signed_duration_since(pub_date);
            let max_age_chrono =
                chrono::Duration::from_std(max_age).unwrap_or(chrono::Duration::MAX);

            if age > max_age_chrono {
                debug!(
                    "Item '{}' rejected: age {:?} > max {:?}",
                    item.title, age, max_age_chrono
                );
                return false;
            }
        }

        debug!("Item '{}' accepted: passed all filter checks", item.title);
        true
    }

    /// Process feed items: check if seen, apply filters, mark as seen, and optionally auto-download
    ///
    /// This method implements the core RSS feed processing logic:
    /// 1. Skips items that have already been seen (checks rss_seen table)
    /// 2. Applies filters to determine if items should be processed
    /// 3. Marks matching items as seen to prevent re-processing
    /// 4. Auto-downloads items if auto_download=true and item has NZB URL
    ///
    /// # Arguments
    /// * `feed_id` - Database ID of the feed (for seen tracking)
    /// * `feed_config` - Feed configuration containing filters and auto_download setting
    /// * `items` - Vector of RSS items from the feed
    ///
    /// # Returns
    /// Number of items that were auto-downloaded (0 if auto_download=false)
    ///
    /// # Errors
    /// Returns error if database operations or NZB downloads fail
    pub async fn process_feed_items(
        &self,
        feed_id: i64,
        feed_config: &RssFeedConfig,
        items: Vec<RssItem>,
    ) -> Result<usize> {
        let mut downloaded_count = 0;

        for item in items {
            // Skip if already seen
            if self.db.is_rss_item_seen(feed_id, &item.guid).await? {
                debug!("Skipping already seen item: {}", item.title);
                continue;
            }

            // Check if item matches any of the configured filters
            let matches = if feed_config.filters.is_empty() {
                // No filters = accept everything
                true
            } else {
                // At least one filter must match
                feed_config
                    .filters
                    .iter()
                    .any(|filter| self.matches_filters(&item, filter))
            };

            if !matches {
                debug!("Item '{}' did not match any filters, skipping", item.title);
                continue;
            }

            // Mark as seen to prevent re-processing
            self.db.mark_rss_item_seen(feed_id, &item.guid).await?;
            info!("New RSS item matched filters: {}", item.title);

            // Auto-download if enabled and NZB URL is available
            if feed_config.auto_download {
                if let Some(nzb_url) = &item.nzb_url {
                    let options = crate::types::DownloadOptions {
                        category: feed_config.category.clone(),
                        destination: None,
                        post_process: None,
                        priority: feed_config.priority,
                        password: None,
                    };

                    match self.downloader.add_nzb_url(nzb_url, options).await {
                        Ok(download_id) => {
                            info!(
                                "Auto-downloaded '{}' from RSS feed (download_id: {})",
                                item.title, download_id
                            );
                            downloaded_count += 1;
                        }
                        Err(e) => {
                            warn!(
                                "Failed to auto-download '{}' from RSS feed: {}",
                                item.title, e
                            );
                        }
                    }
                } else {
                    debug!("Item '{}' has no NZB URL, cannot auto-download", item.title);
                }
            }
        }

        Ok(downloaded_count)
    }
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests;
