//! NZB file parsing, ingestion, duplicate detection, and disk space checks.

use crate::db;
use crate::error::{Error, Result};
use crate::types::{DownloadId, DownloadOptions, DuplicateInfo, Event, Status};
use crate::utils::extract_filename_from_response;

use super::UsenetDownloader;

/// SQLite has a limit of ~999 variables per query. With 6 columns per article,
/// we can insert at most 166 articles per batch (166 * 6 = 996 < 999).
const SQLITE_BATCH_SIZE: usize = 166;

/// Parse a filename from an NZB subject line.
///
/// Usenet subjects typically contain the filename in quotes, e.g.:
/// `Some.Movie.2024 [01/50] - "Some.Movie.2024.part01.rar" yEnc (1/100)`
///
/// Falls back to `file_{index}` if no quoted filename is found, but we return
/// just the parsed portion here — the caller provides a fallback index.
fn parse_filename_from_subject(subject: &str) -> String {
    // Look for the first quoted string in the subject
    if let Some(start) = subject.find('"')
        && let Some(end) = subject[start + 1..].find('"')
    {
        let filename = &subject[start + 1..start + 1 + end];
        if !filename.is_empty() {
            return filename.to_string();
        }
    }
    // Fallback: use a hash of the subject to create a unique name
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    subject.hash(&mut hasher);
    format!("file_{:x}", hasher.finish())
}

/// Timeout for HTTP requests when fetching NZB files from URLs.
const NZB_FETCH_TIMEOUT_SECS: u64 = 30;

impl UsenetDownloader {
    /// Add an NZB to the download queue from raw bytes
    ///
    /// This method parses the NZB content, creates a download record in the database,
    /// and emits a Queued event. The download will be processed by the queue processor,
    /// which will download articles in parallel using all configured NNTP connections.
    ///
    /// # Arguments
    ///
    /// * `content` - Raw NZB file content (XML)
    /// * `name` - Name for this download (typically the NZB filename without extension)
    /// * `options` - Download options (category, destination, priority, etc.)
    ///
    /// # Returns
    ///
    /// The unique DownloadId for this download
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - NZB content is invalid or cannot be parsed
    /// - NZB validation fails (missing segments, invalid structure)
    /// - Database insertion fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use usenet_dl::{UsenetDownloader, Config, DownloadOptions};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let downloader = UsenetDownloader::new(Config::default()).await?;
    ///
    ///     let nzb_content = std::fs::read("example.nzb")?;
    ///     let id = downloader.add_nzb_content(
    ///         &nzb_content,
    ///         "example",
    ///         DownloadOptions::default()
    ///     ).await?;
    ///
    ///     println!("Added download with ID: {}", id);
    ///     Ok(())
    /// }
    /// ```
    pub async fn add_nzb_content(
        &self,
        content: &[u8],
        name: &str,
        options: DownloadOptions,
    ) -> Result<DownloadId> {
        // Check if accepting new downloads (reject during shutdown)
        if !self
            .queue_state
            .accepting_new
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(Error::ShuttingDown);
        }

        // Parse and validate NZB, extract metadata
        let (nzb, nzb_meta_name, nzb_password, nzb_hash) =
            self.parse_and_validate_nzb(content, name).await?;

        // Check for duplicates before proceeding
        self.handle_duplicate_check(content, name).await?;

        // Determine destination directory and post-processing mode from category
        let (destination, post_process) = self.resolve_destination_and_post_process(&options).await;

        // Determine job name (for deobfuscation and duplicate detection)
        // Use NZB meta title if available, otherwise the provided name
        let job_name = nzb_meta_name.clone().unwrap_or_else(|| name.to_string());

        // Merge NZB password with provided password (provided takes priority)
        let final_password = options.password.clone().or(nzb_password);

        // Create and insert download record
        let download_id = self
            .create_download_record(
                name,
                &nzb,
                nzb_meta_name,
                nzb_hash,
                job_name,
                &options,
                destination,
                post_process,
            )
            .await?;

        // Insert all articles and cache password
        self.insert_articles_and_password(&nzb, download_id, final_password)
            .await?;

        // Emit events, trigger webhooks, and add to queue
        self.finalize_nzb_addition(download_id, name, &options)
            .await?;

