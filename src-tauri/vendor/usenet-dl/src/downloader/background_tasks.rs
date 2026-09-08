//! Background tasks for progress reporting and database batch updates.

use crate::types::{DownloadId, Event};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Buffer size for the article status update channel
pub(crate) const ARTICLE_CHANNEL_BUFFER: usize = 500;

/// Interval between progress update emissions
const PROGRESS_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

/// Batch size threshold for flushing article status updates to the database
const ARTICLE_BATCH_SIZE: usize = 100;

/// Parameters for spawning a progress reporter background task
pub(crate) struct ProgressReporterParams {
    /// Download ID
    pub id: DownloadId,
    /// Total number of articles
    pub total_articles: usize,
    /// Total size in bytes
    pub total_size_bytes: u64,
    /// Download start time
    pub download_start: std::time::Instant,
    /// Atomic counter for downloaded articles
    pub downloaded_articles: Arc<AtomicU64>,
    /// Atomic counter for downloaded bytes
    pub downloaded_bytes: Arc<AtomicU64>,
    /// Atomic counter for individually-failed articles
    pub failed_articles: Arc<AtomicU64>,
    /// Event broadcast sender
    pub event_tx: tokio::sync::broadcast::Sender<Event>,
    /// Database handle
    pub db: Arc<crate::db::Database>,
    /// Cancellation token
    pub cancel_token: tokio_util::sync::CancellationToken,
}

/// Spawn a background task that periodically reports download progress.
pub(crate) fn spawn_progress_reporter(
    params: ProgressReporterParams,
) -> tokio::task::JoinHandle<()> {
    let ProgressReporterParams {
        id,
        total_articles,
        total_size_bytes,
        download_start,
        downloaded_articles,
        downloaded_bytes,
        failed_articles,
        event_tx,
        db,
        cancel_token,
    } = params;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(PROGRESS_UPDATE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let current_bytes = downloaded_bytes.load(Ordering::Relaxed);
                    let current_articles = downloaded_articles.load(Ordering::Relaxed);
                    let current_failed = failed_articles.load(Ordering::Relaxed);

                    let progress_percent = if total_size_bytes > 0 {
                        (current_bytes as f32 / total_size_bytes as f32) * 100.0
                    } else {
                        (current_articles as f32 / total_articles as f32) * 100.0
                    };

                    let elapsed_secs = download_start.elapsed().as_secs_f64();
                    let speed_bps = if elapsed_secs > 0.0 {
                        (current_bytes as f64 / elapsed_secs) as u64
                    } else {
                        0
                    };

                    // Compute download health
                    let attempted = current_articles + current_failed;
                    let health_percent = if attempted > 0 {
                        Some(100.0 * (1.0 - current_failed as f32 / attempted as f32))
                    } else {
                        None
                    };

                    if let Err(e) = db.update_progress(
                        id,
                        progress_percent,
                        speed_bps,
                        current_bytes,
                    ).await {
                        tracing::error!(download_id = id.0, error = %e, "Failed to update progress");
                    }

                    event_tx
                        .send(Event::Downloading {
                            id,
                            percent: progress_percent,
                            speed_bps,
                            failed_articles: if current_failed > 0 { Some(current_failed) } else { None },
                            total_articles: Some(total_articles as u64),
                            health_percent,
                        })
                        .ok();
                }
                _ = cancel_token.cancelled() => {
                    break;
                }
            }
        }
    })
}

