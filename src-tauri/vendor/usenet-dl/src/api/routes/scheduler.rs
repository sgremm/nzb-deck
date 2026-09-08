//! Schedule rule management handlers.

use super::ScheduleRuleResponse;
use crate::api::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde_json::json;

/// GET /scheduler - Get schedule rules
#[utoipa::path(
    get,
    path = "/api/v1/scheduler",
    tag = "scheduler",
    responses(
        (status = 200, description = "List of schedule rules", body = Vec<ScheduleRuleResponse>),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn list_schedule_rules(State(state): State<AppState>) -> impl IntoResponse {
    let rules = state.downloader.get_schedule_rules().await;

    let response: Vec<ScheduleRuleResponse> = rules
        .into_iter()
        .enumerate()
        .map(|(id, rule)| ScheduleRuleResponse {
            id: id as i64,
            rule,
        })
        .collect();

    Json(response).into_response()
}

/// POST /scheduler - Add schedule rule
#[utoipa::path(
    post,
    path = "/api/v1/scheduler",
    tag = "scheduler",
    request_body(content = crate::config::ScheduleRule, description = "Schedule rule configuration"),
    responses(
        (status = 201, description = "Schedule rule added successfully", body = i64),
        (status = 400, description = "Invalid schedule rule configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn add_schedule_rule(
    State(state): State<AppState>,
    Json(rule): Json<crate::config::ScheduleRule>,
) -> impl IntoResponse {
    if chrono::NaiveTime::parse_from_str(&rule.start_time, "%H:%M").is_err() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": {"code": "invalid_input", "message": format!("Invalid start_time format: '{}'. Expected HH:MM", rule.start_time)}}))).into_response();
    }
    if chrono::NaiveTime::parse_from_str(&rule.end_time, "%H:%M").is_err() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": {"code": "invalid_input", "message": format!("Invalid end_time format: '{}'. Expected HH:MM", rule.end_time)}}))).into_response();
    }

    let id = state.downloader.add_schedule_rule(rule).await;
    (StatusCode::CREATED, Json(json!({"id": id}))).into_response()
}

/// PUT /scheduler/:id - Update schedule rule
#[utoipa::path(
    put,
    path = "/api/v1/scheduler/{id}",
    tag = "scheduler",
    params(("id" = i64, Path, description = "Schedule rule ID")),
    request_body(content = crate::config::ScheduleRule, description = "Updated schedule rule configuration"),
    responses(
        (status = 204, description = "Schedule rule updated successfully"),
        (status = 404, description = "Schedule rule not found"),
        (status = 400, description = "Invalid schedule rule configuration"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn update_schedule_rule(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(rule): Json<crate::config::ScheduleRule>,
) -> impl IntoResponse {
    let id = crate::scheduler::RuleId::from(id);
    if chrono::NaiveTime::parse_from_str(&rule.start_time, "%H:%M").is_err() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": {"code": "invalid_input", "message": format!("Invalid start_time format: '{}'. Expected HH:MM", rule.start_time)}}))).into_response();
    }
    if chrono::NaiveTime::parse_from_str(&rule.end_time, "%H:%M").is_err() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": {"code": "invalid_input", "message": format!("Invalid end_time format: '{}'. Expected HH:MM", rule.end_time)}}))).into_response();
    }

    match state.downloader.update_schedule_rule(id, rule).await {
        true => StatusCode::NO_CONTENT.into_response(),
        false => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code": "not_found", "message": "Schedule rule not found"}})),
        )
            .into_response(),
    }
}

/// DELETE /scheduler/:id - Delete schedule rule
#[utoipa::path(
    delete,
    path = "/api/v1/scheduler/{id}",
    tag = "scheduler",
    params(("id" = i64, Path, description = "Schedule rule ID")),
    responses(
        (status = 204, description = "Schedule rule deleted successfully"),
        (status = 404, description = "Schedule rule not found"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn delete_schedule_rule(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let id = crate::scheduler::RuleId::from(id);
    match state.downloader.remove_schedule_rule(id).await {
        true => StatusCode::NO_CONTENT.into_response(),
        false => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code": "not_found", "message": "Schedule rule not found"}})),
        )
            .into_response(),
    }
}
