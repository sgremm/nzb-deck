//! Duplicate detection queries.

use crate::error::DatabaseError;
use crate::{Error, Result};

use super::{Database, Download};

impl Database {
    /// Find a download by NZB hash
    ///
    /// This is the most reliable duplicate detection method as it compares
    /// the actual NZB file content (via SHA-256 hash).
    pub async fn find_by_nzb_hash(&self, nzb_hash: &str) -> Result<Option<Download>> {
        let row = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at,
                direct_unpack_state, direct_unpack_extracted_count
            FROM downloads
            WHERE nzb_hash = ?
            LIMIT 1
            "#,
        )
        .bind(nzb_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to find download by nzb_hash: {}",
                e
            )))
        })?;

        Ok(row)
    }

    /// Find a download by exact name match
    ///
    /// This is useful for detecting duplicates when the NZB filename is used
    /// as the download name. Case-sensitive match.
    pub async fn find_by_name(&self, name: &str) -> Result<Option<Download>> {
        let row = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at,
                direct_unpack_state, direct_unpack_extracted_count
            FROM downloads
            WHERE name = ?
            LIMIT 1
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to find download by name: {}",
                e
            )))
        })?;

        Ok(row)
    }

    /// Find a download by job name
    ///
    /// This detects duplicates using the deobfuscated job name, which catches
    /// cases where the same content is uploaded with different NZB filenames.
    pub async fn find_by_job_name(&self, job_name: &str) -> Result<Option<Download>> {
        let row = sqlx::query_as::<_, Download>(
            r#"
            SELECT
                id, name, nzb_path, nzb_meta_name, nzb_hash, job_name,
                category, destination, post_process, priority, status,
                progress, speed_bps, size_bytes, downloaded_bytes,
                error_message, created_at, started_at, completed_at,
                direct_unpack_state, direct_unpack_extracted_count
            FROM downloads
            WHERE job_name = ?
            LIMIT 1
            "#,
        )
        .bind(job_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to find download by job_name: {}",
                e
            )))
        })?;

        Ok(row)
    }
}
