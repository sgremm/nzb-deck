use crate::db::Database;
use crate::error::{Error, PostProcessError, Result};
use crate::types::DownloadId;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use super::password_list::PasswordList;
use super::shared::extract_with_passwords_impl;

/// Archive extractor for RAR files
pub struct RarExtractor;

impl RarExtractor {
    /// Detect RAR archive files in a directory
    ///
    /// Looks for .rar files or .r00, .r01, etc. (split archives)
    /// Returns the main archive file (first part)
    pub fn detect_rar_files(download_path: &Path) -> Result<Vec<PathBuf>> {
        debug!(?download_path, "detecting RAR archives");

        let mut archives = Vec::new();

        // Read directory
        let entries = std::fs::read_dir(download_path).map_err(|e| {
            Error::Io(std::io::Error::other(format!(
                "failed to read directory: {}",
                e
            )))
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| {
                Error::Io(std::io::Error::other(format!(
                    "failed to read entry: {}",
                    e
                )))
            })?;
            let path = entry.path();

            // Skip directories
            if path.is_dir() {
                continue;
            }

            // Check for .rar extension
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();

                // Main RAR file or first part of split archive
                if ext_str == "rar" || ext_str == "r00" {
                    archives.push(path);
                }
            }
        }

        debug!("found {} RAR archive(s)", archives.len());
        Ok(archives)
    }

    /// Test-only public accessor for `is_password_error`
    #[cfg(test)]
    pub(crate) fn is_password_error_pub(error_msg: &str) -> bool {
        Self::is_password_error(error_msg)
    }

    /// Check if an unrar error indicates a password problem
    fn is_password_error(error_msg: &str) -> bool {
        error_msg.contains("password")
            || error_msg.contains("encrypted")
            || error_msg.contains("ERAR_BAD_PASSWORD")
    }

    /// Convert an unrar error to our error type, checking for password errors
    fn convert_unrar_error(e: unrar::error::UnrarError, archive_path: &Path) -> Error {
        let err_str = e.to_string();
        if Self::is_password_error(&err_str) {
            Error::PostProcess(PostProcessError::WrongPassword {
                archive: archive_path.to_path_buf(),
            })
        } else {
            Error::PostProcess(PostProcessError::ExtractionFailed {
                archive: archive_path.to_path_buf(),
                reason: err_str,
            })
        }
    }

    /// Try to extract a RAR archive with a single password
    ///
    /// Returns Ok(extracted_files) on success
    /// Returns Err with ExtractError::WrongPassword if password is incorrect
    /// Returns Err with other errors for corrupt archives, disk full, etc.
    pub fn try_extract(
        archive_path: &Path,
        password: &str,
        dest_path: &Path,
    ) -> Result<Vec<PathBuf>> {
        debug!(
            ?archive_path,
            password_length = password.len(),
            ?dest_path,
            "attempting RAR extraction"
        );

        // Create destination directory if it doesn't exist
        std::fs::create_dir_all(dest_path).map_err(|e| {
            Error::Io(std::io::Error::other(format!(
                "failed to create destination: {}",
                e
            )))
        })?;

        // Create archive with optional password
        let archive = if password.is_empty() {
            unrar::Archive::new(archive_path)
        } else {
            unrar::Archive::with_password(archive_path, password.as_bytes())
        };

        // Open for processing
        let processor = archive
            .open_for_processing()
            .map_err(|e| Self::convert_unrar_error(e, archive_path))?;

        let mut extracted_files = Vec::new();

        // Process each entry using the state machine interface
        let mut at_header = processor;
        loop {
            // Read the next header - transitions to BeforeFile state
            let at_file = match at_header.read_header() {
                Ok(Some(entry_processor)) => entry_processor,
                Ok(None) => break, // No more entries
                Err(e) => return Err(Self::convert_unrar_error(e, archive_path)),
            };

            // Get the file header information (available in BeforeFile state)
            let header = at_file.entry();

            // Sanitize filename to prevent path traversal attacks (e.g., "../../../etc/passwd")
            let sanitized = Path::new(&header.filename)
                .components()
                .filter(|c| matches!(c, std::path::Component::Normal(_)))
                .collect::<PathBuf>();

            if sanitized.as_os_str().is_empty() {
                // Skip entries with no valid path components (e.g., pure ".." entries)
                at_header = at_file.skip().map_err(|e| {
                    Error::PostProcess(PostProcessError::ExtractionFailed {
                        archive: archive_path.to_path_buf(),
                        reason: format!("failed to skip unsafe entry: {}", e),
                    })
                })?;
                continue;
            }

            let file_path = dest_path.join(&sanitized);

            // Check if it's a file (not a directory)
            if !header.is_directory() {
                // Extract the file - transitions back to BeforeHeader state
                at_header = at_file
                    .extract_to(&file_path)
                    .map_err(|e| Self::convert_unrar_error(e, archive_path))?;
                extracted_files.push(file_path);
            } else {
                // Skip directory entries - transitions back to BeforeHeader state
                at_header = at_file.skip().map_err(|e| {
                    Error::PostProcess(PostProcessError::ExtractionFailed {
                        archive: archive_path.to_path_buf(),
                        reason: format!("failed to skip directory: {}", e),
                    })
                })?;
            }
        }

        info!(
            ?archive_path,
            extracted_count = extracted_files.len(),
            "RAR extraction successful"
        );

        Ok(extracted_files)
    }

    /// Extract RAR archive with password attempts
    ///
    /// Tries each password in the list until one works or all fail.
    /// Caches the successful password in the database.
    pub async fn extract_with_passwords(
        download_id: DownloadId,
        archive_path: &Path,
        dest_path: &Path,
        passwords: &PasswordList,
        db: &Database,
    ) -> Result<Vec<PathBuf>> {
        extract_with_passwords_impl(
            "RAR",
            Self::try_extract,
            download_id,
            archive_path,
            dest_path,
            passwords,
            db,
        )
        .await
    }
}
