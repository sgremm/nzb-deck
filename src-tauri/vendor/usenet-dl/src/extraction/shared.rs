use crate::config::ExtractionConfig;
use crate::db::Database;
use crate::error::{Error, PostProcessError, Result};
use crate::types::{ArchiveType, DownloadId};
use std::path::{Path, PathBuf};
use tokio::task::spawn_blocking;
use tracing::{debug, info, warn};

use super::password_list::PasswordList;

/// Shared implementation for archive extraction with password attempts.
///
/// Tries each password in the list by calling `try_extract_fn` via `spawn_blocking`.
/// Caches the successful password in the database.
///
/// This is the single implementation behind `RarExtractor::extract_with_passwords`,
/// `SevenZipExtractor::extract_with_passwords`, and `ZipExtractor::extract_with_passwords`.
pub(crate) async fn extract_with_passwords_impl(
    format_name: &str,
    try_extract_fn: impl Fn(&Path, &str, &Path) -> Result<Vec<PathBuf>> + Send + 'static + Clone,
    download_id: DownloadId,
    archive_path: &Path,
    dest_path: &Path,
    passwords: &PasswordList,
    db: &Database,
) -> Result<Vec<PathBuf>> {
    if passwords.is_empty() {
        warn!(
            download_id = download_id.0,
            ?archive_path,
            "no passwords to try for {} extraction",
            format_name
        );
        return Err(Error::PostProcess(PostProcessError::NoPasswordsAvailable {
            archive: archive_path.to_path_buf(),
        }));
    }

    info!(
        download_id = download_id.0,
        ?archive_path,
        password_count = passwords.len(),
        "attempting {} extraction with {} password(s)",
        format_name,
        passwords.len()
    );

    for (i, password) in passwords.iter().enumerate() {
        debug!(
            download_id = download_id.0,
            attempt = i + 1,
            total = passwords.len(),
            password_length = password.len(),
            "trying password {}/{}",
            i + 1,
            passwords.len()
        );

        // Use spawn_blocking to avoid blocking the async runtime during extraction
        let archive_path_owned = archive_path.to_path_buf();
        let dest_path_owned = dest_path.to_path_buf();
        let password_owned = password.clone();
        let try_fn = try_extract_fn.clone();

        let result =
            spawn_blocking(move || try_fn(&archive_path_owned, &password_owned, &dest_path_owned))
                .await
                .map_err(|e| {
                    Error::PostProcess(PostProcessError::ExtractionFailed {
                        archive: archive_path.to_path_buf(),
                        reason: format!("extraction task panicked: {}", e),
                    })
                })?;

        match result {
            Ok(files) => {
                info!(
                    download_id = download_id.0,
                    ?archive_path,
                    attempt = i + 1,
                    "{} extraction successful on attempt {}/{}",
                    format_name,
                    i + 1,
                    passwords.len()
                );

                // Cache successful password
                if let Err(e) = db.set_correct_password(download_id, password).await {
                    warn!(
                        download_id = download_id.0,
                        error = %e,
                        "failed to cache correct password"
                    );
                }

                return Ok(files);
            }
            Err(Error::PostProcess(PostProcessError::WrongPassword { .. })) => {
                debug!(
                    download_id = download_id.0,
                    attempt = i + 1,
                    "wrong password, trying next"
                );
                continue;
            }
            Err(e) => {
                // Other error (corrupt archive, disk full, etc.)
                warn!(
                    download_id = download_id.0,
                    error = %e,
                    ?archive_path,
                    "{} extraction failed with non-password error",
                    format_name
                );
                return Err(e);
            }
        }
    }

    // All passwords failed
    warn!(
        download_id = download_id.0,
        ?archive_path,
        attempted = passwords.len(),
        "all passwords failed for {} extraction",
        format_name
    );
    Err(Error::PostProcess(PostProcessError::AllPasswordsFailed {
        archive: archive_path.to_path_buf(),
        count: passwords.len(),
    }))
}

/// Detect archive type by file extension
///
/// Returns the archive type based on the file extension.
/// Supports RAR (.rar, .r00), 7z (.7z), and ZIP (.zip) formats.
pub fn detect_archive_type(path: &Path) -> Option<ArchiveType> {
    let ext = path.extension()?.to_str()?.to_lowercase();

    match ext.as_str() {
        "rar" | "r00" => Some(ArchiveType::Rar),
        "7z" => Some(ArchiveType::SevenZip),
        "zip" => Some(ArchiveType::Zip),
        _ => None,
    }
}

