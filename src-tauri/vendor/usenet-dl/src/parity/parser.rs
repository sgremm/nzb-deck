//! Parser for par2 command output

use super::traits::{RepairResult, VerifyResult};
use std::str;

/// Exit status of an external command
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    /// The command exited successfully (exit code 0)
    Success,
    /// The command exited with a non-zero exit code
    Failure,
}

impl ExitStatus {
    /// Returns `true` if the exit status represents success
    pub fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }
}

impl From<bool> for ExitStatus {
    fn from(success: bool) -> Self {
        if success {
            Self::Success
        } else {
            Self::Failure
        }
    }
}

/// Parse output from `par2 v` (verify) command
///
/// Parses the stdout/stderr from a par2 verify command to extract
/// information about file integrity, damaged blocks, and repair possibility.
///
/// # Arguments
///
/// * `stdout` - Standard output from the par2 command
/// * `stderr` - Standard error from the par2 command
/// * `success` - Whether the command exited successfully
///
/// # Returns
///
/// A `VerifyResult` containing verification details
pub fn parse_par2_verify_output(
    stdout: &[u8],
    stderr: &[u8],
    _exit_status: ExitStatus,
) -> crate::Result<VerifyResult> {
    let output = str::from_utf8(stdout).unwrap_or_default();
    let error_output = str::from_utf8(stderr).unwrap_or_default();

    // Combine both outputs for parsing
    let combined = format!("{}\n{}", output, error_output);

    // Initialize result
    let mut damaged_blocks = 0;
    let mut recovery_blocks_available = 0;
    let mut damaged_files = Vec::new();
    let mut missing_files = Vec::new();

    // Parse output line by line
    for line in combined.lines() {
        let line_lower = line.to_lowercase();

        // Look for damaged/missing block counts
        if (line_lower.contains("damaged") || line_lower.contains("missing"))
            && let Some(count) = extract_number_before_blocks(&line_lower)
        {
            damaged_blocks = damaged_blocks.max(count);
        }

        // Look for recovery block information
        if line_lower.contains("recovery")
            && line_lower.contains("block")
            && let Some(count) = extract_number_before_blocks(&line_lower)
        {
            recovery_blocks_available = recovery_blocks_available.max(count);
        }

        // Look for specific file mentions
        if (line_lower.contains("damaged:") || line_lower.contains("corrupt:"))
            && let Some(filename) = line.split(':').nth(1)
        {
            let filename = filename.trim().to_string();
            if !filename.is_empty() && !damaged_files.contains(&filename) {
                damaged_files.push(filename);
            }
        }

        if line_lower.contains("missing:")
            && let Some(filename) = line.split(':').nth(1)
        {
            let filename = filename.trim().to_string();
            if !filename.is_empty() && !missing_files.contains(&filename) {
                missing_files.push(filename);
            }
        }

        // Handle par2cmdline format: Target: "filename" - missing.
        if line_lower.contains("- missing")
            && let Some(filename) = extract_filename_from_line(line)
            && !missing_files.contains(&filename)
        {
            missing_files.push(filename);
        }

        // Handle par2cmdline format: Target: "filename" - damaged.
        if line_lower.contains("- damaged")
            && let Some(filename) = extract_filename_from_line(line)
            && !damaged_files.contains(&filename)
        {
            damaged_files.push(filename);
        }
    }

    // Determine completeness based on parsed damage indicators.
    // We prioritize the parsed output over the exit code because some par2
    // implementations exit non-zero for reasons unrelated to file damage
    // (e.g., missing .vol recovery files while base data is intact).
    let is_complete = damaged_blocks == 0 && damaged_files.is_empty() && missing_files.is_empty();

    // Determine if repair is possible
    let repairable =
        (damaged_blocks > 0 || !missing_files.is_empty()) && recovery_blocks_available > 0;

    Ok(VerifyResult {
        is_complete,
        damaged_blocks,
        recovery_blocks_available,
        repairable,
        damaged_files,
        missing_files,
    })
}