/// Spawn a background task that batches article status updates for SQLite efficiency.
pub(crate) fn spawn_batch_updater(
    id: DownloadId,
    db: Arc<crate::db::Database>,
    mut batch_rx: tokio::sync::mpsc::Receiver<(i64, i32)>,
    cancel_token: tokio_util::sync::CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = Vec::with_capacity(ARTICLE_BATCH_SIZE);
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                msg = batch_rx.recv() => {
                    let Some((article_id, status)) = msg else {
                        // Channel closed — flush remaining and exit
                        break;
                    };
                    buffer.push((article_id, status));

                    if buffer.len() >= ARTICLE_BATCH_SIZE {
                        if let Err(e) = db.update_articles_status_batch(&buffer).await {
                            tracing::error!(download_id = id.0, batch_size = buffer.len(), error = %e, "Failed to batch update article statuses");
                        }
                        buffer.clear();
                    }
                }
                _ = interval.tick() => {
                    if !buffer.is_empty() {
                        if let Err(e) = db.update_articles_status_batch(&buffer).await {
                            tracing::error!(download_id = id.0, batch_size = buffer.len(), error = %e, "Failed to batch update article statuses");
                        }
                        buffer.clear();
                    }
                }
                _ = cancel_token.cancelled() => {
                    if !buffer.is_empty() && let Err(e) = db.update_articles_status_batch(&buffer).await {
                        tracing::error!(download_id = id.0, batch_size = buffer.len(), error = %e, "Failed to batch update article statuses on cancellation");
                    }
                    break;
                }
            }
        }

        // Final flush when task ends (channel closed)
        while let Ok((article_id, status)) = batch_rx.try_recv() {
            buffer.push((article_id, status));
        }
        if !buffer.is_empty()
            && let Err(e) = db.update_articles_status_batch(&buffer).await
        {
            tracing::error!(download_id = id.0, batch_size = buffer.len(), error = %e, "Failed to flush remaining article statuses");
        }
    })
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, NewArticle, NewDownload, article_status};
    use crate::types::{Event, Status};
    use std::sync::atomic::AtomicU64;
    use std::time::Duration;

    /// Helper to create a test database with a download row (no articles).
    async fn setup_db() -> (
        Arc<crate::db::Database>,
        DownloadId,
        tempfile::NamedTempFile,
    ) {
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let db = Database::new(temp_file.path()).await.unwrap();
        let db = Arc::new(db);

        let download_id = db
            .insert_download(&NewDownload {
                name: "test".to_string(),
                nzb_path: "/tmp/test.nzb".to_string(),
                nzb_meta_name: None,
                nzb_hash: None,
                job_name: None,
                category: None,
                destination: "/tmp/dest".to_string(),
                post_process: 0,
                priority: 0,
                status: Status::Downloading.to_i32(),
                size_bytes: 1000,
            })
            .await
            .unwrap();

        (db, download_id, temp_file)
    }

    /// Helper to create a test database with a download and N articles.
    /// Returns (db, download_id, article_row_ids, temp_file).
    async fn setup_db_with_articles(
        count: usize,
    ) -> (Arc<Database>, DownloadId, Vec<i64>, tempfile::NamedTempFile) {
        let (db, download_id, temp_file) = setup_db().await;

        let mut article_ids = Vec::with_capacity(count);
        for i in 0..count {
            let id = db
                .insert_article(&NewArticle {
                    download_id,
                    message_id: format!("<article-{}@test>", i),
                    segment_number: i as i32,
                    file_index: 0,
                    size_bytes: 10,
                })
                .await
                .unwrap();
            article_ids.push(id);
        }

        (db, download_id, article_ids, temp_file)
    }

    // ── Progress reporter tests ─────────────────────────────────────────

    #[tokio::test]
    async fn progress_reporter_emits_downloading_events() {
        let (db, download_id, _temp) = setup_db().await;
        let (event_tx, mut event_rx) = tokio::sync::broadcast::channel(100);
        let cancel_token = tokio_util::sync::CancellationToken::new();

        let _handle = spawn_progress_reporter(ProgressReporterParams {
            id: download_id,
            total_articles: 10,
            total_size_bytes: 1000,
            download_start: std::time::Instant::now(),
            downloaded_articles: Arc::new(AtomicU64::new(0)),
            failed_articles: Arc::new(AtomicU64::new(0)),
            downloaded_bytes: Arc::new(AtomicU64::new(250)),
            event_tx,
            db,
            cancel_token: cancel_token.clone(),
        });

        // Collect events for ~600ms (interval is 500ms, so expect at least one)
        let mut events = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(600);
        loop {
            tokio::select! {
                result = event_rx.recv() => {
                    if let Ok(event) = result {
                        events.push(event);
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    break;
                }
            }
        }

        cancel_token.cancel();

        assert!(
            !events.is_empty(),
            "Should have received at least one event"
        );
        let has_downloading = events
            .iter()
            .any(|e| matches!(e, Event::Downloading { percent, .. } if *percent > 0.0));
        assert!(
            has_downloading,
            "Should have received a Downloading event with percent > 0"
        );
    }

    #[tokio::test]
    async fn progress_reporter_uses_byte_percentage_when_size_known() {
        let (db, download_id, _temp) = setup_db().await;
        let (event_tx, mut event_rx) = tokio::sync::broadcast::channel(100);
        let cancel_token = tokio_util::sync::CancellationToken::new();

        let _handle = spawn_progress_reporter(ProgressReporterParams {
            id: download_id,
            total_articles: 10,
            total_size_bytes: 1000,
            download_start: std::time::Instant::now(),
            downloaded_articles: Arc::new(AtomicU64::new(0)),
            failed_articles: Arc::new(AtomicU64::new(0)),
            downloaded_bytes: Arc::new(AtomicU64::new(500)),
            event_tx,
            db,
            cancel_token: cancel_token.clone(),
        });

        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .unwrap()
            .unwrap();

        cancel_token.cancel();

        match event {
            Event::Downloading { percent, .. } => {
                assert!(
                    (percent - 50.0).abs() < 1.0,
                    "Expected ~50% from bytes (500/1000), got {percent}"
                );
            }
            other => panic!("Expected Downloading event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn progress_reporter_uses_article_percentage_when_size_zero() {
        let (db, download_id, _temp) = setup_db().await;
        let (event_tx, mut event_rx) = tokio::sync::broadcast::channel(100);
        let cancel_token = tokio_util::sync::CancellationToken::new();

        let _handle = spawn_progress_reporter(ProgressReporterParams {
            id: download_id,
            total_articles: 10,
            total_size_bytes: 0, // zero size → falls back to article-based percentage
            download_start: std::time::Instant::now(),
            downloaded_articles: Arc::new(AtomicU64::new(5)),
            failed_articles: Arc::new(AtomicU64::new(0)),
            downloaded_bytes: Arc::new(AtomicU64::new(0)),
            event_tx,
            db,
            cancel_token: cancel_token.clone(),
        });

        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .unwrap()
            .unwrap();

        cancel_token.cancel();

        match event {
            Event::Downloading { percent, .. } => {
                assert!(
                    (percent - 50.0).abs() < 1.0,
                    "Expected ~50% from articles (5/10), got {percent}"
                );
            }
            other => panic!("Expected Downloading event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn progress_reporter_stops_on_cancellation() {
        let (db, download_id, _temp) = setup_db().await;
        let (event_tx, _rx) = tokio::sync::broadcast::channel(100);
        let cancel_token = tokio_util::sync::CancellationToken::new();

        let handle = spawn_progress_reporter(ProgressReporterParams {
            id: download_id,
            total_articles: 10,
            total_size_bytes: 1000,
            download_start: std::time::Instant::now(),
            downloaded_articles: Arc::new(AtomicU64::new(0)),
            failed_articles: Arc::new(AtomicU64::new(0)),
            downloaded_bytes: Arc::new(AtomicU64::new(0)),
            event_tx,
            db,
            cancel_token: cancel_token.clone(),
        });

        cancel_token.cancel();

        let result = tokio::time::timeout(Duration::from_secs(1), handle).await;
        assert!(
            result.is_ok(),
            "Progress reporter should stop within 1 second after cancellation"
        );
        result.unwrap().unwrap();
    }

    // ── Batch updater tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn batch_updater_flushes_at_size_threshold() {
        let (db, download_id, article_ids, _temp) = setup_db_with_articles(100).await;
        let cancel_token = tokio_util::sync::CancellationToken::new();
        let (batch_tx, batch_rx) = tokio::sync::mpsc::channel(500);

        let handle = spawn_batch_updater(download_id, db.clone(), batch_rx, cancel_token.clone());

        // Send exactly ARTICLE_BATCH_SIZE (100) updates
        for &article_id in &article_ids {
            batch_tx
                .send((article_id, article_status::DOWNLOADED))
                .await
                .unwrap();
        }

        // Give the updater a moment to flush (threshold hit → immediate flush)
        tokio::time::sleep(Duration::from_millis(200)).await;

        let pending = db.get_pending_articles(download_id).await.unwrap();
        assert_eq!(
            pending.len(),
            0,
            "All 100 articles should be flushed at batch threshold, but {} still pending",
            pending.len()
        );

        cancel_token.cancel();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn batch_updater_flushes_on_timer() {
        let (db, download_id, article_ids, _temp) = setup_db_with_articles(10).await;
        let cancel_token = tokio_util::sync::CancellationToken::new();
        let (batch_tx, batch_rx) = tokio::sync::mpsc::channel(500);

        let handle = spawn_batch_updater(download_id, db.clone(), batch_rx, cancel_token.clone());

        // Send fewer than ARTICLE_BATCH_SIZE updates
        for &article_id in &article_ids {
            batch_tx
                .send((article_id, article_status::DOWNLOADED))
                .await
                .unwrap();
        }

        // Wait for timer flush (interval is 500ms, add margin)
        tokio::time::sleep(Duration::from_millis(1500)).await;

        let pending = db.get_pending_articles(download_id).await.unwrap();
        assert_eq!(
            pending.len(),
            0,
            "All 10 articles should be flushed by timer, but {} still pending",
            pending.len()
        );

        cancel_token.cancel();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn batch_updater_flushes_remaining_on_channel_close() {
        let (db, download_id, article_ids, _temp) = setup_db_with_articles(5).await;
        let cancel_token = tokio_util::sync::CancellationToken::new();
        let (batch_tx, batch_rx) = tokio::sync::mpsc::channel(500);

        let handle = spawn_batch_updater(download_id, db.clone(), batch_rx, cancel_token.clone());

        // Send 5 updates
        for &article_id in &article_ids {
            batch_tx
                .send((article_id, article_status::DOWNLOADED))
                .await
                .unwrap();
        }

        // Drop sender to close the channel
        drop(batch_tx);

        // Wait for the interval timer to flush remaining items (1s interval + margin)
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // Verify all 5 articles were flushed
        let pending = db.get_pending_articles(download_id).await.unwrap();
        assert_eq!(
            pending.len(),
            0,
            "All 5 articles should be flushed after channel close, but {} still pending",
            pending.len()
        );

        // Cancel to stop the task (loop continues on interval after channel close)
        cancel_token.cancel();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn batch_updater_flushes_on_cancellation() {
        let (db, download_id, article_ids, _temp) = setup_db_with_articles(5).await;
        let cancel_token = tokio_util::sync::CancellationToken::new();
        let (batch_tx, batch_rx) = tokio::sync::mpsc::channel(500);

        let handle = spawn_batch_updater(download_id, db.clone(), batch_rx, cancel_token.clone());

        // Send 5 updates
        for &article_id in &article_ids {
            batch_tx
                .send((article_id, article_status::DOWNLOADED))
                .await
                .unwrap();
        }

        // Small delay to ensure messages are received into the buffer
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Cancel the token — the cancellation handler should flush the buffer
        cancel_token.cancel();
        handle.await.unwrap();

        // Verify all 5 articles were flushed on cancellation
        let pending = db.get_pending_articles(download_id).await.unwrap();
        assert_eq!(
            pending.len(),
            0,
            "All 5 articles should be flushed on cancellation, but {} still pending",
            pending.len()
        );
    }
}
