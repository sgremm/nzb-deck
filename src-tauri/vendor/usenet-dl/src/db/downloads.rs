//! Download queue CRUD operations.

use crate::error::DatabaseError;
use crate::types::DownloadId;
use crate::{Error, Result};

use super::{Database, Download, NewDownload};

impl Database {
    /// Insert a new download record
    pub async fn insert_download(&self, download: &NewDownload) -> Result<DownloadId> {
        let now = chrono::Utc::now().timestamp();

        let result = sqlx::query(
            r#"
            INSERT INTO downloads (
                name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&download.name)
        .bind(&download.nzb_path)
        .bind(&download.nzb_meta_name)
        .bind(&download.nzb_hash)
        .bind(&download.job_name)
        .bind(&download.category)
        .bind(&download.destination)
        .bind(download.post_process)
        .bind(download.priority)
        .bind(download.status)
        .bind(0.0f32) // progress
        .bind(0i64) // speed_bps
        .bind(download.size_bytes)
        .bind(0i64) // downloaded_bytes
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to insert download: {}",
                e
            )))
        })?;

        Ok(DownloadId(result.last_insert_rowid()))
    }

    /// Get a download by ID
    pub async fn get_download(&self, id: DownloadId) -> Result<Option<Download>> {
        let row = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at,
                direct_unpack_state, direct_unpack_extracted_count
            FROM downloads
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get download: {}",
                e
            )))
        })?;

        Ok(row)
    }

    /// List all downloads
    pub async fn list_downloads(&self) -> Result<Vec<Download>> {
        let rows = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at,
                direct_unpack_state, direct_unpack_extracted_count
            FROM downloads
            ORDER BY priority DESC, created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to list downloads: {}",
                e
            )))
        })?;

        Ok(rows)
    }

    /// List downloads with a specific status
    pub async fn list_downloads_by_status(&self, status: i32) -> Result<Vec<Download>> {
        let rows = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at,
                direct_unpack_state, direct_unpack_extracted_count
            FROM downloads
            WHERE status = ?
            ORDER BY priority DESC, created_at ASC
            "#,
        )
        .bind(status)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to list downloads by status: {}",
                e
            )))
        })?;

        Ok(rows)
    }

    /// Update download status
    pub async fn update_status(&self, id: DownloadId, status: i32) -> Result<()> {
        sqlx::query("UPDATE downloads SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to update status: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Update the final destination used by post-processing.
    pub async fn update_destination(&self, id: DownloadId, destination: &str) -> Result<()> {
        sqlx::query("UPDATE downloads SET destination = ? WHERE id = ?")
            .bind(destination)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|error| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to update destination: {error}"
                )))
            })?;
        Ok(())
    }

    /// Update download progress
    pub async fn update_progress(
        &self,
        id: DownloadId,
        progress: f32,
        speed_bps: u64,
        downloaded_bytes: u64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE downloads SET progress = ?, speed_bps = ?, downloaded_bytes = ? WHERE id = ?",
        )
        .bind(progress)
        .bind(speed_bps as i64)
        .bind(downloaded_bytes as i64)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to update progress: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Update download priority
    pub async fn update_priority(&self, id: DownloadId, priority: i32) -> Result<()> {
        sqlx::query("UPDATE downloads SET priority = ? WHERE id = ?")
            .bind(priority)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to update priority: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Set download error message
    pub async fn set_error(&self, id: DownloadId, error: &str) -> Result<()> {
        sqlx::query("UPDATE downloads SET error_message = ? WHERE id = ?")
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to set error: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Set download started timestamp
    pub async fn set_started(&self, id: DownloadId) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query("UPDATE downloads SET started_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to set started timestamp: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Set download completed timestamp
    pub async fn set_completed(&self, id: DownloadId) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query("UPDATE downloads SET completed_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to set completed timestamp: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Delete a download
    pub async fn delete_download(&self, id: DownloadId) -> Result<()> {
        sqlx::query("DELETE FROM downloads WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to delete download: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Get incomplete downloads (for resume on startup)
    pub async fn get_incomplete_downloads(&self) -> Result<Vec<Download>> {
        let rows = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at,
                direct_unpack_state, direct_unpack_extracted_count
            FROM downloads
            WHERE status IN (0, 1, 3)
            ORDER BY priority DESC, created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get incomplete downloads: {}",
                e
            )))
        })?;

        Ok(rows)
    }

    /// Get all downloads (for state persistence during shutdown)
    pub async fn get_all_downloads(&self) -> Result<Vec<Download>> {
        let rows = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at,
                direct_unpack_state, direct_unpack_extracted_count
            FROM downloads
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get all downloads: {}",
                e
            )))
        })?;

        Ok(rows)
    }
}
