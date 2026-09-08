//! DirectUnpack coordinator — polls for completed files and extracts archives during download.
//!
//! Spawned as a background task alongside the article download. Polls the database
//! for newly completed files, applies DirectRename if PAR2 metadata is available,
//! and extracts first RAR volumes as they finish. Cancels immediately if any article
//! failures are detected (falling back to normal post-processing).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::db::Database;
use crate::extraction::PasswordList;
use crate::types::{DownloadId, Event};

use super::rar_detection::{is_first_rar_volume, is_par2_file};
use super::rename::DirectRenameState;

/// DirectUnpack state values stored in the `downloads.direct_unpack_state` column.
pub(crate) mod state {
    pub const NOT_STARTED: i32 = 0;
    pub const ACTIVE: i32 = 1;
    pub const COMPLETED: i32 = 2;
    pub const CANCELLED: i32 = 3;
}

/// Result of a completed DirectUnpack coordinator run.
pub(crate) struct DirectUnpackResult {
    /// Final state (one of the `state::*` constants)
    #[allow(dead_code)]
    pub state: i32,
    /// Files that were successfully extracted during download
    #[allow(dead_code)]
    pub extracted_files: Vec<PathBuf>,
}

/// Coordinator for DirectUnpack — polls for completed files and extracts archives during download.
pub(crate) struct DirectUnpackCoordinator {
    download_id: DownloadId,
    db: Arc<Database>,
    config: Arc<Config>,
    event_tx: tokio::sync::broadcast::Sender<Event>,
    cancel_token: CancellationToken,
    download_temp_dir: PathBuf,
    /// Shared counter — set by the article download pipeline on each failure
    failed_articles: Arc<AtomicU64>,
    /// Flag set by the download pipeline when all articles have been processed
    download_complete: Arc<AtomicBool>,
    /// Channel receiver for instant file completion notifications from the download pipeline
    file_completion_rx: tokio::sync::mpsc::UnboundedReceiver<i32>,
}

