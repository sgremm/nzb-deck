//! Simple speedtest benchmark for usenet-dl
//!
//! Usage: cargo run --release --example speedtest

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::Notify;
use usenet_dl::config::{Config, DownloadConfig, PersistenceConfig, ServerConfig};
use usenet_dl::{DownloadOptions, Event, UsenetDownloader};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    // Get NZB path
    let nzb_path = std::env::var("TEST_NZB_PATH").expect("Set TEST_NZB_PATH to an NZB file path");

    // Load config from env
    let host = std::env::var("NNTP_HOST").expect("NNTP_HOST not set");
    let port: u16 = std::env::var("NNTP_PORT_SSL")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(563);
    let username = std::env::var("NNTP_USERNAME").expect("NNTP_USERNAME not set");
    let password = std::env::var("NNTP_PASSWORD").expect("NNTP_PASSWORD not set");
    let connections: usize = std::env::var("NNTP_CONNECTIONS")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(50);

    println!("═══════════════════════════════════════════════════════════");
    println!("  usenet-dl Speedtest");
    println!("═══════════════════════════════════════════════════════════");
    println!("  Server: {}:{}", host, port);
    println!("  Connections: {}", connections);
    println!("  NZB: {}", nzb_path);
    println!("═══════════════════════════════════════════════════════════");

    // Create temp directory
    let temp_dir = tempfile::tempdir()?;
    let download_dir = temp_dir.path().join("downloads");
    std::fs::create_dir_all(&download_dir)?;

    let config = Config {
        servers: vec![ServerConfig {
            host,
            port,
            tls: true,
            username: Some(username),
            password: Some(password),
            connections,
            priority: 0,
            pipeline_depth: 10,
        }],
        persistence: PersistenceConfig {
            database_path: temp_dir.path().join("test.db"),
            ..Default::default()
        },
        download: DownloadConfig {
            download_dir: download_dir.clone(),
            temp_dir: temp_dir.path().join("temp"),
            max_concurrent_downloads: 1,
            ..Default::default()
        },
        ..Default::default()
    };

    let downloader = Arc::new(UsenetDownloader::new(config).await?);

    // Read NZB content
    let nzb_content = std::fs::read(&nzb_path)?;
    let name = std::path::Path::new(&nzb_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());

    // Add NZB
    let id = downloader
        .add_nzb_content(&nzb_content, &name, DownloadOptions::default())
        .await?;

    println!("\nDownload ID: {}", id);

    // Track progress via events
    let done = Arc::new(Notify::new());
    let done_clone = done.clone();
    let max_speed = Arc::new(AtomicU64::new(0));
    let max_speed_clone = max_speed.clone();

    let min_speed = Arc::new(AtomicU64::new(u64::MAX));
    let min_speed_clone = min_speed.clone();
    let event_count = Arc::new(AtomicU64::new(0));
    let event_count_clone = event_count.clone();

    let mut events = downloader.subscribe();
    let progress_task = tokio::spawn(async move {
        let mut last_percent = -1.0_f32;
        let mut speed_samples: Vec<u64> = Vec::new();
        let mut stall_count: u64 = 0;
        let mut last_speed: u64 = 0;
        let pipeline_start = Instant::now();
        let mut download_elapsed = std::time::Duration::ZERO;
        let mut stage_start = Instant::now();
        let mut printed_speed_stats = false;

        loop {
            match events.recv().await {
                Ok(Event::Downloading {
                    id: _,
                    percent,
                    speed_bps,
                    failed_articles: failed,
                    health_percent: health,
                    ..
                }) => {
                    event_count_clone.fetch_add(1, Ordering::Relaxed);
                    max_speed_clone.fetch_max(speed_bps, Ordering::SeqCst);
                    if speed_bps > 0 {
                        min_speed_clone.fetch_min(speed_bps, Ordering::SeqCst);
                        speed_samples.push(speed_bps);
                    }

                    // Detect stalls (speed dropped >80% from last)
                    if last_speed > 0 && speed_bps < last_speed / 5 {
                        stall_count += 1;
                        let speed_mbps = speed_bps as f64 / 1_000_000.0;
                        eprintln!(
                            "  [STALL #{stall_count}] Speed dropped to {speed_mbps:.2} MB/s at {percent:.1}%"
                        );
                    }
                    last_speed = speed_bps;

                    if (percent - last_percent).abs() >= 5.0 || last_percent < 0.0 {
                        let speed_mbps = speed_bps as f64 / 1_000_000.0;
                        let secs = pipeline_start.elapsed().as_secs();
                        let health_str = match (health, failed) {
                            (Some(h), Some(f)) => format!("  health={h:.0}% ({f} failed)"),
                            _ => String::new(),
                        };
                        println!(
                            "  [{secs:>4}s] {:5.1}%  {:7.2} MB/s{health_str}",
                            percent, speed_mbps
                        );
                        last_percent = percent;
                    }
                }
                Ok(Event::Queued { id, name }) => {
                    println!("  [QUEUED] #{id}: {name}");
                }
                Ok(Event::DownloadComplete {
                    id,
                    articles_failed: dl_failed,
                    articles_total: dl_total,
                }) => {
                    download_elapsed = pipeline_start.elapsed();
                    let fail_info = match (dl_failed, dl_total) {
                        (Some(f), Some(t)) if f > 0 => format!(" ({f} of {t} articles failed)"),
                        _ => String::new(),
                    };
                    println!(
                        "\n  [DOWNLOAD COMPLETE] #{id} in {:.1}s{fail_info}",
                        download_elapsed.as_secs_f64()
                    );
                    // Print speed stats once
                    if !printed_speed_stats && !speed_samples.is_empty() {
                        printed_speed_stats = true;
                        let mut sorted = speed_samples.clone();
                        sorted.sort();
                        let p50 = sorted[sorted.len() / 2] as f64 / 1_000_000.0;
                        let p95 = sorted[sorted.len() * 95 / 100] as f64 / 1_000_000.0;
                        let p5 = sorted[sorted.len() * 5 / 100] as f64 / 1_000_000.0;
                        println!(
                            "  Speed percentiles: P5={p5:.1} P50={p50:.1} P95={p95:.1} MB/s  Stalls: {stall_count}"
                        );
                    }
                    println!("\n  --- Post-Processing Pipeline ---");
                    stage_start = Instant::now();
                }
                Ok(Event::DownloadFailed {
                    id,
                    error,
                    articles_succeeded: a_ok,
                    articles_failed: a_fail,
                    articles_total: a_total,
                }) => {
                    let stats = match (a_ok, a_fail, a_total) {
                        (Some(ok), Some(fail), Some(total)) => {
                            format!(" ({ok} ok, {fail} failed of {total})")
                        }
                        _ => String::new(),
                    };
                    eprintln!("  [DOWNLOAD FAILED] #{id}: {error}{stats}");
                    done_clone.notify_one();
                    break;
                }
                Ok(Event::Verifying { id }) => {
                    println!("  [VERIFY] #{id} Checking PAR2...");
                    stage_start = Instant::now();
                }
                Ok(Event::VerifyComplete { id, damaged }) => {
                    let dt = stage_start.elapsed().as_secs_f64();
                    println!("  [VERIFY DONE] #{id} damaged={damaged} ({dt:.1}s)");
                }
                Ok(Event::Repairing {
                    id,
                    blocks_needed,
                    blocks_available,
                }) => {
                    println!("  [REPAIR] #{id} need={blocks_needed} avail={blocks_available}");
                    stage_start = Instant::now();
                }
                Ok(Event::RepairComplete { id, success }) => {
                    let dt = stage_start.elapsed().as_secs_f64();
                    println!("  [REPAIR DONE] #{id} success={success} ({dt:.1}s)");
                }
                Ok(Event::RepairSkipped { id, reason }) => {
                    println!("  [REPAIR SKIP] #{id} {reason}");
                }
                Ok(Event::Extracting {
                    id,
                    archive,
                    percent,
                }) => {
                    if percent < 1.0 {
                        stage_start = Instant::now();
                    }
                    let dt = stage_start.elapsed().as_secs_f64();
                    println!("  [EXTRACT] #{id} {archive} {percent:.1}% ({dt:.1}s)");
                }
                Ok(Event::ExtractComplete { id }) => {
                    let dt = stage_start.elapsed().as_secs_f64();
                    println!("  [EXTRACT DONE] #{id} ({dt:.1}s)");
                }
                Ok(Event::Moving { id, destination }) => {
                    println!("  [MOVE] #{id} -> {destination:?}");
                    stage_start = Instant::now();
                }
                Ok(Event::Cleaning { id }) => {
                    println!("  [CLEAN] #{id}");
                }
                Ok(Event::Complete { id, path }) => {
                    let total = pipeline_start.elapsed().as_secs_f64();
                    let pp = total - download_elapsed.as_secs_f64();
                    println!("\n  [COMPLETE] #{id} -> {path:?}");
                    println!(
                        "  Total pipeline: {total:.1}s (download: {:.1}s + post-process: {pp:.1}s)",
                        download_elapsed.as_secs_f64()
                    );
                    done_clone.notify_one();
                    break;
                }
                Ok(Event::Failed {
                    id,
                    stage,
                    error,
                    files_kept,
                }) => {
                    let total = pipeline_start.elapsed().as_secs_f64();
                    eprintln!(
                        "  [FAILED] #{id} at {stage:?}: {error} (files kept: {files_kept}) after {total:.1}s"
                    );
                    done_clone.notify_one();
                    break;
                }
                Ok(Event::DuplicateDetected {
                    id,
                    name,
                    method,
                    existing_name,
                }) => {
                    eprintln!(
                        "  [DUPLICATE] #{id} '{name}' matches '{existing_name}' via {method:?}"
                    );
                }
                Ok(Event::SpeedLimitChanged { limit_bps }) => {
                    println!("  [SPEED LIMIT] {:?}", limit_bps);
                }
                Ok(Event::QueuePaused) => println!("  [PAUSED]"),
                Ok(Event::QueueResumed) => println!("  [RESUMED]"),
                Ok(Event::Shutdown) => {
                    println!("  [SHUTDOWN]");
                    break;
                }
                Ok(other) => {
                    println!("  [EVENT] {other:?}");
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("  [WARNING] Event receiver lagged, missed {n} events!");
                }
                Err(_) => break,
            }
        }
    });

    // Start download and measure time
    let start = Instant::now();
    let _processor = downloader.start_queue_processor();

    // Wait for completion with timeout
    tokio::select! {
        _ = done.notified() => {}
        _ = tokio::time::sleep(std::time::Duration::from_secs(1800)) => {
            eprintln!("Timeout!");
        }
    }

    let elapsed = start.elapsed();
    progress_task.abort();

    // Calculate results from disk usage (check both downloads and temp)
    let total_disk_bytes: u64 = walkdir::WalkDir::new(temp_dir.path())
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();

    let secs = elapsed.as_secs_f64();
    let speed_mbps = if secs > 0.0 {
        total_disk_bytes as f64 / secs / 1_000_000.0
    } else {
        0.0
    };

    let peak_speed = max_speed.load(Ordering::SeqCst) as f64 / 1_000_000.0;
    let min_spd = min_speed.load(Ordering::SeqCst);
    let min_speed_val = if min_spd == u64::MAX {
        0.0
    } else {
        min_spd as f64 / 1_000_000.0
    };
    let events_total = event_count.load(Ordering::Relaxed);

    println!("\n═══════════════════════════════════════════════════════════");
    println!("  Results");
    println!("═══════════════════════════════════════════════════════════");
    println!("  Time:        {:.2} seconds", secs);
    println!(
        "  On disk:     {:.2} MB ({:.2} GB)",
        total_disk_bytes as f64 / 1_000_000.0,
        total_disk_bytes as f64 / 1_000_000_000.0
    );
    println!("  Avg Speed:   {:.2} MB/s", speed_mbps);
    println!("  Peak Speed:  {:.2} MB/s", peak_speed);
    println!("  Min Speed:   {:.2} MB/s", min_speed_val);
    println!("  Events:      {}", events_total);
    println!("═══════════════════════════════════════════════════════════");

    downloader.shutdown().await.ok();
    Ok(())
}
