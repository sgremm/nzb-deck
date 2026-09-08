//! Basic download example
//!
//! This example demonstrates the core functionality of usenet-dl:
//! - Configuring NNTP servers
//! - Creating a downloader instance
//! - Subscribing to events
//! - Adding an NZB to the queue
//! - Monitoring download progress

use usenet_dl::config::{Config, DownloadConfig, ServerConfig};
use usenet_dl::{DownloadOptions, Event, Priority, UsenetDownloader};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for logging (optional)
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

    // Build configuration
    let config = Config {
        servers: vec![server],
        download: DownloadConfig {
            download_dir: "downloads".into(),
            temp_dir: "temp".into(),
            max_concurrent_downloads: 3,
            ..Default::default()
        },
        ..Default::default()
    };

    // Create downloader instance
    let downloader = UsenetDownloader::new(config).await?;

    // Subscribe to events
    let mut events = downloader.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            match event {
                Event::Queued { id, name } => {
                    println!("✓ Queued download #{}: {}", id, name);
                }
                Event::Downloading {
                    id,
                    percent,
                    speed_bps,
                    ..
                } => {
                    println!(
                        "⬇ Download #{}: {:.1}% @ {:.2} MB/s",
                        id,
                        percent,
                        speed_bps as f64 / 1_048_576.0
                    );
                }
                Event::DownloadComplete { id, .. } => {
                    println!("✓ Download #{} complete, starting post-processing", id);
                }
                Event::Extracting {
                    id,
                    archive,
                    percent,
                } => {
                    println!("📦 Extracting #{} ({}): {:.1}%", id, archive, percent);
                }
                Event::Complete { id, path } => {
                    println!("✓ Complete #{}: {:?}", id, path);
                }
                Event::Failed {
                    id,
                    stage,
                    error,
                    files_kept,
                } => {
                    println!(
                        "✗ Failed #{} at {:?}: {} (files kept: {})",
                        id, stage, error, files_kept
                    );
                }
                _ => {}
            }
        }
    });

    // Add NZB from file
    let download_id = downloader
        .add_nzb(
            "example.nzb".as_ref(),
            DownloadOptions {
                category: Some("movies".to_string()),
                priority: Priority::Normal,
                ..Default::default()
            },
        )
        .await?;

    println!("Added download with ID: {}", download_id);

    // Keep the program running
    // In a real application, you would:
    // 1. Start the API server: downloader.start_api_server().await?;
    // 2. Or keep monitoring events until all downloads complete
    // 3. Or integrate into your own async runtime

    // For this example, just sleep to keep event handlers running
    tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;

    Ok(())
}
