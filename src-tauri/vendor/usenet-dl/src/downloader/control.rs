//! Download lifecycle control — pause, resume, cancel, priority, reprocess.

use crate::error::{DatabaseError, DownloadError, Error, Result};
use crate::types::{DownloadId, Event, Priority, Stage, Status};
use std::path::PathBuf;

use super::UsenetDownloader;

impl UsenetDownloader {
    /// Pause a download
    ///
    /// This method pauses a download without removing it from the queue.
    /// If the download is currently downloading, it will be stopped gracefully
    /// (after completing the current article). The download will be marked as
    /// Paused in the database and can be resumed later with `resume()`.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to pause
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if the download was successfully paused, or an error if:
    /// - The download doesn't exist
    /// - The download is already paused, complete, or failed
    /// - Database update fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader, id: DownloadId) -> Result<()> {
    /// // Pause a download
    /// downloader.pause(id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn pause(&self, id: DownloadId) -> Result<()> {
        // Fetch download from database
        let download = self.db.get_download(id).await?.ok_or_else(|| {
            Error::Database(DatabaseError::NotFound(format!(
                "Download {} not found",
                id
            )))
        })?;

        let current_status = Status::from_i32(download.status);

        // Check if download can be paused
        match current_status {
            Status::Paused => {
                // Already paused, nothing to do
                return Ok(());
            }
            Status::Complete | Status::Failed => {
                return Err(Error::Download(DownloadError::InvalidState {
                    id: id.into(),
                    operation: "pause".to_string(),
                    current_state: format!("{:?}", current_status),
                }));
            }
            Status::Queued | Status::Downloading | Status::Processing => {
                // Can be paused
            }
        }

        // If download is actively running, cancel its task
        let mut active_downloads = self.queue_state.active_downloads.lock().await;
        if let Some(cancel_token) = active_downloads.get(&id) {
            // Signal the download task to stop
            cancel_token.cancel();
            // Remove from active downloads (task will clean up)
            active_downloads.remove(&id);
        }
        drop(active_downloads); // Release lock

        // Remove from queue if it's still queued (not yet started)
        self.remove_from_queue(id).await;

        // Update status to Paused in database
        self.db.update_status(id, Status::Paused.to_i32()).await?;

