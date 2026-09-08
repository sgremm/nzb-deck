//! Download management handlers.

use super::DeleteDownloadQuery;
use crate::api::AppState;
use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// GET /downloads - List all downloads
#[utoipa::path(
    get,
    path = "/api/v1/downloads",
    tag = "downloads",
    responses(
        (status = 200, description = "List of all downloads", body = Vec<crate::types::DownloadInfo>),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_downloads(State(state): State<AppState>) -> impl IntoResponse {
    match state.downloader.db.list_downloads().await {
        Ok(downloads) => {
            let download_infos: Vec<crate::types::DownloadInfo> = downloads
                .into_iter()
                .map(|d| {
                    let eta_seconds = if d.speed_bps > 0 && d.status == 1 {
                        let remaining = d.size_bytes.saturating_sub(d.downloaded_bytes);
                        if remaining > 0 {
                            Some((remaining as u64) / (d.speed_bps as u64))
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    crate::types::DownloadInfo {
                        id: crate::types::DownloadId(d.id),
                        name: d.name,
                        category: d.category,
                        status: crate::types::Status::from_i32(d.status),
                        progress: d.progress,
                        speed_bps: d.speed_bps as u64,
                        size_bytes: d.size_bytes as u64,
                        downloaded_bytes: d.downloaded_bytes as u64,
                        eta_seconds,
                        priority: crate::types::Priority::from_i32(d.priority),
                        created_at: chrono::DateTime::from_timestamp(d.created_at, 0)
                            .unwrap_or_else(chrono::Utc::now),
                        started_at: d
                            .started_at
                            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0)),
                    }
                })
                .collect();

            (StatusCode::OK, Json(download_infos))
        }
        Err(e) => {
            tracing::error!("Failed to list downloads: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(vec![]))
        }
    }
}

/// GET /downloads/:id - Get single download
#[utoipa::path(
    get,
    path = "/api/v1/downloads/{id}",
    tag = "downloads",
    params(
        ("id" = i64, Path, description = "Download ID")
    ),
    responses(
        (status = 200, description = "Download information", body = crate::types::DownloadInfo),
        (status = 404, description = "Download not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_download(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    match state
        .downloader
        .db
        .get_download(crate::types::DownloadId(id))
        .await
    {
        Ok(Some(d)) => {
            let eta_seconds = if d.speed_bps > 0 && d.status == 1 {
                let remaining = d.size_bytes.saturating_sub(d.downloaded_bytes);
                if remaining > 0 {
                    Some((remaining as u64) / (d.speed_bps as u64))
                } else {
                    None
                }
            } else {
                None
            };

            let download_info = crate::types::DownloadInfo {
                id: crate::types::DownloadId(d.id),
                name: d.name,
                category: d.category,
                status: crate::types::Status::from_i32(d.status),
                progress: d.progress,
                speed_bps: d.speed_bps as u64,
                size_bytes: d.size_bytes as u64,
                downloaded_bytes: d.downloaded_bytes as u64,
                eta_seconds,
                priority: crate::types::Priority::from_i32(d.priority),
                created_at: chrono::DateTime::from_timestamp(d.created_at, 0)
                    .unwrap_or_else(chrono::Utc::now),
                started_at: d
                    .started_at
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0)),
            };

            (StatusCode::OK, Json(download_info)).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "download not found"})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to get download {}: {}", id, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "internal server error"})),
            )
                .into_response()
        }
    }
}

/// POST /downloads - Add NZB from file upload
#[utoipa::path(
    post,
    path = "/api/v1/downloads",
    tag = "downloads",
    request_body(content = Vec<u8>, description = "NZB file upload (multipart/form-data)", content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Download added successfully", body = i64),
        (status = 400, description = "Invalid NZB file"),
        (status = 409, description = "Duplicate download detected"),
        (status = 422, description = "Unprocessable entity"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn add_download(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let mut nzb_content: Option<Vec<u8>> = None;
    let mut nzb_filename: Option<String> = None;
    let mut options_json: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "file" => {
                if let Some(filename) = field.file_name() {
                    nzb_filename = Some(filename.to_string());
                }
                match field.bytes().await {
                    Ok(bytes) => nzb_content = Some(bytes.to_vec()),
                    Err(e) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({"error": {"code": "invalid_file", "message": format!("Failed to read file: {}", e)}}))
                        ).into_response();
                    }
                }
            }
            "options" => {
                if let Ok(bytes) = field.bytes().await
                    && let Ok(s) = String::from_utf8(bytes.to_vec())
                {
                    options_json = Some(s);
                }
            }
            _ => {}
        }
    }

    let nzb_bytes = match nzb_content {
        Some(bytes) => bytes,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {"code": "missing_file", "message": "No NZB file provided in 'file' field"}}))
            ).into_response();
        }
    };

    let options: crate::types::DownloadOptions = match options_json {
        Some(json_str) => match serde_json::from_str(&json_str) {
            Ok(opts) => opts,
            Err(e) => {
                return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": {"code": "invalid_options", "message": format!("Invalid options JSON: {}", e)}}))
                    ).into_response();
            }
        },
        None => crate::types::DownloadOptions::default(),
    };

    let name = nzb_filename.unwrap_or_else(|| "upload.nzb".to_string());

    match state.downloader.add_nzb_content(&nzb_bytes, &name, options).await {
        Ok(download_id) => {
            (StatusCode::CREATED, Json(json!({"id": download_id}))).into_response()
        }
        Err(crate::Error::Duplicate(msg)) => {
            (StatusCode::CONFLICT, Json(json!({"error": {"code": "duplicate", "message": msg}}))).into_response()
        }
        Err(e) => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error": {"code": "nzb_processing_failed", "message": format!("Failed to process NZB: {}", e)}}))).into_response()
        }
    }
}

