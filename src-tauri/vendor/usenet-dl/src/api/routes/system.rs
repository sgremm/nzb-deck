//! System handlers: health, capabilities, OpenAPI, events, shutdown.

use crate::api::AppState;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
};
use serde_json::json;
use std::convert::Infallible;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

/// GET /health - Health check
#[utoipa::path(
    get,
    path = "/api/v1/health",
    tag = "system",
    responses(
        (status = 200, description = "Service is healthy")
    )
)]
pub async fn health_check() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// GET /capabilities - Query system capabilities
#[utoipa::path(
    get,
    path = "/api/v1/capabilities",
    tag = "system",
    responses(
        (status = 200, description = "Current system capabilities", body = crate::types::Capabilities),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_capabilities(State(state): State<AppState>) -> impl IntoResponse {
    let capabilities = state.downloader.capabilities();
    (StatusCode::OK, Json(capabilities))
}

/// GET /openapi.json - OpenAPI specification
#[utoipa::path(
    get,
    path = "/api/v1/openapi.json",
    tag = "system",
    responses(
        (status = 200, description = "OpenAPI 3.1 specification in JSON format")
    )
)]
pub async fn openapi_spec() -> impl IntoResponse {
    use crate::api::openapi::ApiDoc;
    use utoipa::OpenApi;

    Json(ApiDoc::openapi())
}

/// GET /events - Server-sent events stream
#[utoipa::path(
    get,
    path = "/api/v1/events",
    tag = "system",
    responses(
        (status = 200, description = "Server-sent events stream (text/event-stream)", content_type = "text/event-stream"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn event_stream(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    let receiver = state.downloader.subscribe();
    let stream = BroadcastStream::new(receiver);

    let sse_stream = stream.filter_map(|result| match result {
        Ok(event) => match serde_json::to_string(&event) {
            Ok(json_data) => {
                let event_type = match &event {
                    crate::types::Event::Queued { .. } => "queued",
                    crate::types::Event::Removed { .. } => "removed",
                    crate::types::Event::Downloading { .. } => "downloading",
                    crate::types::Event::DownloadComplete { .. } => "download_complete",
                    crate::types::Event::DownloadFailed { .. } => "download_failed",
                    crate::types::Event::Verifying { .. } => "verifying",
                    crate::types::Event::VerifyComplete { .. } => "verify_complete",
                    crate::types::Event::Repairing { .. } => "repairing",
                    crate::types::Event::RepairComplete { .. } => "repair_complete",
                    crate::types::Event::RepairSkipped { .. } => "repair_skipped",
                    crate::types::Event::Extracting { .. } => "extracting",
                    crate::types::Event::ExtractComplete { .. } => "extract_complete",
                    crate::types::Event::Moving { .. } => "moving",
                    crate::types::Event::Cleaning { .. } => "cleaning",
                    crate::types::Event::Complete { .. } => "complete",
                    crate::types::Event::Failed { .. } => "failed",
                    crate::types::Event::SpeedLimitChanged { .. } => "speed_limit_changed",
                    crate::types::Event::QueuePaused => "queue_paused",
                    crate::types::Event::QueueResumed => "queue_resumed",
                    crate::types::Event::WebhookFailed { .. } => "webhook_failed",
                    crate::types::Event::ScriptFailed { .. } => "script_failed",
                    crate::types::Event::DuplicateDetected { .. } => "duplicate_detected",
                    crate::types::Event::DirectUnpackStarted { .. } => "direct_unpack_started",
                    crate::types::Event::FileCompleted { .. } => "file_completed",
                    crate::types::Event::DirectUnpackExtracting { .. } => {
                        "direct_unpack_extracting"
                    }
                    crate::types::Event::DirectUnpackExtracted { .. } => "direct_unpack_extracted",
                    crate::types::Event::DirectUnpackCancelled { .. } => "direct_unpack_cancelled",
                    crate::types::Event::DirectUnpackComplete { .. } => "direct_unpack_complete",
                    crate::types::Event::DirectRenamed { .. } => "direct_renamed",
                    crate::types::Event::Shutdown => "shutdown",
                };

                Some(Ok(SseEvent::default().event(event_type).data(json_data)))
            }
            Err(e) => {
                tracing::warn!("Failed to serialize event to JSON: {}", e);
                None
            }
        },
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(skipped)) => {
            tracing::warn!("SSE client lagged, skipped {} events", skipped);
            Some(Ok(SseEvent::default().event("error").data(format!(
                r#"{{"error":"lagged","skipped":{}}}"#,
                skipped
            ))))
        }
    });

    Sse::new(sse_stream).keep_alive(KeepAlive::default())
}

/// POST /shutdown - Graceful shutdown
#[utoipa::path(
    post,
    path = "/api/v1/shutdown",
    tag = "system",
    responses(
        (status = 202, description = "Shutdown initiated"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn shutdown(State(state): State<AppState>) -> impl IntoResponse {
    // Spawn the shutdown sequence in a background task so we can return the response first
    tokio::spawn(async move {
        // Small delay to allow the HTTP response to be sent
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        if let Err(e) = state.downloader.shutdown().await {
            tracing::error!(error = %e, "Error during graceful shutdown");
        }

        // Exit the process after shutdown completes
        std::process::exit(0);
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({"status": "shutdown initiated"})),
    )
}
