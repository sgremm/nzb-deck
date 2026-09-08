//! Database lifecycle and schema migrations.

use crate::error::DatabaseError;
use crate::{Error, Result};
use sqlx::SqliteConnection;
use sqlx::sqlite::SqlitePool;
use std::path::Path;

use super::Database;

impl Database {
    /// Create a new database connection
    ///
    /// Creates the database file if it doesn't exist and runs migrations.
    pub async fn new(path: &Path) -> Result<Self> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                Error::Database(DatabaseError::ConnectionFailed(format!(
                    "Failed to create database directory: {}",
                    e
                )))
            })?;
        }

        // Connect to database with foreign key enforcement and WAL mode
        use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
        use std::str::FromStr;

        let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", path.display()))
            .map_err(|e| {
                Error::Database(DatabaseError::ConnectionFailed(format!(
                    "Failed to parse database path: {}",
                    e
                )))
            })?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal);

        let pool = SqlitePool::connect_with(options).await.map_err(|e| {
            Error::Database(DatabaseError::ConnectionFailed(format!(
                "Failed to connect to database: {}",
                e
            )))
        })?;

        let db = Self { pool };

        // Run migrations
        db.run_migrations().await?;

        Ok(db)
    }

    /// Run database migrations
    async fn run_migrations(&self) -> Result<()> {
        let mut conn = self.pool.acquire().await.map_err(|e| {
            Error::Database(DatabaseError::ConnectionFailed(format!(
                "Failed to acquire connection: {}",
                e
            )))
        })?;

        // Create schema version table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create schema_version table: {}",
                e
            )))
        })?;

        // Check current version
        let current_version: Option<i64> =
            sqlx::query_scalar("SELECT MAX(version) FROM schema_version")
                .fetch_optional(&mut *conn)
                .await
                .map_err(|e| {
                    Error::Database(DatabaseError::QueryFailed(format!(
                        "Failed to query schema version: {}",
                        e
                    )))
                })?;

        let current_version = current_version.unwrap_or(0);

        // Apply migrations
        if current_version < 1 {
            Self::migrate_v1(&mut conn).await?;
        }
        if current_version < 2 {
            Self::migrate_v2(&mut conn).await?;
        }
        if current_version < 3 {
            Self::migrate_v3(&mut conn).await?;
        }
        if current_version < 4 {
            Self::migrate_v4(&mut conn).await?;
        }
        if current_version < 5 {
            Self::migrate_v5(&mut conn).await?;
        }
        if current_version < 6 {
            Self::migrate_v6(&mut conn).await?;
        }
        if current_version < 7 {
            Self::migrate_v7(&mut conn).await?;
        }

        Ok(())
    }

    /// Migration v1: Create initial schema
    async fn migrate_v1(conn: &mut SqliteConnection) -> Result<()> {
        tracing::info!("Applying database migration v1");

        // Wrap migration in a transaction so partial failures don't leave the DB in a broken state
        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to begin transaction: {}",
                    e
                )))
            })?;

        let result = async {
            Self::create_downloads_schema(conn).await?;
            Self::create_articles_schema(conn).await?;
            Self::create_passwords_table(conn).await?;
            Self::create_processed_nzbs_table(conn).await?;
            Self::create_history_schema(conn).await?;
            Self::record_migration(conn, 1).await?;
            Ok::<(), Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| {
                        Error::Database(DatabaseError::MigrationFailed(format!(
                            "Failed to commit migration v1: {}",
                            e
                        )))
                    })?;
            }
            Err(e) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                return Err(e);
            }
        }

        tracing::info!("Database migration v1 complete");
        Ok(())
    }

    /// Create downloads table and its indexes
    async fn create_downloads_schema(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE downloads (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                nzb_path TEXT NOT NULL,
                nzb_meta_name TEXT,
                nzb_hash TEXT,
                job_name TEXT,
                category TEXT,
                destination TEXT NOT NULL,
                post_process INTEGER NOT NULL,
                priority INTEGER NOT NULL DEFAULT 0,
                status INTEGER NOT NULL DEFAULT 0,
                progress REAL DEFAULT 0.0,
                speed_bps INTEGER DEFAULT 0,
                size_bytes INTEGER DEFAULT 0,
                downloaded_bytes INTEGER DEFAULT 0,
                error_message TEXT,
                created_at INTEGER NOT NULL,
                started_at INTEGER,
                completed_at INTEGER
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create downloads table: {}",
                e
            )))
        })?;

        // Create indexes
        sqlx::query("CREATE INDEX idx_downloads_status ON downloads(status)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

        sqlx::query(
            "CREATE INDEX idx_downloads_priority ON downloads(priority DESC, created_at ASC)",
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create index: {}",
                e
            )))
        })?;

        sqlx::query("CREATE INDEX idx_downloads_nzb_hash ON downloads(nzb_hash)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

        sqlx::query("CREATE INDEX idx_downloads_job_name ON downloads(job_name)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Create download_articles table and its indexes
    async fn create_articles_schema(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE download_articles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
                message_id TEXT NOT NULL,
                segment_number INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL,
                status INTEGER NOT NULL DEFAULT 0,
                downloaded_at INTEGER,
                UNIQUE(download_id, message_id)
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create download_articles table: {}",
                e
            )))
        })?;

        // Create indexes
        sqlx::query("CREATE INDEX idx_articles_download ON download_articles(download_id)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

        sqlx::query("CREATE INDEX idx_articles_status ON download_articles(download_id, status)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Create passwords table
    async fn create_passwords_table(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE passwords (
                download_id INTEGER PRIMARY KEY REFERENCES downloads(id) ON DELETE CASCADE,
                correct_password TEXT NOT NULL
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create passwords table: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Create processed_nzbs table
    async fn create_processed_nzbs_table(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE processed_nzbs (
                path TEXT PRIMARY KEY,
                processed_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create processed_nzbs table: {}",
                e
            )))
        })?;

        Ok(())
    }

    /// Create history table and its index
    async fn create_history_schema(conn: &mut SqliteConnection) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                category TEXT,
                destination TEXT,
                status INTEGER NOT NULL,
                size_bytes INTEGER,
                download_time_secs INTEGER,
                completed_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&mut *conn)
        .await
        .map_err(|e| {
            Error::Database(DatabaseError::MigrationFailed(format!(
                "Failed to create history table: {}",
                e
            )))
        })?;

        // Create index
        sqlx::query("CREATE INDEX idx_history_completed ON history(completed_at DESC)")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Record a migration version
    async fn record_migration(conn: &mut SqliteConnection, version: i32) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query("INSERT INTO schema_version (version, applied_at) VALUES (?, ?)")
            .bind(version)
            .bind(now)
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to record migration: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Migration v2: Add runtime state table for shutdown tracking
    async fn migrate_v2(conn: &mut SqliteConnection) -> Result<()> {
        tracing::info!("Applying database migration v2");

        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to begin transaction: {}",
                    e
                )))
            })?;

        let result = async {
            // Runtime state table for tracking clean/unclean shutdown
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS runtime_state (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
                )
                "#,
            )
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create runtime_state table: {}",
                    e
                )))
            })?;

            // Initialize shutdown state as unclean (will be set to clean on proper startup)
            let now = chrono::Utc::now().timestamp();
            sqlx::query(
                r#"
                INSERT INTO runtime_state (key, value, updated_at)
                VALUES ('clean_shutdown', 'false', ?)
                "#,
            )
            .bind(now)
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to initialize runtime_state: {}",
                    e
                )))
            })?;

            // Record migration
            Self::record_migration(conn, 2).await?;
            Ok::<(), Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| {
                        Error::Database(DatabaseError::MigrationFailed(format!(
                            "Failed to commit migration v2: {}",
                            e
                        )))
                    })?;
            }
            Err(e) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                return Err(e);
            }
        }

        tracing::info!("Database migration v2 complete");
        Ok(())
    }

    /// Migration v3: Add RSS feed tables
    async fn migrate_v3(conn: &mut SqliteConnection) -> Result<()> {
        tracing::info!("Applying database migration v3");

        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to begin transaction: {}",
                    e
                )))
            })?;

        let result = async {
            // RSS feeds table
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS rss_feeds (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    url TEXT NOT NULL,
                    check_interval_secs INTEGER NOT NULL DEFAULT 900,
                    category TEXT,
                    auto_download INTEGER NOT NULL DEFAULT 1,
                    priority INTEGER NOT NULL DEFAULT 0,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    last_check INTEGER,
                    last_error TEXT,
                    created_at INTEGER NOT NULL
                )
                "#,
            )
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create rss_feeds table: {}",
                    e
                )))
            })?;

            // RSS filters table (per feed)
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS rss_filters (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    feed_id INTEGER NOT NULL REFERENCES rss_feeds(id) ON DELETE CASCADE,
                    name TEXT NOT NULL,
                    include_patterns TEXT,
                    exclude_patterns TEXT,
                    min_size INTEGER,
                    max_size INTEGER,
                    max_age_secs INTEGER
                )
                "#,
            )
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create rss_filters table: {}",
                    e
                )))
            })?;

            // RSS seen items table (prevent re-downloading)
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS rss_seen (
                    feed_id INTEGER NOT NULL REFERENCES rss_feeds(id) ON DELETE CASCADE,
                    guid TEXT NOT NULL,
                    seen_at INTEGER NOT NULL,
                    PRIMARY KEY (feed_id, guid)
                )
                "#,
            )
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create rss_seen table: {}",
                    e
                )))
            })?;

            // Record migration
            Self::record_migration(conn, 3).await?;
            Ok::<(), Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| {
                        Error::Database(DatabaseError::MigrationFailed(format!(
                            "Failed to commit migration v3: {}",
                            e
                        )))
                    })?;
            }
            Err(e) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                return Err(e);
            }
        }

        tracing::info!("Database migration v3 complete");
        Ok(())
    }

    /// Migration v4: Add download_files table and file_index column to download_articles
    async fn migrate_v4(conn: &mut SqliteConnection) -> Result<()> {
        tracing::info!("Applying database migration v4");

        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to begin transaction: {}",
                    e
                )))
            })?;

        let result = async {
            // File-level metadata table for NZB files
            sqlx::query(
                r#"
                CREATE TABLE download_files (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
                    file_index INTEGER NOT NULL,
                    filename TEXT NOT NULL,
                    subject TEXT,
                    total_segments INTEGER NOT NULL,
                    UNIQUE(download_id, file_index)
                )
                "#,
            )
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create download_files table: {}",
                    e
                )))
            })?;

            sqlx::query("CREATE INDEX idx_download_files_download ON download_files(download_id)")
                .execute(&mut *conn)
                .await
                .map_err(|e| {
                    Error::Database(DatabaseError::MigrationFailed(format!(
                        "Failed to create index: {}",
                        e
                    )))
                })?;

            // Add file_index column to existing download_articles table
            sqlx::query(
                "ALTER TABLE download_articles ADD COLUMN file_index INTEGER NOT NULL DEFAULT 0",
            )
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to add file_index column: {}",
                    e
                )))
            })?;

            // Record migration
            Self::record_migration(conn, 4).await?;
            Ok::<(), Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| {
                        Error::Database(DatabaseError::MigrationFailed(format!(
                            "Failed to commit migration v4: {}",
                            e
                        )))
                    })?;
            }
            Err(e) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                return Err(e);
            }
        }

        tracing::info!("Database migration v4 complete");
        Ok(())
    }

    /// Migration v5: Add DirectUnpack support columns
    async fn migrate_v5(conn: &mut SqliteConnection) -> Result<()> {
        tracing::info!("Applying database migration v5");

        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to begin transaction: {}",
                    e
                )))
            })?;

        let result = async {
            // Add direct_unpack_state to downloads table
            sqlx::query(
                "ALTER TABLE downloads ADD COLUMN direct_unpack_state INTEGER NOT NULL DEFAULT 0",
            )
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to add direct_unpack_state column: {}",
                    e
                )))
            })?;

            // Add completed flag to download_files table
            sqlx::query(
                "ALTER TABLE download_files ADD COLUMN completed INTEGER NOT NULL DEFAULT 0",
            )
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to add completed column: {}",
                    e
                )))
            })?;

            // Add original_filename to download_files table (for DirectRename)
            sqlx::query("ALTER TABLE download_files ADD COLUMN original_filename TEXT")
                .execute(&mut *conn)
                .await
                .map_err(|e| {
                    Error::Database(DatabaseError::MigrationFailed(format!(
                        "Failed to add original_filename column: {}",
                        e
                    )))
                })?;

            // Index for efficiently finding completed files per download
            sqlx::query(
                "CREATE INDEX idx_download_files_completed ON download_files(download_id, completed)",
            )
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create index: {}",
                    e
                )))
            })?;

            // Record migration
            Self::record_migration(conn, 5).await?;
            Ok::<(), Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| {
                        Error::Database(DatabaseError::MigrationFailed(format!(
                            "Failed to commit migration v5: {}",
                            e
                        )))
                    })?;
            }
            Err(e) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                return Err(e);
            }
        }

        tracing::info!("Database migration v5 complete");
        Ok(())
    }

    /// Migration v6: Add direct_unpack_extracted_count column
    async fn migrate_v6(conn: &mut SqliteConnection) -> Result<()> {
        tracing::info!("Applying database migration v6");

        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to begin transaction: {}",
                    e
                )))
            })?;

        let result = async {
            sqlx::query(
                "ALTER TABLE downloads ADD COLUMN direct_unpack_extracted_count INTEGER NOT NULL DEFAULT 0",
            )
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to add direct_unpack_extracted_count column: {}",
                    e
                )))
            })?;

            Self::record_migration(conn, 6).await?;
            Ok::<(), Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| {
                        Error::Database(DatabaseError::MigrationFailed(format!(
                            "Failed to commit migration v6: {}",
                            e
                        )))
                    })?;
            }
            Err(e) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                return Err(e);
            }
        }

        tracing::info!("Database migration v6 complete");
        Ok(())
    }

    /// Migration v7: Persist per-file paused state.
    async fn migrate_v7(conn: &mut SqliteConnection) -> Result<()> {
        tracing::info!("Applying database migration v7");

        sqlx::query("BEGIN")
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to begin transaction: {}",
                    e
                )))
            })?;

        let result = async {
            sqlx::query("ALTER TABLE download_files ADD COLUMN paused INTEGER NOT NULL DEFAULT 0")
                .execute(&mut *conn)
                .await
                .map_err(|e| {
                    Error::Database(DatabaseError::MigrationFailed(format!(
                        "Failed to add paused column: {}",
                        e
                    )))
                })?;

            sqlx::query(
                "CREATE INDEX idx_download_files_paused ON download_files(download_id, paused)",
            )
            .execute(&mut *conn)
            .await
            .map_err(|e| {
                Error::Database(DatabaseError::MigrationFailed(format!(
                    "Failed to create paused index: {}",
                    e
                )))
            })?;

            Self::record_migration(conn, 7).await?;
            Ok::<(), Error>(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT")
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| {
                        Error::Database(DatabaseError::MigrationFailed(format!(
                            "Failed to commit migration v7: {}",
                            e
                        )))
                    })?;
            }
            Err(e) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                return Err(e);
            }
        }

        tracing::info!("Database migration v7 complete");
        Ok(())
    }

    /// Close the database connection
    pub async fn close(self) {
        self.pool.close().await;
    }

    /// Get the underlying connection pool
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
