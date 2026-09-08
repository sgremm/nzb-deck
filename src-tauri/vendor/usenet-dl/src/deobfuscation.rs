//! Obfuscated filename detection and handling
//!
//! Usenet releases often use obfuscated (random) filenames. This module provides
//! heuristics to detect such filenames and utilities to determine proper names.

use std::fs;
use std::path::{Path, PathBuf};

/// Minimum string length required to reliably detect high entropy.
/// Shorter strings can appear random by chance.
const MIN_ENTROPY_STRING_LENGTH: usize = 24;

/// Lower bound for balanced character type distribution (approximately 1/3).
const ENTROPY_RATIO_LOWER_BOUND: f32 = 0.28;

/// Upper bound for balanced upper/lower case distribution.
const ENTROPY_RATIO_UPPER_BOUND_LETTERS: f32 = 0.38;

/// Lower bound for upper/lower case distribution.
const ENTROPY_RATIO_LOWER_BOUND_LETTERS: f32 = 0.31;

/// Check if a filename appears to be obfuscated (random/meaningless)
///
/// Uses multiple heuristics:
/// - High entropy (random alphanumeric)
/// - UUID-like patterns
/// - Pure hex strings
/// - No vowels (unlikely in real names)
///
/// # Examples
///
/// ```
/// use usenet_dl::deobfuscation::is_obfuscated;
///
/// assert!(is_obfuscated("a3f8b2c9d1e5f7a4b6c8d0e2f4a6b8c0"));
/// assert!(is_obfuscated("550e8400-e29b-41d4-a716-446655440000.mkv"));
/// assert!(is_obfuscated("xkcd1234mnbvcxz.avi"));
/// assert!(!is_obfuscated("Movie.Name.2024.1080p.BluRay.x264.mkv"));
/// ```
#[must_use]
pub fn is_obfuscated(filename: &str) -> bool {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);

    // Check for common obfuscation patterns
    let checks = [
        // Mostly random alphanumeric (high entropy)
        is_high_entropy(stem),
        // UUID-like patterns
        looks_like_uuid(stem),
        // Pure hex strings (longer than typical hashes in filenames)
        is_hex_string(stem) && stem.len() > 16,
        // Random with no vowels (unlikely in real names)
        has_no_vowels(stem) && stem.len() > 8,
    ];

    checks.iter().any(|&c| c)
}

/// Check if a string has high entropy (appears random)
///
/// Calculates a simple entropy measure by checking character distribution.
/// Returns true if the string appears to have random/uniform distribution.
///
/// This is intentionally conservative - it's better to miss some obfuscated files
/// than to false-positive on normal structured filenames.
fn is_high_entropy(s: &str) -> bool {
    if s.len() < MIN_ENTROPY_STRING_LENGTH {
        // Need longer strings to be confident about randomness
        // Short strings can have apparent uniformity by chance
        return false;
    }

    // Count different character types
    let mut upper = 0;
    let mut lower = 0;
    let mut digit = 0;

    for c in s.chars() {
        match c {
            'A'..='Z' => upper += 1,
            'a'..='z' => lower += 1,
            '0'..='9' => digit += 1,
            _ => {} // Ignore separators like dots, hyphens
        }
    }

    let total = (upper + lower + digit) as f32;
    if total < MIN_ENTROPY_STRING_LENGTH as f32 {
        return false;
    }

    let upper_ratio = upper as f32 / total;
    let lower_ratio = lower as f32 / total;
    let digit_ratio = digit as f32 / total;

    // High entropy: ALL three types must be present with very balanced distribution
    // This catches genuinely random strings like "aB3cD5eF7gH9iJ1kL2mN4oP6qR8sT0"
    // but not structured patterns like "EpisodeS01E01720pWEBDL"

    // Require all three types
    if upper == 0 || lower == 0 || digit == 0 {
        return false;
    }

    // Each type must be within tight range of 1/3
    // Real filenames rarely have this perfect balance
    let balanced_upper = (ENTROPY_RATIO_LOWER_BOUND_LETTERS..=ENTROPY_RATIO_UPPER_BOUND_LETTERS)
        .contains(&upper_ratio);
    let balanced_lower = (ENTROPY_RATIO_LOWER_BOUND_LETTERS..=ENTROPY_RATIO_UPPER_BOUND_LETTERS)
        .contains(&lower_ratio);
    let balanced_digit =
        (ENTROPY_RATIO_LOWER_BOUND..=ENTROPY_RATIO_UPPER_BOUND_LETTERS).contains(&digit_ratio);

    balanced_upper && balanced_lower && balanced_digit
}

