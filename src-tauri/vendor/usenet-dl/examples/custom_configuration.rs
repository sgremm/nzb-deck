//! Custom configuration example
//!
//! This example shows how to configure usenet-dl with various options:
//! - Multiple NNTP servers with priorities
//! - Custom directories and concurrent downloads
//! - Speed limiting
//! - Retry configuration
//! - Post-processing settings
//! - Watch folders and RSS feeds
//! - Webhooks and scripts

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use usenet_dl::api::start_api_server;
use usenet_dl::config::{
    ApiConfig, AutomationConfig, CleanupConfig, Config, DeobfuscationConfig, DiskSpaceConfig,
    DownloadConfig, DuplicateAction, DuplicateConfig, DuplicateMethod, ExtractionConfig,
    FileCollisionAction, NotificationConfig, PersistenceConfig, PostProcess, ProcessingConfig,
    RetryConfig, RssFeedConfig, ScheduleAction, ScheduleRule, ScriptConfig, ScriptEvent,
    ServerConfig, ServerIntegrationConfig, ToolsConfig, WatchFolderAction, WatchFolderConfig,
    WebhookConfig, WebhookEvent, Weekday,
};
use usenet_dl::{Priority, UsenetDownloader};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing (optional)
    // Uncomment if you add tracing-subscriber to your dependencies:
    // tracing_subscriber::fmt::init();

    // Primary and backup servers
    let primary_server = ServerConfig {
        host: "news.primary.com".to_string(),
        port: 563,
        tls: true,
        username: Some("user".to_string()),
        password: Some("pass".to_string()),
        connections: 20,
        priority: 0, // Try first
        pipeline_depth: 10,
    };

    let backup_server = ServerConfig {
        host: "news.backup.com".to_string(),
        port: 563,
        tls: true,
        username: Some("user2".to_string()),
        password: Some("pass2".to_string()),
        connections: 10,
        priority: 1, // Try if primary fails
        pipeline_depth: 10,
    };

    // Retry configuration with exponential backoff
    let retry_config = RetryConfig {
        max_attempts: 5,
        initial_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(60),
        backoff_multiplier: 2.0,
        jitter: true,
    };

    // Extraction configuration
    let extraction_config = ExtractionConfig {
        max_recursion_depth: 2, // Extract archives within archives
        archive_extensions: vec![
            "rar".to_string(),
            "zip".to_string(),
            "7z".to_string(),
            "tar".to_string(),
            "gz".to_string(),
        ],
    };

    // Disk space checking
    let disk_space_config = DiskSpaceConfig {
        enabled: true,
        min_free_space: 5 * 1024 * 1024 * 1024, // 5 GB buffer
        size_multiplier: 2.5,                   // Account for extraction overhead
    };

    // Duplicate detection
    let duplicate_config = DuplicateConfig {
        enabled: true,
        action: DuplicateAction::Warn, // Warn but don't block
        methods: vec![DuplicateMethod::NzbHash, DuplicateMethod::JobName],
    };

    // Deobfuscation
    let deobfuscation_config = DeobfuscationConfig {
        enabled: true,
        min_length: 12,
    };

    // Watch folder for movies
    let movies_watch = WatchFolderConfig {
        path: PathBuf::from("/path/to/nzb/movies"),
        after_import: WatchFolderAction::MoveToProcessed,
        category: Some("movies".to_string()),
        scan_interval: Duration::from_secs(5),
    };

    // RSS feed for TV shows
    let tv_rss = RssFeedConfig {
        url: "https://indexer.example.com/rss?cat=tv".to_string(),
        check_interval: Duration::from_secs(900), // 15 minutes
        category: Some("tv".to_string()),
        filters: vec![],
        auto_download: true,
        priority: Priority::Normal,
        enabled: true,
    };

    // Schedule: unlimited speed at night
    let night_schedule = ScheduleRule {
        name: "Night unlimited".to_string(),
        days: vec![], // All days
        start_time: "00:00".to_string(),
        end_time: "06:00".to_string(),
        action: ScheduleAction::Unlimited,
        enabled: true,
    };

    // Schedule: limited during work hours
    let work_schedule = ScheduleRule {
        name: "Work hours limit".to_string(),
        days: vec![
            Weekday::Monday,
            Weekday::Tuesday,
            Weekday::Wednesday,
            Weekday::Thursday,
            Weekday::Friday,
        ],
        start_time: "09:00".to_string(),
        end_time: "17:00".to_string(),
        action: ScheduleAction::SpeedLimit {
            limit_bps: 1_000_000, // 1 MB/s
        },
        enabled: true,
    };

    // Webhook notification
    let webhook = WebhookConfig {
        url: "https://example.com/webhook".to_string(),
        events: vec![WebhookEvent::OnComplete, WebhookEvent::OnFailed],
        auth_header: Some("Bearer secret-token".to_string()),
        timeout: Duration::from_secs(30),
    };

    // Post-processing script
    let script = ScriptConfig {
        path: PathBuf::from("/path/to/post_process.sh"),
        events: vec![ScriptEvent::OnComplete],
        timeout: Duration::from_secs(300),
    };

    // Build complete configuration
    let config = Config {
        // Servers
        servers: vec![primary_server, backup_server],

        // Download behavior (directories, concurrency, post-processing)
        download: DownloadConfig {
            download_dir: PathBuf::from("/data/downloads"),
            temp_dir: PathBuf::from("/data/temp"),
            max_concurrent_downloads: 3,
            speed_limit_bps: None, // Controlled by scheduler
            default_post_process: PostProcess::UnpackAndCleanup,
            delete_samples: true,
            file_collision: FileCollisionAction::Rename,
            ..Default::default()
        },

        // External tools and passwords
        tools: ToolsConfig {
            password_file: Some(PathBuf::from("/etc/usenet-dl/passwords.txt")),
            try_empty_password: true,
            ..Default::default()
        },

        // Notifications (webhooks and scripts)
        notifications: NotificationConfig {
            webhooks: vec![webhook],
            scripts: vec![script],
        },

        // Persistence (database and schedules)
        persistence: PersistenceConfig {
            database_path: PathBuf::from("/data/usenet-dl.db"),
            schedule_rules: vec![night_schedule, work_schedule],
            ..Default::default()
        },

        // Processing (retry, extraction, duplicates, disk space)
        processing: ProcessingConfig {
            retry: retry_config,
            extraction: extraction_config,
            duplicate: duplicate_config,
            disk_space: disk_space_config,
            cleanup: CleanupConfig::default(),
            direct_unpack: Default::default(),
        },

        // Automation (watch folders, RSS, deobfuscation)
        automation: AutomationConfig {
            watch_folders: vec![movies_watch],
            rss_feeds: vec![tv_rss],
            deobfuscation: deobfuscation_config,
        },

        // Server integration (API)
        server: ServerIntegrationConfig {
            api: ApiConfig {
                bind_address: "0.0.0.0:6789".parse().unwrap(),
                api_key: Some("your-secret-key".to_string()),
                swagger_ui: true,
                ..Default::default()
            },
        },
    };

    println!("Configuration:");
    println!("  Servers: {}", config.servers.len());
    println!(
        "  Max concurrent: {}",
        config.download.max_concurrent_downloads
    );
    println!("  Watch folders: {}", config.automation.watch_folders.len());
    println!("  RSS feeds: {}", config.automation.rss_feeds.len());
    println!(
        "  Schedule rules: {}",
        config.persistence.schedule_rules.len()
    );
    println!("  API: {}", config.server.api.bind_address);

    // Create downloader with this configuration
    let downloader = Arc::new(UsenetDownloader::new(config.clone()).await?);
    let config_arc = Arc::new(config);

    println!("✓ Downloader initialized with custom configuration");
    println!("Starting API server...");

    // Start the API server
    start_api_server(downloader, config_arc).await?;

    Ok(())
}