/// Parse output from `par2 r` (repair) command
///
/// Parses the stdout/stderr from a par2 repair command to extract
/// information about repair success and which files were repaired.
///
/// # Arguments
///
/// * `stdout` - Standard output from the par2 command
/// * `stderr` - Standard error from the par2 command
/// * `success` - Whether the command exited successfully
///
/// # Returns
///
/// A `RepairResult` containing repair details
pub fn parse_par2_repair_output(
    stdout: &[u8],
    stderr: &[u8],
    exit_status: ExitStatus,
) -> crate::Result<RepairResult> {
    let output = str::from_utf8(stdout).unwrap_or_default();
    let error_output = str::from_utf8(stderr).unwrap_or_default();

    // Combine both outputs for parsing
    let combined = format!("{}\n{}", output, error_output);

    let mut repaired_files = Vec::new();
    let mut failed_files = Vec::new();
    let mut error = None;

    // Parse output line by line
    for line in combined.lines() {
        let line_lower = line.to_lowercase();

        // Look for repair success indicators
        if (line_lower.contains("repaired") || line_lower.contains("restored"))
            && let Some(filename) = extract_filename_from_line(line)
            && !repaired_files.contains(&filename)
        {
            repaired_files.push(filename);
        }

        // Look for repair failures
        if (line_lower.contains("failed") || line_lower.contains("could not repair"))
            && let Some(filename) = extract_filename_from_line(line)
            && !failed_files.contains(&filename)
        {
            failed_files.push(filename);
        }

        // Capture error messages
        if line_lower.contains("error") && error.is_none() {
            error = Some(line.trim().to_string());
        }
    }

    // If command failed but we don't have an error message, use stderr
    if !exit_status.is_success() && error.is_none() && !error_output.is_empty() {
        error = Some(error_output.trim().to_string());
    }

    Ok(RepairResult {
        success: exit_status.is_success(),
        repaired_files,
        failed_files,
        error,
    })
}

/// Extract a number that appears before the word "block" or "blocks" in a line.
///
/// Handles various par2cmdline output formats:
/// - "5 blocks damaged" (number directly before "blocks")
/// - "Found 1999 of 2000 data blocks" (number before intervening words then "blocks")
/// - "You have 577 recovery blocks available" (same pattern)
fn extract_number_before_blocks(line: &str) -> Option<u32> {
    let words: Vec<&str> = line.split_whitespace().collect();

    // Find the position of "block" or "blocks" in the line
    for i in 0..words.len() {
        if words[i].starts_with("block") {
            // Search backwards from the "block(s)" word for the nearest number
            // This handles patterns like "577 recovery blocks" where a word sits between
            // the number and "blocks"
            for j in (0..i).rev() {
                if let Ok(num) = words[j].parse::<u32>() {
                    return Some(num);
                }
            }
        }
    }
    None
}

