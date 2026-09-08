use crate::db::Database;
use crate::error::{Error, PostProcessError, Result};
use crate::types::DownloadId;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use super::password_list::PasswordList;
use super::shared::extract_with_passwords_impl;

/// Archive extractor for ZIP files
pub struct ZipExtractor;

impl ZipExtractor {
    /// Detect ZIP archive files in a directory
    pub fn detect_zip_files(download_path: &Path) -> Result<Vec<PathBuf>> {
        debug!(?download_path, "detecting ZIP archives");

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

            // Check for .zip extension
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if ext_str == "zip" {
                    archives.push(path);
                }
            }
        }

        debug!("found {} ZIP archive(s)", archives.len());
        Ok(archives)
    }

    /// Open a ZIP entry by index, handling password decryption if needed
    fn open_zip_entry<'a>(
        archive: &'a mut zip::ZipArchive<std::fs::File>,
        index: usize,
        password: &str,
        archive_path: &Path,
    ) -> Result<zip::read::ZipFile<'a>> {
        if password.is_empty() {
            archive.by_index(index).map_err(|e| {
                let err_str = e.to_string();
                if err_str.contains("password") || err_str.contains("encrypted") {
                    Error::PostProcess(PostProcessError::WrongPassword {
                        archive: archive_path.to_path_buf(),
                    })
                } else {
                    Error::PostProcess(PostProcessError::ExtractionFailed {
                        archive: archive_path.to_path_buf(),
                        reason: format!("failed to read ZIP entry: {}", e),
                    })
                }
            })
        } else {
            archive
                .by_index_decrypt(index, password.as_bytes())
                .map_err(|e| {
                    let err_str = e.to_string();
                    if err_str.contains("password") || err_str.contains("encrypted") {
                        Error::PostProcess(PostProcessError::WrongPassword {
                            archive: archive_path.to_path_buf(),
                        })
                    } else {
                        Error::PostProcess(PostProcessError::ExtractionFailed {
                            archive: archive_path.to_path_buf(),
                            reason: format!("failed to read ZIP entry: {}", e),
                        })
                    }
                })?
                .map_err(|_| {
                    Error::PostProcess(PostProcessError::WrongPassword {
                        archive: archive_path.to_path_buf(),
                    })
                })
        }
    }

    /// Extract a single ZIP entry to disk, creating directories as needed
    fn extract_zip_entry(
        mut file: zip::read::ZipFile,
        dest_path: &Path,
        archive_path: &Path,
    ) -> Result<Option<PathBuf>> {
        // Get the file path
        let file_path = match file.enclosed_name() {
            Some(path) => dest_path.join(path),
            None => {
                warn!("skipping entry with unsafe path");
                return Ok(None);
            }
        };

        // Check if it's a directory
        if file.is_dir() {
            // Create directory
            std::fs::create_dir_all(&file_path).map_err(|e| {
                Error::Io(std::io::Error::other(format!(
                    "failed to create directory: {}",
                    e
                )))
            })?;
            Ok(None)
        } else {
            // Create parent directories if needed
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    Error::Io(std::io::Error::other(format!(
                        "failed to create parent directories: {}",
                        e
                    )))
                })?;
            }

            // Extract file
            let mut outfile = std::fs::File::create(&file_path).map_err(|e| {
                Error::Io(std::io::Error::other(format!(
                    "failed to create output file: {}",
                    e
                )))
            })?;

            std::io::copy(&mut file, &mut outfile).map_err(|e| {
                let err_str = e.to_string();
                if err_str.contains("password") || err_str.contains("encrypted") {
                    Error::PostProcess(PostProcessError::WrongPassword {
                        archive: archive_path.to_path_buf(),
                    })
                } else {
                    Error::Io(std::io::Error::other(format!(
                        "failed to extract file: {}",
                        e
                    )))
                }
            })?;

            Ok(Some(file_path))
        }
    }

    /// Try to extract a ZIP archive with a single password
    pub fn try_extract(
        archive_path: &Path,
        password: &str,
        dest_path: &Path,
    ) -> Result<Vec<PathBuf>> {
        debug!(
            ?archive_path,
            password_length = password.len(),
            ?dest_path,
            "attempting ZIP extraction"
        );

        // Create destination directory if it doesn't exist
        std::fs::create_dir_all(dest_path).map_err(|e| {
            Error::Io(std::io::Error::other(format!(
                "failed to create destination: {}",
                e
            )))
        })?;

        // Open the archive
        let file = std::fs::File::open(archive_path).map_err(|e| {
            Error::Io(std::io::Error::other(format!(
                "failed to open ZIP archive: {}",
                e
            )))
        })?;

        let mut archive = zip::ZipArchive::new(file).map_err(|e| {
            Error::PostProcess(PostProcessError::ExtractionFailed {
                archive: archive_path.to_path_buf(),
                reason: format!("failed to read ZIP archive: {}", e),
            })
        })?;

        let mut extracted_files = Vec::new();

        // Extract each file
        for i in 0..archive.len() {
            let file = Self::open_zip_entry(&mut archive, i, password, archive_path)?;

            if let Some(file_path) = Self::extract_zip_entry(file, dest_path, archive_path)? {
                extracted_files.push(file_path);
            }
        }

        info!(
            ?archive_path,
            extracted_count = extracted_files.len(),
            "ZIP extraction successful"
        );

        Ok(extracted_files)
    }

    /// Extract ZIP archive with password attempts
    pub async fn extract_with_passwords(
        download_id: DownloadId,
        archive_path: &Path,
        dest_path: &Path,
        passwords: &PasswordList,
        db: &Database,
    ) -> Result<Vec<PathBuf>> {
        extract_with_passwords_impl(
            "ZIP",
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
