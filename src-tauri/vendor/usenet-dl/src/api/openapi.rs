//! OpenAPI documentation and schema generation
//!
//! This module defines the OpenAPI specification for the usenet-dl REST API
//! using utoipa for compile-time spec generation.

use utoipa::OpenApi;

/// OpenAPI documentation for the usenet-dl REST API
///
/// This struct is used to generate the OpenAPI 3.1 specification that describes
/// all available endpoints, request/response types, and API behavior.
///
/// The spec can be accessed via:
/// - `/api/v1/openapi.json` - JSON format OpenAPI specification
/// - `/swagger-ui` - Interactive Swagger UI documentation
#[derive(OpenApi)]
#[openapi(
    info(
        title = "usenet-dl REST API",
        version = "0.1.0",
        description = "OpenAPI 3.1 compliant REST API for managing Usenet downloads, post-processing, and configuration",
        contact(
            name = "usenet-dl",
            url = "https://github.com/jvz-devx/usenet-dl"
        ),
        license(
            name = "MIT OR Apache-2.0"
        )
    ),
    servers(
        (url = "http://localhost:6789/api/v1", description = "Local development server")
    ),
    paths(
        // Queue Management - Downloads
        crate::api::routes::list_downloads,
        crate::api::routes::get_download,
        crate::api::routes::add_download,
        crate::api::routes::add_download_url,
        crate::api::routes::pause_download,
        crate::api::routes::resume_download,
        crate::api::routes::delete_download,
        crate::api::routes::set_download_priority,
        crate::api::routes::reprocess_download,
        crate::api::routes::reextract_download,

        // Queue-Wide Operations
        crate::api::routes::pause_queue,
        crate::api::routes::resume_queue,
        crate::api::routes::queue_stats,

        // History
        crate::api::routes::get_history,
        crate::api::routes::clear_history,

        // Server Management
        crate::api::routes::test_server,
        crate::api::routes::test_all_servers,

        // Configuration
        crate::api::routes::get_config,
        crate::api::routes::update_config,
        crate::api::routes::get_speed_limit,
        crate::api::routes::set_speed_limit,

        // Categories
        crate::api::routes::list_categories,
        crate::api::routes::create_or_update_category,
        crate::api::routes::delete_category,

        // System
        crate::api::routes::get_capabilities,
        crate::api::routes::health_check,
        crate::api::routes::openapi_spec,
        crate::api::routes::event_stream,
        crate::api::routes::shutdown,

        // RSS Feeds
        crate::api::routes::list_rss_feeds,
        crate::api::routes::add_rss_feed,
        crate::api::routes::update_rss_feed,
        crate::api::routes::delete_rss_feed,
        crate::api::routes::check_rss_feed,

        // Scheduler
        crate::api::routes::list_schedule_rules,
        crate::api::routes::add_schedule_rule,
        crate::api::routes::update_schedule_rule,
        crate::api::routes::delete_schedule_rule,
    ),
    components(schemas(
        // Core types from types.rs
        crate::types::Status,
        crate::types::Priority,
        crate::types::Stage,
        crate::types::ArchiveType,
        crate::types::DownloadInfo,
        crate::types::DownloadOptions,
        crate::types::HistoryEntry,
        crate::types::QueueStats,
        crate::types::Capabilities,
        crate::types::ParityCapabilitiesInfo,
        crate::types::ServerCapabilities,

        // Config types from config.rs
        crate::config::Config,
        crate::config::ConfigUpdate,
        crate::config::ServerConfig,
        crate::config::RetryConfig,
        crate::config::PostProcess,
        crate::config::ExtractionConfig,
        crate::config::FileCollisionAction,
        crate::config::DeobfuscationConfig,
        crate::config::DuplicateConfig,
        crate::config::DuplicateAction,
        crate::config::DuplicateMethod,
        crate::config::DiskSpaceConfig,
        crate::config::CleanupConfig,
        crate::config::ApiConfig,
        crate::config::RateLimitConfig,
        crate::config::ScheduleRule,
        crate::config::ScheduleAction,
        crate::config::Weekday,
        crate::config::WatchFolderConfig,
        crate::config::WatchFolderAction,
        crate::config::WebhookConfig,
        crate::config::WebhookEvent,
        crate::config::ScriptConfig,
        crate::config::ScriptEvent,
        crate::config::CategoryConfig,
        crate::config::RssFeedConfig,
        crate::config::RssFilter,

        // API request/response types from routes.rs
        crate::api::routes::AddRssFeedRequest,
        crate::api::routes::RssFeedResponse,
        crate::api::routes::CheckRssFeedResponse,
        crate::api::routes::ScheduleRuleResponse,

        // Error types from error.rs
        crate::error::ApiError,
        crate::error::ErrorDetail,
    )),
    tags(
        (name = "downloads", description = "Download queue management - Add, pause, resume, and monitor downloads"),
        (name = "queue", description = "Queue-wide operations - Pause/resume all downloads, get statistics"),
        (name = "history", description = "Download history - View completed and failed downloads"),
        (name = "servers", description = "Server management - Test NNTP server connections and configuration"),
        (name = "config", description = "Configuration - Get and update runtime configuration settings"),
        (name = "categories", description = "Categories - Manage download categories and their settings"),
        (name = "system", description = "System endpoints - Health checks, OpenAPI spec, events, shutdown"),
        (name = "rss", description = "RSS feeds - Manage RSS feed subscriptions and automatic downloads"),
        (name = "scheduler", description = "Scheduler - Time-based rules for speed limits and pause/resume"),
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

/// Security addon to add API key authentication scheme to OpenAPI spec
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = &mut openapi.components {
            components.add_security_scheme(
                "api_key",
                utoipa::openapi::security::SecurityScheme::ApiKey(
                    utoipa::openapi::security::ApiKey::Header(
                        utoipa::openapi::security::ApiKeyValue::new("X-Api-Key"),
                    ),
                ),
            );
        }
    }
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openapi_doc_generation() {
        // Test that the OpenAPI spec can be generated without panicking
        let _spec = ApiDoc::openapi();
    }

    #[test]
    fn test_openapi_spec_has_paths() {
        let spec = ApiDoc::openapi();

        // Verify that the spec has paths defined
        assert!(
            !spec.paths.paths.is_empty(),
            "OpenAPI spec should have paths defined"
        );
    }

    #[test]
    fn test_openapi_spec_has_components() {
        let spec = ApiDoc::openapi();

        // Verify that the spec has components (schemas) defined
        assert!(
            spec.components.is_some(),
            "OpenAPI spec should have components defined"
        );

        let components = spec.components.unwrap();
        assert!(
            !components.schemas.is_empty(),
            "OpenAPI spec should have schemas defined"
        );
    }

    #[test]
    fn test_openapi_spec_has_tags() {
        let spec = ApiDoc::openapi();

        // Verify that tags are defined
        assert!(spec.tags.is_some(), "OpenAPI spec should have tags defined");

        let tags = spec.tags.unwrap();
        assert!(
            !tags.is_empty(),
            "OpenAPI spec should have at least one tag"
        );

        // Check for expected tags
        let tag_names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
        assert!(
            tag_names.contains(&"downloads"),
            "Should have 'downloads' tag"
        );
        assert!(tag_names.contains(&"queue"), "Should have 'queue' tag");
        assert!(tag_names.contains(&"config"), "Should have 'config' tag");
        assert!(tag_names.contains(&"system"), "Should have 'system' tag");
    }

    #[test]
    fn test_openapi_spec_info() {
        let spec = ApiDoc::openapi();

        // Verify basic info
        assert_eq!(spec.info.title, "usenet-dl REST API");
        assert_eq!(spec.info.version, "0.1.0");
        assert!(spec.info.description.is_some());
    }

    #[test]
    fn test_openapi_spec_has_security_scheme() {
        let spec = ApiDoc::openapi();

        // Verify that security scheme is defined
        assert!(spec.components.is_some());
        let components = spec.components.unwrap();

        assert!(
            components.security_schemes.contains_key("api_key"),
            "Should have 'api_key' security scheme defined"
        );
    }

    #[test]
    fn test_openapi_json_serialization() {
        let spec = ApiDoc::openapi();

        // Test that the spec can be serialized to JSON
        let json = serde_json::to_string(&spec).expect("Should serialize to JSON");
        assert!(!json.is_empty(), "JSON output should not be empty");

        // Verify it's valid JSON
        let _value: serde_json::Value =
            serde_json::from_str(&json).expect("Generated JSON should be valid");
    }

    #[test]
    fn test_openapi_spec_version() {
        let spec = ApiDoc::openapi();

        // Verify OpenAPI version by serializing to JSON and checking the version field
        let json = serde_json::to_value(&spec).expect("Should serialize to JSON");
        let version = json.get("openapi").and_then(|v| v.as_str());
        assert!(version.is_some(), "Should have openapi version field");
        assert!(
            version.unwrap().starts_with("3."),
            "Should use OpenAPI 3.x version"
        );
    }
}
