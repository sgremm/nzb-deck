//! Download finalization — evaluate results and set final download status.

use crate::types::{Event, Status};

use super::context::DownloadTaskContext;
use super::orchestration::DownloadResults;

/// Evaluate download results and finalize the download status.
///
/// Marks the download as complete or failed based on the success/failure ratio,
/// removes from active tracking, and triggers post-processing on success.
pub(super) async fn finalize_download(
    ctx: DownloadTaskContext,
    results: DownloadResults,
    _total_size_bytes: u64,
) {
    let id = ctx.id;
    let DownloadResults {
        success_count,
        failed_count,
        first_error,
        total_articles,
        individually_failed,
    } = results;

    // Combine batch-level failures with individual article failures
    let total_failed = failed_count as u64 + individually_failed;
    let total = success_count as u64 + total_failed;
    let max_failure_ratio = ctx.config.download.max_failure_ratio;

    // Handle partial or total failures
    if total_failed > 0 {
        tracing::warn!(
            download_id = id.0,
            batch_failed = failed_count,
            individually_failed = individually_failed,
            total_failed = total_failed,
            succeeded = success_count,
            total = total,
            total_articles = total_articles,
            "Download completed with some failures"
        );

        if success_count == 0
            || (total > 0 && (total_failed as f64 / total as f64) > max_failure_ratio)
        {
            let error_msg = if let Some(ref first) = first_error {
                format!(
                    "{} of {} articles failed ({:.0}%). First error: {}",
                    total_failed,
                    total,
                    if total > 0 {
                        total_failed as f64 / total as f64 * 100.0
                    } else {
                        100.0
                    },
                    first,
                )
            } else {
                format!(
                    "{} of {} articles failed ({:.0}%)",
                    total_failed,
                    total,
                    if total > 0 {
                        total_failed as f64 / total as f64 * 100.0
                    } else {
                        100.0
                    },
                )
            };
            tracing::error!(
                download_id = id.0,
                total_failed = total_failed,
                succeeded = success_count,
                "Download failed - too many article failures"
            );
            ctx.mark_failed_with_stats(
                &error_msg,
                Some(success_count as u64),
                Some(total_failed),
                Some(total_articles as u64),
            )
            .await;
            ctx.remove_from_active().await;
            return;
        }
    }

    match ctx.db.has_any_pending_articles(id).await {
        Ok(true) => {
            if let Err(e) = ctx.db.update_status(id, Status::Paused.to_i32()).await {
                tracing::error!(
                    download_id = id.0,
                    error = %e,
                    "Failed to keep partially paused download paused"
                );
            }
            ctx.remove_from_active().await;
            return;
        }
        Ok(false) => {}
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

    // Mark download as complete
    if let Err(e) = ctx.db.update_status(id, Status::Complete.to_i32()).await {
        tracing::error!(download_id = id.0, error = %e, "Failed to mark download complete");
        ctx.remove_from_active().await;
        return;
    }
    if let Err(e) = ctx.db.set_completed(id).await {
        tracing::error!(download_id = id.0, error = %e, "Failed to set completion time");
    }

    ctx.event_tx
        .send(Event::DownloadComplete {
            id,
            articles_failed: if total_failed > 0 {
                Some(total_failed)
            } else {
                None
            },
            articles_total: Some(total_articles as u64),
        })
        .ok();
    ctx.remove_from_active().await;
    ctx.spawn_post_processing();
}
