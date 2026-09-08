//! Startup and shutdown coordination.

use crate::error::Result;
use crate::types::{DownloadId, Event, Status};

use super::UsenetDownloader;

impl UsenetDownloader {
    /// Gracefully shut down the downloader
    ///
    /// This method performs a graceful shutdown sequence:
    /// 1. Cancels all active downloads (using their cancellation tokens)
    /// 2. Waits for active downloads to complete with a timeout (30 seconds)
    /// 3. Persists final state to the database
    /// 4. Closes database connections
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail during shutdown.
    /// The method will attempt to complete as much of the shutdown sequence as possible
    /// even if some steps fail.
    pub async fn shutdown(&self) -> Result<()> {
        tracing::info!("Initiating graceful shutdown");

        // 1. Stop accepting new downloads
        self.queue_state
            .accepting_new
            .store(false, std::sync::atomic::Ordering::SeqCst);
        tracing::info!("Stopped accepting new downloads");

        // 2. Gracefully pause all active downloads (allow current article to finish)
        self.pause_graceful_all().await;
        tracing::info!("Signaled graceful pause to all active downloads");

        // 3. Wait for active downloads to complete with timeout
        let shutdown_timeout = std::time::Duration::from_secs(30);
        let wait_result =
            tokio::time::timeout(shutdown_timeout, self.wait_for_active_downloads()).await;

        match wait_result {
            Ok(Ok(())) => {
                tracing::info!("All active downloads completed gracefully");
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "Error while waiting for downloads to complete");
            }
            Err(_) => {
                tracing::warn!(
                    "Timeout waiting for downloads to complete, proceeding with shutdown"
                );
            }
        }

        // 4. Persist final state
        if let Err(e) = self.persist_all_state().await {
            tracing::error!(error = %e, "Failed to persist final state during shutdown");
            // Continue with shutdown even if persistence fails
        } else {
            tracing::info!("Final state persisted to database");
        }

        // 5. Mark clean shutdown in database
        if let Err(e) = self.db.set_clean_shutdown().await {
            tracing::error!(error = %e, "Failed to mark clean shutdown in database");
            // Continue with shutdown even if this fails
        } else {
            tracing::info!("Marked clean shutdown in database");
        }

        // 6. Emit shutdown event
        let _ = self.event_tx.send(Event::Shutdown);

        // 7. Close database connections
        // Note: Database is in an Arc, so we can't consume it directly.
        // The connection pool will be closed when the last Arc reference is dropped.
        // We log this for observability but don't actually close the pool here.
        tracing::info!(
            "Shutdown complete - database connections will close when downloader is dropped"
        );

        tracing::info!("Graceful shutdown complete");
        Ok(())
    }

    /// Gracefully pause all active downloads by signaling cancellation
    ///
    /// This method triggers a graceful pause of all active downloads. The downloads
    /// will complete their current article before stopping, ensuring no partial
    /// article downloads and maintaining data integrity.
    pub(crate) async fn pause_graceful_all(&self) {
        let active = self.queue_state.active_downloads.lock().await;
        tracing::debug!(
            active_count = active.len(),
            "Gracefully pausing all active downloads"
        );

        for (id, token) in active.iter() {
            tracing::debug!(download_id = id.0, "Signaling graceful pause");
            token.cancel();
        }
    }

    /// Wait for all active downloads to complete
    ///
    /// This is a helper method used during shutdown to wait for active downloads
    /// to finish their current work before closing.
    async fn wait_for_active_downloads(&self) -> Result<()> {
        loop {
            let active_count = {
                let active = self.queue_state.active_downloads.lock().await;
                active.len()
            };

            if active_count == 0 {
                return Ok(());
            }

            tracing::debug!(active_count, "Waiting for active downloads to complete");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Persist all state to the database
    ///
    /// This method ensures that all download state is saved to the database before
    /// shutdown. Since SQLite operations are auto-committed and all state changes
    /// are immediately persisted during normal operation, this method primarily
    /// serves as an explicit checkpoint during graceful shutdown.
    pub(crate) async fn persist_all_state(&self) -> Result<()> {
        tracing::debug!("Persisting all state to database");

        // Get all downloads that are currently in-progress states
        let downloads = self.db.get_all_downloads().await?;

        let mut persisted_count = 0;
        for download in downloads {
            // For downloads in Downloading or Processing state that are no longer active,
            // ensure their state reflects they were interrupted during shutdown
            let is_active = {
                let active = self.queue_state.active_downloads.lock().await;
                active.contains_key(&DownloadId(download.id))
            };

            // If a download is in an active state but not in active_downloads,
            // it means it was interrupted during shutdown
            if !is_active
                && (download.status == Status::Downloading.to_i32()
                    || download.status == Status::Processing.to_i32())
            {
                // Mark as Paused so it can be resumed on next startup
                self.db
                    .update_status(DownloadId(download.id), Status::Paused.to_i32())
                    .await?;
                persisted_count += 1;
                tracing::debug!(
                    download_id = download.id,
                    "Marked interrupted download as Paused for resume on restart"
                );
            }
        }

        if persisted_count > 0 {
            tracing::info!(
                persisted_count,
                "Persisted state for {} interrupted download(s)",
                persisted_count
            );
        } else {
            tracing::debug!("All download states already persisted");
        }

        Ok(())
    }
}
