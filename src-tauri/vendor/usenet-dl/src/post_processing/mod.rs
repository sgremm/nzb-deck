//! Post-processing pipeline for completed downloads
//!
//! This module handles the post-processing pipeline after articles are downloaded:
//! 1. Verify - PAR2 verification
//! 2. Repair - PAR2 repair (if verification fails)
//! 3. Extract - Archive extraction (RAR, 7z, ZIP)
//! 4. Move - Move files to final destination
//! 5. Cleanup - Remove intermediate files (.par2, .nzb, archives, samples)

use crate::config::{Config, PostProcess};
use crate::error::{PostProcessError, Result};
use crate::parity::ParityHandler;
use crate::types::{DownloadId, Event};
use crate::utils::get_unique_path;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

mod cleanup;
mod repair;
mod verify;

// Re-export stages for internal use
use cleanup::run_cleanup_stage;
use repair::run_repair_stage;
use verify::run_verify_stage;

/// Post-processing pipeline executor
pub struct PostProcessor {
    /// Event channel for emitting pipeline events
    event_tx: broadcast::Sender<Event>,
    /// Configuration for file collision handling
    config: Arc<Config>,
    /// PAR2 parity handler for verification and repair
    parity_handler: Arc<dyn ParityHandler>,
    /// Database for password caching during extraction
    db: Arc<crate::db::Database>,
}

impl PostProcessor {
    /// Create a new post-processing pipeline executor
    pub fn new(
        event_tx: broadcast::Sender<Event>,
        config: Arc<Config>,
        parity_handler: Arc<dyn ParityHandler>,
        db: Arc<crate::db::Database>,
    ) -> Self {
        Self {
            event_tx,
            config,
            parity_handler,
            db,
        }
    }

    /// Execute post-processing pipeline for a completed download
    ///
    /// This is the main entry point for post-processing. It orchestrates
    /// the pipeline stages based on the configured PostProcess mode.
    ///
    /// # Arguments
    ///
    /// * `download_id` - The download to post-process
    /// * `download_path` - Path to the downloaded files
    /// * `post_process` - Post-processing mode to use
    /// * `destination` - Final destination for files
    ///
    /// # Returns
    ///
    /// Returns Ok(final_path) on success, Err on failure
    pub async fn start_post_processing(
        &self,
        download_id: DownloadId,
        download_path: PathBuf,
        post_process: PostProcess,
        destination: PathBuf,
    ) -> Result<PathBuf> {
        info!(
            download_id = download_id.0,
            ?post_process,
            ?download_path,
            ?destination,
            "starting post-processing pipeline"
        );

        // Execute pipeline stages based on post-processing mode
        match post_process {
            PostProcess::None => {
                // No post-processing, just return the download path
                debug!(
                    download_id = download_id.0,
                    "skipping post-processing (mode: None)"
                );
                Ok(download_path)
            }

            PostProcess::Verify => {
                // Just verify
                run_verify_stage(
                    download_id,
                    &download_path,
                    &self.event_tx,
                    &*self.parity_handler,
                )
                .await?;
                Ok(download_path)
            }

            PostProcess::Repair => {
                // Verify and repair if needed
                let damaged = run_verify_stage(
                    download_id,
                    &download_path,
                    &self.event_tx,
                    &*self.parity_handler,
                )
                .await?;
                if damaged {
                    run_repair_stage(
                        download_id,
                        &download_path,
                        &self.event_tx,
                        &*self.parity_handler,
                    )
                    .await?;
                }
                Ok(download_path)
            }

            PostProcess::Unpack => {
                // Verify, repair, and extract
                let damaged = run_verify_stage(
                    download_id,
                    &download_path,
                    &self.event_tx,
                    &*self.parity_handler,
                )
                .await?;
                if damaged {
                    run_repair_stage(
                        download_id,
                        &download_path,
                        &self.event_tx,
                        &*self.parity_handler,
                    )
                    .await?;
                }
                let extracted_path = self.run_extract_stage(download_id, &download_path).await?;
                Ok(extracted_path)
            }

            PostProcess::UnpackAndCleanup => {
                // Full pipeline: verify, repair, extract, move, cleanup
                let damaged = run_verify_stage(
                    download_id,
                    &download_path,
                    &self.event_tx,
                    &*self.parity_handler,
                )
                .await?;
                if damaged {
                    run_repair_stage(
                        download_id,
                        &download_path,
                        &self.event_tx,
                        &*self.parity_handler,
                    )
                    .await?;
                }
                let extracted_path = self.run_extract_stage(download_id, &download_path).await?;
                let final_path = self
                    .run_move_stage(download_id, &extracted_path, &destination)
                    .await?;
                run_cleanup_stage(download_id, &download_path, &self.event_tx, &self.config)
                    .await?;
                // PAR2 data has done its job (verify/repair succeeded). Only reachable
                // on success, so the recovery files can be removed from the result.
                self.remove_par2_from_result(download_id, &final_path).await;
                Ok(final_path)
            }
        }
    }

