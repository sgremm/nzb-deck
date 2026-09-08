//! CLI-based PAR2 handler using external par2 binary

use super::parser::{ExitStatus, parse_par2_repair_output, parse_par2_verify_output};
use super::traits::{ParityCapabilities, ParityHandler, RepairResult, VerifyResult};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// CLI-based PAR2 handler using external par2 binary
///
/// This handler executes the external `par2` binary to perform verification
/// and repair operations. It provides full PAR2 functionality including both
/// verification and repair capabilities.
///
/// # Examples
///
/// ```no_run
/// use usenet_dl::parity::{CliParityHandler, ParityHandler};
/// use std::path::{Path, PathBuf};
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Create with explicit path
/// let handler = CliParityHandler::new(PathBuf::from("/usr/bin/par2"));
///
/// // Or auto-discover from PATH
/// let handler = CliParityHandler::from_path()
///     .expect("par2 not found in PATH");
///
/// let result = handler.verify(Path::new("download.par2")).await?;
/// # Ok(())
/// # }
/// ```
pub struct CliParityHandler {
    binary_path: PathBuf,
}

impl CliParityHandler {
    /// Create a new CLI handler with an explicit binary path
    ///
    /// # Arguments
    ///
    /// * `binary_path` - Path to the par2 binary
    pub fn new(binary_path: PathBuf) -> Self {
        Self { binary_path }
    }

    /// Attempt to find par2 in PATH
    ///
    /// Uses the `which` crate to search for the `par2` binary in the system PATH.
    ///
    /// # Returns
    ///
    /// `Some(CliParityHandler)` if the binary is found, `None` otherwise.
    pub fn from_path() -> Option<Self> {
        which::which("par2").ok().map(Self::new)
    }
}

#[async_trait]
impl ParityHandler for CliParityHandler {
    async fn verify(&self, par2_file: &Path) -> crate::Result<VerifyResult> {
        let output = Command::new(&self.binary_path)
            .arg("v") // Verify
            .arg(par2_file)
            .output()
            .await
            .map_err(|e| crate::Error::ExternalTool(format!("Failed to execute par2: {}", e)))?;

        parse_par2_verify_output(
            &output.stdout,
            &output.stderr,
            ExitStatus::from(output.status.success()),
        )
    }

    async fn repair(&self, par2_file: &Path) -> crate::Result<RepairResult> {
        let output = Command::new(&self.binary_path)
            .arg("r") // Repair
            .arg(par2_file)
            .output()
            .await
            .map_err(|e| crate::Error::ExternalTool(format!("Failed to execute par2: {}", e)))?;

        parse_par2_repair_output(
            &output.stdout,
            &output.stderr,
            ExitStatus::from(output.status.success()),
        )
    }

    fn capabilities(&self) -> ParityCapabilities {
        ParityCapabilities {
            can_verify: true,
            can_repair: true,
        }
    }