/// Check if a file is an archive based on its extension
///
/// Uses the configured list of archive extensions to determine if a file
/// should be treated as an archive for nested extraction purposes.
///
/// # Arguments
/// * `path` - Path to the file to check
/// * `archive_extensions` - List of extensions to treat as archives (without dots)
///
/// # Returns
/// `true` if the file extension matches one of the configured archive extensions
pub fn is_archive(path: &Path, archive_extensions: &[String]) -> bool {
    if let Some(ext) = path.extension() {
        let ext_str = ext.to_string_lossy().to_lowercase();
        archive_extensions
            .iter()
            .any(|ae| ae.to_lowercase() == ext_str)
    } else {
        false
    }
}

/// Extract archives recursively to handle nested archives
///
/// This function extracts an archive and then recursively extracts any archives
/// found within the extracted files, up to the configured maximum depth.
///
/// # Arguments
/// * `download_id` - Download ID for password caching and logging
/// * `archive_path` - Path to the archive to extract
/// * `dest_path` - Destination directory for extraction
/// * `passwords` - List of passwords to try
/// * `db` - Database for password caching
/// * `config` - Extraction configuration (recursion depth, extensions)
/// * `current_depth` - Current recursion depth (0 for initial call)
///
/// # Returns
/// * `Ok(Vec<PathBuf>)` - List of all extracted files (including from nested archives)
/// * `Err(Error)` - Extraction error
///
/// # Example
/// ```no_run
/// use usenet_dl::extraction::{extract_recursive, PasswordList};
/// use usenet_dl::config::ExtractionConfig;
/// use std::path::PathBuf;
///
/// # async fn example(db: &usenet_dl::db::Database) -> usenet_dl::error::Result<()> {
/// let passwords = PasswordList::collect(None, Some("pass123"), None, None, true);
/// let config = ExtractionConfig::default();
/// let files = extract_recursive(
///     1,
///     &PathBuf::from("nested.rar"),
///     &PathBuf::from("/tmp/extract"),
///     &passwords,
///     db,
///     &config,
///     0,
/// ).await?;
/// println!("Extracted {} files (including nested)", files.len());
/// # Ok(())
/// # }
/// ```
pub fn extract_recursive<'a>(
    download_id: DownloadId,
    archive_path: &'a Path,
    dest_path: &'a Path,
    passwords: &'a PasswordList,
    db: &'a Database,
    config: &'a ExtractionConfig,
    current_depth: u32,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<PathBuf>>> + Send + 'a>> {
    Box::pin(async move {
        debug!(
            download_id = download_id.0,
            ?archive_path,
            current_depth,
            max_depth = config.max_recursion_depth,
            "extracting archive (depth {}/{})",
            current_depth,
            config.max_recursion_depth
        );

        // Extract the archive
        let extracted =
            crate::extraction::extract_archive(download_id, archive_path, dest_path, passwords, db)
                .await?;

        info!(
            download_id = download_id.0,
            ?archive_path,
            extracted_count = extracted.len(),
            "extracted {} files from archive at depth {}",
            extracted.len(),
            current_depth
        );

        // If we've reached max recursion depth, return immediately
        if current_depth >= config.max_recursion_depth {
            debug!(
                download_id = download_id.0,
                current_depth,
                max_depth = config.max_recursion_depth,
                "reached maximum recursion depth, not extracting nested archives"
            );
            return Ok(extracted);
        }

        // Start with the files we just extracted
        let mut all_files = extracted.clone();

        // Check each extracted file to see if it's an archive
        for file in &extracted {
            if is_archive(file, &config.archive_extensions) {
                info!(
                    download_id = download_id.0,
                    ?file,
                    current_depth,
                    "found nested archive, extracting recursively"
                );

                // Create a unique subdirectory for nested extraction to avoid conflicts
                let nested_dest = dest_path.join(format!(
                    "nested_{}_{}",
                    file.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("archive"),
                    current_depth + 1
                ));

                // Recursively extract the nested archive
                match extract_recursive(
                    download_id,
                    file,
                    &nested_dest,
                    passwords,
                    db,
                    config,
                    current_depth + 1,
                )
                .await
                {
                    Ok(nested_files) => {
                        info!(
                            download_id = download_id.0,
                            ?file,
                            nested_count = nested_files.len(),
                            "successfully extracted {} files from nested archive",
                            nested_files.len()
                        );
                        all_files.extend(nested_files);
                    }
                    Err(e) => {
                        // Log warning but continue with other files
                        // Don't fail the entire extraction if one nested archive fails
                        warn!(
                            download_id = download_id.0,
                            ?file,
                            error = %e,
                            "failed to extract nested archive, continuing with other files"
                        );
                    }
                }
            }
        }

        info!(
            download_id = download_id.0,
            ?archive_path,
            total_files = all_files.len(),
            depth = current_depth,
            "completed extraction with {} total files (including nested) at depth {}",
            all_files.len(),
            current_depth
        );

        Ok(all_files)
    })
}
