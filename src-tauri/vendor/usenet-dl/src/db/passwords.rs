//! Password cache operations for archive extraction.

use crate::error::DatabaseError;
use crate::types::DownloadId;
use crate::{Error, Result};

use super::Database;

impl Database {
    /// Cache a correct password for a download
    ///
    /// This is used after successfully extracting an archive with a password,
    /// so if the download needs to be reprocessed, we can try this password first.
    pub async fn set_correct_password(
        &self,
        download_id: DownloadId,
        password: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO passwords (download_id, correct_password)
            VALUES (?, ?)
            ON CONFLICT(download_id) DO UPDATE SET correct_password = excluded.correct_password
            "#,
        )
        .bind(download_id)
        .bind(password)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to set correct password: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Get the cached correct password for a download
    ///
    /// Returns None if no password is cached for this download.
    pub async fn get_cached_password(&self, download_id: DownloadId) -> Result<Option<String>> {
        let password: Option<String> =
            sqlx::query_scalar("SELECT correct_password FROM passwords WHERE download_id = ?")
                .bind(download_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    Error::Database(DatabaseError::QueryFailed(format!(
                        "Failed to get cached password: {}",
                        e
                    )))
                })?;

        Ok(password)
    }

    /// Delete cached password for a download
    ///
    /// Note: This is automatically deleted via CASCADE when the download is deleted.
    pub async fn delete_cached_password(&self, download_id: DownloadId) -> Result<()> {
        sqlx::query("DELETE FROM passwords WHERE download_id = ?")
            .bind(download_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to delete cached password: {}",
                    e
                )))
            })?;

        Ok(())
    }
}
