//! Server management handlers.

use crate::api::AppState;
use crate::config::ServerConfig;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

/// POST /servers/test - Test server connection
#[utoipa::path(
    post,
    path = "/api/v1/servers/test",
    tag = "servers",
    request_body(content = crate::config::ServerConfig, description = "Server configuration to test"),
    responses(
        (status = 200, description = "Server test result", body = crate::types::ServerTestResult),
        (status = 400, description = "Invalid server configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn test_server(
    State(state): State<AppState>,
    Json(server): Json<ServerConfig>,
) -> impl IntoResponse {
    let result = state.downloader.test_server(&server).await;
    (StatusCode::OK, Json(result))
}

/// GET /servers/test - Test all configured servers
#[utoipa::path(
    get,
    path = "/api/v1/servers/test",
    tag = "servers",
    responses(
        (status = 200, description = "Test results for all servers", body = Vec<(String, crate::types::ServerTestResult)>),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn test_all_servers(State(state): State<AppState>) -> impl IntoResponse {
    let results = state.downloader.test_all_servers().await;
    (StatusCode::OK, Json(results))
}
