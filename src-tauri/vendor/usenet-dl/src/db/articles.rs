//! Article-level tracking operations for download resume support.

use crate::error::DatabaseError;
use crate::types::DownloadId;
use crate::{Error, Result};

use super::{Article, Database, DownloadFile, NewArticle, article_status};

impl Database {
    /// Insert a single article
    pub async fn insert_article(&self, article: &NewArticle) -> Result<i64> {
        let result = sqlx::query(
            r#"
            INSERT INTO download_articles (
                download_id, message_id, segment_number, file_index, size_bytes, status
            ) VALUES (?, ?, ?, ?, ?, 0)
            "#,
        )
        .bind(article.download_id)
        .bind(&article.message_id)
        .bind(article.segment_number)
        .bind(article.file_index)
        .bind(article.size_bytes)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to insert article: {}",
                e
            )))
        })?;

        Ok(result.last_insert_rowid())
    }

    /// Insert multiple articles in a batch (more efficient for large NZB files)
    ///
    /// Automatically chunks the input to stay within SQLite's bind variable limit
    /// (5 variables per article, chunked to max 199 articles per INSERT).
    pub async fn insert_articles_batch(&self, articles: &[NewArticle]) -> Result<()> {
        if articles.is_empty() {
            return Ok(());
        }

        // SQLite default SQLITE_MAX_VARIABLE_NUMBER is 999.
        // Each article uses 6 bind variables, so max 166 articles per batch.
        const MAX_ARTICLES_PER_BATCH: usize = 166;

        for chunk in articles.chunks(MAX_ARTICLES_PER_BATCH) {
            let mut query_builder = sqlx::QueryBuilder::new(
                "INSERT INTO download_articles (download_id, message_id, segment_number, file_index, size_bytes, status) ",
            );

            query_builder.push_values(chunk, |mut b, article| {
                b.push_bind(article.download_id)
                    .push_bind(&article.message_id)
                    .push_bind(article.segment_number)
                    .push_bind(article.file_index)
                    .push_bind(article.size_bytes)
                    .push_bind(0); // status = PENDING
            });

            let query = query_builder.build();
            query.execute(&self.pool).await.map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to insert articles batch: {}",
                    e
                )))
            })?;
        }

        Ok(())
    }

    /// Update article status
    pub async fn update_article_status(&self, article_id: i64, status: i32) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            UPDATE download_articles
            SET status = ?, downloaded_at = ?
            WHERE id = ?
            "#,
        )
        .bind(status)
        .bind(if status == article_status::DOWNLOADED {
            Some(now)
        } else {
            None
        })
        .bind(article_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to update article status: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Update article status by message_id
    pub async fn update_article_status_by_message_id(
        &self,
        download_id: DownloadId,
        message_id: &str,
        status: i32,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            r#"
            UPDATE download_articles
            SET status = ?, downloaded_at = ?
            WHERE download_id = ? AND message_id = ?
            "#,
        )
        .bind(status)
        .bind(if status == article_status::DOWNLOADED {
            Some(now)
        } else {
            None
        })
        .bind(download_id)
        .bind(message_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to update article status: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Update multiple article statuses in a single transaction (more efficient for batch operations)
    ///
    /// # Arguments
    /// * `updates` - Vector of tuples containing (article_id, status)
    ///
    /// # Performance
    /// This method uses a CASE-WHEN statement to update multiple rows in a single query,
    /// which is significantly faster than individual UPDATE statements. With 100 updates,
    /// this can be 50-100x faster than calling `update_article_status` 100 times.
    ///
    /// # Example
    /// ```rust,ignore
    /// let updates = vec![
    ///     (123, article_status::DOWNLOADED),
    ///     (124, article_status::DOWNLOADED),
    ///     (125, article_status::FAILED),
    /// ];
    /// db.update_articles_status_batch(&updates).await?;
    /// ```
    /// Update multiple article statuses in a single transaction (more efficient for batch operations)
    ///
    /// Automatically chunks the input to stay within SQLite's bind variable limit.
    /// Each update uses ~3-4 bind variables (article_id x3 + optional timestamp),
    /// so we chunk to max 100 updates per query.
    pub async fn update_articles_status_batch(&self, updates: &[(i64, i32)]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }

        // Each update uses up to 4 bind variables (id in status CASE, status, id in downloaded_at CASE,
        // optional timestamp, id in WHERE IN). Conservative limit of 100 per batch.
        const MAX_UPDATES_PER_BATCH: usize = 100;

        let now = chrono::Utc::now().timestamp();

        for chunk in updates.chunks(MAX_UPDATES_PER_BATCH) {
            let mut query_builder =
                sqlx::QueryBuilder::new("UPDATE download_articles SET status = CASE ");

            // Build status CASE clause
            for (article_id, status) in chunk {
                query_builder.push("WHEN id = ");
                query_builder.push_bind(*article_id);
                query_builder.push(" THEN ");
                query_builder.push_bind(*status);
                query_builder.push(" ");
            }
            query_builder.push("END, downloaded_at = CASE ");

            // Build downloaded_at CASE clause (only set timestamp for DOWNLOADED status)
            for (article_id, status) in chunk {
                query_builder.push("WHEN id = ");
                query_builder.push_bind(*article_id);
                if *status == article_status::DOWNLOADED {
                    query_builder.push(" THEN ");
                    query_builder.push_bind(now);
                } else {
                    query_builder.push(" THEN downloaded_at"); // Keep existing value
                }
                query_builder.push(" ");
            }
            query_builder.push("END WHERE id IN (");

            // Build WHERE IN clause
            let mut first = true;
            for (article_id, _) in chunk {
                if !first {
                    query_builder.push(", ");
                }
                query_builder.push_bind(*article_id);
                first = false;
            }
            query_builder.push(")");

            let query = query_builder.build();
            query.execute(&self.pool).await.map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to update articles status batch: {}",
                    e
                )))
            })?;
        }

        Ok(())
    }

    /// Get all articles for a download
    pub async fn get_articles(&self, download_id: DownloadId) -> Result<Vec<Article>> {
        let rows = sqlx::query_as::<_, Article>(
            r#"
            SELECT id, download_id, message_id, segment_number, file_index, size_bytes, status, downloaded_at
            FROM download_articles
            WHERE download_id = ?
            ORDER BY file_index ASC, segment_number ASC
            "#,
        )
        .bind(download_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get articles: {}",
                e
            )))
        })?;

        Ok(rows)
    }

    /// Get pending articles for a download, excluding paused files.
    pub async fn get_pending_articles(&self, download_id: DownloadId) -> Result<Vec<Article>> {
        let rows = sqlx::query_as::<_, Article>(
            r#"
            SELECT da.id, da.download_id, da.message_id, da.segment_number, da.file_index, da.size_bytes, da.status, da.downloaded_at
            FROM download_articles da
            LEFT JOIN download_files df
              ON df.download_id = da.download_id
             AND df.file_index = da.file_index
            WHERE da.download_id = ?
              AND da.status = 0
              AND COALESCE(df.paused, 0) = 0
            ORDER BY da.file_index ASC, da.segment_number ASC
            "#,
        )
        .bind(download_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get pending articles: {}",
                e
            )))
        })?;

        Ok(rows)
    }

    /// Get article by message_id
    pub async fn get_article_by_message_id(
        &self,
        download_id: DownloadId,
        message_id: &str,
    ) -> Result<Option<Article>> {
        let row = sqlx::query_as::<_, Article>(
            r#"
            SELECT id, download_id, message_id, segment_number, file_index, size_bytes, status, downloaded_at
            FROM download_articles
            WHERE download_id = ? AND message_id = ?
            "#,
        )
        .bind(download_id)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get article: {}",
                e
            )))
        })?;

        Ok(row)
    }

    /// Count articles by status for a download
    pub async fn count_articles_by_status(
        &self,
        download_id: DownloadId,
        status: i32,
    ) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM download_articles WHERE download_id = ? AND status = ?",
        )
        .bind(download_id)
        .bind(status)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to count articles: {}",
                e
            )))
        })?;

        Ok(count)
    }

    /// Get total article count for a download
    pub async fn count_articles(&self, download_id: DownloadId) -> Result<i64> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM download_articles WHERE download_id = ?")
                .bind(download_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| {
                    Error::Database(DatabaseError::QueryFailed(format!(
                        "Failed to count articles: {}",
                        e
                    )))
                })?;

        Ok(count)
    }

    /// Delete all articles for a download (automatic via CASCADE, but explicit method for clarity)
    pub async fn delete_articles(&self, download_id: DownloadId) -> Result<()> {
        sqlx::query("DELETE FROM download_articles WHERE download_id = ?")
            .bind(download_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to delete articles: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Insert multiple download files in a batch
    pub async fn insert_files_batch(&self, files: &[super::NewDownloadFile]) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }

        // Each file uses 5 bind variables, max 199 per batch
        const MAX_FILES_PER_BATCH: usize = 199;

        for chunk in files.chunks(MAX_FILES_PER_BATCH) {
            let mut query_builder = sqlx::QueryBuilder::new(
                "INSERT INTO download_files (download_id, file_index, filename, subject, total_segments) ",
            );

            query_builder.push_values(chunk, |mut b, file| {
                b.push_bind(file.download_id)
                    .push_bind(file.file_index)
                    .push_bind(&file.filename)
                    .push_bind(&file.subject)
                    .push_bind(file.total_segments);
            });

            let query = query_builder.build();
            query.execute(&self.pool).await.map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to insert files batch: {}",
                    e
                )))
            })?;
        }

        Ok(())
    }

    /// Get all download files for a download
    pub async fn get_download_files(
        &self,
        download_id: DownloadId,
    ) -> Result<Vec<super::DownloadFile>> {
        let rows = sqlx::query_as::<_, super::DownloadFile>(
            r#"
            SELECT id, download_id, file_index, filename, subject, total_segments, paused, completed, original_filename
            FROM download_files
            WHERE download_id = ?
            ORDER BY file_index ASC
            "#,
        )
        .bind(download_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get download files: {}",
                e
            )))
        })?;

        Ok(rows)
    }

    /// Get newly completed files for DirectUnpack processing.
    ///
    /// Returns unpaused files where `completed=0` and all articles have been downloaded.
    pub async fn get_newly_completed_files(
        &self,
        download_id: DownloadId,
    ) -> Result<Vec<super::DownloadFile>> {
        let rows = sqlx::query_as::<_, super::DownloadFile>(
            r#"
            SELECT df.id, df.download_id, df.file_index, df.filename, df.subject,
                   df.total_segments, df.paused, df.completed, df.original_filename
            FROM download_files df
            WHERE df.download_id = ?
              AND df.paused = 0
              AND df.completed = 0
              AND df.total_segments = (
                SELECT COUNT(*) FROM download_articles da
                WHERE da.download_id = df.download_id
                  AND da.file_index = df.file_index
                  AND da.status = 1
              )
            "#,
        )
        .bind(download_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get newly completed files: {}",
                e
            )))
        })?;

        Ok(rows)
    }

    /// Mark a file as completed (all segments downloaded)
    pub async fn mark_file_completed(
        &self,
        download_id: DownloadId,
        file_index: i32,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE download_files SET completed = 1 WHERE download_id = ? AND file_index = ?",
        )
        .bind(download_id)
        .bind(file_index)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to mark file completed: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Update the DirectUnpack state for a download
    pub async fn update_direct_unpack_state(
        &self,
        download_id: DownloadId,
        state: i32,
    ) -> Result<()> {
        sqlx::query("UPDATE downloads SET direct_unpack_state = ? WHERE id = ?")
            .bind(state)
            .bind(download_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to update direct_unpack_state: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Get the DirectUnpack state for a download
    pub async fn get_direct_unpack_state(&self, download_id: DownloadId) -> Result<i32> {
        let state: i32 =
            sqlx::query_scalar("SELECT direct_unpack_state FROM downloads WHERE id = ?")
                .bind(download_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| {
                    Error::Database(DatabaseError::QueryFailed(format!(
                        "Failed to get direct_unpack_state: {}",
                        e
                    )))
                })?;

        Ok(state)
    }

    /// Rename a download file (for DirectRename), storing the original filename
    pub async fn rename_download_file(
        &self,
        download_id: DownloadId,
        file_index: i32,
        new_filename: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE download_files
            SET original_filename = CASE WHEN original_filename IS NULL THEN filename ELSE original_filename END,
                filename = ?
            WHERE download_id = ? AND file_index = ?
            "#,
        )
        .bind(new_filename)
        .bind(download_id)
        .bind(file_index)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to rename download file: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Update the DirectUnpack extracted count for a download
    pub async fn update_direct_unpack_extracted_count(
        &self,
        download_id: DownloadId,
        count: i32,
    ) -> Result<()> {
        sqlx::query("UPDATE downloads SET direct_unpack_extracted_count = ? WHERE id = ?")
            .bind(count)
            .bind(download_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::QueryFailed(format!(
                    "Failed to update direct_unpack_extracted_count: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Get the DirectUnpack extracted count for a download
    pub async fn get_direct_unpack_extracted_count(&self, download_id: DownloadId) -> Result<i32> {
        let count: i32 =
            sqlx::query_scalar("SELECT direct_unpack_extracted_count FROM downloads WHERE id = ?")
                .bind(download_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| {
                    Error::Database(DatabaseError::QueryFailed(format!(
                        "Failed to get direct_unpack_extracted_count: {}",
                        e
                    )))
                })?;

        Ok(count)
    }

    /// Set a file's paused state.
    pub async fn set_file_paused(
        &self,
        download_id: DownloadId,
        file_index: i32,
        paused: bool,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE download_files SET paused = ? WHERE download_id = ? AND file_index = ?",
        )
        .bind(if paused { 1 } else { 0 })
        .bind(download_id)
        .bind(file_index)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to update file paused state: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Get a single download file by download and file index.
    pub async fn get_download_file(
        &self,
        download_id: DownloadId,
        file_index: i32,
    ) -> Result<Option<DownloadFile>> {
        let row = sqlx::query_as::<_, DownloadFile>(
            r#"
            SELECT id, download_id, file_index, filename, subject, total_segments, paused, completed, original_filename
            FROM download_files
            WHERE download_id = ? AND file_index = ?
            "#,
        )
        .bind(download_id)
        .bind(file_index)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to get download file: {}",
                e
            )))
        })?;

        Ok(row)
    }

    /// Return true when a download still has unpaused pending articles.
    pub async fn has_active_pending_articles(&self, download_id: DownloadId) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM download_articles da
            LEFT JOIN download_files df
              ON df.download_id = da.download_id
             AND df.file_index = da.file_index
            WHERE da.download_id = ?
              AND da.status = 0
              AND COALESCE(df.paused, 0) = 0
            "#,
        )
        .bind(download_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to count active pending articles: {}",
                e
            )))
        })?;

        Ok(count > 0)
    }

    /// Return true when a download still has any pending articles, including paused files.
    pub async fn has_any_pending_articles(&self, download_id: DownloadId) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM download_articles WHERE download_id = ? AND status = 0",
        )
        .bind(download_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to count pending articles: {}",
                e
            )))
        })?;

        Ok(count > 0)
    }

    /// Count failed articles for a download
    pub async fn count_failed_articles(&self, download_id: DownloadId) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM download_articles WHERE download_id = ? AND status = 2",
        )
        .bind(download_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::QueryFailed(format!(
                "Failed to count failed articles: {}",
                e
            )))
        })?;

        Ok(count)
    }
}
