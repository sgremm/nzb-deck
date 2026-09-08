//! Utility functions for file operations and path manipulation

use crate::config::FileCollisionAction;
use crate::error::{Error, PostProcessError, Result};
use std::path::{Path, PathBuf};

/// Maximum number of rename attempts when resolving file collisions
const MAX_RENAME_ATTEMPTS: u32 = 9999;

/// Get a unique path for a file, handling collisions according to the specified action
///
/// # Arguments
///
/// * `path` - The desired file path
/// * `action` - How to handle file collisions
///
/// # Returns
///
/// Returns the final path to use. For Rename action, this may have a suffix added.
/// For Skip action, returns an error if the file already exists.
/// For Overwrite action, returns the original path unchanged.
///
/// # Examples
///
/// ```
/// use usenet_dl::utils::get_unique_path;
/// use usenet_dl::config::FileCollisionAction;
/// use std::path::Path;
///
/// let path = Path::new("/tmp/movie.mkv");
/// let unique = get_unique_path(path, FileCollisionAction::Rename).unwrap();
/// // If /tmp/movie.mkv exists, returns /tmp/movie (1).mkv
/// // If that exists too, returns /tmp/movie (2).mkv, etc.
/// ```
pub fn get_unique_path(path: &Path, action: FileCollisionAction) -> Result<PathBuf> {
    match action {
        FileCollisionAction::Overwrite => {
            // Always use the original path, overwriting if it exists
            Ok(path.to_path_buf())
        }
        FileCollisionAction::Skip => {
            // Return error if file exists
            if path.exists() {
                return Err(Error::PostProcess(PostProcessError::FileCollision {
                    path: path.to_path_buf(),
                    reason: "File already exists and collision action is Skip".to_string(),
                }));
            }
            Ok(path.to_path_buf())
        }
        FileCollisionAction::Rename => {
            // If file doesn't exist, use original path
            if !path.exists() {
                return Ok(path.to_path_buf());
            }

            // File exists, need to find a unique name
            let stem = path.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
                Error::PostProcess(PostProcessError::InvalidPath {
                    path: path.to_path_buf(),
                    reason: "Cannot extract file stem".to_string(),
                })
            })?;

            let extension = path.extension().and_then(|e| e.to_str());

            let parent = path.parent().ok_or_else(|| {
                Error::PostProcess(PostProcessError::InvalidPath {
                    path: path.to_path_buf(),
                    reason: "Cannot extract parent directory".to_string(),
                })
            })?;

            // Try adding (1), (2), (3), ... until we find a unique name
            for i in 1..=MAX_RENAME_ATTEMPTS {
                let new_name = match extension {
                    Some(ext) => format!("{} ({}).{}", stem, i, ext),
                    None => format!("{} ({})", stem, i),
                };
                let new_path = parent.join(new_name);
                if !new_path.exists() {
                    return Ok(new_path);
                }
            }

            // If we've tried MAX_RENAME_ATTEMPTS names and they all exist, give up
            Err(Error::PostProcess(PostProcessError::FileCollision {
                path: path.to_path_buf(),
                reason: "Could not find unique filename after 9999 attempts".to_string(),
            }))
        }
    }
}

/// Check if a path appears to be a sample file or folder
///
/// Detects sample files/folders using common naming patterns:
/// - Folders named "sample", "samples", "subs", "proof"
/// - Files with "sample" in the name
/// - Common video sample patterns (e.g., "sample.mkv", "moviename-sample.avi")
///
/// # Arguments
///
/// * `path` - The path to check
///
/// # Returns
///
/// Returns `true` if the path appears to be a sample file or folder
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use usenet_dl::utils::is_sample;
///
/// assert!(is_sample(Path::new("/downloads/Movie/Sample")));
/// assert!(is_sample(Path::new("/downloads/Movie/movie-sample.mkv")));
/// assert!(!is_sample(Path::new("/downloads/Movie/movie.mkv")));
/// ```
#[must_use]
pub fn is_sample(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Common sample folder/file names
    const SAMPLE_PATTERNS: &[&str] = &[
        "sample", "samples", "subs", "proof", "proofs", "cover", "covers", "eac3to",
    ];

    // Check for exact matches (case-insensitive)
    if SAMPLE_PATTERNS.iter().any(|&pattern| name == pattern) {
        return true;
    }

    // Check for "sample" in the filename
    // e.g., "movie-sample.mkv", "sample.avi", "movie.sample.mp4"
    if name.contains("sample") {
        return true;
    }

    false
}

