//! Category management handlers.

use crate::api::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;

/// GET /categories - List categories
#[utoipa::path(
    get,
    path = "/api/v1/categories",
    tag = "categories",
    responses(
        (status = 200, description = "Map of category names to configurations"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_categories(State(state): State<AppState>) -> impl IntoResponse {
    let categories = state.downloader.get_categories().await;
    (StatusCode::OK, Json(categories))
}

/// PUT /categories/:name - Create/update category
#[utoipa::path(
    put,
    path = "/api/v1/categories/{name}",
    tag = "categories",
    params(("name" = String, Path, description = "Category name")),
    request_body(content = crate::config::CategoryConfig, description = "Category configuration"),
    responses(
        (status = 204, description = "Category created/updated successfully"),
        (status = 400, description = "Invalid category configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn create_or_update_category(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(category_config): Json<crate::config::CategoryConfig>,
) -> impl IntoResponse {
    state
        .downloader
        .add_or_update_category(&name, category_config)
        .await;
    StatusCode::NO_CONTENT
}

/// DELETE /categories/:name - Delete category
#[utoipa::path(
    delete,
    path = "/api/v1/categories/{name}",
    tag = "categories",
    params(("name" = String, Path, description = "Category name")),
    responses(
        (status = 204, description = "Category deleted successfully"),
        (status = 404, description = "Category not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_category(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let was_removed = state.downloader.remove_category(&name).await;

    if was_removed {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(json!({"error": {"code": "category_not_found", "message": format!("Category '{}' not found", name)}}))).into_response()
    }
}