        Ok(())
    }

    /// Resume a paused download
    ///
    /// This method restarts a paused download by changing its status back to Queued
    /// and adding it to the priority queue. The queue processor will automatically
    /// pick it up and continue downloading from where it left off.
    ///
    /// Downloads resume at the article level - any articles that were already
    /// downloaded are skipped, and only pending articles are fetched.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to resume
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if the download was successfully resumed, or an error if:
    /// - The download doesn't exist
    /// - The download is not paused (already queued, downloading, complete, or failed)
    /// - Database update fails
    /// - Queue insertion fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader, id: DownloadId) -> Result<()> {
    /// // Resume a paused download
    /// downloader.resume(id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn resume(&self, id: DownloadId) -> Result<()> {
        // Fetch download from database
        let download = self.db.get_download(id).await?.ok_or_else(|| {
            Error::Database(DatabaseError::NotFound(format!(
                "Download {} not found",
                id
            )))
        })?;

        let current_status = Status::from_i32(download.status);

        // Check if download can be resumed
        match current_status {
            Status::Paused => {
                // Can be resumed
            }
            Status::Queued | Status::Downloading | Status::Processing => {
                // Already active, nothing to do (idempotent)
                return Ok(());
            }
            Status::Complete | Status::Failed => {
                return Err(Error::Download(DownloadError::InvalidState {
                    id: id.into(),
                    operation: "resume".to_string(),
                    current_state: format!("{:?}", current_status),
                }));
            }
        }

        // Update status back to Queued
        self.db.update_status(id, Status::Queued.to_i32()).await?;

        // Add back to priority queue for processing
        // The queue processor will automatically pick it up
        // Article-level tracking ensures only pending articles are downloaded
        self.add_to_queue(id).await?;

        Ok(())
    }

    /// Resume a partially downloaded job from where it left off
    ///
    /// This method is the low-level resume operation that queries pending articles
    /// and adds the download back to the queue for processing. It checks if there are
    /// any pending articles remaining - if none, it proceeds directly to post-processing.
    /// If articles remain, it re-queues the download for the queue processor to continue.
    ///
    /// This method is primarily used internally by restore_queue() during startup to
    /// resume interrupted downloads, but can also be called directly for explicit resume operations.
    pub async fn resume_download(&self, id: DownloadId) -> Result<()> {
        // Get pending articles for this download, excluding paused files.
        let pending_articles = self.db.get_pending_articles(id).await?;

        if !pending_articles.is_empty() {
            tracing::info!(
                download_id = id.0,
                pending_articles = pending_articles.len(),
                "Resuming download with pending articles"
            );

            self.db.update_status(id, Status::Queued.to_i32()).await?;
            self.add_to_queue(id).await?;
            return Ok(());
        }

        if self.db.has_any_pending_articles(id).await? {
            tracing::info!(
                download_id = id.0,
                "Only paused file articles remain - keeping download paused"
            );
            self.db.update_status(id, Status::Paused.to_i32()).await?;
            return Ok(());
        }

        tracing::info!(
            download_id = id.0,
            "No pending articles - marking as Processing"
        );

        self.db
            .update_status(id, Status::Processing.to_i32())
            .await?;

        Ok(())
    }

    /// Cancel a download and delete its files
    ///
    /// This method removes a download from the queue, stops it if actively running,
    /// deletes all downloaded files from the temp directory, and removes it from the database.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to cancel
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader, id: DownloadId) -> Result<()> {
    /// // Cancel a download and remove all files
    /// downloader.cancel(id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn cancel(&self, id: DownloadId) -> Result<()> {
        // Verify download exists
        let _download = self.db.get_download(id).await?.ok_or_else(|| {
            Error::Database(DatabaseError::NotFound(format!(
                "Download {} not found",
                id
            )))
        })?;

        // If download is actively running, cancel its task
        let mut active_downloads = self.queue_state.active_downloads.lock().await;
        if let Some(cancel_token) = active_downloads.get(&id) {
            // Signal the download task to stop
            cancel_token.cancel();
            // Remove from active downloads
            active_downloads.remove(&id);
        }
        drop(active_downloads); // Release lock

        // Remove from queue if it's still queued (not yet started)
        self.remove_from_queue(id).await;

        // Delete downloaded files from temp directory
        let download_temp_dir = self
            .config
            .download
            .temp_dir
            .join(format!("download_{}", id.0));
        if download_temp_dir.exists()
            && let Err(e) = tokio::fs::remove_dir_all(&download_temp_dir).await
        {
            tracing::warn!(
                download_id = id.0,
                path = ?download_temp_dir,
                error = %e,
                "Failed to delete download temp directory"
            );
            // Continue anyway - database deletion is more important
        }

        // Delete download from database (cascades to articles, passwords)
        self.db.delete_download(id).await?;

        // Emit Removed event
        self.emit_event(crate::types::Event::Removed { id });

        Ok(())
    }

    /// Set the priority of a download
    ///
    /// This method changes the priority of a download. If the download is queued,
    /// it will be re-queued with the new priority. Active downloads keep running
    /// but the priority is saved for when they're queued again.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to update
    /// * `priority` - The new priority level (Low, Normal, High, or Force)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::*;
    /// # async fn example(downloader: UsenetDownloader, id: DownloadId) -> Result<()> {
    /// // Set download to high priority
    /// downloader.set_priority(id, Priority::High).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_priority(&self, id: DownloadId, priority: Priority) -> Result<()> {
        // Verify download exists
        let download = self.db.get_download(id).await?.ok_or_else(|| {
            Error::Database(DatabaseError::NotFound(format!(
                "Download {} not found",
                id
            )))
        })?;

        let current_status = Status::from_i32(download.status);

        // Update priority in database
        self.db.update_priority(id, priority as i32).await?;

        // If download is queued (not actively downloading), reorder the queue
        // by removing and re-adding with new priority
        if current_status == Status::Queued {
            // Remove from queue
            self.remove_from_queue(id).await;

            // Re-add with new priority
            // We need to fetch the download again to get updated priority
            self.add_to_queue(id).await?;
        }

        Ok(())
    }

    /// Re-run post-processing on a completed or failed download
    ///
    /// This method allows re-running the post-processing pipeline on a download.
    /// This is useful when:
    /// - Extraction failed due to missing password (now added)
    /// - Post-processing settings changed
    /// - Files were manually repaired
    ///
    /// The download files must still exist in the temp directory for reprocessing to work.
    pub async fn reprocess(&self, id: DownloadId) -> Result<()> {
        // Get download from database
        let _download = self
            .db
            .get_download(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Download {} not found", id.0)))?;

        // Determine download path (temp directory)
        let download_path = self
            .config
            .download
            .temp_dir
            .join(format!("download_{}", id.0));

        // Verify download files still exist
        if !download_path.exists() {
            return Err(Error::NotFound(format!(
                "Download files not found at {}. Cannot reprocess.",
                download_path.display()
            )));
        }

        tracing::info!(
            download_id = id.0,
            path = %download_path.display(),
            "Starting reprocessing"
        );

        // Reset status and re-queue for post-processing
        self.db
            .update_status(id, Status::Processing.to_i32())
            .await?;

        // Clear any previous error message
        self.db.set_error(id, "").await?;

        // Emit Verifying event to indicate post-processing is starting
        self.emit_event(Event::Verifying { id });

        // Start post-processing pipeline
        // This will run asynchronously
        let downloader = self.clone();
        tokio::spawn(async move {
            if let Err(e) = downloader.start_post_processing(id).await {
                tracing::error!(
                    download_id = id.0,
                    error = %e,
                    "Reprocessing failed"
                );
            }
        });

        Ok(())
    }

    /// Re-run extraction only (skip verify/repair)
    ///
    /// This method re-runs the extraction stage for a download that has already been downloaded.
    /// Unlike `reprocess()`, this skips PAR2 verification and repair stages and goes straight
    /// to archive extraction. This is useful when:
    /// - Extraction failed due to missing password (now added)
    /// - Extraction settings changed
    /// - User wants to re-extract without re-downloading
    pub async fn reextract(&self, id: DownloadId) -> Result<()> {
        // Get download from database
        let download = self
            .db
            .get_download(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Download {} not found", id.0)))?;

        // Determine download path (temp directory)
        let download_path = self
            .config
            .download
            .temp_dir
            .join(format!("download_{}", id.0));

        // Verify download files still exist
        if !download_path.exists() {
            return Err(Error::NotFound(format!(
                "Download files not found at {}. Cannot re-extract.",
                download_path.display()
            )));
        }

        tracing::info!(
            download_id = id.0,
            path = %download_path.display(),
            "Starting re-extraction (skip verify/repair)"
        );

        // Reset status to processing
        self.db
            .update_status(id, Status::Processing.to_i32())
            .await?;

        // Clear any previous error message
        self.db.set_error(id, "").await?;

        // Emit Extracting event to indicate extraction is starting
        self.emit_event(Event::Extracting {
            id,
            archive: String::new(),
            percent: 0.0,
        });

        // Run extraction stage only (skip verify/repair)
        // This will run asynchronously
        let downloader = self.clone();
        let destination = PathBuf::from(download.destination.clone());
        let post_processor = self.processing.post_processor.clone();
        tokio::spawn(async move {
            // Run re-extraction (extract + move, skip verify/repair)
            match post_processor
                .reextract(id, download_path, destination)
                .await
            {
                Ok(final_path) => {
                    downloader
                        .handle_reextract_success(id, final_path, download)
                        .await;
                }
                Err(e) => {
                    downloader.handle_reextract_failure(id, e, download).await;
                }
            }
        });

        Ok(())
    }

    /// Handle successful re-extraction completion
    async fn handle_reextract_success(
        &self,
        id: DownloadId,
        final_path: PathBuf,
        download: crate::db::Download,
    ) {
        tracing::info!(download_id = id.0, ?final_path, "Re-extraction complete");

        // Update status to complete
        if let Err(e) = self.db.update_status(id, Status::Complete.to_i32()).await {
            tracing::error!(
                download_id = id.0,
                error = %e,
                "Failed to update status to complete"
            );
        }

        // Emit Complete event
        self.emit_event(Event::Complete {
            id,
            path: final_path.clone(),
        });

        // Trigger webhooks for complete event
        self.trigger_webhooks(super::webhooks::TriggerWebhooksParams {
            event_type: crate::config::WebhookEvent::OnComplete,
            download_id: id,
            name: download.name.clone(),
            category: download.category.clone(),
            status: "complete".to_string(),
            destination: Some(final_path.clone()),
            error: None,
        });

        // Trigger scripts for complete event
        self.trigger_scripts(super::webhooks::TriggerScriptsParams {
            event_type: crate::config::ScriptEvent::OnComplete,
            download_id: id,
            name: download.name.clone(),
            category: download.category.clone(),
            status: "complete".to_string(),
            destination: Some(final_path),
            error: None,
            size_bytes: download.size_bytes as u64,
        });
    }

    /// Handle re-extraction failure
    async fn handle_reextract_failure(
        &self,
        id: DownloadId,
        error: crate::error::Error,
        download: crate::db::Download,
    ) {
        // Convert error to string once, reuse throughout
        let error_msg = error.to_string();

        tracing::error!(
            download_id = id.0,
            error = %error_msg,
            "Re-extraction failed"
        );

        // Update status to failed
        if let Err(db_err) = self.db.update_status(id, Status::Failed.to_i32()).await {
            tracing::error!(
                download_id = id.0,
                error = %db_err,
                "Failed to update status to failed"
            );
        }

        // Set error message
        if let Err(db_err) = self.db.set_error(id, &error_msg).await {
            tracing::error!(
                download_id = id.0,
                error = %db_err,
                "Failed to set error message"
            );
        }

        // Emit Failed event
        self.emit_event(Event::Failed {
            id,
            stage: Stage::Extract,
            error: error_msg.clone(),
            files_kept: true,
        });

        // Trigger webhooks for failed event
        self.trigger_webhooks(super::webhooks::TriggerWebhooksParams {
            event_type: crate::config::WebhookEvent::OnFailed,
            download_id: id,
            name: download.name.clone(),
            category: download.category.clone(),
            status: "failed".to_string(),
            destination: None,
            error: Some(error_msg.clone()),
        });

        // Trigger scripts for failed event
        self.trigger_scripts(super::webhooks::TriggerScriptsParams {
            event_type: crate::config::ScriptEvent::OnFailed,
            download_id: id,
            name: download.name.clone(),
            category: download.category.clone(),
            status: "failed".to_string(),
            destination: None,
            error: Some(error_msg),
            size_bytes: download.size_bytes as u64,
        });
    }

    /// Pause all active downloads
    ///
    /// This method pauses all downloads that are currently queued, downloading, or processing.
    /// Already paused, completed, or failed downloads are not affected.
    pub async fn pause_all(&self) -> Result<()> {
        // Get all downloads that can be paused (Queued, Downloading, Processing)
        let all_downloads = self.db.list_downloads().await?;

        let mut paused_count = 0;

        for download in all_downloads {
            let status = Status::from_i32(download.status);

            // Only pause active downloads
            match status {
                Status::Queued | Status::Downloading | Status::Processing => {
                    if let Err(e) = self.pause(DownloadId(download.id)).await {
                        tracing::warn!(
                            download_id = download.id,
                            error = %e,
                            "Failed to pause download during pause_all"
                        );
                        // Continue with other downloads
                    } else {
                        paused_count += 1;
                    }
                }
                Status::Paused | Status::Complete | Status::Failed => {
                    // Skip already paused/finished downloads
                }
            }
        }

        tracing::info!(paused_count = paused_count, "Paused all active downloads");

        // Emit global QueuePaused event
        self.emit_event(crate::types::Event::QueuePaused);

        Ok(())
    }

    /// Resume all paused downloads
    ///
    /// This method resumes all downloads that are currently paused.
    /// Downloads in other states (queued, downloading, complete, failed) are not affected.
    pub async fn resume_all(&self) -> Result<()> {
        // Get all paused downloads
        let paused_downloads = self
            .db
            .list_downloads_by_status(Status::Paused.to_i32())
            .await?;

        let mut resumed_count = 0;

        for download in paused_downloads {
            if let Err(e) = self.resume(DownloadId(download.id)).await {
                tracing::warn!(
                    download_id = download.id,
                    error = %e,
                    "Failed to resume download during resume_all"
                );
                // Continue with other downloads
            } else {
                resumed_count += 1;
            }
        }

        tracing::info!(
            resumed_count = resumed_count,
            "Resumed all paused downloads"
        );

        // Emit global QueueResumed event
        self.emit_event(crate::types::Event::QueueResumed);

        Ok(())
    }
}
