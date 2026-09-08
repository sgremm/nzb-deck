//! Queue processor — manages the download priority queue and spawns download tasks.

use std::sync::Arc;
use std::time::Duration;

use super::UsenetDownloader;
use super::download_task::DownloadTaskContext;

/// Interval between queue polling attempts when the queue is empty
const QUEUE_POLL_INTERVAL: Duration = Duration::from_millis(100);

impl UsenetDownloader {
    /// Start the queue processor task
    ///
    /// This method spawns a background task that continuously:
    /// 1. Waits for the next download in the priority queue
    /// 2. Acquires a permit from the concurrency limiter (respects max_concurrent_downloads)
    /// 3. Spawns a download task for that download
    /// 4. Repeats until shutdown
    ///
    /// The queue processor ensures downloads are started in priority order and
    /// respects the configured concurrency limit.
    ///
    /// # Parallel Download Behavior
    ///
    /// Each spawned download task downloads articles **in parallel** using all configured
    /// NNTP connections. The concurrency is automatically calculated as the sum of connections
    /// across all servers (e.g., 50 connections = 50 articles downloading simultaneously).
    pub fn start_queue_processor(&self) -> tokio::task::JoinHandle<()> {
        let queue = self.queue_state.queue.clone();
        let concurrent_limit = self.queue_state.concurrent_limit.clone();
        let db = self.db.clone();
        let event_tx = self.event_tx.clone();
        let nntp_pools = self.nntp_pools.clone();
        let config = self.config.clone();
        let active_downloads = self.queue_state.active_downloads.clone();
        let speed_limiter = self.speed_limiter.clone();
        let downloader = self.clone();

        tokio::spawn(async move {
            loop {
                // Get the next download from the queue (keep full item for re-push on failure)
                let queued_item = {
                    let mut queue_guard = queue.lock().await;
                    queue_guard.pop()
                };

                if let Some(item) = queued_item {
                    let id = item.id;

                    // Acquire a permit from the semaphore (blocks if at max concurrent downloads)
                    let permit = concurrent_limit.clone().acquire_owned().await;

                    let permit = match permit {
                        Ok(p) => p,
                        Err(_) => {
                            // Semaphore closed — re-push the item so it isn't lost
                            let mut queue_guard = queue.lock().await;
                            queue_guard.push(item);
                            break;
                        }
                    };

                    // Create cancellation token for this download
                    let cancel_token = tokio_util::sync::CancellationToken::new();

                    // Register the cancellation token
                    {
                        let mut active = active_downloads.lock().await;
                        active.insert(id, cancel_token.clone());
                    }

                    let ctx = DownloadTaskContext {
                        id,
                        db: Arc::clone(&db),
                        event_tx: event_tx.clone(),
                        article_provider: Arc::new(super::download_task::NntpArticleProvider::new(
                            Arc::clone(&nntp_pools),
                        )),
                        config: Arc::clone(&config),
                        active_downloads: Arc::clone(&active_downloads),
                        speed_limiter: speed_limiter.clone(),
                        cancel_token,
                        downloader: downloader.clone(),
                    };

                    // Spawn the download task
                    tokio::spawn(async move {
                        let _permit = permit;
                        super::download_task::run_download_task(ctx).await;
                    });
                } else {
                    // Queue is empty, wait a bit before checking again
                    tokio::time::sleep(QUEUE_POLL_INTERVAL).await;
                }
            }
        })
    }
}