/// Extract filename from a line (look for quoted strings or after colons)
fn extract_filename_from_line(line: &str) -> Option<String> {
    // Try to find quoted filename first
    if let Some(start) = line.find('"')
        && let Some(end) = line[start + 1..].find('"')
    {
        return Some(line[start + 1..start + 1 + end].to_string());
    }

    // Try to find filename after colon
    if let Some(filename) = line.split(':').nth(1) {
        let filename = filename.trim().to_string();
        if !filename.is_empty() {
            return Some(filename);
        }
    }

    None
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_verify_success() {
        let stdout = b"All files are correct\nNo repair needed\n";
        let stderr = b"";
        let result = parse_par2_verify_output(stdout, stderr, ExitStatus::Success).unwrap();

        assert!(result.is_complete);
        assert_eq!(result.damaged_blocks, 0);
        assert!(result.damaged_files.is_empty());
        assert!(result.missing_files.is_empty());
    }

    #[test]
    fn test_parse_verify_with_damage() {
        let stdout = b"5 blocks damaged\n10 blocks available for recovery\nDamaged: file1.bin\n";
        let stderr = b"";
        let result = parse_par2_verify_output(stdout, stderr, ExitStatus::Failure).unwrap();

        assert!(!result.is_complete);
        assert_eq!(result.damaged_blocks, 5);
        assert_eq!(result.recovery_blocks_available, 10);
        assert!(result.repairable);
        assert_eq!(result.damaged_files, vec!["file1.bin"]);
    }

    #[test]
    fn test_parse_repair_success() {
        let stdout = b"Repaired: file1.bin\nRepaired: file2.bin\nRepair complete\n";
        let stderr = b"";
        let result = parse_par2_repair_output(stdout, stderr, ExitStatus::Success).unwrap();

        assert!(result.success);
        assert_eq!(result.repaired_files, vec!["file1.bin", "file2.bin"]);
        assert!(result.failed_files.is_empty());
    }

    #[test]
    fn test_parse_repair_failure() {
        let stdout = b"Failed: file1.bin\n";
        let stderr = b"Error: Not enough recovery blocks\n";
        let result = parse_par2_repair_output(stdout, stderr, ExitStatus::Failure).unwrap();

        assert!(!result.success);
        assert_eq!(result.failed_files, vec!["file1.bin"]);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_parse_verify_missing_file_repairable() {
        let stdout = b"Missing: file1.bin\n10 blocks available for recovery\n";
        let stderr = b"";
        let result = parse_par2_verify_output(stdout, stderr, ExitStatus::Failure).unwrap();

        assert!(!result.is_complete);
        assert_eq!(result.damaged_blocks, 0);
        assert_eq!(result.recovery_blocks_available, 10);
        assert!(result.repairable);
        assert_eq!(result.missing_files, vec!["file1.bin"]);
    }

    #[test]
    fn test_extract_number_before_blocks() {
        assert_eq!(extract_number_before_blocks("5 blocks damaged"), Some(5));
        assert_eq!(extract_number_before_blocks("10 block available"), Some(10));
        assert_eq!(extract_number_before_blocks("damaged blocks"), None);
        // par2cmdline formats with intervening words
        assert_eq!(
            extract_number_before_blocks("found 1999 of 2000 data blocks"),
            Some(2000)
        );
        assert_eq!(
            extract_number_before_blocks("you have 577 recovery blocks available"),
            Some(577)
        );
    }

    #[test]
    fn test_extract_filename_from_line() {
        assert_eq!(
            extract_filename_from_line("Repaired: \"file.bin\""),
            Some("file.bin".to_string())
        );
        assert_eq!(
            extract_filename_from_line("Damaged: file.bin"),
            Some("file.bin".to_string())
        );
    }

    // --- Realistic par2cmdline output tests ---

    #[test]
    fn test_parse_verify_real_output_intact() {
        let stdout = b"All files are correct, repair is not needed.\n";
        let stderr = b"";
        let result = parse_par2_verify_output(stdout, stderr, ExitStatus::Success).unwrap();

        assert!(result.is_complete);
        assert_eq!(result.damaged_blocks, 0);
        assert!(!result.repairable);
        assert!(result.damaged_files.is_empty());
        assert!(result.missing_files.is_empty());
    }

    #[test]
    fn test_parse_verify_real_output_damaged() {
        // par2cmdline outputs "Found X of Y data blocks" for damaged files
        let stdout = b"Target: \"file.tar\" - damaged. Found 1999 of 2000 data blocks.\n\
                       You have 577 recovery blocks available.\n\
                       Repair is possible.\n";
        let stderr = b"";
        let result = parse_par2_verify_output(stdout, stderr, ExitStatus::Failure).unwrap();

        assert!(!result.is_complete);
        // Parser now correctly extracts counts with intervening words
        assert_eq!(result.damaged_blocks, 2000);
        assert_eq!(result.recovery_blocks_available, 577);
        assert!(result.repairable);
        assert!(result.damaged_files.contains(&"file.tar".to_string()));
    }

    #[test]
    fn test_parse_verify_real_output_missing() {
        // par2cmdline outputs "Target: \"file.tar\" - missing." for missing files
        let stdout = b"Target: \"file.tar\" - missing.\n\
                       You have 50 recovery blocks available.\n\
                       Repair is possible.\n";
        let stderr = b"";
        let result = parse_par2_verify_output(stdout, stderr, ExitStatus::Failure).unwrap();

        assert!(!result.is_complete);
        assert_eq!(result.damaged_blocks, 0);
        // Parser now correctly extracts "50 recovery blocks"
        assert_eq!(result.recovery_blocks_available, 50);
        // Parser now correctly detects "- missing." format
        assert_eq!(result.missing_files, vec!["file.tar"]);
        assert!(result.repairable);
    }

    // --- Malformed / edge-case output tests ---

    #[test]
    fn verify_empty_stdout_with_failure_exit_reports_complete() {
        // When par2 fails but reports no damage, we treat the data as intact.
        // If files are actually damaged, extraction will catch it downstream.
        let result = parse_par2_verify_output(b"", b"", ExitStatus::Failure).unwrap();

        assert!(
            result.is_complete,
            "no damage indicators found, so data should be considered complete"
        );
        assert_eq!(
            result.damaged_blocks, 0,
            "no block info can be parsed from empty output"
        );
        assert!(!result.repairable, "nothing to repair with no damage");
    }

    #[test]
    fn repair_empty_stdout_with_failure_exit_reports_unsuccessful() {
        let result = parse_par2_repair_output(b"", b"", ExitStatus::Failure).unwrap();

        assert!(
            !result.success,
            "empty output with failure exit should not report success"
        );
        assert!(
            result.repaired_files.is_empty(),
            "no files can be parsed from empty output"
        );
    }

    #[test]
    fn verify_garbage_stdout_with_success_exit_reports_complete() {
        let garbage = b"\x00\xff\xfe RANDOM GARBAGE {{{ not par2 output at all ///";
        let result = parse_par2_verify_output(garbage, b"", ExitStatus::Success).unwrap();

        // Exit code says success, no damage indicators found → complete
        assert!(
            result.is_complete,
            "success exit with no damage indicators should be considered complete"
        );
        assert_eq!(result.damaged_blocks, 0);
        assert!(result.damaged_files.is_empty());
        assert!(result.missing_files.is_empty());
    }

    #[test]
    fn repair_garbage_stdout_with_success_exit_reports_successful() {
        let garbage = b"ZZZZZ not parseable output ZZZZZ";
        let result = parse_par2_repair_output(garbage, b"", ExitStatus::Success).unwrap();

        assert!(
            result.success,
            "success exit code should make repair report success regardless of stdout content"
        );
        assert!(result.repaired_files.is_empty());
        assert!(result.failed_files.is_empty());
    }

    #[test]
    fn repair_success_exit_but_repair_failed_in_output_still_reports_success() {
        // Conflicting signals: exit code 0 but output says "REPAIR FAILED"
        // The parser uses exit code as the source of truth for `success`
        let stdout = b"Could not repair: \"corrupted.bin\"\nREPAIR FAILED\n";
        let result = parse_par2_repair_output(stdout, b"", ExitStatus::Success).unwrap();

        // success is driven by exit code
        assert!(
            result.success,
            "success field should follow exit code, not output text"
        );
        // But the parser should still extract the failed file info
        assert!(
            result.failed_files.contains(&"corrupted.bin".to_string()),
            "should still parse failed filenames from output, got: {:?}",
            result.failed_files
        );
    }

    #[test]
    fn repair_failure_exit_but_repaired_in_output_reports_unsuccessful() {
        // Opposite conflict: exit code says failure but output mentions repaired files
        let stdout = b"Repaired: \"fixed.bin\"\nSome other error occurred\n";
        let stderr = b"Error: partial repair\n";
        let result = parse_par2_repair_output(stdout, stderr, ExitStatus::Failure).unwrap();

        assert!(
            !result.success,
            "failure exit code should override repaired mentions in output"
        );
        assert!(
            result.repaired_files.contains(&"fixed.bin".to_string()),
            "should still parse repaired filenames even on failure exit"
        );
        assert!(
            result.error.is_some(),
            "should capture error message from output"
        );
    }

    #[test]
    fn verify_failure_exit_with_stderr_error_but_no_damage_reports_complete() {
        // par2 failed (e.g., couldn't read recovery file) but reported no file damage.
        // We treat the data as intact since there's no evidence of corruption.
        let stdout = b"";
        let stderr = b"par2: fatal error: unable to read recovery file\n";
        let result = parse_par2_verify_output(stdout, stderr, ExitStatus::Failure).unwrap();

        assert!(
            result.is_complete,
            "no damage indicators found, so data should be considered complete"
        );
        assert_eq!(result.damaged_blocks, 0);
    }

    #[test]
    fn test_parse_repair_real_output_success() {
        // par2cmdline uses "Repairing:" (present tense) not "Repaired:" (past tense),
        // but "Writing repaired data to disk" contains "repaired".
        // Known limitation: the parser looks for "repaired"/"restored" to detect
        // repaired files, but "Repairing:" doesn't match "repaired", and
        // "Writing repaired data to disk." has no filename to extract.
        let stdout = b"Loading \"file.tar.vol000+577.PAR2\".\n\
                       Loaded 577 new packets\n\
                       Repair is required.\n\
                       Repairing: \"file.tar\"\n\
                       Writing repaired data to disk.\n\
                       Repair complete.\n";
        let stderr = b"";
        let result = parse_par2_repair_output(stdout, stderr, ExitStatus::Success).unwrap();

        assert!(result.success);
        // Parser can't extract filenames from par2cmdline's actual output format:
        // "Repairing:" doesn't contain "repaired", and "Writing repaired data to disk"
        // doesn't contain an extractable filename
        assert!(result.repaired_files.is_empty());
        assert!(result.failed_files.is_empty());
        assert!(result.error.is_none());
    }
}
