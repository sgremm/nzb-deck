use crate::db::Database;
use crate::error::{Error, PostProcessError, Result};
use crate::types::DownloadId;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use super::password_list::PasswordList;
use super::shared::extract_with_passwords_impl;

/// Archive extractor for 7z files
pub struct SevenZipExtractor;

impl SevenZipExtractor {
    /// Detect 7z archive files in a directory
    pub fn detect_7z_files(download_path: &Path) -> Result<Vec<PathBuf>> {
        debug!(?download_path, "detecting 7z archives");

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

            // Check for .7z extension
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if ext_str == "7z" {
                    archives.push(path);
                }
            }
        }

        debug!("found {} 7z archive(s)", archives.len());
        Ok(archives)
    }

    /// Try to extract a 7z archive with a single password
    pub fn try_extract(
        archive_path: &Path,
        password: &str,
        dest_path: &Path,
    ) -> Result<Vec<PathBuf>> {
        debug!(
            ?archive_path,
            password_length = password.len(),
            ?dest_path,
            "attempting 7z extraction"
        );

        // Create destination directory if it doesn't exist
        std::fs::create_dir_all(dest_path).map_err(|e| {
            Error::Io(std::io::Error::other(format!(
                "failed to create destination: {}",
                e
            )))
        })?;

        // Decompress with optional password
        use sevenz_rust::Password;
        let result = if password.is_empty() {
            sevenz_rust::decompress_file(archive_path, dest_path)
        } else {
            let pw = Password::from(password);
            sevenz_rust::decompress_file_with_password(archive_path, dest_path, pw)
        };

        match result {
            Ok(()) => {
                // Validate that all extracted files are within dest_path (path traversal protection)
                Self::validate_extracted_paths(dest_path)?;

                // Collect the extracted files by scanning the destination directory
                let extracted_files = Self::collect_extracted_files(dest_path)?;

                info!(
                    ?archive_path,
                    extracted_count = extracted_files.len(),
                    "7z extraction successful"
                );
                Ok(extracted_files)
            }
            Err(e) => {
                let err_str = e.to_string();
                // Check if it's a password error
                if err_str.contains("password")
                    || err_str.contains("encrypted")
                    || err_str.contains("Wrong password")
                {
                    Err(Error::PostProcess(PostProcessError::WrongPassword {
                        archive: archive_path.to_path_buf(),
                    }))
                } else {
                    Err(Error::PostProcess(PostProcessError::ExtractionFailed {
                        archive: archive_path.to_path_buf(),
                        reason: format!("failed to extract 7z archive: {}", e),
                    }))
                }
            }
        }
    }

    /// Test-only public accessor for `validate_extracted_paths`
    #[cfg(test)]
    pub(crate) fn validate_extracted_paths_pub(dest_path: &Path) -> Result<()> {
        Self::validate_extracted_paths(dest_path)
    }

    /// Test-only public accessor for `collect_extracted_files`
    #[cfg(test)]
    pub(crate) fn collect_extracted_files_pub(dir: &Path) -> Result<Vec<PathBuf>> {
        Self::collect_extracted_files(dir)
    }

    /// Validate that all extracted files are within the destination directory.
    /// This protects against path traversal attacks in 7z archives.
    fn validate_extracted_paths(dest_path: &Path) -> Result<()> {
        let canonical_dest = dest_path.canonicalize().map_err(|e| {
            Error::Io(std::io::Error::other(format!(
                "failed to canonicalize destination path: {}",
                e
            )))
        })?;

        fn check_dir(dir: &Path, canonical_dest: &Path) -> Result<()> {
            let entries = std::fs::read_dir(dir).map_err(|e| {
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
                let canonical = path.canonicalize().map_err(|e| {
                    Error::Io(std::io::Error::other(format!(
                        "failed to canonicalize extracted path: {}",
                        e
                    )))
                })?;

                if !canonical.starts_with(canonical_dest) {
                    return Err(Error::PostProcess(PostProcessError::ExtractionFailed {
                        archive: dir.to_path_buf(),
                        reason: format!(
                            "path traversal detected: extracted file {:?} is outside destination",
                            canonical
                        ),
                    }));
                }

                if path.is_dir() {
                    check_dir(&path, canonical_dest)?;
                }
            }
            Ok(())
        }

        check_dir(dest_path, &canonical_dest)
    }

    /// Recursively collect all files (not directories) from a directory
    fn collect_extracted_files(dir: &Path) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();

        fn visit_dir(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
            let entries = std::fs::read_dir(dir).map_err(|e| {
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

                if path.is_dir() {
                    visit_dir(&path, files)?;
                } else {
                    files.push(path);
                }
            }
            Ok(())
        }

        visit_dir(dir, &mut files)?;
        Ok(files)
    }

    /// Extract 7z archive with password attempts
    pub async fn extract_with_passwords(
        download_id: DownloadId,
        archive_path: &Path,
        dest_path: &Path,
        passwords: &PasswordList,
        db: &Database,
    ) -> Result<Vec<PathBuf>> {
        extract_with_passwords_impl(
            "7z",
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
