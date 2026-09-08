//! PAR2 verification stage

use crate::error::Result;
use crate::parity::ParityHandler;
use crate::types::{DownloadId, Event};
use std::path::{Path, PathBuf};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use super::PostProcessError;

/// Execute the verify stage
///
/// Returns `Ok(true)` if files are damaged but repairable, `Ok(false)` if
/// files are intact (or verification was skipped/not supported).
pub(crate) async fn run_verify_stage(
    download_id: DownloadId,
    download_path: &Path,
    event_tx: &broadcast::Sender<Event>,
    parity_handler: &dyn ParityHandler,
) -> Result<bool> {
    debug!(
        download_id = download_id.0,
        ?download_path,
        "running verify stage"
    );

    // Emit Verifying event
    event_tx.send(Event::Verifying { id: download_id }).ok();

    // Find PAR2 files in download directory
    let par2_files = find_par2_files(download_path).await?;

    if par2_files.is_empty() {
        debug!(
            download_id = download_id.0,
            "no PAR2 files found, skipping verification"
        );

        // Emit VerifyComplete event (no damage detected, but also no verification)
        event_tx
            .send(Event::VerifyComplete {
                id: download_id,
                damaged: false,
            })
            .ok();

        return Ok(false);
    }

    // Use the first PAR2 file found (typically the .par2 file, not .vol files)
    let par2_file = &par2_files[0];
    debug!(
        download_id = download_id.0,
        ?par2_file,
        "verifying with PAR2 file"
    );

    // Call parity handler to verify
    let verify_result = match parity_handler.verify(par2_file).await {
        Ok(result) => result,
        Err(crate::Error::NotSupported(ref msg)) => {
            warn!(
                download_id = download_id.0,
                ?par2_file,
                "PAR2 verification not supported: {}",
                msg
            );

            // Emit VerifyComplete event (skipped, assume no damage)
            event_tx
                .send(Event::VerifyComplete {
                    id: download_id,
                    damaged: false,
                })
                .ok();

            return Ok(false);
        }
        Err(e) => return Err(e),
    };

    info!(
        download_id = download_id.0,
        is_complete = verify_result.is_complete,
        damaged_blocks = verify_result.damaged_blocks,
        recovery_blocks = verify_result.recovery_blocks_available,
        repairable = verify_result.repairable,
        "PAR2 verification complete"
    );

    // Emit VerifyComplete event
    event_tx
        .send(Event::VerifyComplete {
            id: download_id,
            damaged: !verify_result.is_complete,
        })
        .ok();

    // If files are damaged and not repairable, fail immediately
    if !verify_result.is_complete && !verify_result.repairable {
        return Err(PostProcessError::VerificationFailed {
            id: download_id.into(),
            reason: format!(
                "files are damaged ({} blocks) but cannot be repaired (need {} more recovery blocks)",
                verify_result.damaged_blocks,
                verify_result.damaged_blocks.saturating_sub(verify_result.recovery_blocks_available)
            ),
        }
        .into());
    }

    // Return whether files are damaged (but repairable)
    Ok(!verify_result.is_complete)
}

/// Find all PAR2 files in the download directory
async fn find_par2_files(download_path: &Path) -> Result<Vec<PathBuf>> {
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
    // Base files typically end in just .par2, while vol files have .vol##-##.par2
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
            (false, true) => std::cmp::Ordering::Less, // a is base file, prefer it
            (true, false) => std::cmp::Ordering::Greater, // b is base file, prefer it
            _ => a.cmp(b),                             // both same type, alphabetical
        }
    });

    Ok(par2_files)
}
