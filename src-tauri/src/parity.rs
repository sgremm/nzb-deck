use async_trait::async_trait;
use std::path::{Path, PathBuf};
use usenet_dl::Error;
use usenet_dl::parity::{
    ParityCapabilities, ParityHandler, RepairResult as DownloadRepairResult,
    VerifyResult as DownloadVerifyResult,
};

pub struct NativeParityHandler;

fn parse_and_verify(path: &Path) -> Result<rust_par2::VerifyResult, String> {
    let file_set = rust_par2::parse(path).map_err(|error| error.to_string())?;
    let directory = path
        .parent()
        .ok_or_else(|| "PAR2 file has no parent directory".to_string())?;
    Ok(rust_par2::verify(&file_set, directory))
}

#[async_trait]
impl ParityHandler for NativeParityHandler {
    async fn verify(&self, par2_file: &Path) -> usenet_dl::Result<DownloadVerifyResult> {
        let path = par2_file.to_path_buf();
        let result = tokio::task::spawn_blocking(move || parse_and_verify(&path))
            .await
            .map_err(|error| Error::Other(format!("PAR2 verification task failed: {error}")))?
            .map_err(|error| Error::Other(format!("PAR2 verification failed: {error}")))?;

        Ok(DownloadVerifyResult {
            is_complete: result.all_correct(),
            damaged_blocks: result.blocks_needed(),
            recovery_blocks_available: result.recovery_blocks_available,
            repairable: result.repair_possible,
            damaged_files: result
                .damaged
                .iter()
                .map(|file| file.filename.clone())
                .collect(),
            missing_files: result
                .missing
                .iter()
                .map(|file| file.filename.clone())
                .collect(),
        })
    }

    async fn repair(&self, par2_file: &Path) -> usenet_dl::Result<DownloadRepairResult> {
        let path = PathBuf::from(par2_file);
        tokio::task::spawn_blocking(move || {
            let file_set = rust_par2::parse(&path)
                .map_err(|error| Error::Other(format!("PAR2 parse failed: {error}")))?;
            let directory = path
                .parent()
                .ok_or_else(|| Error::Other("PAR2 file has no parent directory".to_string()))?;
            let verification = rust_par2::verify(&file_set, directory);
            let repaired_files: Vec<String> = verification
                .damaged
                .iter()
                .map(|file| file.filename.clone())
                .chain(
                    verification
                        .missing
                        .iter()
                        .map(|file| file.filename.clone()),
                )
                .collect();

            match rust_par2::repair_from_verify(&file_set, directory, &verification) {
                Ok(result) => Ok(DownloadRepairResult {
                    success: result.success,
                    repaired_files,
                    failed_files: Vec::new(),
                    error: (!result.success).then_some(result.message),
                }),
                Err(error) => Ok(DownloadRepairResult {
                    success: false,
                    repaired_files: Vec::new(),
                    failed_files: repaired_files,
                    error: Some(error.to_string()),
                }),
            }
        })
        .await
        .map_err(|error| Error::Other(format!("PAR2 repair task failed: {error}")))?
    }

    fn capabilities(&self) -> ParityCapabilities {
        ParityCapabilities {
            can_verify: true,
            can_repair: true,
        }
    }

    fn name(&self) -> &'static str {
        "rust-par2"
    }
}