        Ok(download_id)
    }

    /// Parse and validate NZB content, extract metadata
    ///
    /// Returns: (parsed NZB, meta name, password, hash)
    async fn parse_and_validate_nzb(
        &self,
        content: &[u8],
        _name: &str,
    ) -> Result<(nntp_rs::Nzb, Option<String>, Option<String>, String)> {
        // Parse NZB content from bytes to string
        let nzb_string = String::from_utf8(content.to_vec())
            .map_err(|e| Error::InvalidNzb(format!("NZB content is not valid UTF-8: {}", e)))?;

        // Parse NZB using nntp-rs
        let nzb = nntp_rs::parse_nzb(&nzb_string)
            .map_err(|e| Error::InvalidNzb(format!("Failed to parse NZB: {}", e)))?;

        // Validate NZB structure and segments
        nzb.validate()
            .map_err(|e| Error::InvalidNzb(format!("NZB validation failed: {}", e)))?;

        // Extract metadata from NZB
        let nzb_meta_name = nzb.meta.get("title").map(|s| s.to_string());
        let nzb_password = nzb.meta.get("password").map(|s| s.to_string());

        // Calculate total size
        let size_bytes = nzb.total_bytes() as i64;

        // Check if sufficient disk space is available
        self.check_disk_space(size_bytes).await?;

        // Calculate NZB hash for duplicate detection (sha256)
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content);
        let hash_result = hasher.finalize();
        let nzb_hash = format!("{:x}", hash_result);

        Ok((nzb, nzb_meta_name, nzb_password, nzb_hash))
    }

    /// Check for duplicates and handle according to configuration
    async fn handle_duplicate_check(&self, content: &[u8], name: &str) -> Result<()> {
        if let Some(dup_info) = self.check_duplicate(content, name).await {
            // Emit warning event about duplicate
            self.emit_event(Event::DuplicateDetected {
                id: dup_info.existing_id,
                name: name.to_string(),
                method: dup_info.method,
                existing_name: dup_info.existing_name.clone(),
            });

            // Handle based on configured action
            match self.config.processing.duplicate.action {
                crate::config::DuplicateAction::Block => {
                    return Err(Error::Duplicate(format!(
                        "Duplicate download detected: '{}' (method: {:?}, existing ID: {}, existing name: '{}')",
                        name, dup_info.method, dup_info.existing_id, dup_info.existing_name
                    )));
                }
                crate::config::DuplicateAction::Warn => {
                    // Already emitted warning event, continue with download
                }
                crate::config::DuplicateAction::Allow => {
                    // Silently allow, no event emitted (skip the emit above)
                    // Note: We already emitted the event above, but that's fine
                    // The event is informational in Allow mode
                }
            }
        }
        Ok(())
    }

    /// Determine destination directory and post-processing mode from category
    ///
    /// Returns: (destination, post_process)
    async fn resolve_destination_and_post_process(
        &self,
        options: &DownloadOptions,
    ) -> (std::path::PathBuf, crate::config::PostProcess) {
        // Hold the read lock once to get both values (more efficient than two separate lock acquisitions)
        if let Some(category) = &options.category {
            let categories = self.runtime_config.categories.read().await;
            if let Some(cat_config) = categories.get(category) {
                let dest = options
                    .destination
                    .clone()
                    .unwrap_or_else(|| cat_config.destination.clone());
                let pp = options.post_process.unwrap_or_else(|| {
                    cat_config
                        .post_process
                        .unwrap_or(self.config.download.default_post_process)
                });
                return (dest, pp);
            }
            // Category not found, fall through to defaults
        }
        // No category specified or category not found, use provided options or defaults
        let dest = options
            .destination
            .clone()
            .unwrap_or_else(|| self.config.download.download_dir.clone());
        let pp = options
            .post_process
            .unwrap_or(self.config.download.default_post_process);
        (dest, pp)
    }

    /// Create download record and insert into database
    #[allow(clippy::too_many_arguments)]
    async fn create_download_record(
        &self,
        name: &str,
        nzb: &nntp_rs::Nzb,
        nzb_meta_name: Option<String>,
        nzb_hash: String,
        job_name: String,
        options: &DownloadOptions,
        destination: std::path::PathBuf,
        post_process: crate::config::PostProcess,
    ) -> Result<DownloadId> {
        let new_download = db::NewDownload {
            name: name.to_string(),
            nzb_path: format!("memory:{}", name), // Stored in memory, not from file
            nzb_meta_name,
            nzb_hash: Some(nzb_hash),
            job_name: Some(job_name),
            category: options.category.clone(),
            destination: destination.to_string_lossy().into_owned(),
            post_process: post_process.to_i32(),
            priority: options.priority as i32,
            status: Status::Queued.to_i32(),
            size_bytes: nzb.total_bytes() as i64,
        };

        self.db.insert_download(&new_download).await
    }

    /// Insert all download files, articles (segments), and cache password if provided
    async fn insert_articles_and_password(
        &self,
        nzb: &nntp_rs::Nzb,
        download_id: DownloadId,
        password: Option<String>,
    ) -> Result<()> {
        // Build download_files rows — one per NZB file with parsed filename
        let download_files: Vec<db::NewDownloadFile> = nzb
            .files
            .iter()
            .enumerate()
            .map(|(file_idx, file)| {
                let filename = parse_filename_from_subject(&file.subject);
                db::NewDownloadFile {
                    download_id,
                    file_index: file_idx as i32,
                    filename,
                    subject: Some(file.subject.clone()),
                    total_segments: file.segments.len() as i32,
                }
            })
            .collect();
        self.db.insert_files_batch(&download_files).await?;

        // Insert all articles (segments) for resume support (batch insert for performance)
        let articles: Vec<db::NewArticle> = nzb
            .files
            .iter()
            .enumerate()
            .flat_map(|(file_idx, file)| {
                file.segments.iter().map(move |segment| db::NewArticle {
                    download_id,
                    message_id: segment.message_id.clone(),
                    segment_number: segment.number as i32,
                    file_index: file_idx as i32,
                    size_bytes: segment.bytes as i64,
                })
            })
            .collect();
        for chunk in articles.chunks(SQLITE_BATCH_SIZE) {
            self.db.insert_articles_batch(chunk).await?;
        }

        // Cache password if provided
        if let Some(password) = password {
            self.db.set_correct_password(download_id, &password).await?;
        }

        Ok(())
    }

    /// Emit events, trigger webhooks, and add to queue
    async fn finalize_nzb_addition(
        &self,
        download_id: DownloadId,
        name: &str,
        options: &DownloadOptions,
    ) -> Result<()> {
        // Emit Queued event
        self.emit_event(Event::Queued {
            id: download_id,
            name: name.to_string(),
        });

        // Trigger webhooks for queued event
        self.trigger_webhooks(super::webhooks::TriggerWebhooksParams {
            event_type: crate::config::WebhookEvent::OnQueued,
            download_id,
            name: name.to_string(),
            category: options.category.clone(),
            status: "queued".to_string(),
            destination: None,
            error: None,
        });

        // Add to priority queue for processing
        self.add_to_queue(download_id).await?;

        Ok(())
    }

    /// Add an NZB to the download queue from a file
    ///
    /// This is a convenience method that reads an NZB file from disk and delegates
    /// to `add_nzb_content()`. The filename (without extension) is used as the download name.
    pub async fn add_nzb(
        &self,
        path: &std::path::Path,
        options: DownloadOptions,
    ) -> Result<DownloadId> {
        // Read file content
        let content = tokio::fs::read(path).await.map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("Failed to read NZB file '{}': {}", path.display(), e),
            ))
        })?;

        // Extract filename without extension as download name
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Delegate to add_nzb_content
        self.add_nzb_content(&content, &name, options).await
    }

    /// Add NZB from URL
    ///
    /// This method fetches an NZB file from a given HTTP(S) URL and adds it to the queue.
    pub async fn add_nzb_url(&self, url: &str, options: DownloadOptions) -> Result<DownloadId> {
        // Create HTTP client with timeout to prevent hanging
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(NZB_FETCH_TIMEOUT_SECS))
            .build()
            .map_err(|e| {
                Error::Io(std::io::Error::other(format!(
                    "Failed to create HTTP client: {}",
                    e
                )))
            })?;

        // Fetch NZB from URL with timeout
        let response = client.get(url).send().await.map_err(|e| {
            let error_msg = if e.is_timeout() {
                format!(
                    "Timeout fetching NZB from URL '{}' (exceeded {} seconds)",
                    url, NZB_FETCH_TIMEOUT_SECS
                )
            } else if e.is_connect() {
                format!("Connection failed for URL '{}': {}", url, e)
            } else {
                format!("Failed to fetch NZB from URL '{}': {}", url, e)
            };
            Error::Io(std::io::Error::other(error_msg))
        })?;

        // Check HTTP status
        if !response.status().is_success() {
            return Err(Error::Io(std::io::Error::other(format!(
                "HTTP error fetching NZB: {} {}",
                response.status(),
                url
            ))));
        }

        // Extract filename from Content-Disposition header or URL
        let name = extract_filename_from_response(&response, url);

        // Read response body
        let content = response.bytes().await.map_err(|e| {
            Error::Io(std::io::Error::other(format!(
                "Failed to read response body from '{}': {}",
                url, e
            )))
        })?;

        // Delegate to add_nzb_content
        self.add_nzb_content(&content, &name, options).await
    }

    /// Mark an NZB file as processed in the database
    ///
    /// This is used by the folder watcher with WatchFolderAction::Keep to track
    /// which NZB files have already been processed to avoid re-adding them.
    pub async fn mark_nzb_processed(&self, path: &std::path::Path) -> Result<()> {
        self.db.mark_nzb_processed(path).await
    }

    /// Check if an NZB is a duplicate of an existing download
    ///
    /// This method checks for duplicates using the configured detection methods
    /// (NZB hash, NZB name, or job name). Returns information about the duplicate
    /// if found, or None if this is a new download.
    pub(crate) async fn check_duplicate(
        &self,
        nzb_content: &[u8],
        name: &str,
    ) -> Option<DuplicateInfo> {
        // Early return if duplicate detection is disabled
        if !self.config.processing.duplicate.enabled {
            return None;
        }

        // Check each configured detection method in order
        for method in &self.config.processing.duplicate.methods {
            match method {
                crate::config::DuplicateMethod::NzbHash => {
                    // Calculate SHA256 hash of NZB content
                    use sha2::{Digest, Sha256};
                    let mut hasher = Sha256::new();
                    hasher.update(nzb_content);
                    let hash_bytes = hasher.finalize();
                    let hash = format!("{:x}", hash_bytes);

                    // Check if this hash exists in database
                    if let Ok(Some(existing)) = self.db.find_by_nzb_hash(&hash).await {
                        return Some(DuplicateInfo {
                            method: *method,
                            existing_id: existing.id.into(),
                            existing_name: existing.name,
                        });
                    }
                }
                crate::config::DuplicateMethod::NzbName => {
                    // Check if download with this name already exists
                    if let Ok(Some(existing)) = self.db.find_by_name(name).await {
                        return Some(DuplicateInfo {
                            method: *method,
                            existing_id: existing.id.into(),
                            existing_name: existing.name,
                        });
                    }
                }
                crate::config::DuplicateMethod::JobName => {
                    // Extract job name from filename and check database
                    let job_name = Self::extract_job_name(name);
                    if let Ok(Some(existing)) = self.db.find_by_job_name(&job_name).await {
                        return Some(DuplicateInfo {
                            method: *method,
                            existing_id: existing.id.into(),
                            existing_name: existing.name,
                        });
                    }
                }
            }
        }

        None
    }

    /// Check if there is sufficient disk space for download
    ///
    /// This method checks if there is enough disk space available before starting
    /// a download. It accounts for:
    /// - The download size multiplied by a configurable multiplier (default 2.5x)
    ///   to account for extraction overhead (compressed + extracted + headroom)
    /// - A minimum free space buffer (default 1GB) to prevent filling the disk
    pub(crate) async fn check_disk_space(&self, size_bytes: i64) -> Result<()> {
        // Skip check if disabled
        if !self.config.processing.disk_space.enabled {
            return Ok(());
        }

        // Calculate required space: download size × multiplier + buffer
        let required =
            (size_bytes as f64 * self.config.processing.disk_space.size_multiplier) as u64;
        let required_with_buffer = required + self.config.processing.disk_space.min_free_space;

        // Determine path to check - use download_dir if it exists, otherwise check parent
        let check_path = if self.config.download.download_dir.exists() {
            &self.config.download.download_dir
        } else {
            // If download_dir doesn't exist yet, check parent directory
            // This allows checking space before creating the download directory
            self.config.download.download_dir.parent().ok_or_else(|| {
                Error::DiskSpaceCheckFailed(format!(
                    "Cannot determine parent directory of '{}'",
                    self.config.download.download_dir.display()
                ))
            })?
        };

        // Get available space from filesystem
        let available = crate::utils::get_available_space(check_path).map_err(|e| {
            Error::DiskSpaceCheckFailed(format!(
                "Failed to check disk space for '{}': {}",
                check_path.display(),
                e
            ))
        })?;

        // Check if sufficient space is available
        if available < required_with_buffer {
            return Err(Error::InsufficientSpace {
                required: required_with_buffer,
                available,
            });
        }

        Ok(())
    }

    /// Extract job name from NZB filename
    ///
    /// This removes the file extension and any obfuscation patterns to get
    /// a clean job name for duplicate detection.
    ///
    /// # Arguments
    ///
    /// * `name` - NZB filename or download name
    ///
    /// # Returns
    ///
    /// Extracted job name (filename stem)
    ///
    /// # Example
    ///
    /// ```
    /// # use usenet_dl::UsenetDownloader;
    /// let job_name = UsenetDownloader::extract_job_name("My.Movie.2024.nzb");
    /// assert_eq!(job_name, "My.Movie.2024");
    /// ```
    pub fn extract_job_name(name: &str) -> String {
        // Remove .nzb extension if present
        let name = name.strip_suffix(".nzb").unwrap_or(name);

        // For now, just return the cleaned name
        // Future enhancement: could apply deobfuscation logic here
        name.to_string()
    }
}