/// POST /downloads/url - Add NZB from URL
#[utoipa::path(
    post,
    path = "/api/v1/downloads/url",
    tag = "downloads",
    request_body(content = String, description = "URL to NZB file"),
    responses(
        (status = 201, description = "Download added successfully", body = i64),
        (status = 400, description = "Invalid URL or NZB content"),
        (status = 409, description = "Duplicate download detected"),
        (status = 422, description = "Unprocessable entity"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn add_download_url(
    State(state): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let url = match payload.get("url").and_then(|v| v.as_str()) {
        Some(url) => url,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {"code": "missing_url", "message": "Missing required field: url"}}))
            ).into_response();
        }
    };

    let options = if let Some(options_value) = payload.get("options") {
        match serde_json::from_value(options_value.clone()) {
            Ok(opts) => opts,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": {"code": "invalid_options", "message": format!("Invalid download options: {}", e)}}))
                ).into_response();
            }
        }
    } else {
        crate::types::DownloadOptions::default()
    };

    match state.downloader.add_nzb_url(url, options).await {
        Ok(id) => (StatusCode::CREATED, Json(json!({"id": id}))).into_response(),
        Err(e) => {
            let (status, code, message) = match e {
                crate::error::Error::Duplicate(msg) => (StatusCode::CONFLICT, "duplicate", msg),
                crate::error::Error::Io(ref e) => (
                    StatusCode::BAD_REQUEST,
                    "io_error",
                    format!("I/O error: {}", e),
                ),
                crate::error::Error::Network(ref e) => (
                    StatusCode::BAD_REQUEST,
                    "network_error",
                    format!("Network error: {}", e),
                ),
                crate::error::Error::InvalidNzb(ref e) => (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_nzb",
                    format!("Invalid NZB: {}", e),
                ),
                _ => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "add_failed",
                    format!("Failed to add NZB from URL: {}", e),
                ),
            };

            (
                status,
                Json(json!({"error": {"code": code, "message": message}})),
            )
                .into_response()
        }
    }
}

/// POST /downloads/:id/pause - Pause download
#[utoipa::path(
    post,
    path = "/api/v1/downloads/{id}/pause",
    tag = "downloads",
    params(("id" = i64, Path, description = "Download ID")),
    responses(
        (status = 204, description = "Download paused successfully"),
        (status = 404, description = "Download not found"),
        (status = 409, description = "Download already paused"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn pause_download(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match state.downloader.pause(crate::types::DownloadId(id)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": {"code": "not_found", "message": error_msg}})),
                )
                    .into_response()
            } else if error_msg.contains("cannot pause") {
                (
                    StatusCode::CONFLICT,
                    Json(json!({"error": {"code": "invalid_state", "message": error_msg}})),
                )
                    .into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"code": "internal_error", "message": error_msg}})),
                )
                    .into_response()
            }
        }
    }
}

/// POST /downloads/:id/resume - Resume download
#[utoipa::path(
    post,
    path = "/api/v1/downloads/{id}/resume",
    tag = "downloads",
    params(("id" = i64, Path, description = "Download ID")),
    responses(
        (status = 204, description = "Download resumed successfully"),
        (status = 404, description = "Download not found"),
        (status = 409, description = "Download not paused"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn resume_download(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match state.downloader.resume(crate::types::DownloadId(id)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": {"code": "not_found", "message": error_msg}})),
                )
                    .into_response()
            } else if error_msg.contains("cannot resume") {
                (
                    StatusCode::CONFLICT,
                    Json(json!({"error": {"code": "invalid_state", "message": error_msg}})),
                )
                    .into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"code": "internal_error", "message": error_msg}})),
                )
                    .into_response()
            }
        }
    }
}

