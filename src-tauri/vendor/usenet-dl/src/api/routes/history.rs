//! History management handlers.

use super::{ClearHistoryQuery, HistoryQuery};
use crate::api::AppState;
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;

/// GET /history - Get download history (with pagination)
#[utoipa::path(
    get,
    path = "/api/v1/history",
    tag = "history",
    params(
        ("limit" = Option<i64>, Query, description = "Maximum number of items to return"),
        ("offset" = Option<i64>, Query, description = "Number of items to skip"),
        ("status" = Option<String>, Query, description = "Filter by status (complete/failed)")
    ),
    responses(
        (status = 200, description = "Download history", body = Vec<crate::types::HistoryEntry>),
        (status = 400, description = "Invalid query parameters"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_history(
    State(state): State<AppState>,
    Query(query): Query<HistoryQuery>,
) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(50).clamp(1, 1000) as usize;
    let offset = query.offset.unwrap_or(0).max(0) as usize;

    let status_filter = if let Some(status_str) = query.status {
        match status_str.to_lowercase().as_str() {
            "complete" => Some(4),
            "failed" => Some(5),
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": {"code": "invalid_status", "message": "Invalid status filter. Must be 'complete' or 'failed'"}})),
                ).into_response();
            }
        }
    } else {
        None
    };

    match state
        .downloader
        .db
        .query_history(status_filter, limit, offset)
        .await
    {
        Ok(entries) => match state.downloader.db.count_history(status_filter).await {
            Ok(total) => {
                let response = json!({
                    "items": entries,
                    "total": total,
                    "limit": limit,
                    "offset": offset
                });
                (StatusCode::OK, Json(response)).into_response()
            }
            Err(e) => {
                tracing::error!("Failed to count history: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "database_error", "message": "Failed to count history entries"}}))).into_response()
            }
        },
        Err(e) => {
            tracing::error!("Failed to query history: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "database_error", "message": "Failed to retrieve history"}}))).into_response()
        }
    }
}

/// DELETE /history - Clear history
#[utoipa::path(
    delete,
    path = "/api/v1/history",
    tag = "history",
    params(
        ("before" = Option<i64>, Query, description = "Clear entries before this timestamp"),
        ("status" = Option<String>, Query, description = "Clear only entries with this status (complete/failed)")
    ),
    responses(
        (status = 200, description = "Number of deleted entries"),
        (status = 400, description = "Invalid status filter"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn clear_history(
    State(state): State<AppState>,
    Query(query): Query<ClearHistoryQuery>,
) -> impl IntoResponse {
    let status_filter = if let Some(status_str) = query.status {
        match status_str.to_lowercase().as_str() {
            "complete" => Some(4),
            "failed" => Some(5),
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": {"code": "invalid_status", "message": "Invalid status filter. Must be 'complete' or 'failed'"}})),
                ).into_response();
            }
        }
    } else {
        None
    };

    match state
        .downloader
        .db
        .delete_history_filtered(query.before, status_filter)
        .await
    {
        Ok(deleted_count) => {
            (StatusCode::OK, Json(json!({"deleted": deleted_count}))).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to clear history");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "clear_failed", "message": format!("Failed to clear history: {}", e)}}))).into_response()
        }
    }
}
