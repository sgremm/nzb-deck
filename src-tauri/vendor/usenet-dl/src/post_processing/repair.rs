//! PAR2 repair stage

use crate::error::Result;
use crate::parity::ParityHandler;
use crate::types::{DownloadId, Event};
use std::path::Path;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use super::PostProcessError;

/// Find all PAR2 files in the download directory
///
/// This is duplicated from verify.rs to keep the modules independent
async fn find_par2_files(download_path: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut par2_files = Vec::new();

    let mut entries = tokio::fs::read_dir(download_path)
        .await
        .map_err(|e| std::io::Error::other(format!("failed to read directory: {}", e)))?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        let metadata = entry.metadata().await?;
        if metadata.is_file()
            && let Some(ext) = path.extension()
            && ext.eq_ignore_ascii_case("par2")
        {
            par2_files.push(path);
        }
    }

    // Sort to prioritize base .par2 files over .vol files
    par2_files.sort_by(|a, b| {
        let a_is_vol = a
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.contains(".vol"))
            .unwrap_or(false);
        let b_is_vol = b
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.contains(".vol"))
            .unwrap_or(false);

        match (a_is_vol, b_is_vol) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => a.cmp(b),
        }
    });

    Ok(par2_files)
}

/// Execute the repair stage
pub(crate) async fn run_repair_stage(
    download_id: DownloadId,
    download_path: &Path,
    event_tx: &broadcast::Sender<Event>,
    parity_handler: &dyn ParityHandler,
) -> Result<()> {
    debug!(
        download_id = download_id.0,
        ?download_path,
        "running repair stage"
    );

    // Find PAR2 files in download directory
    let par2_files = find_par2_files(download_path).await?;

    if par2_files.is_empty() {
        debug!(
            download_id = download_id.0,
            "no PAR2 files found, skipping repair"
        );
        return Ok(());
    }

    // Use the first PAR2 file found (typically the .par2 file, not .vol files)
    let par2_file = &par2_files[0];
    debug!(
        download_id = download_id.0,
        ?par2_file,
        "repairing with PAR2 file"
    );

    // First verify to get block counts for event emission
    let verify_result = match parity_handler.verify(par2_file).await {
        Ok(result) => result,
        Err(crate::Error::NotSupported(ref msg)) => {
            warn!(
                download_id = download_id.0,
                ?par2_file,
                "PAR2 verification not supported (skipping repair): {}",
                msg
            );

            // Emit RepairSkipped event
            event_tx
                .send(Event::RepairSkipped {
                    id: download_id,
                    reason: format!("PAR2 verification not supported: {}", msg),
                })
                .ok();

            return Ok(());
        }
        Err(e) => return Err(e),
    };

    // Emit Repairing event
    event_tx
        .send(Event::Repairing {
            id: download_id,
            blocks_needed: verify_result.damaged_blocks,
            blocks_available: verify_result.recovery_blocks_available,
        })
        .ok();

    // Call parity handler to repair
    let repair_result = match parity_handler.repair(par2_file).await {
        Ok(result) => result,
        Err(crate::Error::NotSupported(ref msg)) => {
            warn!(
                download_id = download_id.0,
                ?par2_file,
                "PAR2 repair not supported: {}",
                msg
            );

            // Emit RepairSkipped event
            event_tx
                .send(Event::RepairSkipped {
                    id: download_id,
                    reason: msg.clone(),
                })
                .ok();

            return Ok(());
        }
        Err(e) => return Err(e),
    };

    info!(
        download_id = download_id.0,
        success = repair_result.success,
        repaired_files = repair_result.repaired_files.len(),
        failed_files = repair_result.failed_files.len(),
        "PAR2 repair complete"
    );

    // Emit RepairComplete event
    event_tx
        .send(Event::RepairComplete {
            id: download_id,
            success: repair_result.success,
        })
        .ok();

    // If repair failed, return error
    if !repair_result.success {
        return Err(PostProcessError::RepairFailed {
            id: download_id.into(),
            reason: repair_result.error.unwrap_or_else(|| {
                format!(
                    "repair failed for {} file(s): {}",
                    repair_result.failed_files.len(),
                    repair_result.failed_files.join(", ")
                )
            }),
        }
        .into());
    }

    Ok(())
}
