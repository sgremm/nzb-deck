//! Multiple event subscribers example
//!
//! This example demonstrates how multiple parts of your application
//! can independently subscribe to download events.

use usenet_dl::config::{Config, ServerConfig};
use usenet_dl::{Event, UsenetDownloader};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing (optional)
    // Uncomment if you add tracing-subscriber to your dependencies:
    // tracing_subscriber::fmt::init();

    // Configure server
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

    let config = Config {
        servers: vec![server],
        ..Default::default()
    };

    let downloader = UsenetDownloader::new(config).await?;

    // UI subscriber - only cares about progress updates
    let mut ui_events = downloader.subscribe();
    tokio::spawn(async move {
        println!("[UI] Starting UI event subscriber");
        while let Ok(event) = ui_events.recv().await {
            match event {
                Event::Downloading {
                    id,
                    percent,
                    speed_bps,
                    ..
                } => {
                    // Update progress bar
                    println!(
                        "[UI] Download {} progress: {:.1}% @ {:.2} MB/s",
                        id,
                        percent,
                        speed_bps as f64 / 1_048_576.0
                    );
                }
                Event::Extracting { id, percent, .. } => {
                    println!("[UI] Extraction {} progress: {:.1}%", id, percent);
                }
                Event::Complete { id, path } => {
                    println!("[UI] Download {} complete: {:?}", id, path);
                }
                _ => {}
            }
        }
    });

    // Logging subscriber - logs everything
    let mut log_events = downloader.subscribe();
    tokio::spawn(async move {
        println!("[LOG] Starting logging subscriber");
        while let Ok(event) = log_events.recv().await {
            println!("[LOG] Event: {:?}", event);
        }
    });

    // Notification subscriber - only cares about completion/failure
    let mut notification_events = downloader.subscribe();
    tokio::spawn(async move {
        println!("[NOTIFY] Starting notification subscriber");
        while let Ok(event) = notification_events.recv().await {
            match event {
                Event::Complete { id, path } => {
                    println!("[NOTIFY] Sending success notification for download {}", id);
                    // Send push notification, email, webhook, etc.
                    println!("[NOTIFY] Download complete: {:?}", path);
                }
                Event::Failed { id, error, .. } => {
                    println!("[NOTIFY] Sending failure notification for download {}", id);
                    println!("[NOTIFY] Error: {}", error);
                }
                _ => {}
            }
        }
    });

    // Statistics subscriber - collects metrics
    let mut stats_events = downloader.subscribe();
    tokio::spawn(async move {
        println!("[STATS] Starting statistics collector");
        let mut _total_downloaded: u64 = 0;
        let mut completed_count: u32 = 0;
        let mut failed_count: u32 = 0;

        while let Ok(event) = stats_events.recv().await {
            match event {
                Event::DownloadComplete { .. } => {
                    completed_count += 1;
                    println!(
                        "[STATS] Total completed: {} (failed: {})",
                        completed_count, failed_count
                    );
                }
                Event::Failed { .. } => {
                    failed_count += 1;
                }
                Event::Downloading { speed_bps, .. } => {
                    _total_downloaded += speed_bps;
                }
                _ => {}
            }
        }
    });

    println!("✓ All subscribers started");
    println!("Starting downloader...");

    // Keep the program running to process events
    // In a real application, start the API server or integrate into your runtime
    tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;

    Ok(())
}
