//! Runtime state tracking: shutdown detection, NZB processing, RSS seen items.

use crate::error::DatabaseError;
use crate::{Error, Result};

use super::Database;

impl Database {
    /// Check if the last shutdown was unclean
    ///
    /// Returns true if the previous session did not call set_clean_shutdown(),
    /// indicating a crash or forced termination.
    ///
    /// This method is called on startup to determine if state recovery is needed.
    pub async fn was_unclean_shutdown(&self) -> Result<bool> {
        let value: Option<String> = sqlx::query_scalar(
            r#"
            SELECT value FROM runtime_state WHERE key = 'clean_shutdown'
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to check shutdown state: {}",
                e
            )))
        })?;

        // If the value is missing or "false", it was an unclean shutdown
        Ok(value.is_none_or(|v| v != "true"))
    }

    /// Mark that the application has started cleanly
    ///
    /// This should be called during UsenetDownloader::new() to indicate that
    /// the application is running. If shutdown() is not called before the next
    /// startup, was_unclean_shutdown() will return true.
    pub async fn set_clean_start(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            r#"
            INSERT INTO runtime_state (key, value, updated_at)
            VALUES ('clean_shutdown', 'false', ?)
            ON CONFLICT(key) DO UPDATE SET value = 'false', updated_at = ?
            "#,
        )
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to set clean start: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Mark that the application is shutting down cleanly
    ///
    /// This should be called during UsenetDownloader::shutdown() to indicate
    /// a graceful shutdown. If this is not called before the process exits,
    /// the next startup will detect an unclean shutdown.
    pub async fn set_clean_shutdown(&self) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            r#"
            INSERT INTO runtime_state (key, value, updated_at)
            VALUES ('clean_shutdown', 'true', ?)
            ON CONFLICT(key) DO UPDATE SET value = 'true', updated_at = ?
            "#,
        )
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to set clean shutdown: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Mark an NZB file as processed
    ///
    /// This is used by the folder watcher with WatchFolderAction::Keep to track
    /// which NZB files have already been processed to avoid re-adding them.
    pub async fn mark_nzb_processed(&self, path: &std::path::Path) -> Result<()> {
        let path_str = path.to_string_lossy().into_owned();
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            INSERT INTO processed_nzbs (path, processed_at)
            VALUES (?, ?)
            ON CONFLICT(path) DO UPDATE SET processed_at = ?
            "#,
        )
        .bind(&path_str)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to mark NZB as processed: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Check if an NZB file has been processed
    pub async fn is_nzb_processed(&self, path: &std::path::Path) -> Result<bool> {
        let path_str = path.to_string_lossy().into_owned();

        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM processed_nzbs WHERE path = ?
            "#,
        )
        .bind(&path_str)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to check if NZB is processed: {}",
                e
            )))
        })?;

        Ok(count > 0)
    }

    /// Check if an RSS feed item has been seen before
    pub async fn is_rss_item_seen(&self, feed_id: i64, guid: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM rss_seen WHERE feed_id = ? AND guid = ?
            "#,
        )
        .bind(feed_id)
        .bind(guid)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to check if RSS item is seen: {}",
                e
            )))
        })?;

        Ok(count > 0)
    }

    /// Mark an RSS feed item as seen
    pub async fn mark_rss_item_seen(&self, feed_id: i64, guid: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            INSERT INTO rss_seen (feed_id, guid, seen_at)
            VALUES (?, ?, ?)
            ON CONFLICT(feed_id, guid) DO UPDATE SET seen_at = ?
            "#,
        )
        .bind(feed_id)
        .bind(guid)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to mark RSS item as seen: {}",
                e
            )))
        })?;

        Ok(())
    }
}
