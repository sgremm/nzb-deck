//! History management operations.

use crate::types::HistoryEntry;
use crate::{Error, Result};

use super::{Database, HistoryRow, NewHistoryEntry};

impl Database {
    /// Insert a download into history
    ///
    /// This is typically called when a download is completed (successfully or failed)
    /// to create a historical record separate from the active downloads table.
    pub async fn insert_history(&self, entry: &NewHistoryEntry) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO history (
                name, category, destination, status, size_bytes,
                download_time_secs, completed_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&entry.name)
        .bind(&entry.category)
        .bind(
            entry
                .destination
                .as_ref()
                .and_then(|p| p.to_str().map(String::from)),
        )
        .bind(entry.status)
        .bind(entry.size_bytes as i64)
        .bind(entry.download_time_secs)
        .bind(entry.completed_at)
        .execute(&self.pool)
        .await
        .map_err(Error::Sqlx)?;

        Ok(result.last_insert_rowid())
    }

    /// Query history with pagination and optional status filter
    ///
    /// Returns history entries ordered by completion time (most recent first).
    /// Use limit and offset for pagination.
    pub async fn query_history(
        &self,
        status_filter: Option<i32>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<HistoryEntry>> {
        let query = if let Some(status) = status_filter {
            sqlx::query_as::<_, HistoryRow>(
                r#"
                SELECT id, name, category, destination, status, size_bytes,
                       download_time_secs, completed_at
                FROM history
                WHERE status = ?
                ORDER BY completed_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(status)
            .bind(limit as i64)
            .bind(offset as i64)
        } else {
            sqlx::query_as::<_, HistoryRow>(
                r#"
                SELECT id, name, category, destination, status, size_bytes,
                       download_time_secs, completed_at
                FROM history
                ORDER BY completed_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(limit as i64)
            .bind(offset as i64)
        };

        let rows = query.fetch_all(&self.pool).await.map_err(Error::Sqlx)?;

        Ok(rows.into_iter().map(HistoryEntry::from).collect())
    }

    /// Count history entries (optionally filtered by status)
    ///
    /// Useful for pagination - returns total count of records matching the filter.
    pub async fn count_history(&self, status_filter: Option<i32>) -> Result<i64> {
        let count = if let Some(status) = status_filter {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM history WHERE status = ?")
                .bind(status)
                .fetch_one(&self.pool)
                .await
                .map_err(Error::Sqlx)?
        } else {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM history")
                .fetch_one(&self.pool)
                .await
                .map_err(Error::Sqlx)?
        };

        Ok(count)
    }

    /// Delete history entries older than the specified timestamp
    ///
    /// Returns the number of records deleted.
    /// This is useful for cleanup - e.g., delete history older than 30 days.
    pub async fn delete_history_before(&self, before_timestamp: i64) -> Result<u64> {
        let result = sqlx::query("DELETE FROM history WHERE completed_at < ?")
            .bind(before_timestamp)
            .execute(&self.pool)
            .await
            .map_err(Error::Sqlx)?;

        Ok(result.rows_affected())
    }

    /// Delete history entries with a specific status
    ///
    /// Returns the number of records deleted.
    /// This is useful for cleanup - e.g., delete all failed downloads from history.
    pub async fn delete_history_by_status(&self, status: i32) -> Result<u64> {
        let result = sqlx::query("DELETE FROM history WHERE status = ?")
            .bind(status)
            .execute(&self.pool)
            .await
            .map_err(Error::Sqlx)?;

        Ok(result.rows_affected())
    }

    /// Clear all history
    ///
    /// Returns the number of records deleted.
    /// This is a destructive operation - use with caution.
    pub async fn clear_history(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM history")
            .execute(&self.pool)
            .await
            .map_err(Error::Sqlx)?;

        Ok(result.rows_affected())
    }

    /// Delete history entries with optional filters
    ///
    /// Returns the number of records deleted.
    /// Supports filtering by:
    /// - before_timestamp: Delete entries completed before this timestamp
    /// - status: Delete only entries with this status
    ///
    /// If both filters are None, deletes all history (same as clear_history).
    pub async fn delete_history_filtered(
        &self,
        before_timestamp: Option<i64>,
        status: Option<i32>,
    ) -> Result<u64> {
        match (before_timestamp, status) {
            (None, None) => {
                // No filters - delete all
                self.clear_history().await
            }
            (Some(before), None) => {
                // Only timestamp filter
                self.delete_history_before(before).await
            }
            (None, Some(status_val)) => {
                // Only status filter
                self.delete_history_by_status(status_val).await
            }
            (Some(before), Some(status_val)) => {
                // Both filters
                let result =
                    sqlx::query("DELETE FROM history WHERE completed_at < ? AND status = ?")
                        .bind(before)
                        .bind(status_val)
                        .execute(&self.pool)
                        .await
                        .map_err(Error::Sqlx)?;

                Ok(result.rows_affected())
            }
        }
    }

    /// Get a single history entry by ID
    pub async fn get_history_entry(&self, id: i64) -> Result<Option<HistoryEntry>> {
        let row = sqlx::query_as::<_, HistoryRow>(
            r#"
            SELECT id, name, category, destination, status, size_bytes,
                   download_time_secs, completed_at
            FROM history
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Error::Sqlx)?;

        Ok(row.map(HistoryEntry::from))
    }
}
