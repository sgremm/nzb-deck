//! Article batching and download orchestration — record fetching, batch preparation,
//! background task management, and parallel batch downloading.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::stream::{self, StreamExt};

use crate::types::{DownloadId, Event, Status};

use super::batch_processor::{FetchArticleBatchParams, fetch_article_batch};
use super::context::{BatchResultVec, DownloadTaskContext, OutputFiles};
use super::orchestration::DownloadResults;

/// Fetch the download record and its pending articles, transitioning to Downloading state.
///
/// Returns `None` if the download is not found, the DB fails, or there's an error
/// updating the status -- in all cases the download is removed from active tracking.
pub(super) async fn fetch_download_record(
    ctx: &DownloadTaskContext,
) -> Option<(crate::db::Download, Vec<crate::db::Article>)> {
    let id = ctx.id;

    // Fetch download record
    let download = match ctx.db.get_download(id).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            tracing::warn!(download_id = id.0, "Download not found in database");
            ctx.remove_from_active().await;
            return None;
        }
        Err(e) => {
            tracing::error!(download_id = id.0, error = %e, "Failed to fetch download");
            ctx.remove_from_active().await;
            return None;
        }
    };

    // Update status to Downloading and record start time
    if let Err(e) = ctx.db.update_status(id, Status::Downloading.to_i32()).await {
        tracing::error!(download_id = id.0, error = %e, "Failed to update status");
        ctx.remove_from_active().await;
        return None;
    }
    if let Err(e) = ctx.db.set_started(id).await {
        tracing::error!(download_id = id.0, error = %e, "Failed to set start time");
        ctx.remove_from_active().await;
        return None;
    }

    // Emit Downloading event (initial progress 0%)
    ctx.event_tx
        .send(Event::Downloading {
            id,
            percent: 0.0,
            speed_bps: 0,
            failed_articles: None,
            total_articles: None,
            health_percent: None,
        })
        .ok();

    // Get all pending articles
    let pending_articles = match ctx.db.get_pending_articles(id).await {
        Ok(articles) => articles,
        Err(e) => {
            tracing::error!(download_id = id.0, error = %e, "Failed to get pending articles");
            ctx.remove_from_active().await;
            return None;
        }
    };

    Some((download, pending_articles))
}

/// Download all pending articles in parallel batches with progress tracking.
///
/// Sets up background tasks for progress reporting and database batching,
/// then streams article batches through pipelined NNTP fetches.
///
/// The `failed_articles` counter is created externally so it can be shared with
/// the DirectUnpack coordinator (which cancels on any article failure).
pub(super) async fn download_articles(
    ctx: &DownloadTaskContext,
    pending_articles: Vec<crate::db::Article>,
    total_size_bytes: u64,
    download_temp_dir: &std::path::Path,
    output_files: &Arc<OutputFiles>,
    failed_articles: &Arc<AtomicU64>,
    file_completion_tracker: &Arc<super::context::FileCompletionTracker>,
) -> DownloadResults {
    let id = ctx.id;
    let total_articles = pending_articles.len();
    let counters = DownloadCounters {
        downloaded_articles: Arc::new(AtomicU64::new(0)),
        downloaded_bytes: Arc::new(AtomicU64::new(0)),
        failed_articles: Arc::clone(failed_articles),
    };
    let download_start = std::time::Instant::now();

    // Set up background tasks for progress reporting and DB updates
    let (progress_task, batch_tx, batch_task) = spawn_background_tasks(
        id,
        total_articles,
        total_size_bytes,
        download_start,
        &counters,
        ctx,
    );

    // Spawn fast-fail watcher: if most articles in an early sample are missing, cancel early
    let fast_fail_task = spawn_fast_fail_watcher(
        &counters.downloaded_articles,
        &counters.failed_articles,
        ctx.config.download.fast_fail_threshold,
        ctx.config.download.fast_fail_sample_size,
        ctx.cancel_token.clone(),
    );

    // Calculate concurrency and split articles into batches
    let (concurrency, pipeline_depth, article_batches) =
        prepare_batches(&ctx.config, pending_articles, None);

    // Download all batches in parallel
    let results = download_all_batches(DownloadAllBatchesParams {
        id,
        article_batches,
        ctx,
        batch_tx: &batch_tx,
        downloaded_bytes: &counters.downloaded_bytes,
        downloaded_articles: &counters.downloaded_articles,
        failed_articles: &counters.failed_articles,
        download_temp_dir,
        output_files,
        concurrency,
        pipeline_depth,
        file_completion_tracker,
    })
    .await;

    // Clean up background tasks
    fast_fail_task.abort();
    cleanup_background_tasks(id, progress_task, batch_tx, batch_task).await;

    // Aggregate and return results
    let mut agg = super::batch_processor::aggregate_results(results);
    agg.total_articles = total_articles;
    agg.individually_failed = counters.failed_articles.load(Ordering::Relaxed);
    agg
}

