# usenet-dl Examples

This directory contains runnable examples demonstrating various features of usenet-dl.

## Running Examples

All examples can be run using `cargo run --example <name>`:

```bash
# Run the basic download example
cargo run --example basic_download

# Run the REST API server example
cargo run --example rest_api_server

# Run the multi-subscriber example
cargo run --example multi_subscriber

# Run the custom configuration example
cargo run --example custom_configuration
```

## Available Examples

### 1. `basic_download.rs`

**What it demonstrates:**
- Basic configuration setup
- Creating a UsenetDownloader instance
- Subscribing to events
- Adding an NZB to the queue
- Monitoring download progress with event handling

**Key concepts:**
- ServerConfig setup
- Event subscription pattern
- DownloadOptions usage
- Event types (Queued, Downloading, Complete, Failed)

**Use when:** You want to understand the core workflow of usenet-dl.

---

### 2. `rest_api_server.rs`

**What it demonstrates:**
- Running usenet-dl as a REST API server
- API configuration
- Accessing Swagger UI
- Example curl commands for API usage

**Key concepts:**
- ApiConfig setup
- Swagger UI access
- CORS configuration
- API authentication (optional)

**Use when:** You want to control usenet-dl via HTTP REST API.

**After starting:**
- Swagger UI: http://localhost:6789/swagger-ui
- API docs: http://localhost:6789/api/v1
- Events stream: http://localhost:6789/api/v1/events

---

### 3. `multi_subscriber.rs`

**What it demonstrates:**
- Multiple independent event subscribers
- Different use cases for event handling
- Async task spawning for subscribers

**Key concepts:**
- Broadcast channel pattern
- Multiple consumers of the same event stream
- Filtering events by type
- Real-time statistics collection

**Subscriber types shown:**
1. **UI subscriber** - Updates progress bars
2. **Logging subscriber** - Logs all events
3. **Notification subscriber** - Sends alerts on completion/failure
4. **Statistics subscriber** - Collects metrics

**Use when:** Your application needs multiple independent components to react to download events.

---

### 4. `custom_configuration.rs`

**What it demonstrates:**
- Comprehensive configuration with all available options
- Multiple NNTP servers with priorities
- Speed limiting and scheduling
- Watch folders and RSS feeds
- Webhooks and post-processing scripts
- Retry configuration
- Extraction settings
- Duplicate detection

**Key concepts:**
- ServerConfig with priority (backup servers)
- RetryConfig with exponential backoff
- ExtractionConfig for nested archives
- DiskSpaceConfig for safety checks
- DuplicateConfig for preventing re-downloads
- WatchFolderConfig for auto-importing NZBs
- RssFeedConfig for auto-downloading from feeds
- ScheduleRule for time-based speed limits
- WebhookConfig and ScriptConfig for notifications
- ApiConfig with authentication

**Use when:** You need to see all available configuration options in one place.

---

## Configuration Notes

### NNTP Server Setup

Before running examples, update the server configuration with your NNTP credentials:

```rust
let server = ServerConfig {
    host: "news.example.com".to_string(),  // Your NNTP server
    port: 563,                              // SSL/TLS port (or 119 for plain)
    tls: true,                              // Use SSL/TLS
    username: Some("your_username".to_string()),
    password: Some("your_password".to_string()),
    connections: 10,                        // Connection pool size
    priority: 0,                            // Lower = tried first
    pipeline_depth: 10,                     // Pipelined NNTP commands
};
```

### Directories

The examples use default directories:
- `downloads/` - Final destination for completed downloads
- `temp/` - Temporary working directory during downloads
- `usenet-dl.db` - SQLite database for persistence

These are created automatically if they don't exist.

### NZB Files

To test the `basic_download.rs` example, you'll need an NZB file. Place it in the current directory and update the path:

```rust
let download_id = downloader
    .add_nzb(
        "your-file.nzb".as_ref(),  // Update this path
        DownloadOptions::default(),
    )
    .await?;
```

Alternatively, use `add_nzb_url()` to fetch from a URL:

```rust
let download_id = downloader
    .add_nzb_url(
        "https://example.com/file.nzb",
        DownloadOptions::default(),
    )
    .await?;
```

## Event Types Reference

Common events you'll encounter:

| Event | When | Fields |
|-------|------|--------|
| `Queued` | Download added to queue | `id`, `name` |
| `Downloading` | Download in progress | `id`, `percent`, `speed_bps`, `failed_articles`?, `total_articles`?, `health_percent`? |
| `DownloadComplete` | All articles downloaded | `id`, `articles_failed`?, `articles_total`? |
| `DownloadFailed` | Download failed | `id`, `error`, `articles_succeeded`?, `articles_failed`?, `articles_total`? |
| `Verifying` | PAR2 verification started | `id` |
| `VerifyComplete` | PAR2 verification done | `id`, `damaged` |
| `Repairing` | PAR2 repair in progress | `id`, `blocks_needed`, `blocks_available` |
| `RepairComplete` | PAR2 repair done | `id`, `success` |
| `RepairSkipped` | PAR2 repair skipped | `id`, `reason` |
| `Extracting` | Archive extraction in progress | `id`, `archive`, `percent` |
| `ExtractComplete` | Archive extraction done | `id` |
| `Complete` | Everything finished successfully | `id`, `path` |
| `Failed` | Post-processing failed | `id`, `stage`, `error`, `files_kept` |
| `DuplicateDetected` | Duplicate download detected | `id`, `name`, `method`, `existing_name` |
| `Shutdown` | System shutting down | (none) |

## Further Reading

- **README.md** - Project overview and quick start
- **tests/manual/api-testing.md** - REST API testing guide
- **tests/manual/server-testing.md** - Server health check testing
- **docs/architecture.md** - System design and architecture overview

## Need Help?

For more examples and use cases, see:
1. The test suite in `src/` directories
2. The `docs/test_api.sh` script for API usage examples
3. The README.md for additional code snippets
4. The Swagger UI at http://localhost:6789/swagger-ui when running the API server