    /// Run only move and cleanup stages (skip verify/repair/extract).
    ///
    /// Used when DirectUnpack has already extracted archives during download.
    /// The extracted files are expected to be in `download_path/extracted`.
    pub async fn run_move_and_cleanup(
        &self,
        download_id: DownloadId,
        download_path: PathBuf,
        destination: PathBuf,
    ) -> Result<PathBuf> {
        info!(
            download_id = download_id.0,
            ?download_path,
            ?destination,
            "running move+cleanup only (DirectUnpack completed)"
        );

        // The extracted files should be in the same location the extract stage uses
        let extracted_path = download_path.join("extracted");
        let source = if extracted_path.is_dir() {
            extracted_path
        } else {
            // No extracted subdirectory — use download_path directly
            download_path.clone()
        };

        let final_path = self
            .run_move_stage(download_id, &source, &destination)
            .await?;
        run_cleanup_stage(download_id, &download_path, &self.event_tx, &self.config).await?;
        self.remove_par2_from_result(download_id, &final_path).await;
        Ok(final_path)
    }

    /// Remove PAR2 recovery files from a successfully processed result folder.
    ///
    /// Verification and repair already completed at this point, so the recovery
    /// files no longer serve a purpose. Files are only removed when the folder
    /// still contains real content; a download consisting solely of PAR2 data is
    /// left untouched. Failures are logged, never fatal.
    async fn remove_par2_from_result(&self, download_id: DownloadId, folder: &Path) {
        let Ok(mut entries) = tokio::fs::read_dir(folder).await else {
            return;
        };
        let mut par2_files = Vec::new();
        let mut content_files = 0usize;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let is_par2 = entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("par2"));
            if is_par2 {
                par2_files.push(entry.path());
            } else {
                content_files += 1;
            }
        }

        if par2_files.is_empty() || content_files == 0 {
            return;
        }

        let mut removed = 0usize;
        for path in par2_files {
            match tokio::fs::remove_file(&path).await {
                Ok(()) => removed += 1,
                Err(e) => warn!(
                    download_id = download_id.0,
                    ?path,
                    error = %e,
                    "failed to remove PAR2 file"
                ),
            }
        }
        if removed > 0 {
            info!(
                download_id = download_id.0,
                ?folder,
                removed,
                "removed {removed} PAR2 file(s) after successful processing"
            );
        }
    }

    /// Re-run extraction only (skip verify/repair)
    ///
    /// This method runs only the extraction and move stages, skipping
    /// PAR2 verification and repair. Useful for re-extracting archives
    /// after adding passwords or changing extraction settings.
    ///
    /// # Arguments
    ///
    /// * `download_id` - The download to re-extract
    /// * `download_path` - Path to the downloaded files
    /// * `destination` - Final destination for extracted files
    ///
    /// # Returns
    ///
    /// Returns Ok(final_path) on success, Err on failure
    pub async fn reextract(
        &self,
        download_id: DownloadId,
        download_path: PathBuf,
        destination: PathBuf,
    ) -> Result<PathBuf> {
        info!(
            download_id = download_id.0,
            ?download_path,
            ?destination,
            "starting re-extraction (skip verify/repair)"
        );

        // Run only extract and move stages
        let extracted_path = self.run_extract_stage(download_id, &download_path).await?;

        let final_path = self
            .run_move_stage(download_id, &extracted_path, &destination)
            .await?;

        Ok(final_path)
    }

    /// Execute the extract stage
    async fn run_extract_stage(
        &self,
        download_id: DownloadId,
        download_path: &Path,
    ) -> Result<PathBuf> {
        debug!(
            download_id = download_id.0,
            ?download_path,
            "running extract stage"
        );

        // Emit Extracting event (initial progress)
        self.event_tx
            .send(Event::Extracting {
                id: download_id,
                archive: String::new(),
                percent: 0.0,
            })
            .ok();

        // Detect all archives in the download directory
        let archives = self.detect_all_archives(download_path)?;

        if archives.is_empty() {
            info!(
                download_id = download_id.0,
                ?download_path,
                "no archives found in directory, skipping extraction"
            );

            // Emit ExtractComplete event
            self.event_tx
                .send(Event::ExtractComplete { id: download_id })
                .ok();

            return Ok(download_path.to_path_buf());
        }

        info!(
            download_id = download_id.0,
            archive_count = archives.len(),
            "found {} archive(s) to extract",
            archives.len()
        );

        // Create extraction destination directory
        let extract_dest = download_path.join("extracted");
        tokio::fs::create_dir_all(&extract_dest).await?;

        // Collect passwords from all sources
        let passwords = self.collect_extraction_passwords(download_id).await;

        // Extract all archives with progress tracking. A failed archive must keep the
        // downloaded source intact and fail the job instead of moving an empty result.
        self.extract_archives(download_id, &archives, &extract_dest, &passwords)
            .await?;

        // Emit ExtractComplete event
        self.event_tx
            .send(Event::ExtractComplete { id: download_id })
            .ok();

        info!(
            download_id = download_id.0,
            ?extract_dest,
            "extraction stage complete, extracted files in: {:?}",
            extract_dest
        );

        Ok(extract_dest)
    }

    /// Collect passwords from all sources for extraction
    ///
    /// Gathers passwords from:
    /// 1. Cached password from the database (includes NZB metadata and per-download passwords)
    /// 2. Global password file from configuration
    /// 3. Empty password if configured to try
    async fn collect_extraction_passwords(
        &self,
        download_id: DownloadId,
    ) -> crate::extraction::PasswordList {
        // Get cached password for this download (if any)
        // This includes any password from DownloadOptions or NZB metadata that was cached during download
        let cached_password = match self.db.get_cached_password(download_id).await {
            Ok(Some(pw)) => Some(pw),
            _ => None,
        };

        // Collect passwords from all sources
        // Note: Per-download and NZB metadata passwords are already cached in the database
        // and retrieved above as cached_password (highest priority if extraction succeeded before)
        let passwords = crate::extraction::PasswordList::collect(
            cached_password.as_deref(),
            None, // Per-download password already in cached_password
            None, // NZB metadata password already in cached_password
            self.config.tools.password_file.as_deref(), // Global password file
            self.config.tools.try_empty_password, // Try empty password as fallback
        )
        .await;

        info!(
            download_id = download_id.0,
            password_count = passwords.len(),
            "collected {} password(s) for extraction",
            passwords.len()
        );

        passwords
    }

    /// Extract all archives with progress tracking
    ///
    /// Iterates through all detected archives and extracts them with recursive
    /// nested archive support. Emits progress events and logs errors but continues
    /// extraction even if individual archives fail.
    async fn extract_archives(
        &self,
        download_id: DownloadId,
        archives: &[PathBuf],
        extract_dest: &Path,
        passwords: &crate::extraction::PasswordList,
    ) -> Result<()> {
        for (i, archive_path) in archives.iter().enumerate() {
            let archive_name = archive_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            info!(
                download_id = download_id.0,
                ?archive_path,
                progress = i + 1,
                total = archives.len(),
                "extracting archive {}/{}: {}",
                i + 1,
                archives.len(),
                archive_name
            );

            // Emit progress event
            self.event_tx
                .send(Event::Extracting {
                    id: download_id,
                    archive: archive_name.to_string(),
                    percent: (((i as f64) / (archives.len() as f64)) * 100.0) as f32,
                })
                .ok();

            // Extract with recursive nested archive support
            match crate::extraction::extract_recursive(
                download_id,
                archive_path,
                extract_dest,
                passwords,
                &self.db,
                &self.config.processing.extraction,
                0, // Start at depth 0
            )
            .await
            {
                Ok(extracted_files) => {
                    info!(
                        download_id = download_id.0,
                        ?archive_path,
                        extracted_count = extracted_files.len(),
                        "successfully extracted {} files from {}",
                        extracted_files.len(),
                        archive_name
                    );
                }
                Err(error) => {
                    warn!(
                        download_id = download_id.0,
                        ?archive_path,
                        error = %error,
                        "failed to extract archive {}",
                        archive_name
                    );
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Detect all archives in the download directory
    ///
    /// Scans for RAR, 7z, and ZIP archives
    fn detect_all_archives(&self, download_path: &Path) -> Result<Vec<PathBuf>> {
        let mut all_archives = Vec::new();

        // Detect RAR archives
        let rar_archives = crate::extraction::RarExtractor::detect_rar_files(download_path)?;
        all_archives.extend(rar_archives);

        // Detect 7z archives
        let sevenzip_archives =
            crate::extraction::SevenZipExtractor::detect_7z_files(download_path)?;
        all_archives.extend(sevenzip_archives);

        // Detect ZIP archives
        let zip_archives = crate::extraction::ZipExtractor::detect_zip_files(download_path)?;
        all_archives.extend(zip_archives);

        Ok(all_archives)
    }

    /// Execute the move stage
    async fn run_move_stage(
        &self,
        download_id: DownloadId,
        source_path: &Path,
        destination: &Path,
    ) -> Result<PathBuf> {
        debug!(
            download_id = download_id.0,
            ?source_path,
            ?destination,
            "running move stage"
        );

        // Emit Moving event
        self.event_tx
            .send(Event::Moving {
                id: download_id,
                destination: destination.to_path_buf(),
            })
            .ok();

        // Perform the actual file move with collision handling
        self.move_files(download_id, source_path, destination).await
    }

    /// Move files from source to destination with collision handling
    ///
    /// This function handles moving files/directories from the source path to the
    /// destination path, applying the configured FileCollisionAction when files
    /// already exist at the destination.
    ///
    /// # Arguments
    ///
    /// * `download_id` - The download ID for logging
    /// * `source_path` - Path to the source files/directory
    /// * `destination` - Path to the destination directory
    ///
    /// # Returns
    ///
    /// Returns Ok(final_path) where final_path is the actual destination used,
    /// or Err if the move operation fails
    async fn move_files(
        &self,
        download_id: DownloadId,
        source_path: &Path,
        destination: &Path,
    ) -> Result<PathBuf> {
        use tokio::fs;

        debug!(
            download_id = download_id.0,
            ?source_path,
            ?destination,
            "moving files with collision action: {:?}",
            self.config.download.file_collision
        );

        // Check if source exists and get its type
        let source_metadata = match fs::metadata(source_path).await {
            Ok(meta) => meta,
            Err(_) => {
                return Err(crate::error::Error::PostProcess(
                    PostProcessError::InvalidPath {
                        path: source_path.to_path_buf(),
                        reason: "Source path does not exist".to_string(),
                    },
                ));
            }
        };

        // Ensure destination parent directory exists
        if let Some(parent) = destination.parent() {
            // create_dir_all handles the case when directory already exists
            fs::create_dir_all(parent).await?;
        }

        // If source is a file, move it directly
        if source_metadata.is_file() {
            return self
                .move_single_file(download_id, source_path, destination)
                .await;
        }

        // If source is a directory, move all its contents
        if source_metadata.is_dir() {
            return self
                .move_directory_contents(download_id, source_path, destination)
                .await;
        }

        // If we get here, source is neither file nor directory
        Err(crate::error::Error::PostProcess(
            PostProcessError::InvalidPath {
                path: source_path.to_path_buf(),
                reason: "Source is neither a file nor a directory".to_string(),
            },
        ))
    }

    /// Move a single file to destination with collision handling
    async fn move_single_file(
        &self,
        download_id: DownloadId,
        source_file: &Path,
        destination: &Path,
    ) -> Result<PathBuf> {
        use tokio::fs;

        // Apply collision handling to get the actual destination path
        let final_destination = get_unique_path(destination, self.config.download.file_collision)?;

        debug!(
            download_id = download_id.0,
            ?source_file,
            ?final_destination,
            "moving single file"
        );

        // Perform the move
        fs::rename(source_file, &final_destination).await?;

        info!(
            download_id = download_id.0,
            ?source_file,
            ?final_destination,
            "successfully moved file"
        );

        Ok(final_destination)
    }

    /// Move directory contents to destination with collision handling
    fn move_directory_contents<'a>(
        &'a self,
        download_id: DownloadId,
        source_dir: &'a Path,
        destination: &'a Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<PathBuf>> + Send + 'a>> {
        Box::pin(async move {
            use tokio::fs;

            debug!(
                download_id = download_id.0,
                ?source_dir,
                ?destination,
                "moving directory contents"
            );

            // Create destination directory (create_dir_all handles existing)
            fs::create_dir_all(destination).await?;

            // Read all entries in source directory
            let mut entries = fs::read_dir(source_dir).await?;

            // Move each entry
            while let Some(entry) = entries.next_entry().await? {
                let source_entry_path = entry.path();
                let entry_name = entry.file_name();
                let dest_entry_path = destination.join(&entry_name);

                // Get file type from the entry (avoids extra syscall)
                let file_type = entry.file_type().await?;

                if file_type.is_file() {
                    // Move file with collision handling
                    self.move_single_file(download_id, &source_entry_path, &dest_entry_path)
                        .await?;
                } else if file_type.is_dir() {
                    // Recursively move subdirectory
                    self.move_directory_contents(download_id, &source_entry_path, &dest_entry_path)
                        .await?;

                    // Remove the now-empty source subdirectory
                    fs::remove_dir(&source_entry_path).await?;
                }
            }

            info!(
                download_id = download_id.0,
                ?source_dir,
                ?destination,
                "successfully moved directory contents"
            );

            Ok(destination.to_path_buf())
        })
    }
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests;
