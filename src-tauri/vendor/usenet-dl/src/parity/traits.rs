//! Traits and types for PAR2 parity handling

use async_trait::async_trait;
use std::path::Path;

/// Result of PAR2 verification
#[must_use]
#[derive(Debug, Clone)]
pub struct VerifyResult {
    /// Whether all files are intact
    pub is_complete: bool,
    /// Number of damaged/missing blocks
    pub damaged_blocks: u32,
    /// Number of recovery blocks available
    pub recovery_blocks_available: u32,
    /// Whether repair is possible with available recovery data
    pub repairable: bool,
    /// List of damaged files
    pub damaged_files: Vec<String>,
    /// List of missing files
    pub missing_files: Vec<String>,
}

/// Result of PAR2 repair
#[must_use]
#[derive(Debug, Clone)]
pub struct RepairResult {
    /// Whether repair was successful
    pub success: bool,
    /// Files that were repaired
    pub repaired_files: Vec<String>,
    /// Files that could not be repaired
    pub failed_files: Vec<String>,
    /// Error message if repair failed
    pub error: Option<String>,
}

/// Capabilities of a parity handler implementation
#[derive(Debug, Clone, Copy)]
pub struct ParityCapabilities {
    /// Can verify file integrity
    pub can_verify: bool,
    /// Can repair damaged files
    pub can_repair: bool,
}

/// Trait for PAR2 parity handling
///
/// This trait defines the interface for PAR2 verification and repair operations.
/// Implementations can use external binaries, pure Rust libraries, or provide
/// stub functionality for graceful degradation.
///
/// # Examples
///
/// ```no_run
/// use usenet_dl::parity::{CliParityHandler, ParityHandler};
/// use std::path::Path;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let handler = CliParityHandler::from_path()
///     .expect("par2 binary not found");
///
/// let result = handler.verify(Path::new("download.par2")).await?;
/// if !result.is_complete && result.repairable {
///     let repair = handler.repair(Path::new("download.par2")).await?;
///     println!("Repair successful: {}", repair.success);
/// }
/// # Ok(())
/// # }
/// ```
#[async_trait]
pub trait ParityHandler: Send + Sync {
    /// Verify integrity of files using PAR2
    ///
    /// # Arguments
    ///
    /// * `par2_file` - Path to the .par2 file
    ///
    /// # Returns
    ///
    /// A `VerifyResult` containing information about file integrity and
    /// whether repair is possible.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The PAR2 file cannot be read or parsed
    /// - The external binary fails to execute (for CLI implementations)
    /// - The operation is not supported (for stub implementations)
    async fn verify(&self, par2_file: &Path) -> crate::Result<VerifyResult>;

    /// Attempt to repair damaged files using PAR2 recovery data
    ///
    /// # Arguments
    ///
    /// * `par2_file` - Path to the .par2 file
    ///
    /// # Returns
    ///
    /// A `RepairResult` indicating success and which files were repaired.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The PAR2 file cannot be read or parsed
    /// - Not enough recovery blocks are available
    /// - The external binary fails to execute (for CLI implementations)
    /// - The operation is not supported (for stub implementations)
    async fn repair(&self, par2_file: &Path) -> crate::Result<RepairResult>;

    /// Query capabilities of this handler
    ///
    /// Returns information about what operations this handler supports.
    /// Useful for UI/API to determine what functionality is available.
    fn capabilities(&self) -> ParityCapabilities;

    /// Human-readable name for logging
    ///
    /// Returns a string identifying this handler implementation,
    /// useful for debugging and capability reporting.
    fn name(&self) -> &'static str;
}