/// Extract filename from HTTP response
///
/// Tries to extract the filename from Content-Disposition header,
/// falls back to the URL path if not found.
///
/// # Arguments
///
/// * `response` - The reqwest Response object
/// * `url` - The original URL (used as fallback)
///
/// # Returns
///
/// Returns the extracted filename (without extension) or "download" as last resort
///
/// # Examples
///
/// ```ignore
/// let response = reqwest::get("https://example.com/file.nzb").await?;
/// let filename = extract_filename_from_response(&response, "https://example.com/file.nzb");
/// // Returns "file"
/// ```
pub fn extract_filename_from_response(response: &reqwest::Response, url: &str) -> String {
    // Try to extract from Content-Disposition header
    if let Some(content_disposition) = response.headers().get("content-disposition")
        && let Ok(value) = content_disposition.to_str()
    {
        // Parse filename from Content-Disposition header
        // Format: attachment; filename="file.nzb" or filename*=UTF-8''file.nzb
        for part in value.split(';') {
            let part = part.trim();
            if part.starts_with("filename=") {
                let filename = part
                    .trim_start_matches("filename=")
                    .trim_matches('"')
                    .to_string();
                // Remove extension
                if let Some(stem) = std::path::Path::new(&filename).file_stem()
                    && let Some(stem_str) = stem.to_str()
                {
                    return stem_str.to_string();
                }
                return filename;
            } else if part.starts_with("filename*=") {
                // Handle RFC 5987 encoded filename
                let filename = part.trim_start_matches("filename*=");
                // Format is: charset'lang'encoded-filename
                if let Some(idx) = filename.rfind('\'') {
                    let encoded = &filename[idx + 1..];
                    // URL decode the filename
                    if let Ok(decoded) = urlencoding::decode(encoded) {
                        if let Some(stem) = std::path::Path::new(decoded.as_ref()).file_stem()
                            && let Some(stem_str) = stem.to_str()
                        {
                            return stem_str.to_string();
                        }
                        return decoded.to_string();
                    }
                }
            }
        }
    }

    // Fall back to extracting from URL path
    if let Ok(parsed_url) = url::Url::parse(url)
        && let Some(mut segments) = parsed_url.path_segments()
        && let Some(last_segment) = segments.next_back()
        && !last_segment.is_empty()
    {
        // Remove extension
        if let Some(stem) = std::path::Path::new(last_segment).file_stem()
            && let Some(stem_str) = stem.to_str()
        {
            return stem_str.to_string();
        }
        return last_segment.to_string();
    }

    // Last resort fallback
    "download".to_string()
}

