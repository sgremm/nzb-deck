//! PAR2 verification stage

use crate::error::Result;
use crate::parity::ParityHandler;
use crate::types::{DownloadId, Event};
use std::path::{Path, PathBuf};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use super::PostProcessError;

/// PAR2 packet magic ("PAR2\0PKT") found at offset 8 of every PAR2 file.
const PAR2_MAGIC: &[u8; 8] = b"PAR2\0PKT";

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

    // Restore real filenames from PAR2 metadata before verification. Usenet posts
    // frequently obfuscate filenames; without the real names no archive can be
    // detected and verification cannot match files against the recovery set.
    deobfuscate_from_par2(download_path).await?;

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

/// Restore real filenames using PAR2 metadata and normalize PAR2 extensions.
///
/// Scans every file in `download_path` for the PAR2 packet magic (independent of
/// filename), parses the most complete recovery set, and renames matching source
/// files from their obfuscated names to the names recorded in the set. Files that
/// carry PAR2 data but no `.par2` extension are given one so verification and
/// repair can find every recovery volume.
async fn deobfuscate_from_par2(download_path: &Path) -> Result<()> {
    let mut entries = match tokio::fs::read_dir(download_path).await {
        Ok(entries) => entries,
        Err(e) => {
            return Err(crate::error::Error::Io(std::io::Error::other(format!(
                "failed to read directory for deobfuscation: {e}"
            ))));
        }
    };

    let mut ordinary_files = Vec::new();
    let mut par2_candidates = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let metadata = entry.metadata().await?;
        if !metadata.is_file() {
            continue;
        }
        match is_par2_magic(&path) {
            Ok(true) => par2_candidates.push(path),
            Ok(false) => ordinary_files.push(path),
            Err(e) => warn!(?path, error = %e, "could not inspect file for PAR2 magic"),
        }
    }

    if par2_candidates.is_empty() {
        return Ok(());
    }
    info!(
        par2_candidates = par2_candidates.len(),
        "PAR2 data found, restoring filenames from recovery set"
    );

    // Parse every PAR2 file; the index carries the fullest file description.
    let mut parsed = Vec::new();
    for path in &par2_candidates {
        match rust_par2::parse(path) {
            Ok(file_set) => parsed.push((path.clone(), file_set)),
            Err(e) => warn!(?path, error = %e, "failed to parse PAR2 file"),
        }
    }
    let Some((index_path, index_set)) = parsed.iter().max_by_key(|(_, set)| set.files.len()) else {
        return Ok(());
    };

    // Match source files by size; disambiguate equal sizes with the 16 KiB hash.
    let mut unmatched: Vec<PathBuf> = ordinary_files
        .into_iter()
        .filter(|path| path != index_path)
        .collect();

    let mut sources: Vec<&rust_par2::types::Par2File> = index_set.files.values().collect();
    sources.sort_by_key(|file| std::cmp::Reverse(file.size));

    let mut restored = 0usize;
    for source in sources {
        let Some(candidate) = find_matching_file(&source, &unmatched) else {
            continue;
        };
        let target = safe_target_name(download_path, &source.filename);
        if target == candidate {
            continue;
        }
        if std::fs::rename(&candidate, &target).is_ok() {
            restored += 1;
            info!(from = %candidate.display(), to = %target.display(), "restored filename from PAR2 metadata");
            unmatched.retain(|path| path != &candidate);
        }
    }

    // Make sure every PAR2 file is discoverable by extension-based scans.
    for path in par2_candidates {
        let has_par2_extension = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("par2"));
        if !has_par2_extension {
            let target = path.with_extension("par2");
            if std::fs::rename(&path, &target).is_ok() {
                info!(from = %path.display(), to = %target.display(), "normalized PAR2 extension");
            }
        }
    }

    if restored > 0 {
        info!(
            restored,
            "restored {restored} filename(s) from PAR2 metadata"
        );
    }
    Ok(())
}

fn is_par2_magic(path: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut header = [0u8; 16];
    let read = file.read(&mut header)?;
    Ok(read == 16 && &header[8..16] == PAR2_MAGIC)
}

fn find_matching_file(
    source: &rust_par2::types::Par2File,
    candidates: &[PathBuf],
) -> Option<PathBuf> {
    let by_size = candidates
        .iter()
        .filter(|path| {
            std::fs::metadata(path)
                .map(|metadata| metadata.len() == source.size)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    if by_size.is_empty() {
        return None;
    }
    if by_size.len() == 1 {
        return Some(by_size[0].clone());
    }
    by_size
        .into_iter()
        .find(|path| {
            rust_par2::compute_hash_16k(path)
                .map(|hash| hash == source.hash_16k)
                .unwrap_or(false)
        })
        .cloned()
}

fn safe_target_name(directory: &Path, metadata_name: &str) -> PathBuf {
    let file_name = Path::new(metadata_name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("restored.dat");
    directory.join(file_name)
}
