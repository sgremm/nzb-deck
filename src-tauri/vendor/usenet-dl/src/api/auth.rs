//! Authentication middleware for the REST API
//!
//! Provides optional API key authentication via X-Api-Key header.
//! When ApiConfig::api_key is set, all requests must include a matching
//! X-Api-Key header or they will receive a 401 Unauthorized response.

use axum::{
    Json,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// Authentication middleware that checks for a valid API key in the X-Api-Key header
///
/// # Arguments
///
/// * `State(expected_api_key)` - The API key that must be present in the X-Api-Key header
/// * `request` - The incoming HTTP request
/// * `next` - The next middleware/handler in the chain
///
/// # Returns
///
/// Returns either:
/// - 401 Unauthorized if the API key is missing or invalid
/// - The response from the next handler if authentication succeeds
///
/// # Examples
///
/// ```no_run
/// use axum::{Router, middleware};
/// use usenet_dl::api::auth::require_api_key;
///
/// let api_key = Some("secret-key-123".to_string());
/// let router = Router::new()
///     .layer(middleware::from_fn_with_state(
///         api_key,
///         require_api_key
///     ));
/// ```
pub async fn require_api_key(
    State(expected_api_key): State<Option<String>>,
    request: Request,
    next: Next,
) -> Response {
    // If no API key is configured, allow all requests through
    let Some(expected_key) = expected_api_key else {
        return next.run(request).await;
    };

    // Extract the X-Api-Key header
    let api_key_header = request
        .headers()
        .get("x-api-key")
        .and_then(|value| value.to_str().ok());

    // Check if the provided API key matches the expected one
    // Uses constant-time comparison to prevent timing side-channel attacks
    match api_key_header {
        Some(provided_key)
            if constant_time_eq(provided_key.as_bytes(), expected_key.as_bytes()) =>
        {
            // API key is valid, proceed to the next handler
            next.run(request).await
        }
        Some(_) => {
            // API key is present but invalid
            unauthorized_response("Invalid API key")
        }
        None => {
            // API key is missing
            unauthorized_response("Missing X-Api-Key header")
        }
    }
}

/// Constant-time byte comparison to prevent timing side-channel attacks.
/// Always compares all bytes regardless of where the first mismatch occurs.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

/// Helper function to create a 401 Unauthorized response with a JSON error message
fn unauthorized_response(message: &str) -> Response {
    let body = Json(json!({
        "error": {
            "code": "unauthorized",
            "message": message
        }
    }));

    (StatusCode::UNAUTHORIZED, body).into_response()
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
    };
    use tower::ServiceExt; // for oneshot

    // Simple test handler that returns 200 OK
    async fn test_handler() -> impl IntoResponse {
        (StatusCode::OK, "Success")
    }

    #[tokio::test]
    async fn test_no_api_key_configured() {
        // When no API key is configured, all requests should pass through
        let app =
            Router::new()
                .route("/test", get(test_handler))
                .layer(middleware::from_fn_with_state(
                    None::<String>,
                    require_api_key,
                ));

        let request = Request::builder().uri("/test").body(Body::empty()).unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_valid_api_key() {
        // When a valid API key is provided, the request should succeed
        let api_key = Some("test-secret-key".to_string());

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(api_key, require_api_key));

        let request = Request::builder()
            .uri("/test")
            .header("X-Api-Key", "test-secret-key")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_invalid_api_key() {
        // When an invalid API key is provided, should return 401
        let api_key = Some("correct-key".to_string());

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(api_key, require_api_key));

        let request = Request::builder()
            .uri("/test")
            .header("X-Api-Key", "wrong-key")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        // Check the response body contains error message
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Invalid API key"));
    }

    #[tokio::test]
    async fn test_missing_api_key() {
        // When API key is required but not provided, should return 401
        let api_key = Some("required-key".to_string());

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(api_key, require_api_key));

        let request = Request::builder().uri("/test").body(Body::empty()).unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        // Check the response body contains error message
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Missing X-Api-Key header"));
    }

    #[tokio::test]
    async fn test_api_key_case_sensitive() {
        // API keys should be case-sensitive
        let api_key = Some("CaseSensitiveKey".to_string());

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(api_key, require_api_key));

        let request = Request::builder()
            .uri("/test")
            .header("X-Api-Key", "casesensitivekey") // lowercase version
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_header_name_case_insensitive() {
        // HTTP headers are case-insensitive, so X-Api-Key, x-api-key, etc. should all work
        let api_key = Some("test-key".to_string());

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(api_key, require_api_key));

        // Test with lowercase header name
        let request = Request::builder()
            .uri("/test")
            .header("x-api-key", "test-key")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_whitespace_in_api_key() {
        // API keys with whitespace should be compared exactly (no trimming)
        let api_key = Some("key-with-space ".to_string());

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(api_key, require_api_key));

        // Without trailing space - should fail
        let request = Request::builder()
            .uri("/test")
            .header("X-Api-Key", "key-with-space")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
