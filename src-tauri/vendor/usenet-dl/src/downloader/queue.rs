//! Priority queue management for download ordering.

use crate::error::{DatabaseError, Error, Result};
use crate::types::{DownloadId, Priority, Status};

use super::{QueuedDownload, UsenetDownloader};

impl UsenetDownloader {
    /// Add a download to the in-memory priority queue
    ///
    /// This method adds a download ID to the priority queue for processing.
    /// Downloads are ordered by priority (High > Normal > Low) and then by creation time (FIFO).
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to add to the queue
    ///
    /// # Errors
    ///
    /// Returns an error if the download doesn't exist in the database
    pub(crate) async fn add_to_queue(&self, id: DownloadId) -> Result<()> {
        // Fetch download from database to get priority and created_at
        let download = self.db.get_download(id).await?.ok_or_else(|| {
            Error::Database(DatabaseError::NotFound(format!(
                "Download {} not found",
                id
            )))
        })?;

        let queued_download = QueuedDownload {
            id,
            priority: Priority::from_i32(download.priority),
            created_at: download.created_at,
        };

        // Add to priority queue
        let mut queue = self.queue_state.queue.lock().await;
        queue.push(queued_download);

        Ok(())
    }

    /// Remove a download from the in-memory priority queue
    ///
    /// This method removes a download from the queue without starting it.
    /// Used when a download is cancelled or removed.
    ///
    /// # Arguments
    ///
    /// * `id` - The download ID to remove from the queue
    ///
    /// # Returns
    ///
    /// Returns true if the download was found and removed, false otherwise
    pub(crate) async fn remove_from_queue(&self, id: DownloadId) -> bool {
        let mut queue = self.queue_state.queue.lock().await;

        let original_len = queue.len();

        // Collect all items except the one we want to remove
        let items: Vec<_> = queue.drain().filter(|item| item.id != id).collect();

        let was_removed = items.len() < original_len;

        // Rebuild queue without the removed item
        *queue = items.into_iter().collect();

        was_removed
    }

    /// Restore incomplete downloads from database on startup
    ///
    /// This method is called automatically during initialization to restore
    /// any downloads that were in progress when the application last shut down.
    ///
    /// The restoration process:
    /// 1. Queries database for downloads with status: Queued, Downloading, or Processing
    /// 2. For downloads in Downloading or Processing state, calls resume_download()
    /// 3. For downloads in Queued state, adds them back to the priority queue
    ///
    /// Downloads with status Complete or Failed are not restored (they're in history).
    /// Paused downloads are also not restored (user explicitly paused them).
    pub async fn restore_queue(&self) -> Result<Vec<DownloadId>> {
        tracing::info!("Restoring queue from database");

        // Get all incomplete downloads (status IN (0=Queued, 1=Downloading, 3=Processing))
        let incomplete_downloads = self.db.get_incomplete_downloads().await?;

        if incomplete_downloads.is_empty() {
            tracing::info!("No incomplete downloads to restore");
            return Ok(Vec::new());
        }

        tracing::info!(
            count = incomplete_downloads.len(),
            "Found incomplete downloads to restore"
        );

        // Store count before iterating
        let restore_count = incomplete_downloads.len();

        // Collect IDs that need post-processing (status became Processing after resume)
        let mut needs_post_processing = Vec::new();

        // Process each download based on its status
        for download in incomplete_downloads {
            let id = DownloadId(download.id);
            let status = Status::from_i32(download.status);

            match status {
                Status::Downloading | Status::Processing => {
                    // These were actively running - resume them
                    tracing::info!(
                        download_id = download.id,
                        status = ?status,
                        "Resuming interrupted download"
                    );
                    self.resume_download(id).await?;

                    // Check if resume_download set the status to Processing
                    // (meaning all unpaused articles are done and it needs post-processing).
                    let updated = self.db.get_download(id).await?;
                    if let Some(dl) = updated
                        && Status::from_i32(dl.status) == Status::Processing
                    {
                        needs_post_processing.push(id);
                    }
                }
                Status::Queued => {
                    // These were waiting in queue - add back to queue
                    tracing::info!(
                        download_id = download.id,
                        "Re-adding queued download to priority queue"
                    );
                    self.add_to_queue(id).await?;
                }
                _ => {
                    // Shouldn't happen (get_incomplete_downloads filters by status)
                    tracing::warn!(
                        download_id = download.id,
                        status = ?status,
                        "Unexpected download status during restore - skipping"
                    );
                }
            }
        }

        tracing::info!(restored_count = restore_count, "Queue restoration complete");

        Ok(needs_post_processing)
    }
}