/// Get available disk space for a given path
///
/// Uses platform-specific APIs to query filesystem statistics:
/// - Linux: statvfs
/// - macOS: statvfs
/// - Windows: GetDiskFreeSpaceExW
///
/// # Arguments
///
/// * `path` - The path to check (typically the download directory)
///
/// # Returns
///
/// Returns the available disk space in bytes, or an IO error if the check fails.
///
/// # Examples
///
/// ```ignore
/// let available = get_available_space(Path::new("/downloads"))?;
/// println!("Available space: {} GB", available / (1024 * 1024 * 1024));
/// ```
pub fn get_available_space(path: &Path) -> std::io::Result<u64> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        // Convert path to C string for statvfs call
        let c_path = CString::new(path.as_os_str().as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        // SAFETY: This is safe because:
        // 1. c_path is a valid, null-terminated C string created from the input path
        // 2. stat is properly initialized with zeroed memory before the call
        // 3. We check the return value and propagate any OS errors
        // 4. The statvfs struct is only read after a successful call
        unsafe {
            let mut stat: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c_path.as_ptr(), &mut stat) != 0 {
                return Err(std::io::Error::last_os_error());
            }

            // Available space = available blocks * block size
            // f_bavail is available blocks for unprivileged users
            // f_frsize is the fragment size (preferred over f_bsize)
            let available_bytes = (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64);
            Ok(available_bytes)
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use winapi::um::fileapi::GetDiskFreeSpaceExW;

        // Convert path to wide string for Windows API
        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0)) // null terminator
            .collect();

        // SAFETY: This is safe because:
        // 1. wide_path is a valid, null-terminated wide string
        // 2. All output pointers point to valid, properly aligned u64 variables
        // 3. We check the return value and propagate any OS errors
        // 4. The output variables are only read after a successful call
        unsafe {
            let mut free_bytes_available: u64 = 0;
            let mut _total_bytes: u64 = 0;
            let mut _total_free_bytes: u64 = 0;

            if GetDiskFreeSpaceExW(
                wide_path.as_ptr(),
                &mut free_bytes_available as *mut u64 as *mut _,
                &mut _total_bytes as *mut u64 as *mut _,
                &mut _total_free_bytes as *mut u64 as *mut _,
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }

            Ok(free_bytes_available)
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        // Unsupported platform - return an error
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Disk space checking is not supported on this platform",
        ))
    }
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    use wiremock::MockServer;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    #[test]
    fn test_get_unique_path_nonexistent_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        // File doesn't exist, should return original path for all actions
        assert_eq!(
            get_unique_path(&path, FileCollisionAction::Rename).unwrap(),
            path
        );
        assert_eq!(
            get_unique_path(&path, FileCollisionAction::Overwrite).unwrap(),
            path
        );
        assert_eq!(
            get_unique_path(&path, FileCollisionAction::Skip).unwrap(),
            path
        );
    }

    #[test]
    fn test_get_unique_path_rename_with_extension() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        // Create the original file
        fs::write(&path, "original").unwrap();

        // Rename action should add (1) suffix
        let unique = get_unique_path(&path, FileCollisionAction::Rename).unwrap();
        assert_eq!(unique, temp_dir.path().join("test (1).txt"));

        // Create the (1) file and try again
        fs::write(&unique, "first rename").unwrap();
        let unique2 = get_unique_path(&path, FileCollisionAction::Rename).unwrap();
        assert_eq!(unique2, temp_dir.path().join("test (2).txt"));
    }

    #[test]
    fn test_get_unique_path_rename_without_extension() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test");

        // Create the original file
        fs::write(&path, "original").unwrap();

        // Rename action should add (1) suffix
        let unique = get_unique_path(&path, FileCollisionAction::Rename).unwrap();
        assert_eq!(unique, temp_dir.path().join("test (1)"));
    }

    #[test]
    fn test_get_unique_path_overwrite() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        // Create the original file
        fs::write(&path, "original").unwrap();

        // Overwrite action should return original path
        let result = get_unique_path(&path, FileCollisionAction::Overwrite).unwrap();
        assert_eq!(result, path);
    }

    #[test]
    fn test_get_unique_path_skip_existing() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        // Create the original file
        fs::write(&path, "original").unwrap();

        // Skip action should return error if file exists
        let result = get_unique_path(&path, FileCollisionAction::Skip);
        assert!(result.is_err());
        match result {
            Err(Error::PostProcess(PostProcessError::FileCollision { path: p, reason: _ })) => {
                assert_eq!(p, path);
            }
            _ => panic!("Expected FileCollision error"),
        }
    }

    #[test]
    fn test_get_unique_path_multiple_dots() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.tar.gz");

        // Create the original file
        fs::write(&path, "original").unwrap();

        // Should handle multiple dots correctly (only last extension)
        let unique = get_unique_path(&path, FileCollisionAction::Rename).unwrap();
        assert_eq!(unique, temp_dir.path().join("test.tar (1).gz"));
    }

    #[test]
    fn test_get_unique_path_sequential() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.txt");

        // Create files test.txt, test (1).txt, test (2).txt
        fs::write(&path, "original").unwrap();
        fs::write(temp_dir.path().join("test (1).txt"), "first").unwrap();
        fs::write(temp_dir.path().join("test (2).txt"), "second").unwrap();

        // Should find test (3).txt
        let unique = get_unique_path(&path, FileCollisionAction::Rename).unwrap();
        assert_eq!(unique, temp_dir.path().join("test (3).txt"));
    }

    #[test]
    fn test_is_sample_folder_exact_match() {
        // Exact match sample folder names (case-insensitive)
        assert!(is_sample(Path::new("/downloads/Movie/Sample")));
        assert!(is_sample(Path::new("/downloads/Movie/sample")));
        assert!(is_sample(Path::new("/downloads/Movie/SAMPLE")));
        assert!(is_sample(Path::new("/downloads/Movie/Samples")));
        assert!(is_sample(Path::new("/downloads/Movie/Subs")));
        assert!(is_sample(Path::new("/downloads/Movie/Proof")));
        assert!(is_sample(Path::new("/downloads/Movie/Cover")));
    }

    #[test]
    fn test_is_sample_file_with_sample_in_name() {
        // Files with "sample" in the name
        assert!(is_sample(Path::new("/downloads/movie-sample.mkv")));
        assert!(is_sample(Path::new("/downloads/sample.avi")));
        assert!(is_sample(Path::new("/downloads/movie.sample.mp4")));
        assert!(is_sample(Path::new("/downloads/SAMPLE.MKV")));
        assert!(is_sample(Path::new("/downloads/Movie-SAMPLE-Scene.mkv")));
    }

    #[test]
    fn test_is_sample_not_sample() {
        // Normal files/folders that are not samples
        assert!(!is_sample(Path::new("/downloads/Movie/movie.mkv")));
        assert!(!is_sample(Path::new("/downloads/Movie/Video")));
        assert!(!is_sample(Path::new("/downloads/Movie/Season 01")));
        assert!(!is_sample(Path::new("/downloads/Movie/extras")));
        assert!(!is_sample(Path::new("/downloads/Movie.2020.1080p.mkv")));
    }

    #[test]
    fn test_is_sample_edge_cases() {
        // Edge cases - paths that might be confusing
        // "sampling" does NOT contain "sample" - they are different words
        assert!(!is_sample(Path::new("/downloads/sampling-documentary.mkv")));
        assert!(!is_sample(Path::new("/downloads/examples/movie.mkv")));

        // But these DO contain "sample" as substring
        assert!(is_sample(Path::new("/downloads/resampled-audio.mkv"))); // "resampled" = "re" + "sample" + "d"

        // Normal files that should not be detected
        assert!(!is_sample(Path::new("/downloads/Movie.2020.mkv")));

        // Empty path
        assert!(!is_sample(Path::new("")));

        // Just extension
        assert!(!is_sample(Path::new(".mkv")));
    }

    #[test]
    fn test_get_available_space_valid_path() {
        // Test with a valid path (temp directory should always exist)
        let temp_dir = TempDir::new().unwrap();
        let available = get_available_space(temp_dir.path()).unwrap();

        // Available space should be greater than 0
        assert!(available > 0, "Available space should be greater than 0");

        // Available space should be reasonable (less than 1 PB = 10^15 bytes)
        assert!(
            available < 1_000_000_000_000_000,
            "Available space seems unreasonably large"
        );
    }

    #[test]
    fn test_get_available_space_nonexistent_path() {
        // Test with a path that doesn't exist
        let result = get_available_space(Path::new("/nonexistent/path/that/should/not/exist"));

        // Should return an error
        assert!(result.is_err(), "Should return error for nonexistent path");
    }

    #[test]
    fn test_get_available_space_current_dir() {
        // Test with current directory
        let available = get_available_space(Path::new(".")).unwrap();

        // Should succeed and return reasonable value
        assert!(
            available > 0,
            "Current directory should have available space"
        );
    }

    // =========================================================================
    // extract_filename_from_response
    // =========================================================================

    /// Helper: start a mock server, register a response, make a GET request, return the response.
    async fn mock_response(
        path_str: &str,
        template: ResponseTemplate,
    ) -> (reqwest::Response, String) {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(path_str))
            .respond_with(template)
            .mount(&server)
            .await;

        let url = format!("{}{}", server.uri(), path_str);
        let resp = reqwest::get(&url).await.unwrap();
        (resp, url)
    }

    #[tokio::test]
    async fn extract_filename_from_content_disposition_quoted() {
        let (resp, url) = mock_response(
            "/download/123",
            ResponseTemplate::new(200).insert_header(
                "Content-Disposition",
                r#"attachment; filename="Movie.2024.1080p.nzb""#,
            ),
        )
        .await;

        let name = extract_filename_from_response(&resp, &url);

        assert_eq!(
            name, "Movie.2024.1080p",
            "should strip .nzb extension and return stem"
        );
    }

    #[tokio::test]
    async fn extract_filename_from_content_disposition_unquoted() {
        let (resp, url) = mock_response(
            "/download/456",
            ResponseTemplate::new(200)
                .insert_header("Content-Disposition", "attachment; filename=report.pdf"),
        )
        .await;

        let name = extract_filename_from_response(&resp, &url);

        assert_eq!(
            name, "report",
            "should strip .pdf extension from unquoted filename"
        );
    }

    #[tokio::test]
    async fn extract_filename_from_rfc5987_encoded_header() {
        let (resp, url) = mock_response(
            "/download/789",
            ResponseTemplate::new(200).insert_header(
                "Content-Disposition",
                "attachment; filename*=UTF-8''file%20name%20with%20spaces.nzb",
            ),
        )
        .await;

        let name = extract_filename_from_response(&resp, &url);

        assert_eq!(
            name, "file name with spaces",
            "should URL-decode RFC 5987 filename and strip extension"
        );
    }

    #[tokio::test]
    async fn extract_filename_falls_back_to_url_path_without_header() {
        let (resp, url) = mock_response("/files/Movie.2024.nzb", ResponseTemplate::new(200)).await;

        let name = extract_filename_from_response(&resp, &url);

        assert_eq!(
            name, "Movie.2024",
            "without Content-Disposition, should use URL path stem"
        );
    }

    #[tokio::test]
    async fn extract_filename_falls_back_to_download_when_no_useful_url() {
        // Use a root path "/" so path_segments().next_back() is empty
        let (resp, _url) = mock_response("/", ResponseTemplate::new(200)).await;

        // Pass a URL with no meaningful path segment
        let name = extract_filename_from_response(&resp, "http://example.com/");

        assert_eq!(
            name, "download",
            "should return 'download' when URL has no useful filename"
        );
    }

    #[tokio::test]
    async fn extract_filename_with_multiple_dots_keeps_all_but_last_extension() {
        let (resp, url) = mock_response("/Movie.2024.720p.nzb", ResponseTemplate::new(200)).await;

        let name = extract_filename_from_response(&resp, &url);

        assert_eq!(
            name, "Movie.2024.720p",
            "file_stem strips only the last extension"
        );
    }

    #[tokio::test]
    async fn extract_filename_content_disposition_takes_priority_over_url() {
        // URL has one name, Content-Disposition has another
        let (resp, url) = mock_response(
            "/api/v1/nzb/download/generic-id",
            ResponseTemplate::new(200).insert_header(
                "Content-Disposition",
                r#"attachment; filename="Real.Movie.Name.nzb""#,
            ),
        )
        .await;

        let name = extract_filename_from_response(&resp, &url);

        assert_eq!(
            name, "Real.Movie.Name",
            "Content-Disposition filename should take priority over URL path"
        );
    }

    #[tokio::test]
    async fn extract_filename_no_extension_returns_full_filename() {
        let (resp, url) = mock_response(
            "/download/123",
            ResponseTemplate::new(200)
                .insert_header("Content-Disposition", r#"attachment; filename="README""#),
        )
        .await;

        let name = extract_filename_from_response(&resp, &url);

        assert_eq!(
            name, "README",
            "filename without extension should return the full name"
        );
    }

    #[tokio::test]
    async fn extract_filename_from_invalid_url_falls_back_to_download() {
        let (resp, _url) = mock_response("/test", ResponseTemplate::new(200)).await;

        // Pass a totally unparseable URL
        let name = extract_filename_from_response(&resp, "not a url at all");

        assert_eq!(
            name, "download",
            "unparseable URL should fall back to 'download'"
        );
    }

    // --- get_unique_path permission edge cases (Linux only) ---

    #[cfg(unix)]
    #[test]
    fn get_unique_path_rename_on_untraversable_directory_returns_original_path() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let restricted_dir = temp_dir.path().join("noperm");
        fs::create_dir(&restricted_dir).unwrap();

        // Create a file inside the directory BEFORE removing permissions
        let file_path = restricted_dir.join("existing.txt");
        fs::write(&file_path, "data").unwrap();

        // Remove execute permission on directory — prevents stat() on files inside
        fs::set_permissions(&restricted_dir, fs::Permissions::from_mode(0o000)).unwrap();

        // Ensure cleanup happens even if assertions panic
        struct RestorePerms<'a>(&'a std::path::Path);
        impl Drop for RestorePerms<'_> {
            fn drop(&mut self) {
                let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
            }
        }
        let _guard = RestorePerms(&restricted_dir);

        // path.exists() returns false when parent directory lacks execute permission,
        // so get_unique_path with Rename thinks the file doesn't exist and returns
        // the original path — this documents the current behavior at this boundary
        let result = get_unique_path(&file_path, FileCollisionAction::Rename).unwrap();
        assert_eq!(
            result, file_path,
            "with no directory traverse permission, exists() returns false, \
             so Rename returns the original path as if the file doesn't exist"
        );
    }

    #[cfg(unix)]
    #[test]
    fn get_unique_path_skip_on_untraversable_directory_succeeds_because_file_appears_absent() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let restricted_dir = temp_dir.path().join("noperm_skip");
        fs::create_dir(&restricted_dir).unwrap();

        // Create a file inside BEFORE removing permissions
        let file_path = restricted_dir.join("existing.txt");
        fs::write(&file_path, "data").unwrap();

        // Remove execute permission
        fs::set_permissions(&restricted_dir, fs::Permissions::from_mode(0o000)).unwrap();

        struct RestorePerms<'a>(&'a std::path::Path);
        impl Drop for RestorePerms<'_> {
            fn drop(&mut self) {
                let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
            }
        }
        let _guard = RestorePerms(&restricted_dir);

        // Skip action checks exists() — which returns false on permission error,
        // so it does NOT return FileCollision. This documents a subtle edge case:
        // the file actually exists but the function can't see it.
        let result = get_unique_path(&file_path, FileCollisionAction::Skip).unwrap();
        assert_eq!(
            result, file_path,
            "Skip returns Ok(path) because exists() is false — the permission error \
             is invisible to get_unique_path"
        );
    }
}
