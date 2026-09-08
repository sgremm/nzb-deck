//! Cleanup stage for removing intermediate files

use crate::config::Config;
use crate::error::Result;
use crate::types::{DownloadId, Event};
use std::path::{Path, PathBuf};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// Execute the cleanup stage
pub(crate) async fn run_cleanup_stage(
    download_id: DownloadId,
    download_path: &Path,
    event_tx: &broadcast::Sender<Event>,
    config: &Config,
) -> Result<()> {
    debug!(
        download_id = download_id.0,
        ?download_path,
        "running cleanup stage"
    );

    // Emit Cleaning event
    event_tx.send(Event::Cleaning { id: download_id }).ok();

    // Check if cleanup is enabled
    if !config.processing.cleanup.enabled {
        debug!(download_id = download_id.0, "cleanup disabled, skipping");
        return Ok(());
    }

    // Perform cleanup
    cleanup(download_id, download_path, config).await
}

/// Remove intermediate files and sample folders
///
/// This function removes:
/// - Files with target extensions (.par2, .nzb, .sfv, .srr, .nfo)
/// - Archive files after extraction (.rar, .zip, .7z, etc.)
/// - Sample folders (if delete_samples is enabled)
///
/// Errors are logged as warnings but don't cause the cleanup to fail.
///
/// # Arguments
///
/// * `download_id` - The download ID for logging
/// * `download_path` - Path to the download directory to clean
/// * `config` - Configuration for cleanup settings
async fn cleanup(
    download_id: DownloadId,
    download_path: &Path,
    config: &Config,
) -> Result<()> {
    use tokio::fs;

    debug!(
        download_id = download_id.0,
        ?download_path,
        "cleaning up intermediate files"
    );

    // Check if download path exists using async fs
    if fs::metadata(download_path).await.is_err() {
        debug!(
            download_id = download_id.0,
            ?download_path,
            "download path does not exist, skipping cleanup"
        );
        return Ok(());
    }

    // Collect all target extensions (keep original case, compare case-insensitively)
    let target_extensions: Vec<&str> = config
        .processing
        .cleanup
        .target_extensions
        .iter()
        .chain(config.processing.cleanup.archive_extensions.iter())
        .map(|ext| ext.as_str())
        .collect();

    // Recursively walk the directory and collect files/folders to delete
    let mut files_to_delete = Vec::new();
    let mut folders_to_delete = Vec::new();

    collect_cleanup_targets(
        download_path,
        &target_extensions,
        &mut files_to_delete,
        &mut folders_to_delete,
        config,
    )
    .await;

    // Delete files
    let mut deleted_files = 0;
    for file in &files_to_delete {
        match fs::remove_file(file).await {
            Ok(_) => {
                debug!(
                    download_id = download_id.0,
                    ?file,
                    "deleted intermediate file"
                );
                deleted_files += 1;
            }
            Err(e) => {
                warn!(download_id = download_id.0, ?file, error = %e, "failed to delete file");
            }
        }
    }

    // Delete sample folders
    let mut deleted_folders = 0;
    for folder in &folders_to_delete {
        match fs::remove_dir_all(folder).await {
            Ok(_) => {
                debug!(
                    download_id = download_id.0,
                    ?folder,
                    "deleted sample folder"
                );
                deleted_folders += 1;
            }
            Err(e) => {
                warn!(download_id = download_id.0, ?folder, error = %e, "failed to delete folder");
            }
        }
    }

    info!(
        download_id = download_id.0,
        deleted_files, deleted_folders, "cleanup complete"
    );

    Ok(())
}

/// Recursively collect files and folders to delete during cleanup
fn collect_cleanup_targets<'a>(
    path: &'a Path,
    target_extensions: &'a [&'a str],
    files_to_delete: &'a mut Vec<PathBuf>,
    folders_to_delete: &'a mut Vec<PathBuf>,
    config: &'a Config,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        use tokio::fs;

        // Check if this is a sample folder (using async metadata check)
        let is_dir = fs::metadata(path)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false);
        if is_dir
            && config.processing.cleanup.delete_samples
            && let Some(folder_name) = path.file_name().and_then(|n| n.to_str())
        {
            // Check if folder name matches any sample folder names (case-insensitive)
            let is_sample = config
                .processing
                .cleanup
                .sample_folder_names
                .iter()
                .any(|sample_name| folder_name.eq_ignore_ascii_case(sample_name));

            if is_sample {
                // Mark this entire folder for deletion
                folders_to_delete.push(path.to_path_buf());
                return; // Don't recurse into sample folders
            }
        }

        // Read directory entries
        let mut entries = match fs::read_dir(path).await {
            Ok(entries) => entries,
            Err(e) => {
                warn!(?path, error = %e, "failed to read directory during cleanup");
                return;
            }
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let entry_path = entry.path();

            // Get file type from the entry (async, avoids extra syscall)
            let file_type = match entry.file_type().await {
                Ok(ft) => ft,
                Err(_) => continue,
            };

            if file_type.is_file()
                && let Some(extension) = entry_path.extension().and_then(|e| e.to_str())
                && target_extensions
                    .iter()
                    .any(|ext| ext.eq_ignore_ascii_case(extension))
            {
                files_to_delete.push(entry_path);
            } else if file_type.is_dir() {
                // Recursively check subdirectories
                collect_cleanup_targets(
                    &entry_path,
                    target_extensions,
                    files_to_delete,
                    folders_to_delete,
                    config,
                )
                .await;
            }
        }
    })
}
