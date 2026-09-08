//! Archive extraction with password support
//!
//! This module handles extracting RAR, 7z, and ZIP archives with password attempts.
//! It supports multiple password sources (cached, per-download, NZB meta, global file, empty).

mod password_list;
mod rar;
mod sevenz;
mod shared;
mod zip;

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests;

// Re-exports
pub use password_list::PasswordList;
pub use rar::RarExtractor;
pub use sevenz::SevenZipExtractor;
pub use shared::{detect_archive_type, extract_recursive, is_archive};
pub use zip::ZipExtractor;

use crate::db::Database;
use crate::error::{Error, PostProcessError, Result};
use crate::types::DownloadId;
use std::path::{Path, PathBuf};
use tracing::info;

/// Unified archive extraction dispatcher
///
/// Detects the archive type and routes to the appropriate extractor (RAR, 7z, or ZIP).
/// Tries multiple passwords from the PasswordList and caches the successful password.
///
/// # Arguments
/// * `download_id` - Download ID for password caching
/// * `archive_path` - Path to the archive file
/// * `dest_path` - Destination directory for extraction
/// * `passwords` - List of passwords to try (in priority order)
/// * `db` - Database for caching successful passwords
///
/// # Returns
/// * `Ok(Vec<PathBuf>)` - List of extracted files on success
/// * `Err(Error)` - Extraction error (wrong password, corruption, unknown type, etc.)
///
/// # Example
/// ```no_run
/// use usenet_dl::extraction::{extract_archive, PasswordList};
/// use std::path::PathBuf;
///
/// # async fn example(db: &usenet_dl::db::Database) -> usenet_dl::error::Result<()> {
/// let passwords = PasswordList::collect(None, Some("pass123"), None, None, true);
/// let files = extract_archive(
///     1,
///     &PathBuf::from("movie.rar"),
///     &PathBuf::from("/tmp/extract"),
///     &passwords,
///     db,
/// ).await?;
/// println!("Extracted {} files", files.len());
/// # Ok(())
/// # }
/// ```
pub async fn extract_archive(
    download_id: DownloadId,
    archive_path: &Path,
    dest_path: &Path,
    passwords: &PasswordList,
    db: &Database,
) -> Result<Vec<PathBuf>> {
    // Detect archive type by extension
    let archive_type = detect_archive_type(archive_path).ok_or_else(|| {
        Error::PostProcess(PostProcessError::ExtractionFailed {
            archive: archive_path.to_path_buf(),
            reason: format!("unknown archive type for file: {}", archive_path.display()),
        })
    })?;

    info!(
        download_id = download_id.0,
        ?archive_path,
        ?archive_type,
        "dispatching extraction to appropriate extractor"
    );

    // Route to the appropriate extractor
    match archive_type {
        crate::types::ArchiveType::Rar => {
            RarExtractor::extract_with_passwords(
                download_id,
                archive_path,
                dest_path,
                passwords,
                db,
            )
            .await
        }
        crate::types::ArchiveType::SevenZip => {
            SevenZipExtractor::extract_with_passwords(
                download_id,
                archive_path,
                dest_path,
                passwords,
                db,
            )
            .await
        }
        crate::types::ArchiveType::Zip => {
            ZipExtractor::extract_with_passwords(
                download_id,
                archive_path,
                dest_path,
                passwords,
                db,
            )
            .await
        }
    }
}