    fn name(&self) -> &'static str {
        "cli-par2"
    }
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_path_returns_none_for_nonexistent_binary() {
        // This test will pass as long as there's no binary named "nonexistent-par2-binary-xyz"
        let result = which::which("nonexistent-par2-binary-xyz");
        assert!(result.is_err());
    }

    #[test]
    fn test_from_path_binary_discovery() {
        // Test the from_path() method behavior
        // This test doesn't depend on whether par2 is actually installed

        // First, check if par2 exists in PATH using which directly
        let which_result = which::which("par2");

        // Now test from_path() behavior
        let from_path_result = CliParityHandler::from_path();

        match which_result {
            Ok(expected_path) => {
                // If which finds it, from_path should return Some
                assert!(
                    from_path_result.is_some(),
                    "from_path() should return Some when par2 is in PATH"
                );

                let handler = from_path_result.unwrap();
                assert_eq!(
                    handler.binary_path, expected_path,
                    "from_path() should use the path found by which"
                );

                // Verify the handler has correct capabilities
                let caps = handler.capabilities();
                assert!(caps.can_verify, "CLI handler should support verification");
                assert!(caps.can_repair, "CLI handler should support repair");
                assert_eq!(
                    handler.name(),
                    "cli-par2",
                    "CLI handler should have correct name"
                );
            }
            Err(_) => {
                // If which doesn't find it, from_path should return None
                assert!(
                    from_path_result.is_none(),
                    "from_path() should return None when par2 is not in PATH"
                );
            }
        }
    }

    #[test]
    fn test_from_path_consistency_with_which_crate() {
        // Verify that from_path() always returns results consistent with which::which()
        let which_result = which::which("par2");
        let from_path_result = CliParityHandler::from_path();

        // Both should agree on whether the binary exists
        assert_eq!(
            which_result.is_ok(),
            from_path_result.is_some(),
            "from_path() should return Some if and only if which::which() succeeds"
        );
    }

    // Integration tests that require actual par2 binary
    // Run with: cargo test --lib parity::cli -- --ignored

    #[tokio::test]
    #[ignore] // Requires par2 binary in PATH
    async fn test_verify_with_nonexistent_file() {
        let handler = match CliParityHandler::from_path() {
            Some(h) => h,
            None => {
                println!("Skipping test: par2 binary not found in PATH");
                return;
            }
        };

        let result = handler
            .verify(Path::new("/tmp/nonexistent-file.par2"))
            .await;

        // Should fail because the file doesn't exist
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore] // Requires par2 binary in PATH
    async fn test_repair_with_nonexistent_file() {
        let handler = match CliParityHandler::from_path() {
            Some(h) => h,
            None => {
                println!("Skipping test: par2 binary not found in PATH");
                return;
            }
        };

        let result = handler
            .repair(Path::new("/tmp/nonexistent-file.par2"))
            .await;

        // Should fail because the file doesn't exist
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_verify_with_invalid_binary_path() {
        let handler = CliParityHandler::new(PathBuf::from("/nonexistent/path/to/par2"));

        let result = handler.verify(Path::new("test.par2")).await;

        // Should return ExternalTool error
        assert!(result.is_err());
        if let Err(e) = result {
            match e {
                crate::Error::ExternalTool(msg) => {
                    assert!(msg.contains("Failed to execute par2"));
                }
                _ => panic!("Expected ExternalTool error, got: {:?}", e),
            }
        }
    }

    #[tokio::test]
    async fn test_repair_with_invalid_binary_path() {
        let handler = CliParityHandler::new(PathBuf::from("/nonexistent/path/to/par2"));

        let result = handler.repair(Path::new("test.par2")).await;

        // Should return ExternalTool error
        assert!(result.is_err());
        if let Err(e) = result {
            match e {
                crate::Error::ExternalTool(msg) => {
                    assert!(msg.contains("Failed to execute par2"));
                }
                _ => panic!("Expected ExternalTool error, got: {:?}", e),
            }
        }
    }

    // Integration tests with real PAR2 files
    // These tests require the par2 binary in PATH and will create temporary test files
    // Run with: cargo test --lib parity::cli::tests::integration -- --ignored --nocapture

    #[tokio::test]
    #[ignore] // Requires par2 binary in PATH
    async fn integration_test_verify_intact_files() {
        use std::fs;
        use tempfile::TempDir;

        let handler = match CliParityHandler::from_path() {
            Some(h) => h,
            None => {
                println!("Skipping test: par2 binary not found in PATH");
                return;
            }
        };

        // Create temporary directory
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file_path = temp_dir.path().join("test.txt");
        let par2_file_path = temp_dir.path().join("test.txt.par2");

        // Create test file with known content
        let test_content = b"Hello, PAR2! This is test content for verification.\n";
        fs::write(&test_file_path, test_content).expect("Failed to write test file");

        // Create PAR2 recovery data using external par2 binary
        let create_output = tokio::process::Command::new(&handler.binary_path)
            .arg("c") // Create
            .arg("-r10") // 10% recovery
            .arg(&test_file_path)
            .current_dir(temp_dir.path())
            .output()
            .await
            .expect("Failed to create PAR2 file");

        if !create_output.status.success() {
            panic!(
                "Failed to create PAR2 file: {}",
                String::from_utf8_lossy(&create_output.stderr)
            );
        }

        // Verify the intact files
        let result = handler.verify(&par2_file_path).await;

        assert!(result.is_ok(), "Verify should succeed for intact files");
        let verify_result = result.unwrap();

        assert!(
            verify_result.is_complete,
            "Files should be complete and intact"
        );
        assert_eq!(
            verify_result.damaged_blocks, 0,
            "No blocks should be damaged"
        );
        assert!(
            verify_result.recovery_blocks_available > 0,
            "Recovery blocks should be available"
        );
        assert!(
            verify_result.damaged_files.is_empty(),
            "No files should be damaged"
        );
        assert!(
            verify_result.missing_files.is_empty(),
            "No files should be missing"
        );
    }

    #[tokio::test]
    #[ignore] // Requires par2 binary in PATH
    async fn integration_test_verify_damaged_file() {
        use std::fs::{self, OpenOptions};
        use std::io::Write;
        use tempfile::TempDir;

        let handler = match CliParityHandler::from_path() {
            Some(h) => h,
            None => {
                println!("Skipping test: par2 binary not found in PATH");
                return;
            }
        };

        // Create temporary directory
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file_path = temp_dir.path().join("test.txt");
        let par2_file_path = temp_dir.path().join("test.txt.par2");

        // Create test file with known content
        let test_content = b"Hello, PAR2! This is test content that will be damaged.\n";
        fs::write(&test_file_path, test_content).expect("Failed to write test file");

        // Create PAR2 recovery data
        let create_output = tokio::process::Command::new(&handler.binary_path)
            .arg("c")
            .arg("-r20") // 20% recovery for better repair chances
            .arg(&test_file_path)
            .current_dir(temp_dir.path())
            .output()
            .await
            .expect("Failed to create PAR2 file");

        if !create_output.status.success() {
            panic!(
                "Failed to create PAR2 file: {}",
                String::from_utf8_lossy(&create_output.stderr)
            );
        }

        // Damage the test file by corrupting some bytes
        {
            let mut file = OpenOptions::new()
                .write(true)
                .open(&test_file_path)
                .expect("Failed to open test file for corruption");

            // Overwrite the beginning with different content
            file.write_all(b"CORRUPTED DATA!!!")
                .expect("Failed to corrupt file");
        }

        // Verify the damaged file
        let result = handler.verify(&par2_file_path).await;

        assert!(
            result.is_ok(),
            "Verify should succeed even with damaged files"
        );
        let verify_result = result.unwrap();

        assert!(
            !verify_result.is_complete,
            "Files should not be complete (damaged)"
        );
        assert!(
            verify_result.damaged_blocks > 0,
            "Should detect damaged blocks"
        );
        assert!(
            verify_result.recovery_blocks_available > 0,
            "Recovery blocks should be available"
        );
        assert!(
            verify_result.repairable,
            "Damage should be repairable with available recovery data"
        );
    }

    #[tokio::test]
    #[ignore] // Requires par2 binary in PATH
    async fn integration_test_repair_damaged_file() {
        use std::fs::{self, OpenOptions};
        use std::io::Write;
        use tempfile::TempDir;

        let handler = match CliParityHandler::from_path() {
            Some(h) => h,
            None => {
                println!("Skipping test: par2 binary not found in PATH");
                return;
            }
        };

        // Create temporary directory
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file_path = temp_dir.path().join("test.txt");
        let par2_file_path = temp_dir.path().join("test.txt.par2");

        // Create test file with known content
        let original_content =
            b"Hello, PAR2! This is test content that will be damaged and repaired.\n";
        fs::write(&test_file_path, original_content).expect("Failed to write test file");

        // Create PAR2 recovery data with sufficient redundancy
        let create_output = tokio::process::Command::new(&handler.binary_path)
            .arg("c")
            .arg("-r30") // 30% recovery for reliable repair
            .arg(&test_file_path)
            .current_dir(temp_dir.path())
            .output()
            .await
            .expect("Failed to create PAR2 file");

        if !create_output.status.success() {
            panic!(
                "Failed to create PAR2 file: {}",
                String::from_utf8_lossy(&create_output.stderr)
            );
        }

        // Damage the test file
        {
            let mut file = OpenOptions::new()
                .write(true)
                .open(&test_file_path)
                .expect("Failed to open test file for corruption");

            file.write_all(b"CORRUPTED!!!!")
                .expect("Failed to corrupt file");
        }

        // Repair the damaged file
        let result = handler.repair(&par2_file_path).await;

        assert!(result.is_ok(), "Repair should succeed");
        let repair_result = result.unwrap();

        assert!(repair_result.success, "Repair should be successful");
        assert!(
            !repair_result.repaired_files.is_empty() || repair_result.failed_files.is_empty(),
            "Should report repaired files or have no failed files"
        );
        assert!(
            repair_result.error.is_none(),
            "Should have no error message on success"
        );

        // Verify the file is now intact by reading and comparing content
        let repaired_content = fs::read(&test_file_path).expect("Failed to read repaired file");
        assert_eq!(
            repaired_content, original_content,
            "Repaired file should match original content"
        );
    }

    #[tokio::test]
    #[ignore] // Requires par2 binary in PATH
    async fn integration_test_verify_missing_file() {
        use std::fs;
        use tempfile::TempDir;

        let handler = match CliParityHandler::from_path() {
            Some(h) => h,
            None => {
                println!("Skipping test: par2 binary not found in PATH");
                return;
            }
        };

        // Create temporary directory
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file_path = temp_dir.path().join("test.txt");
        let par2_file_path = temp_dir.path().join("test.txt.par2");

        // Create test file
        let test_content = b"This file will be deleted to test missing file detection.\n";
        fs::write(&test_file_path, test_content).expect("Failed to write test file");

        // Create PAR2 recovery data
        let create_output = tokio::process::Command::new(&handler.binary_path)
            .arg("c")
            .arg("-r10")
            .arg(&test_file_path)
            .current_dir(temp_dir.path())
            .output()
            .await
            .expect("Failed to create PAR2 file");

        if !create_output.status.success() {
            panic!(
                "Failed to create PAR2 file: {}",
                String::from_utf8_lossy(&create_output.stderr)
            );
        }

        // Delete the test file to simulate missing file
        fs::remove_file(&test_file_path).expect("Failed to delete test file");

        // Verify with missing file
        let result = handler.verify(&par2_file_path).await;

        assert!(
            result.is_ok(),
            "Verify should succeed and report missing file"
        );
        let verify_result = result.unwrap();

        assert!(
            !verify_result.is_complete,
            "Files should not be complete (file missing)"
        );
        assert!(
            !verify_result.missing_files.is_empty(),
            "Should detect missing files"
        );
        assert!(
            verify_result.repairable,
            "Missing file should be recoverable from PAR2 data"
        );
    }

    #[tokio::test]
    #[ignore] // Requires par2 binary in PATH
    async fn integration_test_repair_missing_file() {
        use std::fs;
        use tempfile::TempDir;

        let handler = match CliParityHandler::from_path() {
            Some(h) => h,
            None => {
                println!("Skipping test: par2 binary not found in PATH");
                return;
            }
        };

        // Create temporary directory
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file_path = temp_dir.path().join("test.txt");
        let par2_file_path = temp_dir.path().join("test.txt.par2");

        // Create test file
        let original_content = b"This file will be deleted and then recovered from PAR2 data.\n";
        fs::write(&test_file_path, original_content).expect("Failed to write test file");

        // Create PAR2 recovery data with high redundancy
        let create_output = tokio::process::Command::new(&handler.binary_path)
            .arg("c")
            .arg("-r50") // 50% recovery to ensure we can restore the file
            .arg(&test_file_path)
            .current_dir(temp_dir.path())
            .output()
            .await
            .expect("Failed to create PAR2 file");

        if !create_output.status.success() {
            panic!(
                "Failed to create PAR2 file: {}",
                String::from_utf8_lossy(&create_output.stderr)
            );
        }

        // Delete the test file
        fs::remove_file(&test_file_path).expect("Failed to delete test file");

        // Repair should restore the missing file
        let result = handler.repair(&par2_file_path).await;

        assert!(result.is_ok(), "Repair should succeed");
        let repair_result = result.unwrap();

        assert!(
            repair_result.success,
            "Repair should successfully restore missing file"
        );
        assert!(
            repair_result.error.is_none(),
            "Should have no error on successful repair"
        );

        // Verify the file was restored with correct content
        assert!(
            test_file_path.exists(),
            "Test file should be restored after repair"
        );
        let restored_content = fs::read(&test_file_path).expect("Failed to read restored file");
        assert_eq!(
            restored_content, original_content,
            "Restored file should match original content"
        );
    }
}
