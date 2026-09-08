# usenet-dl

A high-performance, highly configurable backend library for building Usenet download applications in Rust.

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Rust Version: 1.93+](https://img.shields.io/badge/rust-1.93%2B-orange.svg)](https://www.rust-lang.org)

## Features

### Core Capabilities

- **Queue Management**: Priority-based download queue with pause/resume/cancel
- **Resume Support**: Article-level download tracking, survives crashes and restarts
- **Parallel Downloads**: Concurrent article fetching using all configured connections (~N× speedup with N connections)
- **Speed Limiting**: Global bandwidth control with token bucket algorithm
- **Retry Logic**: Exponential backoff with jitter for transient failures
- **Event System**: Real-time events via `tokio::broadcast` channels
- **Graceful Shutdown**: Signal handling with state preservation

### Post-Processing

- **DirectUnpack**: Extract RAR archives while downloads are still in progress (overlaps extraction with download time)
- **DirectRename**: Fix obfuscated filenames mid-download using PAR2 metadata
- **Archive Extraction**: RAR, 7z, and ZIP with password support
- **Nested Extraction**: Automatic recursive extraction (configurable depth)
- **Password Management**: Multi-source passwords (per-download, NZB metadata, global file, cache)
- **Deobfuscation**: Automatic detection and renaming of obfuscated filenames
- **File Collision Handling**: Rename, overwrite, or skip on conflicts
- **Smart Cleanup**: Remove .par2, .nzb, .sfv, sample folders, and archives after extraction

### REST API

- **OpenAPI 3.1 Compliant**: Full schema generation with utoipa
- **Swagger UI**: Interactive API documentation at `/swagger-ui`
- **Server-Sent Events**: Real-time updates via `/events` endpoint
- **37 Endpoints**: Complete CRUD for downloads, queue, history, config, categories, RSS, scheduler
- **Authentication**: Optional API key protection
- **CORS**: Configurable cross-origin support for frontend development
- **Rate Limiting**: Optional per-IP rate limiting (disabled by default)

### Automation

- **Folder Watching**: Auto-import NZB files from watched directories
- **URL Fetching**: Download NZBs directly from HTTP(S) URLs
- **RSS Feed Monitoring**: Automatic download with regex filters and scheduling
- **Time-Based Scheduler**: Speed limits and pause/resume based on time rules
- **Duplicate Detection**: Hash and name-based duplicate checking

### Notifications

- **Webhooks**: HTTP POST on download events (complete, failed, queued)
- **Script Execution**: Run external scripts with environment variables
- **Category Scripts**: Per-category script configuration
- **Disk Space Checks**: Pre-download validation with configurable buffer
- **Server Health Checks**: Test NNTP server connectivity and capabilities

## Design Philosophy

**usenet-dl is a library-first backend.** No CLI, no UI - just a solid Rust crate that frontend applications can embed.

- **Highly Configurable**: Almost every behavior can be customized
- **Sensible Defaults**: Works out of the box with minimal configuration
- **Event-Driven**: Subscribe to events, no polling required
- **Async Native**: Built on tokio for efficient concurrent operations

## Architecture

```
┌─────────────────────────────────────────┐
│            Your Application              │
├─────────────────────────────────────────┤
│              usenet-dl                  │
│   (Queue, Post-processing, API, DB)     │
├─────────────────────────────────────────┤
│              nntp-rs                    │
│   (NNTP, NZB parsing, yEnc, PAR2)      │
└─────────────────────────────────────────┘
```

### Responsibility Split

**usenet-dl handles:**
- Download queue management and persistence (SQLite)
- Post-processing pipeline (verify, repair, extract, rename, cleanup)
- Archive extraction (RAR/7z/ZIP) with password management
- File organization and collision handling
- REST API with OpenAPI documentation
- Event broadcasting to subscribers
- Folder watching and RSS feed monitoring
- Scheduler for time-based rules
- External notifications (webhooks, scripts)
- Disk space checking and health monitoring

**nntp-rs handles:**
- NNTP protocol implementation (RFC 3977)
- NZB file parsing
- yEnc decoding
- PAR2 verification and repair
- Connection pooling

## Installation

**Not yet published to crates.io**

Add as a path or git dependency:

```toml
[dependencies]
usenet-dl = { path = "../usenet-dl" }
# or
usenet-dl = { git = "https://github.com/jvz-devx/usenet-dl" }
```

### Requirements

- Rust 1.93 or later
- SQLite (embedded via sqlx)
- Optional: unrar command-line tool for RAR extraction
- Optional: 7z command-line tool for 7z extraction

## Quick Start

### Basic Usage

```rust
use usenet_dl::config::{Config, DownloadConfig, ServerConfig};
use usenet_dl::{UsenetDownloader, DownloadOptions, Event, Priority};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure NNTP server
    let config = Config {
        servers: vec![
            ServerConfig {
                host: "news.example.com".to_string(),
                port: 563,
                tls: true,
                username: Some("user".to_string()),
                password: Some("pass".to_string()),
                connections: 10,
                priority: 0,
                pipeline_depth: 10,
            }
        ],
        download: DownloadConfig {
            download_dir: "downloads".into(),
            temp_dir: "temp".into(),
            ..Default::default()
        },
        ..Default::default()
    };

    // Create downloader
    let downloader = UsenetDownloader::new(config).await?;

    // Subscribe to events
    let mut events = downloader.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            match event {
                Event::Downloading { id, percent, speed_bps, .. } => {
                    println!("Download {}: {:.1}% @ {} MB/s",
                        id, percent, speed_bps / 1_000_000);
                }
                Event::Complete { id, path } => {
                    println!("Download {} complete: {:?}", id, path);
                }
                Event::Failed { id, stage, error, .. } => {
                    eprintln!("Download {} failed at {:?}: {}", id, stage, error);
                }
                _ => {}
            }
        }
    });

    // Add NZB download
    let id = downloader.add_nzb(
        "file.nzb".as_ref(),
        DownloadOptions {
            category: Some("movies".into()),
            priority: Priority::Normal,
            ..Default::default()
        }
    ).await?;

    println!("Queued download: {}", id);

    // Control downloads
    downloader.pause(id).await?;
    downloader.resume(id).await?;
    downloader.set_speed_limit(Some(10_000_000)).await; // 10 MB/s

    Ok(())
}
```

### REST API

Start the API server:

```rust
use std::sync::Arc;
use usenet_dl::api::start_api_server;
use usenet_dl::config::{Config, DownloadConfig, ServerConfig};

let config = Config { /* ... */ };
let downloader = Arc::new(UsenetDownloader::new(config.clone()).await?);
let config = Arc::new(config);
start_api_server(downloader, config).await?;
```

The API will be available at `http://localhost:6789` with Swagger UI at `http://localhost:6789/swagger-ui`.

#### Example API Calls

```bash
# Add NZB from URL
curl -X POST http://localhost:6789/api/v1/downloads/url \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/file.nzb", "options": {"category": "movies"}}'

# List all downloads
curl http://localhost:6789/api/v1/downloads

# Get download status
curl http://localhost:6789/api/v1/downloads/1

# Pause download
curl -X POST http://localhost:6789/api/v1/downloads/1/pause

# Set speed limit to 10 MB/s
curl -X PUT http://localhost:6789/api/v1/config/speed-limit \
  -H "Content-Type: application/json" \
  -d '{"limit_bps": 10485760}'

# Stream real-time events
curl -N http://localhost:6789/api/v1/events
```

## Configuration

All settings have sensible defaults. Only NNTP server configuration is required.

### Default Settings

| Setting | Default | Rationale |
|---------|---------|-----------|
| **Download directory** | `./downloads` | Current directory, easy to find |
| **Temp directory** | `./temp` | Separate from final downloads |
| **Concurrent downloads** | 3 | Balanced throughput without overwhelming |
| **Speed limit** | Unlimited | Users expect full speed by default |
| **Post-processing** | Unpack + Cleanup | Most users want ready-to-use files |
| **DirectUnpack** | Disabled | Opt-in for advanced users |
| **Failed download action** | Keep files | Don't delete potentially recoverable data |
| **File collision** | Rename (add number) | Never lose data silently |
| **Nested extraction depth** | 2 levels | Handle common archive-in-archive |
| **Deobfuscation** | Enabled | Most users want readable names |
| **Duplicate detection** | Warn only | Alert but don't block |
| **Try empty password** | Yes | Common for public releases |
| **Delete samples** | Yes | Usually unwanted |
| **Disk space check** | Enabled, 1GB buffer | Prevent failed extractions |
| **Retry attempts** | 5 with exponential backoff | Resilient to transient failures |
| **Pipeline depth** | 10 articles per connection | Reduces round-trip latency overhead |
| **API bind address** | 127.0.0.1:6789 | Localhost only for security |
| **API authentication** | None | Easy local development |
| **CORS** | Enabled for all origins | Easy frontend development |
| **Swagger UI** | Enabled | Self-documenting API |
| **Rate limiting** | Disabled | Trust local network |

For a complete configuration example with all options, see [Configuration Guide](docs/configuration.md).

## Documentation

| Topic | Link |
|-------|------|
| Getting Started | [docs/getting-started.md](docs/getting-started.md) |
| Configuration | [docs/configuration.md](docs/configuration.md) |
| REST API Reference | [docs/api-reference.md](docs/api-reference.md) |
| Architecture | [docs/architecture.md](docs/architecture.md) |
| Post-Processing | [docs/post-processing.md](docs/post-processing.md) |
| Contributing | [docs/contributing.md](docs/contributing.md) |
| Manual Testing | [tests/manual/](tests/manual/) |
| API Documentation | Run `cargo doc --open` for inline Rustdoc |

Interactive API docs are available at `/swagger-ui` when the API server is running.

## Known Issues & Limitations

- **PAR2 repair not yet implemented in nntp-rs** - Verification works, repair planned
- **Archive extraction requires external tools** - unrar and 7z must be in PATH for RAR/7z support
- **No Windows testing yet** - Primarily developed and tested on Linux/macOS

## Contributing

Contributions are welcome! Please see [docs/contributing.md](docs/contributing.md) for development guidelines and workflow.

**Quick start:**

```bash
# Create feature branch
git checkout -b feature/my-feature

# Make changes and test
nix-shell --run "cargo test"
nix-shell --run "cargo clippy --all-targets"

# Commit with descriptive message
git commit -m "feat: Add my feature"

# Push and create PR
git push origin feature/my-feature
```

## License

Licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)

at your option.

## Acknowledgments

- The Rust community for excellent libraries and tooling
- Contributors to nntp-rs for NNTP protocol implementation

## Support

- **Issues**: https://github.com/jvz-devx/usenet-dl/issues
- **Discussions**: https://github.com/jvz-devx/usenet-dl/discussions
- **Documentation**: Run `cargo doc --open` or visit https://docs.rs/usenet-dl

---

Built with Rust