/// Atomic counters shared across the download pipeline.
struct DownloadCounters {
    downloaded_articles: Arc<AtomicU64>,
    downloaded_bytes: Arc<AtomicU64>,
    failed_articles: Arc<AtomicU64>,
}

/// Spawn progress reporter and database batch updater background tasks.
fn spawn_background_tasks(
    id: DownloadId,
    total_articles: usize,
    total_size_bytes: u64,
    download_start: std::time::Instant,
    counters: &DownloadCounters,
    ctx: &DownloadTaskContext,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::Sender<(i64, i32)>,
    tokio::task::JoinHandle<()>,
) {
    let progress_task = super::super::background_tasks::spawn_progress_reporter(
        super::super::background_tasks::ProgressReporterParams {
            id,
            total_articles,
            total_size_bytes,
            download_start,
            downloaded_articles: Arc::clone(&counters.downloaded_articles),
            downloaded_bytes: Arc::clone(&counters.downloaded_bytes),
            failed_articles: Arc::clone(&counters.failed_articles),
            event_tx: ctx.event_tx.clone(),
            db: Arc::clone(&ctx.db),
            cancel_token: ctx.cancel_token.child_token(),
        },
    );

    let (batch_tx, batch_rx) = tokio::sync::mpsc::channel::<(i64, i32)>(
        super::super::background_tasks::ARTICLE_CHANNEL_BUFFER,
    );
    let batch_task = super::super::background_tasks::spawn_batch_updater(
        id,
        Arc::clone(&ctx.db),
        batch_rx,
        ctx.cancel_token.child_token(),
    );

    (progress_task, batch_tx, batch_task)
}

/// Spawn a fast-fail watcher that cancels the download if too many early articles are missing.
///
/// Polls the atomic counters every 200ms. After `sample_size` articles have been attempted
/// (downloaded + failed), if the failure ratio >= `threshold`, cancels via the token.
pub(super) fn spawn_fast_fail_watcher(
    downloaded_articles: &Arc<AtomicU64>,
    failed_articles: &Arc<AtomicU64>,
    threshold: f64,
    sample_size: usize,
    cancel_token: tokio_util::sync::CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let downloaded = Arc::clone(downloaded_articles);
    let failed = Arc::clone(failed_articles);
    let child_token = cancel_token.child_token();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(200));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let d = downloaded.load(Ordering::Relaxed);
                    let f = failed.load(Ordering::Relaxed);
                    let attempted = d + f;
                    if attempted >= sample_size as u64 && attempted > 0 {
                        let fail_ratio = f as f64 / attempted as f64;
                        if fail_ratio >= threshold {
                            tracing::warn!(
                                failed = f,
                                attempted = attempted,
                                ratio = %format!("{:.1}%", fail_ratio * 100.0),
                                "Fast-fail: too many articles missing, cancelling download"
                            );
                            cancel_token.cancel();
                            return;
                        }
                        // Once we've passed the sample without triggering, stop watching
                        return;
                    }
                }
                _ = child_token.cancelled() => {
                    return;
                }
            }
        }
    })
}

