//! RSS feed management — CRUD operations and feed checking.

use crate::error::{Error, Result};
use std::sync::Arc;

use super::UsenetDownloader;

impl UsenetDownloader {
    /// Get all RSS feeds
    pub async fn get_rss_feeds(&self) -> Result<Vec<crate::config::RssFeedConfig>> {
        use std::time::Duration;

        let feeds = self.db.get_all_rss_feeds().await?;
        let mut result = Vec::new();

        for feed in feeds {
            // Get filters for this feed
            let filter_rows = self.db.get_rss_filters(feed.id).await?;
            let filters = filter_rows
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

            result.push(crate::config::RssFeedConfig {
                url: feed.url,
                check_interval: Duration::from_secs(feed.check_interval_secs as u64),
                category: feed.category,
                filters,
                auto_download: feed.auto_download != 0,
                priority: crate::types::Priority::from_i32(feed.priority),
                enabled: feed.enabled != 0,
            });
        }

        Ok(result)
    }

    /// Get RSS feed by ID with its configuration
    pub async fn get_rss_feed(
        &self,
        id: i64,
    ) -> Result<Option<(i64, String, crate::config::RssFeedConfig)>> {
        use std::time::Duration;

        let feed = match self.db.get_rss_feed(id).await? {
            Some(f) => f,
            None => return Ok(None),
        };

        // Get filters for this feed
        let filter_rows = self.db.get_rss_filters(feed.id).await?;
        let filters = filter_rows
            .into_iter()
            .map(|row| crate::config::RssFilter {
                name: row.name,
                include: row
                    .include_patterns
                    .map(|s| serde_json::from_str(&s).unwrap_or_default())
                    .unwrap_or_default(),
                exclude: row
                    .exclude_patterns
                    .map(|s| serde_json::from_str(&s).unwrap_or_default())
                    .unwrap_or_default(),
                min_size: row.min_size.map(|s| s as u64),
                max_size: row.max_size.map(|s| s as u64),
                max_age: row.max_age_secs.map(|s| Duration::from_secs(s as u64)),
            })
            .collect();

        let config = crate::config::RssFeedConfig {
            url: feed.url,
            check_interval: Duration::from_secs(feed.check_interval_secs as u64),
            category: feed.category,
            filters,
            auto_download: feed.auto_download != 0,
            priority: crate::types::Priority::from_i32(feed.priority),
            enabled: feed.enabled != 0,
        };

        Ok(Some((feed.id, feed.name, config)))
    }

    /// Add a new RSS feed
    pub async fn add_rss_feed(
        &self,
        name: &str,
        config: crate::config::RssFeedConfig,
    ) -> Result<i64> {
        // Insert the feed
        let feed_id = self
            .db
            .insert_rss_feed(crate::db::InsertRssFeedParams {
                name,
                url: &config.url,
                check_interval_secs: config.check_interval.as_secs() as i64,
                category: config.category.as_deref(),
                auto_download: config.auto_download,
                priority: config.priority as i32,
                enabled: config.enabled,
            })
            .await?;

        // Insert filters
        for filter in &config.filters {
            let include_json = if filter.include.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&filter.include)?)
            };

            let exclude_json = if filter.exclude.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&filter.exclude)?)
            };

            self.db
                .insert_rss_filter(crate::db::InsertRssFilterParams {
                    feed_id,
                    name: &filter.name,
                    include_patterns: include_json.as_deref(),
                    exclude_patterns: exclude_json.as_deref(),
                    min_size: filter.min_size.map(|s| s as i64),
                    max_size: filter.max_size.map(|s| s as i64),
                    max_age_secs: filter.max_age.map(|d| d.as_secs() as i64),
                })
                .await?;
        }

        Ok(feed_id)
    }

    /// Update an existing RSS feed
    pub async fn update_rss_feed(
        &self,
        id: i64,
        name: &str,
        config: crate::config::RssFeedConfig,
    ) -> Result<bool> {
        // Update the feed
        let updated = self
            .db
            .update_rss_feed(crate::db::UpdateRssFeedParams {
                id,
                name,
                url: &config.url,
                check_interval_secs: config.check_interval.as_secs() as i64,
                category: config.category.as_deref(),
                auto_download: config.auto_download,
                priority: config.priority as i32,
                enabled: config.enabled,
            })
            .await?;

        if !updated {
            return Ok(false);
        }

        // Delete old filters and insert new ones
        self.db.delete_rss_filters(id).await?;

        for filter in &config.filters {
            let include_json = if filter.include.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&filter.include)?)
            };

            let exclude_json = if filter.exclude.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&filter.exclude)?)
            };

            self.db
                .insert_rss_filter(crate::db::InsertRssFilterParams {
                    feed_id: id,
                    name: &filter.name,
                    include_patterns: include_json.as_deref(),
                    exclude_patterns: exclude_json.as_deref(),
                    min_size: filter.min_size.map(|s| s as i64),
                    max_size: filter.max_size.map(|s| s as i64),
                    max_age_secs: filter.max_age.map(|d| d.as_secs() as i64),
                })
                .await?;
        }

        Ok(true)
    }

    /// Delete an RSS feed
    pub async fn delete_rss_feed(&self, id: i64) -> Result<bool> {
        self.db.delete_rss_feed(id).await
    }

    /// Force check an RSS feed now (for manual triggering via API)
    pub async fn check_rss_feed_now(&self, id: i64) -> Result<usize> {
        // Get the feed configuration
        let (feed_id, _name, config) = match self.get_rss_feed(id).await? {
            Some(f) => f,
            None => return Err(Error::NotFound(format!("RSS feed {} not found", id))),
        };

        // Create a temporary RssManager with just this feed
        let rss_manager = crate::rss_manager::RssManager::new(
            self.db.clone(),
            Arc::new(self.clone()),
            vec![config.clone()],
        )?;

        // Check the feed
        let items = rss_manager.check_feed(&config).await?;

        // Process items (auto-download if enabled)
        let queued = rss_manager
            .process_feed_items(feed_id, &config, items)
            .await?;

        // Update last check status
        self.db.update_rss_feed_check_status(id, None).await?;

        Ok(queued)
    }
}
