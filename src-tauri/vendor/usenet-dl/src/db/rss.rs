//! RSS feed CRUD operations.

use crate::error::DatabaseError;
use crate::{Error, Result};

use super::{
    Database, InsertRssFeedParams, InsertRssFilterParams, RssFeed, RssFilterRow,
    UpdateRssFeedParams,
};

impl Database {
    /// Get all RSS feeds
    pub async fn get_all_rss_feeds(&self) -> Result<Vec<RssFeed>> {
        let feeds = sqlx::query_as::<_, RssFeed>(
            r#"
            SELECT id, name, url, check_interval_secs, category, auto_download,
                   priority, enabled, last_check, last_error, created_at
            FROM rss_feeds
            ORDER BY id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get RSS feeds: {}",
                e
            )))
        })?;

        Ok(feeds)
    }

    /// Get RSS feed by ID
    pub async fn get_rss_feed(&self, id: i64) -> Result<Option<RssFeed>> {
        let feed = sqlx::query_as::<_, RssFeed>(
            r#"
            SELECT id, name, url, check_interval_secs, category, auto_download,
                   priority, enabled, last_check, last_error, created_at
            FROM rss_feeds
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get RSS feed: {}",
                e
            )))
        })?;

        Ok(feed)
    }

    /// Insert a new RSS feed
    pub async fn insert_rss_feed(&self, params: InsertRssFeedParams<'_>) -> Result<i64> {
        let InsertRssFeedParams {
            name,
            url,
            check_interval_secs,
            category,
            auto_download,
            priority,
            enabled,
        } = params;
        let now = chrono::Utc::now().timestamp();

        let result = sqlx::query(
            r#"
            INSERT INTO rss_feeds (name, url, check_interval_secs, category, auto_download,
                                  priority, enabled, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(name)
        .bind(url)
        .bind(check_interval_secs)
        .bind(category)
        .bind(auto_download as i32)
        .bind(priority)
        .bind(enabled as i32)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to insert RSS feed: {}",
                e
            )))
        })?;

        Ok(result.last_insert_rowid())
    }

    /// Update an existing RSS feed
    pub async fn update_rss_feed(&self, params: UpdateRssFeedParams<'_>) -> Result<bool> {
        let UpdateRssFeedParams {
            id,
            name,
            url,
            check_interval_secs,
            category,
            auto_download,
            priority,
            enabled,
        } = params;
        let result = sqlx::query(
            r#"
            UPDATE rss_feeds
            SET name = ?, url = ?, check_interval_secs = ?, category = ?,
                auto_download = ?, priority = ?, enabled = ?
            WHERE id = ?
            "#,
        )
        .bind(name)
        .bind(url)
        .bind(check_interval_secs)
        .bind(category)
        .bind(auto_download as i32)
        .bind(priority)
        .bind(enabled as i32)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to update RSS feed: {}",
                e
            )))
        })?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete an RSS feed (cascades to filters and seen items)
    pub async fn delete_rss_feed(&self, id: i64) -> Result<bool> {
        let result = sqlx::query("DELETE FROM rss_feeds WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to delete RSS feed: {}",
                    e
                )))
            })?;

        Ok(result.rows_affected() > 0)
    }

    /// Get all filters for a specific RSS feed
    pub async fn get_rss_filters(&self, feed_id: i64) -> Result<Vec<RssFilterRow>> {
        let filters = sqlx::query_as::<_, RssFilterRow>(
            r#"
            SELECT id, feed_id, name, include_patterns, exclude_patterns,
                   min_size, max_size, max_age_secs
            FROM rss_filters
            WHERE feed_id = ?
            ORDER BY id ASC
            "#,
        )
        .bind(feed_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get RSS filters: {}",
                e
            )))
        })?;

        Ok(filters)
    }

    /// Insert a new RSS filter
    pub async fn insert_rss_filter(&self, params: InsertRssFilterParams<'_>) -> Result<i64> {
        let InsertRssFilterParams {
            feed_id,
            name,
            include_patterns,
            exclude_patterns,
            min_size,
            max_size,
            max_age_secs,
        } = params;
        let result = sqlx::query(
            r#"
            INSERT INTO rss_filters (feed_id, name, include_patterns, exclude_patterns,
                                    min_size, max_size, max_age_secs)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(feed_id)
        .bind(name)
        .bind(include_patterns)
        .bind(exclude_patterns)
        .bind(min_size)
        .bind(max_size)
        .bind(max_age_secs)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to insert RSS filter: {}",
                e
            )))
        })?;

        Ok(result.last_insert_rowid())
    }

    /// Delete all filters for a feed (used during update)
    pub async fn delete_rss_filters(&self, feed_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM rss_filters WHERE feed_id = ?")
            .bind(feed_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to delete RSS filters: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Update last check time and error for an RSS feed
    pub async fn update_rss_feed_check_status(
        &self,
        id: i64,
        last_error: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            UPDATE rss_feeds
            SET last_check = ?, last_error = ?
            WHERE id = ?
            "#,
        )
        .bind(now)
        .bind(last_error)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to update RSS feed status: {}",
                e
            )))
        })?;

        Ok(())
    }
}