impl DirectUnpackCoordinator {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        download_id: DownloadId,
        db: Arc<Database>,
        config: Arc<Config>,
        event_tx: tokio::sync::broadcast::Sender<Event>,
        cancel_token: CancellationToken,
        download_temp_dir: PathBuf,
        failed_articles: Arc<AtomicU64>,
        download_complete: Arc<AtomicBool>,
        file_completion_rx: tokio::sync::mpsc::UnboundedReceiver<i32>,
    ) -> Self {
        Self {
            download_id,
            db,
            config,
            event_tx,
            cancel_token,
            download_temp_dir,
            failed_articles,
            download_complete,
            file_completion_rx,
        }
    }

    /// Run the DirectUnpack coordinator loop.
    ///
    /// Returns when the download completes (all files processed), the download is
    /// cancelled, or article failures are detected.
    pub(crate) async fn run(mut self) -> DirectUnpackResult {
        let id = self.download_id;
        let poll_interval =
            std::time::Duration::from_millis(self.config.processing.direct_unpack.poll_interval_ms);
        let direct_rename_enabled = self.config.processing.direct_unpack.direct_rename;

        // Set state to Active in DB
        if let Err(e) = self.db.update_direct_unpack_state(id, state::ACTIVE).await {
            tracing::error!(
                download_id = id.0,
                error = %e,
                "DirectUnpack: failed to set active state"
            );
            return DirectUnpackResult {
                state: state::CANCELLED,
                extracted_files: vec![],
            };
        }
        self.event_tx.send(Event::DirectUnpackStarted { id }).ok();

        let mut rename_state = DirectRenameState::new();
        let mut extracted_files: Vec<PathBuf> = Vec::new();
        let mut pending_first_volumes: Vec<String> = Vec::new();
        let mut processed_indices: HashSet<i32> = HashSet::new();

        // Create extraction destination (same path post-processing uses)
        let extract_dest = self.download_temp_dir.join("extracted");
        if let Err(e) = tokio::fs::create_dir_all(&extract_dest).await {
            tracing::warn!(
                download_id = id.0,
                error = %e,
                "DirectUnpack: failed to create extraction directory"
            );
            self.set_db_state(state::CANCELLED).await;
            self.emit_cancelled("Failed to create extraction directory");
            return DirectUnpackResult {
                state: state::CANCELLED,
                extracted_files: vec![],
            };
        }

        let mut interval = tokio::time::interval(poll_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            // Wait for either: cancellation, a file completion notification, or the poll timer
            let poll_now = tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    tracing::info!(download_id = id.0, "DirectUnpack: cancelled via token");
                    self.set_db_state(state::CANCELLED).await;
                    self.emit_cancelled("Cancelled via cancellation token");
                    return DirectUnpackResult {
                        state: state::CANCELLED,
                        extracted_files,
                    };
                }
                Some(_file_index) = self.file_completion_rx.recv() => {
                    // Drain any additional queued notifications to batch-process them
                    while self.file_completion_rx.try_recv().is_ok() {}
                    true
                }
                _ = interval.tick() => {
                    true
                }
            };

            if !poll_now {
                continue;
            }

            // Check for article failures → immediate cancellation
            if self.failed_articles.load(Ordering::Relaxed) > 0 {
                tracing::info!(
                    download_id = id.0,
                    "DirectUnpack: cancelling due to article failures"
                );
                self.set_db_state(state::CANCELLED).await;
                self.emit_cancelled("Article failures detected");
                return DirectUnpackResult {
                    state: state::CANCELLED,
                    extracted_files,
                };
            }

            // Poll for newly completed files
            let newly_completed = match self.db.get_newly_completed_files(id).await {
                Ok(files) => files,
                Err(e) => {
                    tracing::warn!(
                        download_id = id.0,
                        error = %e,
                        "DirectUnpack: failed to query completed files"
                    );
                    continue;
                }
            };

            for file in &newly_completed {
                if processed_indices.contains(&file.file_index) {
                    continue;
                }
                processed_indices.insert(file.file_index);

                // Mark file as completed in DB
                if let Err(e) = self.db.mark_file_completed(id, file.file_index).await {
                    tracing::warn!(
                        download_id = id.0,
                        file_index = file.file_index,
                        error = %e,
                        "DirectUnpack: failed to mark file completed"
                    );
                }

                self.event_tx
                    .send(Event::FileCompleted {
                        id,
                        file_index: file.file_index,
                        filename: file.filename.clone(),
                    })
                    .ok();

                let filename = &file.filename;

                // Handle PAR2 files for DirectRename
                if direct_rename_enabled && is_par2_file(filename) {
                    let par2_path = self.download_temp_dir.join(filename);
                    match rename_state.load_par2_metadata(&par2_path) {
                        Ok(count) => {
                            tracing::info!(
                                download_id = id.0,
                                filename = %filename,
                                entries = count,
                                "DirectRename: loaded PAR2 metadata"
                            );
                            // Retroactively rename already-completed files
                            self.retroactive_rename(&rename_state, &processed_indices)
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                download_id = id.0,
                                filename = %filename,
                                error = %e,
                                "DirectRename: failed to parse PAR2 metadata"
                            );
                        }
                    }
                }

                // Try DirectRename on non-PAR2 files
                if direct_rename_enabled
                    && rename_state.metadata_loaded
                    && !is_par2_file(filename)
                {
                    rename_state
                        .try_rename_file(
                            id,
                            file.file_index,
                            filename,
                            &self.download_temp_dir,
                            &self.db,
                            &self.event_tx,
                        )
                        .await;
                }

                // Check if this is a first RAR volume → attempt extraction.
                // Re-read filename from DB in case DirectRename changed it.
                let current_filename = self.current_filename(file.file_index, filename).await;
                if is_first_rar_volume(&current_filename) {
                    match self
                        .try_extract(&current_filename, &extract_dest, &mut extracted_files)
                        .await
                    {
                        ExtractAttempt::Success => {}
                        ExtractAttempt::VolumeNotReady => {
                            pending_first_volumes.push(current_filename);
                        }
                        ExtractAttempt::Failed => {
                            // Non-fatal: post-processing fallback handles it
                        }
                    }
                }
            }

            // Retry pending first volumes
            let mut still_pending = Vec::new();
            for volume in pending_first_volumes.drain(..) {
                match self
                    .try_extract(&volume, &extract_dest, &mut extracted_files)
                    .await
                {
                    ExtractAttempt::Success => {}
                    ExtractAttempt::VolumeNotReady => {
                        still_pending.push(volume);
                    }
                    ExtractAttempt::Failed => {}
                }
            }
            pending_first_volumes = still_pending;

            // Check if download is done and all work is processed
            if self.download_complete.load(Ordering::Acquire)
                && newly_completed.is_empty()
                && pending_first_volumes.is_empty()
            {
                break;
            }
        }

        // Successfully completed
        self.set_db_state(state::COMPLETED).await;
        self.set_db_extracted_count(extracted_files.len() as i32)
            .await;
        self.event_tx.send(Event::DirectUnpackComplete { id }).ok();

        tracing::info!(
            download_id = id.0,
            extracted_count = extracted_files.len(),
            "DirectUnpack: completed successfully"
        );

        DirectUnpackResult {
            state: state::COMPLETED,
            extracted_files,
        }
    }

    /// Attempt to extract a first RAR volume.
    async fn try_extract(
        &self,
        filename: &str,
        extract_dest: &std::path::Path,
        extracted_files: &mut Vec<PathBuf>,
    ) -> ExtractAttempt {
        let archive_path = self.download_temp_dir.join(filename);
        if !archive_path.exists() {
            return ExtractAttempt::Failed;
        }

        self.event_tx
            .send(Event::DirectUnpackExtracting {
                id: self.download_id,
                filename: filename.to_string(),
            })
            .ok();

        // Collect passwords
        let cached_pw = self
            .db
            .get_cached_password(self.download_id)
            .await
            .ok()
            .flatten();
        let passwords = PasswordList::collect(
            cached_pw.as_deref(),
            None,
            None,
            self.config.tools.password_file.as_deref(),
            self.config.tools.try_empty_password,
        )
        .await;

        match crate::extraction::extract_archive(
            self.download_id,
            &archive_path,
            extract_dest,
            &passwords,
            &self.db,
        )
        .await
        {
            Ok(files) => {
                let file_names: Vec<String> = files
                    .iter()
                    .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .collect();

                self.event_tx
                    .send(Event::DirectUnpackExtracted {
                        id: self.download_id,
                        filename: filename.to_string(),
                        extracted_files: file_names,
                    })
                    .ok();

                extracted_files.extend(files);
                ExtractAttempt::Success
            }
            Err(e) => {
                let error_msg = e.to_string();
                if is_volume_not_ready_error(&error_msg) {
                    tracing::debug!(
                        download_id = self.download_id.0,
                        filename = %filename,
                        "DirectUnpack: next volume not ready, will retry"
                    );
                    ExtractAttempt::VolumeNotReady
                } else {
                    tracing::warn!(
                        download_id = self.download_id.0,
                        filename = %filename,
                        error = %error_msg,
                        "DirectUnpack: extraction failed"
                    );
                    ExtractAttempt::Failed
                }
            }
        }
    }

    /// Re-read the current filename from DB (may have been renamed by DirectRename).
    async fn current_filename(&self, file_index: i32, fallback: &str) -> String {
        match self.db.get_download_files(self.download_id).await {
            Ok(files) => files
                .iter()
                .find(|f| f.file_index == file_index)
                .map(|f| f.filename.clone())
                .unwrap_or_else(|| fallback.to_string()),
            Err(_) => fallback.to_string(),
        }
    }

    /// Try to rename already-processed files retroactively after PAR2 metadata loads.
    async fn retroactive_rename(
        &self,
        rename_state: &DirectRenameState,
        processed_indices: &HashSet<i32>,
    ) {
        let files = match self.db.get_download_files(self.download_id).await {
            Ok(f) => f,
            Err(_) => return,
        };

        for file in &files {
            if !processed_indices.contains(&file.file_index) {
                continue;
            }
            if is_par2_file(&file.filename) {
                continue;
            }
            rename_state
                .try_rename_file(
                    self.download_id,
                    file.file_index,
                    &file.filename,
                    &self.download_temp_dir,
                    &self.db,
                    &self.event_tx,
                )
                .await;
        }
    }

    /// Update the direct_unpack_state column in the database.
    async fn set_db_state(&self, db_state: i32) {
        if let Err(e) = self
            .db
            .update_direct_unpack_state(self.download_id, db_state)
            .await
        {
            tracing::warn!(
                download_id = self.download_id.0,
                error = %e,
                "DirectUnpack: failed to update state in DB"
            );
        }
    }

    /// Update the direct_unpack_extracted_count column in the database.
    async fn set_db_extracted_count(&self, count: i32) {
        if let Err(e) = self
            .db
            .update_direct_unpack_extracted_count(self.download_id, count)
            .await
        {
            tracing::warn!(
                download_id = self.download_id.0,
                error = %e,
                "DirectUnpack: failed to update extracted count in DB"
            );
        }
    }

    /// Emit a DirectUnpackCancelled event.
    fn emit_cancelled(&self, reason: &str) {
        self.event_tx
            .send(Event::DirectUnpackCancelled {
                id: self.download_id,
                reason: reason.to_string(),
            })
            .ok();
    }
}

/// Outcome of a single extraction attempt.
enum ExtractAttempt {
    /// Archive extracted successfully.
    Success,
    /// Next RAR volume not downloaded yet — should retry later.
    VolumeNotReady,
    /// Extraction failed for a non-recoverable reason.
    Failed,
}

/// Heuristic: check if an extraction error indicates a missing RAR volume.
///
/// When `unrar` can't find the next volume in a multi-part set, the error message
/// typically mentions the missing file. This lets the coordinator retry later
/// when more volumes have been downloaded.
fn is_volume_not_ready_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("cannot find volume")
        || lower.contains("next volume")
        || lower.contains("missing volume")
        || lower.contains("no such file")
        || lower.contains("volume not found")
}
