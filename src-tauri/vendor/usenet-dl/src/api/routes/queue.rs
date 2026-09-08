//! Queue-wide operation handlers.

use crate::api::AppState;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// POST /queue/pause - Pause all downloads
#[utoipa::path(
    post,
    path = "/api/v1/queue/pause",
    tag = "queue",
    responses(
        (status = 204, description = "Queue paused successfully"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn pause_queue(State(state): State<AppState>) -> impl IntoResponse {
    match state.downloader.pause_all().await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to pause queue");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "pause_failed", "message": format!("Failed to pause queue: {}", e)}}))).into_response()
        }
    }
}

/// POST /queue/resume - Resume all downloads
#[utoipa::path(
    post,
    path = "/api/v1/queue/resume",
    tag = "queue",
    responses(
        (status = 204, description = "Queue resumed successfully"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn resume_queue(State(state): State<AppState>) -> impl IntoResponse {
    match state.downloader.resume_all().await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to resume queue");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "resume_failed", "message": format!("Failed to resume queue: {}", e)}}))).into_response()
        }
    }
}

/// GET /queue/stats - Get queue statistics
#[utoipa::path(
    get,
    path = "/api/v1/queue/stats",
    tag = "queue",
    responses(
        (status = 200, description = "Queue statistics", body = crate::types::QueueStats),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn queue_stats(State(state): State<AppState>) -> Response {
    match state.downloader.db.get_all_downloads().await {
        Ok(downloads) => {
            let mut queued = 0;
            let mut downloading = 0;
            let mut paused = 0;
            let mut processing = 0;
            let mut total_speed_bps = 0u64;
            let mut total_size_bytes = 0u64;
            let mut downloaded_bytes = 0u64;

            for download in &downloads {
                let status = crate::types::Status::from_i32(download.status);
                match status {
                    crate::types::Status::Queued => queued += 1,
                    crate::types::Status::Downloading => downloading += 1,
                    crate::types::Status::Paused => paused += 1,
                    crate::types::Status::Processing => processing += 1,
                    _ => {}
                }
                if status == crate::types::Status::Downloading {
                    total_speed_bps += download.speed_bps as u64;
                }
                total_size_bytes += download.size_bytes as u64;
                downloaded_bytes += download.downloaded_bytes as u64;
            }

            let total = downloads.len();
            let overall_progress = if total_size_bytes > 0 {
                (downloaded_bytes as f32 / total_size_bytes as f32) * 100.0
            } else {
                0.0
            };

            let speed_limit_bps = state.downloader.speed_limiter.get_limit();
            let accepting_new = state
                .downloader
                .queue_state
                .accepting_new
                .load(std::sync::atomic::Ordering::SeqCst);

            let stats = crate::types::QueueStats {
                total,
                queued,
                downloading,
                paused,
                processing,
                total_speed_bps,
                total_size_bytes,
                downloaded_bytes,
                overall_progress,
                speed_limit_bps,
                accepting_new,
            };

            (StatusCode::OK, Json(stats)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to get queue statistics");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "stats_failed", "message": format!("Failed to get queue statistics: {}", e)}}))).into_response()
        }
    }
}
