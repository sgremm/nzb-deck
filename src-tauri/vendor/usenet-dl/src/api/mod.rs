//! REST API server module
//!
//! Provides an OpenAPI 3.1 compliant REST API for managing downloads,
//! configuration, and monitoring the download queue.

use crate::{Config, Result, UsenetDownloader};
use axum::{
    Router,
    http::HeaderValue,
    middleware,
    routing::{delete, get, patch, post, put},
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

pub mod auth;
pub mod error_response;
pub mod openapi;
pub mod rate_limit;
pub mod routes;
pub mod state;

pub use openapi::ApiDoc;
pub use state::AppState;

/// Create the API router with all route definitions
///
/// # Routes
///
/// ## Queue Management
/// - `GET /downloads` - List all downloads
/// - `GET /downloads/:id` - Get single download
/// - `POST /downloads` - Add NZB from file upload
/// - `POST /downloads/url` - Add NZB from URL
/// - `POST /downloads/:id/pause` - Pause download
/// - `POST /downloads/:id/resume` - Resume download
/// - `DELETE /downloads/:id` - Cancel/remove download
/// - `PATCH /downloads/:id/priority` - Set priority
/// - `POST /downloads/:id/reprocess` - Re-run post-processing
/// - `POST /downloads/:id/reextract` - Re-run extraction only
///
/// ## Queue-Wide Operations
/// - `POST /queue/pause` - Pause all downloads
/// - `POST /queue/resume` - Resume all downloads
/// - `GET /queue/stats` - Get queue statistics
///
/// ## History
/// - `GET /history` - Get download history (with pagination)
/// - `DELETE /history` - Clear history
///
/// ## Server Management
/// - `POST /servers/test` - Test server connection
/// - `GET /servers/test` - Test all configured servers
///
/// ## Configuration
/// - `GET /config` - Get current config (sensitive fields redacted)
/// - `PATCH /config` - Update config
/// - `GET /config/speed-limit` - Get speed limit
/// - `PUT /config/speed-limit` - Set speed limit
///
/// ## Categories
/// - `GET /categories` - List categories
/// - `PUT /categories/:name` - Create/update category
/// - `DELETE /categories/:name` - Delete category
///
/// ## System
/// - `GET /capabilities` - Query system capabilities
/// - `GET /health` - Health check
/// - `GET /openapi.json` - OpenAPI specification
/// - `GET /swagger-ui` - Interactive Swagger UI documentation (if enabled)
/// - `GET /events` - Server-sent events stream
/// - `POST /shutdown` - Graceful shutdown
///
/// ## RSS Feeds
/// - `GET /rss` - List RSS feeds
/// - `POST /rss` - Add RSS feed
/// - `PUT /rss/:id` - Update RSS feed
/// - `DELETE /rss/:id` - Delete RSS feed
/// - `POST /rss/:id/check` - Force check feed now
///
/// ## Scheduler
/// - `GET /scheduler` - Get schedule rules
/// - `POST /scheduler` - Add schedule rule
/// - `PUT /scheduler/:id` - Update schedule rule
/// - `DELETE /scheduler/:id` - Delete schedule rule
pub fn create_router(downloader: Arc<UsenetDownloader>, config: Arc<Config>) -> Router {
    let state = AppState::new(downloader, config.clone());

    // Build the router with all routes
    let router = Router::new()
        // Queue Management - Downloads
        .route("/downloads", get(routes::list_downloads))
        .route("/downloads", post(routes::add_download))
        .route("/downloads/:id", get(routes::get_download))
        .route("/downloads/:id", delete(routes::delete_download))
        .route("/downloads/:id/pause", post(routes::pause_download))
        .route("/downloads/:id/resume", post(routes::resume_download))
        .route(
            "/downloads/:id/priority",
            patch(routes::set_download_priority),
        )
        .route("/downloads/:id/reprocess", post(routes::reprocess_download))
        .route("/downloads/:id/reextract", post(routes::reextract_download))
        // URL-based NZB adding
        .route("/downloads/url", post(routes::add_download_url))
        // Queue-Wide Operations
        .route("/queue/pause", post(routes::pause_queue))
        .route("/queue/resume", post(routes::resume_queue))
        .route("/queue/stats", get(routes::queue_stats))
        // History
        .route("/history", get(routes::get_history))
        .route("/history", delete(routes::clear_history))
        // Server Management
        .route("/servers/test", post(routes::test_server))
        .route("/servers/test", get(routes::test_all_servers))
        // Configuration
        .route("/config", get(routes::get_config))
        .route("/config", patch(routes::update_config))
        .route("/config/speed-limit", get(routes::get_speed_limit))
        .route("/config/speed-limit", put(routes::set_speed_limit))
        // Categories
        .route("/categories", get(routes::list_categories))
        .route("/categories/:name", put(routes::create_or_update_category))
        .route("/categories/:name", delete(routes::delete_category))
        // System
        .route("/capabilities", get(routes::get_capabilities))
        .route("/health", get(routes::health_check))
        .route("/openapi.json", get(routes::openapi_spec))
        .route("/events", get(routes::event_stream))
        .route("/shutdown", post(routes::shutdown))
        // RSS Feeds
        .route("/rss", get(routes::list_rss_feeds))
        .route("/rss", post(routes::add_rss_feed))
        .route("/rss/:id", put(routes::update_rss_feed))
        .route("/rss/:id", delete(routes::delete_rss_feed))
        .route("/rss/:id/check", post(routes::check_rss_feed))
        // Scheduler
        .route("/scheduler", get(routes::list_schedule_rules))
        .route("/scheduler", post(routes::add_schedule_rule))
        .route("/scheduler/:id", put(routes::update_schedule_rule))
        .route("/scheduler/:id", delete(routes::delete_schedule_rule));

    // Merge Swagger UI routes if enabled in config (before applying state)
    // Note: SwaggerUi will use the existing /openapi.json endpoint we already defined
    let router = if config.server.api.swagger_ui {
        router.merge(SwaggerUi::new("/swagger-ui").url("/api/v1/openapi.json", ApiDoc::openapi()))
    } else {
        router
    };

    // Add state to all routes
    let router = router.with_state(state);

    // Middleware layer ordering: In Axum's onion model, the LAST layer applied
    // is the OUTERMOST (runs first on requests). We want:
    //   Request → Rate Limit → Auth → Handler
    // So we apply auth FIRST (innermost), then rate limiting SECOND (outermost).

    // Apply authentication middleware if API key is configured (innermost)
    let router = if config.server.api.api_key.is_some() {
        router.layer(middleware::from_fn_with_state(
            config.server.api.api_key.clone(),
            auth::require_api_key,
        ))
    } else {
        router
    };

    // Apply rate limiting middleware if enabled in config (outermost — runs first)
    let router = if config.server.api.rate_limit.enabled {
        let limiter = Arc::new(rate_limit::RateLimiter::new(
            config.server.api.rate_limit.clone(),
        ));
        router.layer(middleware::from_fn_with_state(
            limiter,
            rate_limit::rate_limit_middleware,
        ))
    } else {
        router
    };

    // Apply CORS middleware if enabled in config
    if config.server.api.cors_enabled {
        let cors = build_cors_layer(&config.server.api.cors_origins);
        router.layer(cors)
    } else {
        router
    }
}

/// Build a CORS layer based on configured origins
///
/// # Arguments
///
/// * `origins` - List of allowed origins (supports "*" for any origin)
///
/// # Returns
///
/// A configured CorsLayer that allows the specified origins, all methods,
/// and all headers for cross-origin requests.
fn build_cors_layer(origins: &[String]) -> CorsLayer {
    // Check if "*" (all origins) is in the list
    let allow_any = origins.iter().any(|o| o == "*");

    if allow_any || origins.is_empty() {
        // Allow all origins (default for local development)
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        // Allow specific origins
        let allowed: Vec<HeaderValue> = origins.iter().filter_map(|o| o.parse().ok()).collect();

        CorsLayer::new()
            .allow_origin(AllowOrigin::list(allowed))
            .allow_methods(Any)
            .allow_headers(Any)
    }
}

/// Start the API server on the configured bind address.
///
/// This function creates a TCP listener, binds it to the configured address,
/// and starts serving the API router. It runs until the server is shut down.
///
/// # Arguments
///
/// * `downloader` - Arc-wrapped UsenetDownloader instance to handle API requests
/// * `config` - Arc-wrapped Config containing API configuration
///
/// # Returns
///
/// Returns a Result<()> that completes when the server stops, either due to
/// an error or graceful shutdown.
///
/// # Example
///
/// ```no_run
/// use usenet_dl::{UsenetDownloader, Config};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = Arc::new(Config::default());
/// let downloader = Arc::new(UsenetDownloader::new((*config).clone()).await?);
///
/// // Start API server (blocks until shutdown)
/// usenet_dl::api::start_api_server(downloader, config).await?;
/// # Ok(())
/// # }
/// ```
pub async fn start_api_server(
    downloader: Arc<UsenetDownloader>,
    config: Arc<Config>,
) -> Result<()> {
    let bind_address = config.server.api.bind_address;

    tracing::info!(
        address = %bind_address,
        "Starting API server"
    );

    // Create the router with all routes
    let app = create_router(downloader, config);

    // Bind TCP listener to the configured address
    let listener = TcpListener::bind(bind_address)
        .await
        .map_err(crate::error::Error::Io)?;

    tracing::info!(
        address = %bind_address,
        "API server listening"
    );

    // Serve the API using the listener
    // Must use into_make_service_with_connect_info to provide ConnectInfo<SocketAddr>
    // for the rate limiting middleware
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(|e| crate::error::Error::ApiServerError(e.to_string()))?;

    tracing::info!("API server stopped");
    Ok(())
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests;