/// Calculate concurrency settings and split articles into pipeline-sized batches.
///
/// When `download_files` is provided and DirectRename is enabled, PAR2 file articles
/// are sorted to the front so their metadata is available early for renaming obfuscated files.
pub(super) fn prepare_batches(
    config: &crate::config::Config,
    mut pending_articles: Vec<crate::db::Article>,
    download_files: Option<&[crate::db::DownloadFile]>,
) -> (usize, usize, Vec<Vec<crate::db::Article>>) {
    let concurrency: usize = config.servers.iter().map(|s| s.connections).sum();
    let pipeline_depth = config
        .servers
        .first()
        .map(|s| s.pipeline_depth.max(1))
        .unwrap_or(10);

    // When DirectRename is enabled, prioritize PAR2 file articles so metadata loads early
    if config.processing.direct_unpack.direct_rename
        && let Some(files) = download_files
    {
        let par2_indices: std::collections::HashSet<i32> = files
            .iter()
            .filter(|f| f.filename.to_lowercase().ends_with(".par2"))
            .map(|f| f.file_index)
            .collect();

        if !par2_indices.is_empty() {
            // Stable sort: PAR2 articles first, preserving order within groups
            pending_articles.sort_by_key(|a| {
                if par2_indices.contains(&a.file_index) {
                    0
                } else {
                    1
                }
            });
        }
    }

    let article_batches: Vec<Vec<_>> = pending_articles
        .chunks(pipeline_depth)
        .map(|chunk| chunk.to_vec())
        .collect();

    (concurrency, pipeline_depth, article_batches)
}

/// Parameters for downloading all article batches
struct DownloadAllBatchesParams<'a> {
    id: DownloadId,
    article_batches: Vec<Vec<crate::db::Article>>,
    ctx: &'a DownloadTaskContext,
    batch_tx: &'a tokio::sync::mpsc::Sender<(i64, i32)>,
    downloaded_bytes: &'a Arc<AtomicU64>,
    downloaded_articles: &'a Arc<AtomicU64>,
    failed_articles: &'a Arc<AtomicU64>,
    download_temp_dir: &'a std::path::Path,
    output_files: &'a Arc<OutputFiles>,
    concurrency: usize,
    pipeline_depth: usize,
    file_completion_tracker: &'a Arc<super::context::FileCompletionTracker>,
}

/// Download all article batches in parallel using a buffered stream.
async fn download_all_batches(params: DownloadAllBatchesParams<'_>) -> BatchResultVec {
    let DownloadAllBatchesParams {
        id,
        article_batches,
        ctx,
        batch_tx,
        downloaded_bytes,
        downloaded_articles,
        failed_articles,
        download_temp_dir,
        output_files,
        concurrency,
        pipeline_depth,
        file_completion_tracker,
    } = params;
    stream::iter(article_batches)
        .map(|article_batch| {
            let article_provider = Arc::clone(&ctx.article_provider);
            let batch_tx = batch_tx.clone();
            let speed_limiter = ctx.speed_limiter.clone();
            let cancel_token = ctx.cancel_token.clone();
            let download_temp_dir = download_temp_dir.to_path_buf();
            let downloaded_bytes = Arc::clone(downloaded_bytes);
            let downloaded_articles = Arc::clone(downloaded_articles);
            let failed_articles = Arc::clone(failed_articles);
            let output_files = Arc::clone(output_files);
            let file_completion_tracker = Arc::clone(file_completion_tracker);

            async move {
                fetch_article_batch(FetchArticleBatchParams {
                    id,
                    article_batch,
                    article_provider,
                    batch_tx,
                    speed_limiter,
                    cancel_token,
                    download_temp_dir,
                    downloaded_bytes,
                    downloaded_articles,
                    failed_articles,
                    output_files,
                    pipeline_depth,
                    file_completion_tracker,
                })
                .await
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await
}

/// Stop progress task and flush final database updates.
pub(super) async fn cleanup_background_tasks(
    id: DownloadId,
    progress_task: tokio::task::JoinHandle<()>,
    batch_tx: tokio::sync::mpsc::Sender<(i64, i32)>,
    batch_task: tokio::task::JoinHandle<()>,
) {
    progress_task.abort();
    drop(batch_tx);
    if let Err(e) = batch_task.await {
        tracing::error!(download_id = id.0, error = %e, "Batch update task panicked");
    }
}
