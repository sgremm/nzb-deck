//! HTTP error response handling for the API
//!
//! This module provides conversions from domain errors to HTTP responses
//! with appropriate status codes and JSON error bodies.

use crate::error::{ApiError, Error, ToHttpStatus};
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

/// Implement IntoResponse for Error to automatically convert errors to HTTP responses
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status_code =
            StatusCode::from_u16(self.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        let api_error: ApiError = self.into();

        (status_code, Json(api_error)).into_response()
    }
}

/// Implement IntoResponse for ApiError for explicit error responses
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // Default to 500 if we're directly converting an ApiError
        // (usually errors go through Error::into_response which has the status code)
        (StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
    }
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{DatabaseError, DownloadError, PostProcessError};
    use std::path::PathBuf;

    #[test]
    fn test_error_to_http_status_not_found() {
        let error = Error::NotFound("test".to_string());
        assert_eq!(error.status_code(), 404);
        assert_eq!(error.error_code(), "not_found");
    }

    #[test]
    fn test_error_to_http_status_download_not_found() {
        let error = Error::Download(DownloadError::NotFound { id: 123 });
        assert_eq!(error.status_code(), 404);
        assert_eq!(error.error_code(), "download_not_found");
    }

    #[test]
    fn test_error_to_http_status_conflict() {
        let error = Error::Download(DownloadError::AlreadyInState {
            id: 123,
            state: "paused".to_string(),
        });
        assert_eq!(error.status_code(), 409);
        assert_eq!(error.error_code(), "already_in_state");
    }

    #[test]
    fn test_error_to_http_status_unprocessable() {
        let error = Error::InvalidNzb("bad nzb".to_string());
        assert_eq!(error.status_code(), 422);
        assert_eq!(error.error_code(), "invalid_nzb");
    }

    #[test]
    fn test_error_to_http_status_service_unavailable() {
        let error = Error::ShuttingDown;
        assert_eq!(error.status_code(), 503);
        assert_eq!(error.error_code(), "shutting_down");
    }

    #[test]
    fn test_error_to_http_status_internal_server() {
        let error = Error::Database(DatabaseError::QueryFailed("query failed".to_string()));
        assert_eq!(error.status_code(), 500);
        assert_eq!(error.error_code(), "database_error");
    }

    #[test]
    fn test_error_to_api_error_with_details() {
        let error = Error::Download(DownloadError::NotFound { id: 123 });
        let api_error: ApiError = error.into();

        assert_eq!(api_error.error.code, "download_not_found");
        assert!(api_error.error.message.contains("123"));
        assert!(api_error.error.details.is_some());

        let details = api_error.error.details.unwrap();
        assert_eq!(details["download_id"], 123);
    }

    #[test]
    fn test_error_to_api_error_insufficient_space() {
        let error = Error::InsufficientSpace {
            required: 1000,
            available: 500,
        };
        let api_error: ApiError = error.into();

        assert_eq!(api_error.error.code, "insufficient_space");
        assert!(api_error.error.message.contains("1000"));
        assert!(api_error.error.message.contains("500"));

        let details = api_error.error.details.unwrap();
        assert_eq!(details["required_bytes"], 1000);
        assert_eq!(details["available_bytes"], 500);
    }

    #[test]
    fn test_error_to_api_error_post_process() {
        let error = Error::PostProcess(PostProcessError::WrongPassword {
            archive: PathBuf::from("/path/to/archive.rar"),
        });
        let api_error: ApiError = error.into();

        assert_eq!(api_error.error.code, "wrong_password");
        assert!(api_error.error.details.is_some());

        let details = api_error.error.details.unwrap();
        assert!(details["archive"].as_str().unwrap().contains("archive.rar"));
    }

    #[tokio::test]
    async fn test_error_into_response() {
        let error = Error::NotFound("test resource".to_string());
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // Extract and verify the JSON body
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let api_error: ApiError = serde_json::from_slice(&body).unwrap();

        assert_eq!(api_error.error.code, "not_found");
        assert!(api_error.error.message.contains("test resource"));
    }

    #[tokio::test]
    async fn test_download_error_into_response() {
        let error = Error::Download(DownloadError::AlreadyInState {
            id: 456,
            state: "downloading".to_string(),
        });
        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::CONFLICT);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let api_error: ApiError = serde_json::from_slice(&body).unwrap();

        assert_eq!(api_error.error.code, "already_in_state");
        assert_eq!(
            api_error.error.details.as_ref().unwrap()["download_id"],
            456
        );
        assert_eq!(
            api_error.error.details.as_ref().unwrap()["state"],
            "downloading"
        );
    }
}
