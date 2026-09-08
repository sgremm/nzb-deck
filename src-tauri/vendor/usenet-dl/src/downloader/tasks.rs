//! Background task orchestration — legacy download task spawning.
//!
//! This module contains the legacy `spawn_download_task` function which is kept for
//! backward compatibility. New code should use the queue processor instead.

use crate::db::article_status;
use crate::db::{Article, Database};
use crate::error::{DatabaseError, Error, Result};
use crate::types::{DownloadId, Event, Status};
use futures::stream::{self, StreamExt};
use nntp_rs::NntpPool;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast;

use super::UsenetDownloader;

/// Maximum article failure ratio before considering a download failed (50%)
const MAX_FAILURE_RATIO: f64 = 0.5;

/// Fetch a single article from the NNTP server and write it to disk
async fn fetch_article(
    pool: &NntpPool,
    article: Article,
    temp_dir: &Path,
    db: &Database,
    download_id: DownloadId,
) -> std::result::Result<(i32, u64), String> {
    // Get a connection from the pool
    let mut conn = pool
        .get()
        .await
        .map_err(|e| format!("Failed to get NNTP connection: {}", e))?;

    // Fetch the article from the server
    let message_id = if article.message_id.starts_with('<') {
        article.message_id.clone()
    } else {
        format!("<{}>", article.message_id)
    };

    let response = match conn.fetch_article_binary(&message_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                download_id = download_id.0,
                article_id = article.id,
                error = %e,
                "Article fetch failed"
            );
            let _ = db
                .update_article_status(article.id, article_status::FAILED)
                .await;
            return Err(format!("Article fetch failed: {}", e));
        }
    };

    let article_file = temp_dir.join(format!("article_{}.dat", article.segment_number));

    if let Err(e) = tokio::fs::write(&article_file, &response.data).await {
        tracing::warn!(
            download_id = download_id.0,
            article_id = article.id,
            error = %e,
            "Failed to write article file"
        );
        let _ = db
            .update_article_status(article.id, article_status::FAILED)
            .await;
        return Err(format!("Failed to write article file: {}", e));
    }

    if let Err(e) = db
        .update_article_status(article.id, article_status::DOWNLOADED)
        .await
    {
        tracing::warn!(
            download_id = download_id.0,
            article_id = article.id,
            error = %e,
            "Failed to update article status"
        );
        return Err(format!("Failed to update article status: {}", e));
    }

    Ok((article.segment_number, article.size_bytes as u64))
}

