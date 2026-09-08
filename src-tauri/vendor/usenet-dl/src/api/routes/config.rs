//! Configuration handlers.

use super::SetSpeedLimitRequest;
use crate::api::AppState;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde_json::json;

/// GET /config - Get current config (sensitive fields redacted)
#[utoipa::path(
    get,
    path = "/api/v1/config",
    tag = "config",
    responses(
        (status = 200, description = "Current configuration", body = crate::config::Config),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_config(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.downloader.get_config();

    let mut redacted_config = (*config).clone();

    // Redact server passwords
    for server in &mut redacted_config.servers {
        if server.password.is_some() {
            server.password = Some("***REDACTED***".to_string());
        }
    }

    // Redact API key
    if redacted_config.server.api.api_key.is_some() {
        redacted_config.server.api.api_key = Some("***REDACTED***".to_string());
    }

    // Redact webhook auth headers
    for webhook in &mut redacted_config.notifications.webhooks {
        if webhook.auth_header.is_some() {
            webhook.auth_header = Some("***REDACTED***".to_string());
        }
    }

    (StatusCode::OK, Json(redacted_config))
}

/// PATCH /config - Update config
#[utoipa::path(
    patch,
    path = "/api/v1/config",
    tag = "config",
    request_body(content = crate::config::ConfigUpdate, description = "Configuration updates (runtime-changeable fields only)"),
    responses(
        (status = 200, description = "Configuration updated", body = crate::config::Config),
        (status = 400, description = "Invalid configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_config(
    State(state): State<AppState>,
    Json(updates): Json<crate::config::ConfigUpdate>,
) -> impl IntoResponse {
    state.downloader.update_config(updates).await;

    // Return the updated config (with redaction)
    get_config(State(state)).await
}

/// GET /config/speed-limit - Get speed limit
#[utoipa::path(
    get,
    path = "/api/v1/config/speed-limit",
    tag = "config",
    responses(
        (status = 200, description = "Current speed limit in bytes per second", body = inline(Object)),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_speed_limit(State(state): State<AppState>) -> impl IntoResponse {
    let limit_bps = state.downloader.get_speed_limit();
    Json(json!({"limit_bps": limit_bps}))
}

/// PUT /config/speed-limit - Set speed limit
#[utoipa::path(
    put,
    path = "/api/v1/config/speed-limit",
    tag = "config",
    request_body = SetSpeedLimitRequest,
    responses(
        (status = 204, description = "Speed limit updated successfully"),
        (status = 400, description = "Invalid speed limit value"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn set_speed_limit(
    State(state): State<AppState>,
    Json(request): Json<SetSpeedLimitRequest>,
) -> impl IntoResponse {
    state.downloader.set_speed_limit(request.limit_bps).await;
    StatusCode::NO_CONTENT
}