/// Check if a string looks like a UUID
///
/// Matches patterns like: 550e8400-e29b-41d4-a716-446655440000
/// or without hyphens: 550e8400e29b41d4a716446655440000
fn looks_like_uuid(s: &str) -> bool {
    // UUID with hyphens: 8-4-4-4-12 hex digits
    if s.len() == 36 && s.chars().filter(|&c| c == '-').count() == 4 {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() == 5
            && parts[0].len() == 8
            && parts[1].len() == 4
            && parts[2].len() == 4
            && parts[3].len() == 4
            && parts[4].len() == 12
        {
            return parts
                .iter()
                .all(|p| p.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    // UUID without hyphens: 32 hex digits
    if s.len() == 32 {
        return s.chars().all(|c| c.is_ascii_hexdigit());
    }

    false
}

/// Check if a string is entirely hexadecimal digits
fn is_hex_string(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Check if a string has no vowels
///
/// Real words and names almost always contain vowels.
/// Random strings may not have any vowels.
fn has_no_vowels(s: &str) -> bool {
    let vowels = ['a', 'e', 'i', 'o', 'u', 'A', 'E', 'I', 'O', 'U'];
    !s.chars().any(|c| vowels.contains(&c))
}

/// Determine the final name for a download using priority-based sources
///
/// Priority order:
/// 1. Job name (NZB filename without extension)
/// 2. NZB meta title (<meta type="name"> from NZB)
/// 3. Largest non-obfuscated file in extracted files
/// 4. Fallback to job name even if obfuscated
///
/// # Arguments
///
/// * `job_name` - The NZB filename without extension
/// * `nzb_meta_name` - Optional title from NZB metadata
/// * `extracted_files` - List of files extracted from archives
///
/// # Examples
///
/// ```
/// use std::path::PathBuf;
/// use usenet_dl::deobfuscation::determine_final_name;
///
/// let job_name = "Movie.Name.2024.1080p";
/// let extracted = vec![PathBuf::from("movie.mkv")];
/// let name = determine_final_name(job_name, None, &extracted);
/// assert_eq!(name, "Movie.Name.2024.1080p");
/// ```
pub fn determine_final_name(
    job_name: &str,
    nzb_meta_name: Option<&str>,
    extracted_files: &[PathBuf],
) -> String {
    // 1. Job name (NZB filename) - if not obfuscated
    if !is_obfuscated(job_name) {
        return job_name.to_string();
    }

    // 2. NZB meta title - if present and not obfuscated
    if let Some(meta_name) = nzb_meta_name
        && !is_obfuscated(meta_name)
    {
        return meta_name.to_string();
    }

    // 3. Largest non-obfuscated file
    if let Some(largest) = find_largest_file(extracted_files)
        && let Some(name) = largest.file_stem().and_then(|s| s.to_str())
        && !is_obfuscated(name)
    {
        return name.to_string();
    }

    // Fallback to job name even if obfuscated
    job_name.to_string()
}

/// Find the largest file in a list of paths
///
/// Returns the path to the largest file by size, or None if the list is empty
/// or all files fail to stat.
///
/// # Arguments
///
/// * `files` - List of file paths to check
///
/// # Examples
///
/// ```no_run
/// use std::path::PathBuf;
/// use usenet_dl::deobfuscation::find_largest_file;
///
/// let files = vec![
///     PathBuf::from("small.txt"),
///     PathBuf::from("large.mkv"),
/// ];
/// let largest = find_largest_file(&files);
/// ```
pub fn find_largest_file(files: &[PathBuf]) -> Option<PathBuf> {
    let mut largest_idx: Option<usize> = None;
    let mut largest_size: u64 = 0;

    for (idx, file) in files.iter().enumerate() {
        // Skip directories
        if file.is_dir() {
            continue;
        }

        // Get file size
        if let Ok(metadata) = fs::metadata(file) {
            let size = metadata.len();
            if largest_idx.is_none() || size > largest_size {
                largest_idx = Some(idx);
                largest_size = size;
            }
        }
    }

    // Only clone the winner at the end
    largest_idx.map(|idx| files[idx].clone())
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_high_entropy() {
        // High entropy (random-looking) - long with perfect balance
        assert!(is_high_entropy("aB3cD5eF7gH9iJ1kL2mN4oP6qR8sT0uV2"));
        assert!(is_high_entropy("Xk4mP9wRt2Yz8QvN3Lb6Hj5Mk7Np1"));
        assert!(is_high_entropy("aB3cD5eF7gH9iJ1kL2mN4oP6")); // Exactly 24 chars, perfectly balanced

        // Low entropy (structured)
        assert!(!is_high_entropy("MovieName2024"));
        assert!(!is_high_entropy("episode01"));
        assert!(!is_high_entropy("short")); // Too short
        assert!(!is_high_entropy("EpisodeS01E01720pWEBDL")); // Structured pattern, only 22 alphanumeric
        assert!(!is_high_entropy("aB3cD5eF7gH9iJ1kL2mN4o")); // 23 chars - just under threshold
    }

    #[test]
    fn test_looks_like_uuid() {
        // Valid UUID patterns
        assert!(looks_like_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(looks_like_uuid("550e8400e29b41d4a716446655440000"));
        assert!(looks_like_uuid("A1B2C3D4-E5F6-7890-ABCD-EF1234567890"));

        // Not UUIDs
        assert!(!looks_like_uuid("not-a-uuid-at-all"));
        assert!(!looks_like_uuid("550e8400-e29b-41d4-a716")); // Too short
        assert!(!looks_like_uuid(
            "550e8400-e29b-41d4-a716-446655440000-extra"
        )); // Too long
    }

    #[test]
    fn test_is_hex_string() {
        assert!(is_hex_string("0123456789abcdef"));
        assert!(is_hex_string("ABCDEF123456"));
        assert!(is_hex_string("deadbeef"));

        assert!(!is_hex_string("not hex"));
        assert!(!is_hex_string("g123456")); // 'g' not hex
        assert!(!is_hex_string("")); // Empty
    }

    #[test]
    fn test_has_no_vowels() {
        assert!(has_no_vowels("xkcdmnbvcxz"));
        assert!(has_no_vowels("1234567890"));
        assert!(has_no_vowels("bcdfghjklmnpqrstvwxyz"));

        assert!(!has_no_vowels("hello"));
        assert!(!has_no_vowels("movie"));
        assert!(!has_no_vowels("A"));
    }

    #[test]
    fn test_is_obfuscated_uuid_patterns() {
        // UUID-like patterns should be detected as obfuscated
        assert!(is_obfuscated("550e8400-e29b-41d4-a716-446655440000.mkv"));
        assert!(is_obfuscated("550e8400e29b41d4a716446655440000.avi"));
    }

    #[test]
    fn test_is_obfuscated_hex_strings() {
        // Long hex strings should be detected
        assert!(is_obfuscated("a3f8b2c9d1e5f7a4b6c8d0e2f4a6b8c0.mp4"));
        assert!(is_obfuscated("deadbeef1234567890abcdef.mkv"));

        // Short hex strings might be version/CRC codes (not obfuscated)
        assert!(!is_obfuscated("Movie[1a2b3c4d].mkv"));
    }

    #[test]
    fn test_is_obfuscated_no_vowels() {
        // Long strings with no vowels should be detected
        assert!(is_obfuscated("xkcd1234mnbvcxz.avi"));
        assert!(is_obfuscated("bcdfghjklmnp.mp4"));

        // Short strings are acceptable
        assert!(!is_obfuscated("cd1.mkv"));
    }

    #[test]
    fn test_is_obfuscated_high_entropy() {
        // Random alphanumeric should be detected
        assert!(is_obfuscated("aB3cD5eF7gH9iJ1kL2mN4oP6.mkv"));
        assert!(is_obfuscated("Xk4mP9wRt2Yz8QvN3Lb6.avi"));
    }

    #[test]
    fn test_is_obfuscated_normal_filenames() {
        // Normal filenames should NOT be detected as obfuscated
        assert!(!is_obfuscated("Movie.Name.2024.1080p.BluRay.x264.mkv"));
        assert!(!is_obfuscated("Episode.S01E01.720p.WEB-DL.mkv"));
        assert!(!is_obfuscated("Documentary.Title.2024.mp4"));
        assert!(!is_obfuscated("album_track01.mp3"));
        assert!(!is_obfuscated("my-vacation-video.avi"));
    }

    #[test]
    fn test_is_obfuscated_edge_cases() {
        // Empty/short filenames
        assert!(!is_obfuscated(""));
        assert!(!is_obfuscated("a.mkv"));
        assert!(!is_obfuscated("ab.mp4"));

        // Extension handling
        assert!(is_obfuscated("a3f8b2c9d1e5f7a4b6c8d0e2f4a6b8c0")); // No extension
    }

    #[test]
    fn test_is_obfuscated_mixed_cases() {
        // Real-world examples from Usenet
        assert!(is_obfuscated("98234ksdfj2398sdkjf.avi")); // Random

        // Short hex strings are ambiguous (could be CRC codes)
        assert!(!is_obfuscated("7a4b9c2d.mkv")); // Could be CRC
        assert!(!is_obfuscated("[1a2b3c4d].mkv")); // CRC in brackets

        // Common false positives to avoid
        assert!(!is_obfuscated("x264.mkv")); // Codec name
        assert!(!is_obfuscated("h264.mp4")); // Codec name
        assert!(!is_obfuscated("BD1080p.mkv")); // Quality tag
    }

    #[test]
    fn test_determine_final_name_from_job_name() {
        // Job name is not obfuscated - use it
        let job_name = "Movie.Name.2024.1080p";
        let extracted = vec![PathBuf::from("movie.mkv")];
        let name = determine_final_name(job_name, None, &extracted);
        assert_eq!(name, "Movie.Name.2024.1080p");
    }

    #[test]
    fn test_determine_final_name_from_nzb_meta() {
        // Job name is obfuscated, but NZB meta is good
        let job_name = "a3f8b2c9d1e5f7a4b6c8d0e2f4a6b8c0";
        let nzb_meta = Some("Movie.Name.2024.1080p");
        let extracted = vec![PathBuf::from("random.mkv")];
        let name = determine_final_name(job_name, nzb_meta, &extracted);
        assert_eq!(name, "Movie.Name.2024.1080p");
    }

    #[test]
    fn test_determine_final_name_from_largest_file() {
        // Job name and NZB meta are obfuscated, use largest file
        let job_name = "a3f8b2c9d1e5f7a4b6c8d0e2f4a6b8c0";
        let nzb_meta = Some("550e8400-e29b-41d4-a716-446655440000");

        // Create temp files for testing
        let temp_dir = std::env::temp_dir().join("usenet_dl_test_determine_name");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let small_file = temp_dir.join("Movie.Name.2024.sample.mkv");
        let large_file = temp_dir.join("Movie.Name.2024.1080p.mkv");

        fs::write(&small_file, b"small").unwrap();
        fs::write(&large_file, b"large content here").unwrap();

        let extracted = vec![small_file.clone(), large_file.clone()];
        let name = determine_final_name(job_name, nzb_meta, &extracted);

        // Should use the largest file's stem
        assert_eq!(name, "Movie.Name.2024.1080p");

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_determine_final_name_fallback_to_obfuscated_job_name() {
        // Everything is obfuscated - fallback to job name
        let job_name = "a3f8b2c9d1e5f7a4b6c8d0e2f4a6b8c0";
        let nzb_meta = Some("550e8400-e29b-41d4-a716-446655440000");
        let extracted = vec![PathBuf::from("xkcd1234mnbvcxz.mkv")];
        let name = determine_final_name(job_name, nzb_meta, &extracted);
        assert_eq!(name, "a3f8b2c9d1e5f7a4b6c8d0e2f4a6b8c0");
    }

    #[test]
    fn test_determine_final_name_empty_extracted_files() {
        // No extracted files - should use job name
        let job_name = "Movie.Name.2024";
        let extracted = vec![];
        let name = determine_final_name(job_name, None, &extracted);
        assert_eq!(name, "Movie.Name.2024");
    }

    #[test]
    fn test_find_largest_file_basic() {
        let temp_dir = std::env::temp_dir().join("usenet_dl_test_largest_basic");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let file1 = temp_dir.join("small.txt");
        let file2 = temp_dir.join("large.mkv");
        let file3 = temp_dir.join("medium.avi");

        fs::write(&file1, b"small").unwrap();
        fs::write(&file2, b"large content here with more bytes").unwrap();
        fs::write(&file3, b"medium size").unwrap();

        let files = vec![file1.clone(), file2.clone(), file3.clone()];
        let largest = find_largest_file(&files);

        assert_eq!(largest, Some(file2));

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_find_largest_file_empty_list() {
        let files = vec![];
        let largest = find_largest_file(&files);
        assert_eq!(largest, None);
    }

    #[test]
    fn test_find_largest_file_ignores_directories() {
        let temp_dir = std::env::temp_dir().join("usenet_dl_test_largest_dirs");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let file = temp_dir.join("file.mkv");
        let subdir = temp_dir.join("subdir");

        fs::write(&file, b"content").unwrap();
        fs::create_dir(&subdir).unwrap();

        let files = vec![subdir.clone(), file.clone()];
        let largest = find_largest_file(&files);

        // Should return the file, not the directory
        assert_eq!(largest, Some(file));

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_find_largest_file_nonexistent_files() {
        // Non-existent files should be skipped
        let files = vec![
            PathBuf::from("/nonexistent/file1.mkv"),
            PathBuf::from("/nonexistent/file2.avi"),
        ];
        let largest = find_largest_file(&files);
        assert_eq!(largest, None);
    }

    #[test]
    fn test_determine_final_name_with_extensions() {
        // Ensure extensions don't affect obfuscation detection
        let job_name = "Movie.Name.2024";
        let extracted = vec![PathBuf::from("video.mkv")];
        let name = determine_final_name(job_name, None, &extracted);
        assert_eq!(name, "Movie.Name.2024");
    }
}
