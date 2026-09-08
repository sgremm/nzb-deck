//! Download task orchestration — top-level lifecycle for a single download.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

use crate::config::PostProcess;
use crate::types::Event;

use super::batching::{download_articles, fetch_download_record};
use super::context::{DownloadTaskContext, OutputFiles};
use super::finalization::finalize_download;

/// Aggregated result counts from downloading article batches.
pub(super) struct DownloadResults {
    pub(super) success_count: usize,
    pub(super) failed_count: usize,
    pub(super) first_error: Option<String>,
    /// Total articles in the download (for stats)
    pub(super) total_articles: usize,
    /// Count of individually-failed articles (tracked via atomic, separate from batch failures)
    pub(super) individually_failed: u64,
}

/// Core download task -- orchestrates the full lifecycle of a single download.
///
/// Phases:
/// 1. Fetch and validate the download record
/// 2. Transition to Downloading state
/// 3. Download all pending articles in parallel batches
///    3b. Spawn DirectUnpack coordinator (if enabled)
/// 4. Evaluate results and finalize status
/// 5. Trigger post-processing
pub(crate) async fn run_download_task(ctx: DownloadTaskContext) {
    let id = ctx.id;

    // Phase 1: Fetch download record and pending articles
    let (download, pending_articles) = match fetch_download_record(&ctx).await {
        Some(pair) => pair,
        None => return, // Already logged and cleaned up
    };

    // Phase 2: Handle empty article list.
    if pending_articles.is_empty() {
        match ctx.db.has_any_pending_articles(id).await {
            Ok(true) => {
                if let Err(e) = ctx.db.update_status(id, crate::types::Status::Paused.to_i32()).await {
                    tracing::error!(
                        download_id = id.0,
                        error = %e,
                        "Failed to keep paused-only download paused"
                    );
                }
                ctx.remove_from_active().await;
                return;
            }
            Ok(false) => {
                ctx.event_tx
                    .send(Event::DownloadComplete {
                        id,
                        articles_failed: None,
                        articles_total: None,
                    })
                    .ok();
                ctx.remove_from_active().await;
                ctx.spawn_post_processing();
                return;
            }
            Err(e) => {
                tracing::error!(
                    download_id = id.0,
                    error = %e,
                    "Failed to check for remaining paused work"
                );
                ctx.remove_from_active().await;
                return;
            }
        }
    }

    // Phase 3: Create temp directory
    let download_temp_dir = ctx
        .config
        .download
        .temp_dir
        .join(format!("download_{}", id.0));
    if let Err(e) = tokio::fs::create_dir_all(&download_temp_dir).await {
        let msg = format!("Failed to create temp directory: {}", e);
        tracing::error!(download_id = id.0, error = %e, "Failed to create temp directory");
        ctx.mark_failed(&msg).await;
        ctx.remove_from_active().await;
        return;
    }

    // Phase 3b: Create output files for DirectWrite
    let download_files = match ctx.db.get_download_files(id).await {
        Ok(files) => files,
        Err(e) => {
            let msg = format!("Failed to get download files: {}", e);
            tracing::error!(download_id = id.0, error = %e, "Failed to get download files");
            ctx.mark_failed(&msg).await;
            ctx.remove_from_active().await;
            return;
        }
    };

    let output_files = if download_files.is_empty() {
        // Legacy downloads without download_files rows -- no DirectWrite
        Arc::new(OutputFiles {
            files: HashMap::new(),
        })
    } else {
        match OutputFiles::create(&download_files, &download_temp_dir) {
            Ok(of) => Arc::new(of),
            Err(e) => {
                let msg = format!("Failed to create output files: {}", e);
                tracing::error!(download_id = id.0, error = %e, "Failed to create output files");
                ctx.mark_failed(&msg).await;
                ctx.remove_from_active().await;
                return;
            }
        }
    };

    // Create shared failed_articles counter (shared between download pipeline and DirectUnpack)
    let failed_articles = Arc::new(AtomicU64::new(0));

    // Create file completion channel for DirectUnpack notification
    let (file_completion_tx, file_completion_rx) =
        tokio::sync::mpsc::unbounded_channel::<i32>();

    // Build per-file article counts for the completion tracker
    let file_article_counts: HashMap<i32, u32> = {
        let mut counts: HashMap<i32, u32> = HashMap::new();
        for article in &pending_articles {
            *counts.entry(article.file_index).or_default() += 1;
        }
        counts
    };
    let file_completion_tracker = Arc::new(
        super::context::FileCompletionTracker::new(file_article_counts, file_completion_tx),
    );

    // Phase 3c: Spawn DirectUnpack coordinator if enabled and post-process includes unpack
    let post_process = PostProcess::from_i32(download.post_process);
    let direct_unpack_enabled = ctx.config.processing.direct_unpack.enabled
        && matches!(
            post_process,
            PostProcess::Unpack | PostProcess::UnpackAndCleanup
        );

    let download_complete = Arc::new(AtomicBool::new(false));
    let direct_unpack_handle = if direct_unpack_enabled {
        let coordinator = super::super::direct_unpack::DirectUnpackCoordinator::new(
            id,
            Arc::clone(&ctx.db),
            Arc::clone(&ctx.config),
            ctx.event_tx.clone(),
            ctx.cancel_token.child_token(),
            download_temp_dir.clone(),
            Arc::clone(&failed_articles),
            Arc::clone(&download_complete),
            file_completion_rx,
        );
        Some(tokio::spawn(coordinator.run()))
    } else {
        None
    };

    // Phase 4: Download articles
    let _total_articles = pending_articles.len();
    let total_size_bytes = download.size_bytes as u64;
    let results = download_articles(
        &ctx,
        pending_articles,
        total_size_bytes,
        &download_temp_dir,
        &output_files,
        &failed_articles,
        &file_completion_tracker,
    )
    .await;

    // Signal DirectUnpack coordinator that downloading is done
    download_complete.store(true, std::sync::atomic::Ordering::Release);

    // Wait for DirectUnpack coordinator to finish processing remaining files
    if let Some(handle) = direct_unpack_handle {
        match handle.await {
            Ok(_result) => {
                tracing::debug!(download_id = id.0, "DirectUnpack coordinator finished");
            }
            Err(e) => {
                tracing::error!(
                    download_id = id.0,
                    error = %e,
                    "DirectUnpack coordinator task panicked"
                );
            }
        }
    }

    // Phase 5: Finalize based on results
    finalize_download(ctx, results, total_size_bytes).await;
}
