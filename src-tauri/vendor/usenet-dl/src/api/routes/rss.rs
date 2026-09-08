//! RSS feed management handlers.

use super::{AddRssFeedRequest, CheckRssFeedResponse, RssFeedResponse};
use crate::api::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;

/// GET /rss - List RSS feeds
#[utoipa::path(
    get,
    path = "/api/v1/rss",
    tag = "rss",
    responses(
        (status = 200, description = "List of RSS feeds", body = Vec<RssFeedResponse>),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_rss_feeds(State(state): State<AppState>) -> impl IntoResponse {
    let feeds = match state.downloader.db.get_all_rss_feeds().await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("Failed to get RSS feeds: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "database_error", "message": format!("Failed to get RSS feeds: {}", e)}}))).into_response();
        }
    };

    let mut responses = Vec::new();

    for feed in feeds {
        let filter_rows = match state.downloader.db.get_rss_filters(feed.id).await {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("Failed to get filters for feed {}: {}", feed.id, e);
                continue;
            }
        };

        let filters = filter_rows
            .into_iter()
            .map(|row| {
                use std::time::Duration;
                crate::config::RssFilter {
                    name: row.name,
                    include: row
                        .include_patterns
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    exclude: row
                        .exclude_patterns
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    min_size: row.min_size.map(|s| s as u64),
                    max_size: row.max_size.map(|s| s as u64),
                    max_age: row.max_age_secs.map(|s| Duration::from_secs(s as u64)),
                }
            })
            .collect();

        responses.push(RssFeedResponse {
            id: feed.id,
            name: feed.name,
            config: crate::config::RssFeedConfig {
                url: feed.url,
                check_interval: std::time::Duration::from_secs(feed.check_interval_secs as u64),
                category: feed.category,
                filters,
                auto_download: feed.auto_download != 0,
                priority: crate::types::Priority::from_i32(feed.priority),
                enabled: feed.enabled != 0,
            },
        });
    }

    (StatusCode::OK, Json(responses)).into_response()
}

/// POST /rss - Add RSS feed
#[utoipa::path(
    post,
    path = "/api/v1/rss",
    tag = "rss",
    request_body = AddRssFeedRequest,
    responses(
        (status = 201, description = "RSS feed added successfully", body = i64),
        (status = 400, description = "Invalid RSS feed configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn add_rss_feed(
    State(state): State<AppState>,
    Json(request): Json<AddRssFeedRequest>,
) -> impl IntoResponse {
    if request.config.url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({"error": {"code": "invalid_input", "message": "Feed URL cannot be empty"}}),
            ),
        )
            .into_response();
    }

    // Validate URL scheme and host to prevent SSRF attacks
    if let Err(msg) = validate_feed_url(&request.config.url) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code": "invalid_input", "message": msg}})),
        )
            .into_response();
    }

    match state
        .downloader
        .add_rss_feed(&request.name, request.config)
        .await
    {
        Ok(id) => (StatusCode::CREATED, Json(json!({"id": id}))).into_response(),
        Err(e) => {
            tracing::error!("Failed to add RSS feed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "database_error", "message": format!("Failed to add RSS feed: {}", e)}}))).into_response()
        }
    }
}

/// PUT /rss/:id - Update RSS feed
#[utoipa::path(
    put,
    path = "/api/v1/rss/{id}",
    tag = "rss",
    params(("id" = i64, Path, description = "RSS feed ID")),
    request_body = AddRssFeedRequest,
    responses(
        (status = 204, description = "RSS feed updated successfully"),
        (status = 404, description = "RSS feed not found"),
        (status = 400, description = "Invalid RSS feed configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_rss_feed(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(request): Json<AddRssFeedRequest>,
) -> impl IntoResponse {
    if request.config.url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({"error": {"code": "invalid_input", "message": "Feed URL cannot be empty"}}),
            ),
        )
            .into_response();
    }

    // Validate URL scheme and host to prevent SSRF attacks
    if let Err(msg) = validate_feed_url(&request.config.url) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"code": "invalid_input", "message": msg}})),
        )
            .into_response();
    }

    match state
        .downloader
        .update_rss_feed(id, &request.name, request.config)
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code": "not_found", "message": "RSS feed not found"}})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to update RSS feed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "database_error", "message": format!("Failed to update RSS feed: {}", e)}}))).into_response()
        }
    }
}

/// DELETE /rss/:id - Delete RSS feed
#[utoipa::path(
    delete,
    path = "/api/v1/rss/{id}",
    tag = "rss",
    params(("id" = i64, Path, description = "RSS feed ID")),
    responses(
        (status = 204, description = "RSS feed deleted successfully"),
        (status = 404, description = "RSS feed not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_rss_feed(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match state.downloader.delete_rss_feed(id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code": "not_found", "message": "RSS feed not found"}})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to delete RSS feed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "database_error", "message": format!("Failed to delete RSS feed: {}", e)}}))).into_response()
        }
    }
}

/// Validate that a feed URL is safe (not targeting internal services).
fn validate_feed_url(url_str: &str) -> std::result::Result<(), String> {
    let parsed = url::Url::parse(url_str).map_err(|_| "Invalid URL format".to_string())?;

    // Only allow http and https schemes
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(format!(
                "URL scheme '{}' is not allowed; only http and https are supported",
                scheme
            ));
        }
    }

    // Check for localhost / loopback / private IP ranges
    if let Some(host) = parsed.host_str() {
        let host_lower = host.to_lowercase();
        if host_lower == "localhost"
            || host_lower == "127.0.0.1"
            || host_lower == "::1"
            || host_lower == "[::1]"
            || host_lower == "0.0.0.0"
            || host_lower.starts_with("10.")
            || host_lower.starts_with("192.168.")
            || host_lower == "169.254.169.254"
            || host_lower.ends_with(".internal")
            || host_lower.ends_with(".local")
        {
            return Err("URL targets a private/internal address".to_string());
        }
        // Check 172.16.0.0/12 range
        if host_lower.starts_with("172.")
            && let Some(second_octet) = host_lower
                .strip_prefix("172.")
                .and_then(|s| s.split('.').next())
            && let Ok(octet) = second_octet.parse::<u8>()
            && (16..=31).contains(&octet)
        {
            return Err("URL targets a private/internal address".to_string());
        }
    } else {
        return Err("URL has no host".to_string());
    }

    Ok(())
}

/// POST /rss/:id/check - Force check feed now
#[utoipa::path(
    post,
    path = "/api/v1/rss/{id}/check",
    tag = "rss",
    params(("id" = i64, Path, description = "RSS feed ID")),
    responses(
        (status = 200, description = "Number of new items queued", body = CheckRssFeedResponse),
        (status = 404, description = "RSS feed not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn check_rss_feed(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match state.downloader.check_rss_feed_now(id).await {
        Ok(queued) => (StatusCode::OK, Json(CheckRssFeedResponse { queued })).into_response(),
        Err(crate::Error::NotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code": "not_found", "message": "RSS feed not found"}})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to check RSS feed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"code": "check_failed", "message": format!("Failed to check RSS feed: {}", e)}}))).into_response()
        }
    }
}
