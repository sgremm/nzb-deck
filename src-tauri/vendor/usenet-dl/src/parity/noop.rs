//! No-op PAR2 handler for graceful degradation

use super::traits::{ParityCapabilities, ParityHandler, RepairResult, VerifyResult};
use async_trait::async_trait;
use std::path::Path;

/// No-op PAR2 handler used when PAR2 support is unavailable
///
/// This handler is used when no external PAR2 binary is available or configured.
/// It provides graceful degradation by returning `Error::NotSupported` for both
/// verification and repair operations.
///
/// This allows the post-processing pipeline to continue even when PAR2
/// functionality is not available.
///
/// # Examples
///
/// ```
/// use usenet_dl::parity::{NoOpParityHandler, ParityHandler};
/// use std::path::Path;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let handler = NoOpParityHandler;
///
/// // Verify returns NotSupported error
/// let verify_result = handler.verify(Path::new("download.par2")).await;
/// assert!(verify_result.is_err());
///
/// // Repair returns NotSupported error
/// let repair_result = handler.repair(Path::new("download.par2")).await;
/// assert!(repair_result.is_err());
/// # Ok(())
/// # }
/// ```
pub struct NoOpParityHandler;

#[async_trait]
impl ParityHandler for NoOpParityHandler {
    async fn verify(&self, _par2_file: &Path) -> crate::Result<VerifyResult> {
        Err(crate::Error::NotSupported(
            "PAR2 verification requires external par2 binary. \
             Configure par2_path in config or ensure par2 is in PATH."
                .into(),
        ))
    }

    async fn repair(&self, _par2_file: &Path) -> crate::Result<RepairResult> {
        Err(crate::Error::NotSupported(
            "PAR2 repair requires external par2 binary. \
             Configure par2_path in config or ensure par2 is in PATH."
                .into(),
        ))
    }

    fn capabilities(&self) -> ParityCapabilities {
        ParityCapabilities {
            can_verify: false,
            can_repair: false,
        }
    }

    fn name(&self) -> &'static str {
        "noop"
    }
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_verify_returns_not_supported() {
        let handler = NoOpParityHandler;
        let result = handler.verify(Path::new("test.par2")).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(crate::Error::NotSupported(_))));
    }

    #[tokio::test]
    async fn test_repair_returns_not_supported() {
        let handler = NoOpParityHandler;
        let result = handler.repair(Path::new("test.par2")).await;
        assert!(result.is_err());
        match result {
            Err(crate::Error::NotSupported(msg)) => {
                assert!(msg.contains("par2 binary"));
            }
            _ => panic!("Expected NotSupported error"),
        }
    }

    #[tokio::test]
    async fn test_repair_error_message_content() {
        let handler = NoOpParityHandler;
        let result = handler.repair(Path::new("test.par2")).await;

        match result {
            Err(crate::Error::NotSupported(msg)) => {
                // Verify error message contains key information
                assert!(
                    msg.contains("PAR2 repair"),
                    "Error message should mention PAR2 repair"
                );
                assert!(
                    msg.contains("external par2 binary"),
                    "Error message should mention external binary requirement"
                );
                assert!(
                    msg.contains("par2_path") || msg.contains("PATH"),
                    "Error message should mention configuration or PATH"
                );
            }
            _ => panic!("Expected NotSupported error"),
        }
    }

    #[tokio::test]
    async fn test_repair_error_is_not_supported_variant() {
        let handler = NoOpParityHandler;
        let result = handler.repair(Path::new("test.par2")).await;

        // Verify the error is specifically NotSupported, not another variant
        assert!(matches!(result, Err(crate::Error::NotSupported(_))));
    }
}
