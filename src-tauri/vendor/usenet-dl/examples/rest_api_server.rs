//! REST API server example
//!
//! This example shows how to run usenet-dl with the REST API enabled,
//! allowing control via HTTP endpoints.
//!
//! After starting, you can:
//! - View Swagger UI at http://localhost:6789/swagger-ui
//! - Add downloads via POST http://localhost:6789/api/v1/downloads/url
//! - Monitor progress via GET http://localhost:6789/api/v1/downloads
//! - Stream events via GET http://localhost:6789/api/v1/events

use std::net::SocketAddr;
use std::sync::Arc;
use usenet_dl::UsenetDownloader;
use usenet_dl::api::start_api_server;
use usenet_dl::config::{ApiConfig, Config, DownloadConfig, ServerConfig, ServerIntegrationConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing (optional)
    // Uncomment if you add tracing-subscriber to your dependencies:
    // tracing_subscriber::fmt::init();

    // Configure NNTP server
    let server = ServerConfig {
        host: "news.example.com".to_string(),
        port: 563,
        tls: true,
        username: Some("your_username".to_string()),
        password: Some("your_password".to_string()),
        connections: 10,
        priority: 0,
        pipeline_depth: 10,
    };

    // Configure API
    let api_config = ApiConfig {
        bind_address: "127.0.0.1:6789".parse::<SocketAddr>().unwrap(),
        api_key: None, // No authentication for local use
        cors_enabled: true,
        cors_origins: vec!["*".to_string()],
        swagger_ui: true,
        ..Default::default()
    };

    // Build configuration
    let config = Config {
        servers: vec![server],
        download: DownloadConfig {
            download_dir: "downloads".into(),
            temp_dir: "temp".into(),
            ..Default::default()
        },
        server: ServerIntegrationConfig { api: api_config },
        ..Default::default()
    };

    // Create downloader instance
    let downloader = Arc::new(UsenetDownloader::new(config.clone()).await?);
    let config_arc = Arc::new(config);

    println!("🚀 Starting usenet-dl REST API server");
    println!("📖 Swagger UI: http://localhost:6789/swagger-ui");
    println!("📡 API Base: http://localhost:6789/api/v1");
    println!("🔄 Events stream: http://localhost:6789/api/v1/events");
    println!();
    println!("Example commands:");
    println!("  # Add download from URL");
    println!("  curl -X POST http://localhost:6789/api/v1/downloads/url \\");
    println!("    -H 'Content-Type: application/json' \\");
    println!(
        "    -d '{{\"url\": \"https://example.com/file.nzb\", \"options\": {{\"category\": \"movies\"}}}}'"
    );
    println!();
    println!("  # List all downloads");
    println!("  curl http://localhost:6789/api/v1/downloads");
    println!();
    println!("  # Stream events (Server-Sent Events)");
    println!("  curl -N http://localhost:6789/api/v1/events");

    // Start the API server (runs indefinitely)
    start_api_server(downloader, config_arc).await?;

    Ok(())
}