/// DELETE /downloads/:id - Cancel/remove download
#[utoipa::path(
    delete,
    path = "/api/v1/downloads/{id}",
    tag = "downloads",
    params(
        ("id" = i64, Path, description = "Download ID"),
        ("delete_files" = Option<bool>, Query, description = "Whether to delete downloaded files from the destination directory (default: false, always deletes temp files)")
    ),
    responses(
        (status = 204, description = "Download deleted successfully"),
        (status = 404, description = "Download not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_download(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(params): Query<DeleteDownloadQuery>,
) -> impl IntoResponse {
    let download_id = crate::types::DownloadId(id);

    // If delete_files is requested, look up the destination path before cancelling
    // (cancel deletes the DB record, so we need the path first)
    let destination = if params.delete_files {
        match state.downloader.db.get_download(download_id).await {
            Ok(Some(d)) => Some(std::path::PathBuf::from(d.destination)),
            Ok(None) => {
                return (StatusCode::NOT_FOUND, Json(json!({"error": {"code": "not_found", "message": format!("Download {} not found", id)}}))).into_response();
            }
            Err(e) => {
                tracing::error!(download_id = id, error = %e, "Failed to get download for file deletion");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"code": "internal_error", "message": e.to_string()}})),
                )
                    .into_response();
            }
        }
    } else {
        None
    };

    match state.downloader.cancel(download_id).await {
        Ok(_) => {
            // If delete_files was requested, remove the destination directory
            if let Some(dest) = destination
                && dest.exists()
                && let Err(e) = tokio::fs::remove_dir_all(&dest).await
            {
                tracing::warn!(
                    download_id = id,
                    path = ?dest,
                    error = %e,
                    "Failed to delete destination files"
                );
                // Return success anyway — the download record is already gone
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            if e.to_string().contains("not found") {
                (StatusCode::NOT_FOUND, Json(json!({"error": {"code": "not_found", "message": format!("Download {} not found", id)}}))).into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"code": "internal_error", "message": e.to_string()}})),
                )
                    .into_response()
            }
        }
    }
}

/// PATCH /downloads/:id/priority - Set priority
#[utoipa::path(
    patch,
    path = "/api/v1/downloads/{id}/priority",
    tag = "downloads",
    params(("id" = i64, Path, description = "Download ID")),
    request_body(content = crate::types::Priority, description = "New priority level"),
    responses(
        (status = 204, description = "Priority updated successfully"),
        (status = 404, description = "Download not found"),
        (status = 400, description = "Invalid priority value"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn set_download_priority(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let priority = match payload.get("priority") {
        Some(priority_value) => {
            match serde_json::from_value::<crate::types::Priority>(priority_value.clone()) {
                Ok(p) => p,
                Err(e) => {
                    return (StatusCode::BAD_REQUEST, Json(json!({"error": {"code": "invalid_priority", "message": format!("Invalid priority value: {}", e)}}))).into_response();
                }
            }
        }
        None => {
            return (StatusCode::BAD_REQUEST, Json(json!({"error": {"code": "missing_priority", "message": "Request body must include 'priority' field"}}))).into_response();
        }
    };

    match state
        .downloader
        .set_priority(crate::types::DownloadId(id), priority)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": {"code": "not_found", "message": error_msg}})),
                )
                    .into_response()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"code": "internal_error", "message": error_msg}})),
                )
                    .into_response()
            }
        }
    }
}

/// POST /downloads/:id/reprocess - Re-run post-processing
#[utoipa::path(
    post,
    path = "/api/v1/downloads/{id}/reprocess",
    tag = "downloads",
    params(("id" = i64, Path, description = "Download ID")),
    responses(
        (status = 204, description = "Reprocessing started successfully"),
        (status = 404, description = "Download not found"),
        (status = 400, description = "Download files not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn reprocess_download(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match state
        .downloader
        .reprocess(crate::types::DownloadId(id))
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(crate::Error::NotFound(msg)) => {
            let error_code = if msg.contains("Download files not found") {
                "files_not_found"
            } else {
                "not_found"
            };
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": {"code": error_code, "message": msg}})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(download_id = id, error = %e, "Failed to reprocess download");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "internal_error", "message": format!("Failed to reprocess download: {}", e)}}))).into_response()
        }
    }
}

/// POST /downloads/:id/reextract - Re-run extraction only
#[utoipa::path(
    post,
    path = "/api/v1/downloads/{id}/reextract",
    tag = "downloads",
    params(("id" = i64, Path, description = "Download ID")),
    responses(
        (status = 204, description = "Re-extraction started successfully"),
        (status = 404, description = "Download not found"),
        (status = 400, description = "Download files not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn reextract_download(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match state
        .downloader
        .reextract(crate::types::DownloadId(id))
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(crate::Error::NotFound(msg)) => {
            let error_code = if msg.contains("Download files not found") {
                "files_not_found"
            } else {
                "not_found"
            };
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": {"code": error_code, "message": msg}})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(download_id = id, error = %e, "Failed to reextract download");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "internal_error", "message": format!("Failed to reextract download: {}", e)}}))).into_response()
        }
    }
}