/// Tally download results into successes, failures, and first error
fn tally_results(
    results: Vec<std::result::Result<(i32, u64), String>>,
) -> (usize, usize, Option<String>) {
    let mut successes = 0;
    let mut failures = 0;
    let mut first_error: Option<String> = None;

    for result in results {
        match result {
            Ok(_) => successes += 1,
            Err(e) => {
                failures += 1;
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    (successes, failures, first_error)
}

/// Handle download failure by updating status and emitting events
async fn handle_download_failure(
    db: &Database,
    event_tx: &broadcast::Sender<Event>,
    download_id: DownloadId,
    failures: usize,
    successes: usize,
    total_articles: usize,
    first_error: Option<String>,
) -> Result<()> {
    let error_msg = first_error.unwrap_or_else(|| "Unknown error".to_string());
    tracing::error!(
        download_id = download_id.0,
        failed = failures,
        succeeded = successes,
        "Download failed - too many article failures"
    );

    db.update_status(download_id, Status::Failed.to_i32())
        .await?;
    db.set_error(download_id, &error_msg).await?;

    event_tx
        .send(Event::DownloadFailed {
            id: download_id,
            error: error_msg.clone(),
            articles_succeeded: Some(successes as u64),
            articles_failed: Some(failures as u64),
            articles_total: Some(total_articles as u64),
        })
        .ok();

    Err(Error::Nntp(format!(
        "Download failed: {} of {} articles failed. First error: {}",
        failures, total_articles, error_msg
    )))
}

/// Parameters for emitting final progress
struct FinalProgressParams<'a> {
    db: &'a Database,
    event_tx: &'a broadcast::Sender<Event>,
    download_id: DownloadId,
    downloaded_bytes: u64,
    downloaded_articles: u64,
    total_size_bytes: u64,
    total_articles: usize,
    download_start: std::time::Instant,
}

/// Emit final progress event and update database
async fn emit_final_progress(params: FinalProgressParams<'_>) -> Result<()> {
    let FinalProgressParams {
        db,
        event_tx,
        download_id,
        downloaded_bytes,
        downloaded_articles,
        total_size_bytes,
        total_articles,
        download_start,
    } = params;
    let final_percent = if total_size_bytes > 0 {
        (downloaded_bytes as f32 / total_size_bytes as f32) * 100.0
    } else {
        (downloaded_articles as f32 / total_articles as f32) * 100.0
    };
    let elapsed_secs = download_start.elapsed().as_secs_f64();
    let final_speed_bps = if elapsed_secs > 0.0 {
        (downloaded_bytes as f64 / elapsed_secs) as u64
    } else {
        0
    };

    db.update_progress(
        download_id,
        final_percent,
        final_speed_bps,
        downloaded_bytes,
    )
    .await?;

    event_tx
        .send(Event::Downloading {
            id: download_id,
            percent: final_percent,
            speed_bps: final_speed_bps,
            failed_articles: None,
            total_articles: Some(total_articles as u64),
            health_percent: None,
        })
        .ok();

    Ok(())
}

impl UsenetDownloader {
    /// Spawn an asynchronous download task for a queued download
    ///
    /// This internal method creates a background task that handles the entire download lifecycle.
    ///
    /// # Legacy Implementation
    ///
    /// This is a legacy implementation kept for backward compatibility. It does NOT use the
    /// queue processor and downloads articles sequentially rather than using the optimized
    /// pipelined batch fetching. New code should use `start_queue_processor` instead.
    pub fn spawn_download_task(
        &self,
        download_id: DownloadId,
    ) -> tokio::task::JoinHandle<Result<()>> {
        let db = self.db.clone();
        let event_tx = self.event_tx.clone();
        let nntp_pools = self.nntp_pools.clone();
        let config = self.config.clone();
        let downloader = self.clone();

        tokio::spawn(async move {
            // Fetch download record
            let download = match db.get_download(download_id).await? {
                Some(d) => d,
                None => {
                    return Err(Error::Database(DatabaseError::NotFound(format!(
                        "Download with ID {} not found",
                        download_id
                    ))));
                }
            };

            // Update status to Downloading and record start time
            db.update_status(download_id, Status::Downloading.to_i32())
                .await?;
            db.set_started(download_id).await?;

            // Emit Downloading event (initial progress 0%)
            event_tx
                .send(Event::Downloading {
                    id: download_id,
                    percent: 0.0,
                    speed_bps: 0,
                    failed_articles: None,
                    total_articles: None,
                    health_percent: None,
                })
                .ok();

            // Get all pending articles
            let pending_articles = db.get_pending_articles(download_id).await?;

            if pending_articles.is_empty() {
                // No articles to download - mark as complete
                event_tx
                    .send(Event::DownloadComplete {
                        id: download_id,
                        articles_failed: None,
                        articles_total: None,
                    })
                    .ok();

                // Start post-processing pipeline asynchronously
                tokio::spawn(async move {
                    if let Err(e) = downloader.start_post_processing(download_id).await {
                        tracing::error!(
                            download_id = download_id.0,
                            error = %e,
                            "Post-processing failed"
                        );
                    }
                });

                return Ok(());
            }

            let total_articles = pending_articles.len();
            let total_size_bytes = download.size_bytes as u64;
            let downloaded_articles = Arc::new(AtomicU64::new(0));
            let downloaded_bytes = Arc::new(AtomicU64::new(0));

            // Track download start time for speed calculation
            let download_start = std::time::Instant::now();

            // Create temp directory for this download
            let download_temp_dir = config
                .download
                .temp_dir
                .join(format!("download_{}", download_id));
            tokio::fs::create_dir_all(&download_temp_dir)
                .await
                .map_err(|e| {
                    Error::Io(std::io::Error::new(
                        e.kind(),
                        format!("Failed to create temp directory: {}", e),
                    ))
                })?;

            // Calculate concurrency limit from server connections
            let concurrency: usize = config.servers.iter().map(|s| s.connections).sum();

            // Download articles in parallel using buffered stream
            let results: Vec<std::result::Result<(i32, u64), String>> =
                stream::iter(pending_articles)
                    .map(|article| {
                        let nntp_pools = Arc::clone(&nntp_pools);
                        let db = Arc::clone(&db);
                        let download_temp_dir = download_temp_dir.clone();
                        let downloaded_articles = Arc::clone(&downloaded_articles);
                        let downloaded_bytes = Arc::clone(&downloaded_bytes);

                        async move {
                            // Get a connection from the first NNTP pool
                            let pool = nntp_pools
                                .first()
                                .ok_or_else(|| "No NNTP pools configured".to_string())?;

                            let result = fetch_article(
                                pool,
                                article.clone(),
                                &download_temp_dir,
                                &db,
                                download_id,
                            )
                            .await?;

                            downloaded_articles.fetch_add(1, Ordering::Relaxed);
                            downloaded_bytes.fetch_add(result.1, Ordering::Relaxed);

                            Ok::<(i32, u64), String>(result)
                        }
                    })
                    .buffer_unordered(concurrency)
                    .collect()
                    .await;

            // Process results and check for failures
            let (successes, failures, first_error) = tally_results(results);

            if failures > 0 {
                tracing::warn!(
                    download_id = download_id.0,
                    failed = failures,
                    succeeded = successes,
                    total = total_articles,
                    "Download completed with some failures"
                );

                if successes == 0 || (failures as f64 / total_articles as f64) > MAX_FAILURE_RATIO {
                    return handle_download_failure(
                        &db,
                        &event_tx,
                        download_id,
                        failures,
                        successes,
                        total_articles,
                        first_error,
                    )
                    .await;
                }
            }

            // Emit final progress event
            let final_bytes = downloaded_bytes.load(Ordering::Relaxed);
            let final_articles = downloaded_articles.load(Ordering::Relaxed);
            emit_final_progress(FinalProgressParams {
                db: &db,
                event_tx: &event_tx,
                download_id,
                downloaded_bytes: final_bytes,
                downloaded_articles: final_articles,
                total_size_bytes,
                total_articles,
                download_start,
            })
            .await?;

            // All articles downloaded successfully
            db.update_status(download_id, Status::Complete.to_i32())
                .await?;
            db.set_completed(download_id).await?;

            event_tx
                .send(Event::DownloadComplete {
                    id: download_id,
                    articles_failed: if failures > 0 {
                        Some(failures as u64)
                    } else {
                        None
                    },
                    articles_total: Some(total_articles as u64),
                })
                .ok();

            // Start post-processing pipeline asynchronously
            tokio::spawn(async move {
                if let Err(e) = downloader.start_post_processing(download_id).await {
                    tracing::error!(
                        download_id = download_id.0,
                        error = %e,
                        "Post-processing failed"
                    );
                }
            });

            Ok(())
        })
    }
}
