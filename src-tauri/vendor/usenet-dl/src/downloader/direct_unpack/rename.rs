//! DirectRename — use PAR2 metadata to fix obfuscated filenames during download.
//!
//! When a PAR2 file completes during download, its metadata is parsed to build a
//! mapping of 16KB MD5 hashes to real filenames. As other files complete, their
//! first 16KB is hashed and matched against this mapping to detect and fix
//! obfuscated names.

use std::collections::HashMap;
use std::path::Path;

use crate::db::Database;
use crate::parity::par2_metadata::{compute_16k_md5, parse_par2_file_entries};
use crate::types::{DownloadId, Event};

/// Manages DirectRename state and operations for a single download.
pub(crate) struct DirectRenameState {
    /// Mapping from 16KB MD5 hash to real filename (from PAR2 metadata)
    hash_to_name: HashMap<[u8; 16], String>,
    /// Whether PAR2 metadata has been loaded
    pub(crate) metadata_loaded: bool,
}

impl DirectRenameState {
    pub(crate) fn new() -> Self {
        Self {
            hash_to_name: HashMap::new(),
            metadata_loaded: false,
        }
    }

    /// Load PAR2 metadata from a completed PAR2 file.
    ///
    /// Parses the PAR2 binary format to extract filename-to-hash mappings.
    /// Can be called multiple times (for multiple PAR2 files) — entries accumulate.
    pub(crate) fn load_par2_metadata(&mut self, par2_path: &Path) -> crate::Result<usize> {
        let entries = parse_par2_file_entries(par2_path)?;
        let count = entries.len();

        for entry in entries {
            self.hash_to_name.insert(entry.hash_16k, entry.filename);
        }

        self.metadata_loaded = true;
        Ok(count)
    }

    /// Get all loaded PAR2 file entries (hash → filename mappings).
    #[allow(dead_code)]
    pub(crate) fn entries(&self) -> impl Iterator<Item = (&[u8; 16], &String)> {
        self.hash_to_name.iter()
    }

    /// Try to rename a completed file using PAR2 metadata.
    ///
    /// Computes the MD5 of the first 16KB, looks up the hash in the PAR2 metadata,
    /// and if the filename differs, renames the file on disk and in the database.
    ///
    /// Returns `Some((old_name, new_name))` if the file was renamed, `None` otherwise.
    pub(crate) async fn try_rename_file(
        &self,
        download_id: DownloadId,
        file_index: i32,
        current_filename: &str,
        temp_dir: &Path,
        db: &Database,
        event_tx: &tokio::sync::broadcast::Sender<Event>,
    ) -> Option<(String, String)> {
        if !self.metadata_loaded || self.hash_to_name.is_empty() {
            return None;
        }

        let file_path = temp_dir.join(current_filename);
        if !file_path.exists() {
            return None;
        }

        // Compute 16KB MD5 hash
        let hash = match compute_16k_md5(&file_path) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(
                    download_id = download_id.0,
                    filename = current_filename,
                    error = %e,
                    "Failed to compute 16KB MD5 for DirectRename"
                );
                return None;
            }
        };

        // Look up the real filename
        let real_name = self.hash_to_name.get(&hash)?;

        // Only rename if the name actually differs
        if real_name == current_filename {
            return None;
        }

        let new_path = temp_dir.join(real_name);
        let old_name = current_filename.to_string();
        let new_name = real_name.clone();

        // Rename on disk
        if let Err(e) = tokio::fs::rename(&file_path, &new_path).await {
            tracing::warn!(
                download_id = download_id.0,
                old_name = %old_name,
                new_name = %new_name,
                error = %e,
                "DirectRename: failed to rename file on disk"
            );
            return None;
        }

        // Update database
        if let Err(e) = db
            .rename_download_file(download_id, file_index, &new_name)
            .await
        {
            tracing::warn!(
                download_id = download_id.0,
                old_name = %old_name,
                new_name = %new_name,
                error = %e,
                "DirectRename: failed to update filename in database"
            );
            // File was already renamed on disk — log but continue
        }

        // Emit event
        event_tx
            .send(Event::DirectRenamed {
                id: download_id,
                old_name: old_name.clone(),
                new_name: new_name.clone(),
            })
            .ok();

        tracing::info!(
            download_id = download_id.0,
            old_name = %old_name,
            new_name = %new_name,
            "DirectRename: renamed obfuscated file"
        );

        Some((old_name, new_name))
    }
}
