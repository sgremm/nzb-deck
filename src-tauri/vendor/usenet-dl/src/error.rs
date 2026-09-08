//! Error types for usenet-dl
//!
//! This module provides comprehensive error handling for the library, including:
//! - Domain-specific error types (Download, PostProcess, Config, etc.)
//! - HTTP status code mapping for API integration
//! - Structured error responses with machine-readable error codes
//! - Context information (stage, file path, download ID, etc.)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use utoipa::ToSchema;

/// Result type alias for usenet-dl operations
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for usenet-dl
///
/// This is the primary error type used throughout the library. Each variant includes
/// contextual information to help diagnose issues.
#[derive(Debug, Error)]
pub enum Error {
    /// Configuration error with context about which setting is invalid
    #[error("configuration error: {message}")]
    Config {
        /// Human-readable error message describing the configuration issue
        message: String,
        /// The configuration key that caused the error (e.g., "download_dir")
        key: Option<String>,
    },

    /// Database operation failed
    #[error("database error: {0}")]
    Database(#[from] DatabaseError),

    /// SQLx database error
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    /// NNTP protocol or connection error
    #[error("NNTP error: {0}")]
    Nntp(String),

    /// Download-related error
    #[error("download error: {0}")]
    Download(#[from] DownloadError),

    /// Post-processing error (verify, repair, extract, etc.)
    #[error("post-processing error: {0}")]
    PostProcess(#[from] PostProcessError),

    /// Invalid NZB file
    #[error("invalid NZB: {0}")]
    InvalidNzb(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Download not found
    #[error("download not found: {0}")]
    NotFound(String),

    /// Shutdown in progress - not accepting new downloads
    #[error("shutdown in progress: not accepting new downloads")]
    ShuttingDown,

    /// Network error
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// API server error
    #[error("API server error: {0}")]
    ApiServerError(String),

    /// Folder watching error
    #[error("folder watch error: {0}")]
    FolderWatch(String),

    /// Duplicate download detected
    #[error("duplicate download: {0}")]
    Duplicate(String),

    /// Insufficient disk space
    #[error("insufficient disk space: need {required} bytes, have {available} bytes")]
    InsufficientSpace {
        /// Number of bytes required for the operation
        required: u64,
        /// Number of bytes currently available on disk
        available: u64,
    },

    /// Failed to check disk space
    #[error("failed to check disk space: {0}")]
    DiskSpaceCheckFailed(String),

    /// External tool execution failed (par2, unrar, etc.)
    #[error("external tool error: {0}")]
    ExternalTool(String),

    /// Operation not supported (missing binary, not implemented, etc.)
    #[error("not supported: {0}")]
    NotSupported(String),

    /// Other error
    #[error("{0}")]
    Other(String),
}

/// Database-related errors
#[derive(Debug, Error)]
pub enum DatabaseError {
    /// Failed to connect to database
    #[error("failed to connect to database: {0}")]
    ConnectionFailed(String),

    /// Failed to run migrations
    #[error("failed to run migrations: {0}")]
    MigrationFailed(String),

    /// Query failed
    #[error("query failed: {0}")]
    QueryFailed(String),

    /// Record not found
    #[error("record not found: {0}")]
    NotFound(String),

    /// Constraint violation (e.g., duplicate key)
    #[error("constraint violation: {0}")]
    ConstraintViolation(String),
}

/// Download-related errors
#[derive(Debug, Error)]
pub enum DownloadError {
    /// Download not found in queue or database
    #[error("download {id} not found")]
    NotFound {
        /// The download ID that was not found
        id: i64,
    },

    /// Download files not found on disk
    #[error("download {id} files not found at {path}")]
    FilesNotFound {
        /// The download ID whose files were not found
        id: i64,
        /// The path where the files were expected to be
        path: PathBuf,
    },

    /// Download already in requested state
    #[error("download {id} is already {state}")]
    AlreadyInState {
        /// The download ID that is already in the requested state
        id: i64,
        /// The current state (e.g., "paused", "completed")
        state: String,
    },

    /// Cannot perform operation in current state
    #[error("cannot {operation} download {id} in state {current_state}")]
    InvalidState {
        /// The download ID that is in an invalid state for the operation
        id: i64,
        /// The operation that was attempted (e.g., "pause", "resume", "retry")
        operation: String,
        /// The current state that prevents the operation (e.g., "downloading", "completed")
        current_state: String,
    },

    /// Insufficient disk space to start download
    #[error("insufficient disk space: need {required} bytes, have {available} bytes")]
    InsufficientSpace {
        /// Number of bytes required for the download
        required: u64,
        /// Number of bytes currently available on disk
        available: u64,
    },
}

/// Post-processing errors (PAR2 verify, repair, extraction, etc.)
#[derive(Debug, Error)]
pub enum PostProcessError {
    /// PAR2 verification failed
    #[error("PAR2 verification failed for download {id}: {reason}")]
    VerificationFailed {
        /// The download ID for which verification failed
        id: i64,
        /// The reason verification failed
        reason: String,
    },

    /// PAR2 repair failed
    #[error("PAR2 repair failed for download {id}: {reason}")]
    RepairFailed {
        /// The download ID for which repair failed
        id: i64,
        /// The reason repair failed
        reason: String,
    },

    /// Archive extraction failed
    #[error("extraction failed for {archive}: {reason}")]
    ExtractionFailed {
        /// The archive file that failed to extract
        archive: PathBuf,
        /// The reason extraction failed
        reason: String,
    },

    /// Wrong password for encrypted archive
    #[error("wrong password for encrypted archive {archive}")]
    WrongPassword {
        /// The encrypted archive that could not be opened
        archive: PathBuf,
    },

    /// All passwords failed for archive extraction
    #[error("all {count} passwords failed for archive {archive}")]
    AllPasswordsFailed {
        /// The encrypted archive that could not be opened
        archive: PathBuf,
        /// The number of passwords that were tried
        count: usize,
    },

    /// No passwords available for encrypted archive
    #[error("no passwords available for encrypted archive {archive}")]
    NoPasswordsAvailable {
        /// The encrypted archive that requires a password
        archive: PathBuf,
    },

    /// File move/rename failed
    #[error("failed to move {source_path} to {dest_path}: {reason}")]
    MoveFailed {
        /// The source path of the file being moved
        source_path: PathBuf,
        /// The destination path where the file should be moved
        dest_path: PathBuf,
        /// The reason the move failed
        reason: String,
    },

    /// File collision at destination
    #[error("file collision at {path}: {reason}")]
    FileCollision {
        /// The path where the collision occurred
        path: PathBuf,
        /// The reason for the collision (e.g., "file already exists")
        reason: String,
    },

    /// Cleanup failed (non-fatal, usually logged as warning)
    #[error("cleanup failed for download {id}: {reason}")]
    CleanupFailed {
        /// The download ID for which cleanup failed
        id: i64,
        /// The reason cleanup failed
        reason: String,
    },

    /// Invalid path encountered during post-processing
    #[error("invalid path {path}: {reason}")]
    InvalidPath {
        /// The invalid path that was encountered
        path: PathBuf,
        /// The reason the path is invalid
        reason: String,
    },

    /// DirectUnpack failed during download
    #[error("DirectUnpack failed for download {id}: {reason}")]
    DirectUnpackFailed {
        /// The download ID for which DirectUnpack failed
        id: i64,
        /// The reason DirectUnpack failed
        reason: String,
    },

    /// DirectRename failed during download
    #[error("DirectRename failed for download {id}: {reason}")]
    DirectRenameFailed {
        /// The download ID for which DirectRename failed
        id: i64,
        /// The reason DirectRename failed
        reason: String,
    },
}

/// API error response format
///
/// This structure is returned by API endpoints when an error occurs.
/// It follows a standard format with machine-readable error codes,
/// human-readable messages, and optional contextual details.
///
/// # Example JSON Response
///
/// ```json
/// {
///   "error": {
///     "code": "not_found",
///     "message": "Download 123 not found",
///     "details": {
///       "download_id": 123
///     }
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiError {
    /// The error details
    pub error: ErrorDetail,
}

/// Detailed error information for API responses
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ErrorDetail {
    /// Machine-readable error code (e.g., "not_found", "validation_error")
    ///
    /// Clients can use this for programmatic error handling.
    pub code: String,

    /// Human-readable error message
    ///
    /// This is suitable for displaying to end users.
    pub message: String,

    /// Optional additional context about the error
    ///
    /// This can include fields like download_id, file paths, validation errors, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ApiError {
    /// Create a new API error with code and message
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                code: code.into(),
                message: message.into(),
                details: None,
            },
        }
    }

    /// Create an API error with additional details
    pub fn with_details(
        code: impl Into<String>,
        message: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self {
            error: ErrorDetail {
                code: code.into(),
                message: message.into(),
                details: Some(details),
            },
        }
    }

    /// Create a "not found" error
    pub fn not_found(resource: impl Into<String>) -> Self {
        Self::new("not_found", format!("{} not found", resource.into()))
    }

    /// Create a "validation error" error
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new("validation_error", message)
    }

    /// Create a "conflict" error
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new("conflict", message)
    }

    /// Create an "internal server error"
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal_error", message)
    }

    /// Create an "unauthorized" error
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new("unauthorized", message)
    }

    /// Create a "service unavailable" error
    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new("service_unavailable", message)
    }
}

/// Convert errors to HTTP status codes for API responses
///
/// This trait maps domain errors to appropriate HTTP status codes.
pub trait ToHttpStatus {
    /// Get the HTTP status code for this error
    fn status_code(&self) -> u16;

    /// Get the machine-readable error code
    fn error_code(&self) -> &str;
}

impl ToHttpStatus for Error {
    fn status_code(&self) -> u16 {
        match self {
            // 400 Bad Request - Client error (invalid input)
            Error::Config { .. } => 400,
            Error::InvalidNzb(_) => 422, // Unprocessable Entity
            Error::Duplicate(_) => 409,  // Conflict

            // 404 Not Found
            Error::NotFound(_) => 404,
            Error::Download(DownloadError::NotFound { .. }) => 404,
            Error::Download(DownloadError::FilesNotFound { .. }) => 404,

            // 409 Conflict - Resource already in desired state
            Error::Download(DownloadError::AlreadyInState { .. }) => 409,
            Error::Download(DownloadError::InvalidState { .. }) => 409,

            // 422 Unprocessable Entity - Semantic errors
            Error::PostProcess(_) => 422,
            Error::Download(DownloadError::InsufficientSpace { .. }) => 422,
            Error::InsufficientSpace { .. } => 422,

            // 500 Internal Server Error - Server-side issues
            Error::Database(_) => 500,
            Error::Sqlx(_) => 500,
            Error::Io(_) => 500,
            Error::ApiServerError(_) => 500,
            Error::FolderWatch(_) => 500,
            Error::DiskSpaceCheckFailed(_) => 500,
            Error::Other(_) => 500,

            // 502 Bad Gateway - External service errors
            Error::Nntp(_) => 502,
            Error::Network(_) => 502,

            // 503 Service Unavailable
            Error::ShuttingDown => 503,
            Error::ExternalTool(_) => 503,

            // 501 Not Implemented - Feature not supported
            Error::NotSupported(_) => 501,

            // 500 for serialization errors
            Error::Serialization(_) => 500,
        }
    }

    fn error_code(&self) -> &str {
        match self {
            Error::Config { .. } => "config_error",
            Error::Database(_) => "database_error",
            Error::Sqlx(_) => "database_error",
            Error::Nntp(_) => "nntp_error",
            Error::Download(e) => match e {
                DownloadError::NotFound { .. } => "download_not_found",
                DownloadError::FilesNotFound { .. } => "files_not_found",
                DownloadError::AlreadyInState { .. } => "already_in_state",
                DownloadError::InvalidState { .. } => "invalid_state",
                DownloadError::InsufficientSpace { .. } => "insufficient_space",
            },
            Error::PostProcess(e) => match e {
                PostProcessError::VerificationFailed { .. } => "verification_failed",
                PostProcessError::RepairFailed { .. } => "repair_failed",
                PostProcessError::ExtractionFailed { .. } => "extraction_failed",
                PostProcessError::WrongPassword { .. } => "wrong_password",
                PostProcessError::AllPasswordsFailed { .. } => "all_passwords_failed",
                PostProcessError::NoPasswordsAvailable { .. } => "no_passwords_available",
                PostProcessError::MoveFailed { .. } => "move_failed",
                PostProcessError::FileCollision { .. } => "file_collision",
                PostProcessError::CleanupFailed { .. } => "cleanup_failed",
                PostProcessError::InvalidPath { .. } => "invalid_path",
                PostProcessError::DirectUnpackFailed { .. } => "direct_unpack_failed",
                PostProcessError::DirectRenameFailed { .. } => "direct_rename_failed",
            },
            Error::InvalidNzb(_) => "invalid_nzb",
            Error::Io(_) => "io_error",
            Error::NotFound(_) => "not_found",
            Error::ShuttingDown => "shutting_down",
            Error::Network(_) => "network_error",
            Error::Serialization(_) => "serialization_error",
            Error::ApiServerError(_) => "api_server_error",
            Error::FolderWatch(_) => "folder_watch_error",
            Error::Duplicate(_) => "duplicate",
            Error::InsufficientSpace { .. } => "insufficient_space",
            Error::DiskSpaceCheckFailed(_) => "disk_space_check_failed",
            Error::ExternalTool(_) => "external_tool_error",
            Error::NotSupported(_) => "not_supported",
            Error::Other(_) => "internal_error",
        }
    }
}

impl From<Error> for ApiError {
    fn from(error: Error) -> Self {
        let code = error.error_code().to_string();
        let message = error.to_string();

        // Add contextual details for specific error types
        let details = match &error {
            Error::Download(DownloadError::NotFound { id }) => Some(serde_json::json!({
                "download_id": id,
            })),
            Error::Download(DownloadError::FilesNotFound { id, path }) => Some(serde_json::json!({
                "download_id": id,
                "path": path,
            })),
            Error::Download(DownloadError::AlreadyInState { id, state }) => {
                Some(serde_json::json!({
                    "download_id": id,
                    "state": state,
                }))
            }
            Error::Download(DownloadError::InvalidState {
                id,
                operation,
                current_state,
            }) => Some(serde_json::json!({
                "download_id": id,
                "operation": operation,
                "current_state": current_state,
            })),
            Error::Download(DownloadError::InsufficientSpace {
                required,
                available,
            }) => Some(serde_json::json!({
                "required_bytes": required,
                "available_bytes": available,
            })),
            Error::InsufficientSpace {
                required,
                available,
            } => Some(serde_json::json!({
                "required_bytes": required,
                "available_bytes": available,
            })),
            Error::PostProcess(PostProcessError::FileCollision { path, .. }) => {
                Some(serde_json::json!({
                    "path": path,
                }))
            }
            Error::PostProcess(PostProcessError::WrongPassword { archive }) => {
                Some(serde_json::json!({
                    "archive": archive,
                }))
            }
            Error::PostProcess(PostProcessError::AllPasswordsFailed { archive, count }) => {
                Some(serde_json::json!({
                    "archive": archive,
                    "password_count": count,
                }))
            }
            _ => None,
        };

        ApiError {
            error: ErrorDetail {
                code,
                message,
                details,
            },
        }
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Helpers: construct every Error variant for status/error_code tests
    // -----------------------------------------------------------------------

    /// Returns a vec of (Error, expected_status_code, expected_error_code) for
    /// every reachable match arm in ToHttpStatus.
    fn all_error_variants() -> Vec<(Error, u16, &'static str)> {
        vec![
            // Top-level variants
            (
                Error::Config {
                    message: "bad value".into(),
                    key: Some("download_dir".into()),
                },
                400,
                "config_error",
            ),
            (
                Error::InvalidNzb("missing segments".into()),
                422,
                "invalid_nzb",
            ),
            (Error::Duplicate("already queued".into()), 409, "duplicate"),
            (Error::NotFound("download 99".into()), 404, "not_found"),
            (
                Error::Database(DatabaseError::QueryFailed("timeout".into())),
                500,
                "database_error",
            ),
            (
                Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "gone")),
                500,
                "io_error",
            ),
            (
                Error::ApiServerError("bind failed".into()),
                500,
                "api_server_error",
            ),
            (
                Error::FolderWatch("inotify error".into()),
                500,
                "folder_watch_error",
            ),
            (
                Error::DiskSpaceCheckFailed("statvfs failed".into()),
                500,
                "disk_space_check_failed",
            ),
            (Error::Other("unknown".into()), 500, "internal_error"),
            (Error::Nntp("connection reset".into()), 502, "nntp_error"),
            (Error::ShuttingDown, 503, "shutting_down"),
            (
                Error::ExternalTool("par2 not found".into()),
                503,
                "external_tool_error",
            ),
            (
                Error::NotSupported("par2 binary missing".into()),
                501,
                "not_supported",
            ),
            (
                Error::InsufficientSpace {
                    required: 1_000_000,
                    available: 500,
                },
                422,
                "insufficient_space",
            ),
            // DownloadError variants
            (
                Error::Download(DownloadError::NotFound { id: 42 }),
                404,
                "download_not_found",
            ),
            (
                Error::Download(DownloadError::FilesNotFound {
                    id: 42,
                    path: PathBuf::from("/tmp/dl"),
                }),
                404,
                "files_not_found",
            ),
            (
                Error::Download(DownloadError::AlreadyInState {
                    id: 42,
                    state: "paused".into(),
                }),
                409,
                "already_in_state",
            ),
            (
                Error::Download(DownloadError::InvalidState {
                    id: 42,
                    operation: "pause".into(),
                    current_state: "completed".into(),
                }),
                409,
                "invalid_state",
            ),
            (
                Error::Download(DownloadError::InsufficientSpace {
                    required: 2_000_000,
                    available: 1_000,
                }),
                422,
                "insufficient_space",
            ),
            // PostProcessError variants
            (
                Error::PostProcess(PostProcessError::VerificationFailed {
                    id: 1,
                    reason: "corrupt".into(),
                }),
                422,
                "verification_failed",
            ),
            (
                Error::PostProcess(PostProcessError::RepairFailed {
                    id: 1,
                    reason: "too many missing".into(),
                }),
                422,
                "repair_failed",
            ),
            (
                Error::PostProcess(PostProcessError::ExtractionFailed {
                    archive: PathBuf::from("test.rar"),
                    reason: "crc error".into(),
                }),
                422,
                "extraction_failed",
            ),
            (
                Error::PostProcess(PostProcessError::WrongPassword {
                    archive: PathBuf::from("secret.rar"),
                }),
                422,
                "wrong_password",
            ),
            (
                Error::PostProcess(PostProcessError::AllPasswordsFailed {
                    archive: PathBuf::from("secret.rar"),
                    count: 5,
                }),
                422,
                "all_passwords_failed",
            ),
            (
                Error::PostProcess(PostProcessError::NoPasswordsAvailable {
                    archive: PathBuf::from("secret.rar"),
                }),
                422,
                "no_passwords_available",
            ),
            (
                Error::PostProcess(PostProcessError::MoveFailed {
                    source_path: PathBuf::from("/tmp/a"),
                    dest_path: PathBuf::from("/tmp/b"),
                    reason: "permission denied".into(),
                }),
                422,
                "move_failed",
            ),
            (
                Error::PostProcess(PostProcessError::FileCollision {
                    path: PathBuf::from("/dest/file.mkv"),
                    reason: "file already exists".into(),
                }),
                422,
                "file_collision",
            ),
            (
                Error::PostProcess(PostProcessError::CleanupFailed {
                    id: 1,
                    reason: "directory not empty".into(),
                }),
                422,
                "cleanup_failed",
            ),
            (
                Error::PostProcess(PostProcessError::InvalidPath {
                    path: PathBuf::from("../escape"),
                    reason: "path traversal".into(),
                }),
                422,
                "invalid_path",
            ),
            (
                Error::PostProcess(PostProcessError::DirectUnpackFailed {
                    id: 1,
                    reason: "extraction error".into(),
                }),
                422,
                "direct_unpack_failed",
            ),
            (
                Error::PostProcess(PostProcessError::DirectRenameFailed {
                    id: 1,
                    reason: "rename error".into(),
                }),
                422,
                "direct_rename_failed",
            ),
        ]
    }

    // -----------------------------------------------------------------------
    // 1. Every Error variant -> correct HTTP status code
    // -----------------------------------------------------------------------

    #[test]
    fn every_variant_maps_to_expected_status_code() {
        for (error, expected_status, expected_code) in all_error_variants() {
            let actual_status = error.status_code();
            assert_eq!(
                actual_status, expected_status,
                "Error variant with error_code={expected_code} returned status {actual_status}, expected {expected_status}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 2. Every Error variant -> correct machine-readable error code
    // -----------------------------------------------------------------------

    #[test]
    fn every_variant_maps_to_expected_error_code() {
        for (error, expected_status, expected_code) in all_error_variants() {
            let actual_code = error.error_code();
            assert_eq!(
                actual_code, expected_code,
                "Error variant with expected status={expected_status} returned error_code={actual_code}, expected {expected_code}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Targeted status code tests for boundary categories to catch regressions
    // if someone moves a variant between match arms.
    // -----------------------------------------------------------------------

    #[test]
    fn config_error_is_400_not_500() {
        let err = Error::Config {
            message: "bad".into(),
            key: None,
        };
        assert_eq!(err.status_code(), 400);
    }

    #[test]
    fn invalid_nzb_is_422_not_400() {
        let err = Error::InvalidNzb("bad xml".into());
        assert_eq!(err.status_code(), 422);
    }

    #[test]
    fn duplicate_is_409_conflict() {
        let err = Error::Duplicate("same hash".into());
        assert_eq!(err.status_code(), 409);
    }

    #[test]
    fn download_not_found_is_404() {
        let err = Error::Download(DownloadError::NotFound { id: 1 });
        assert_eq!(err.status_code(), 404);
    }

    #[test]
    fn download_files_not_found_is_404() {
        let err = Error::Download(DownloadError::FilesNotFound {
            id: 1,
            path: PathBuf::from("/tmp"),
        });
        assert_eq!(err.status_code(), 404);
    }

    #[test]
    fn download_already_in_state_is_409() {
        let err = Error::Download(DownloadError::AlreadyInState {
            id: 1,
            state: "paused".into(),
        });
        assert_eq!(err.status_code(), 409);
    }

    #[test]
    fn download_invalid_state_is_409() {
        let err = Error::Download(DownloadError::InvalidState {
            id: 1,
            operation: "resume".into(),
            current_state: "completed".into(),
        });
        assert_eq!(err.status_code(), 409);
    }

    #[test]
    fn nntp_error_is_502_bad_gateway() {
        let err = Error::Nntp("connection refused".into());
        assert_eq!(err.status_code(), 502);
    }

    #[test]
    fn shutting_down_is_503() {
        assert_eq!(Error::ShuttingDown.status_code(), 503);
    }

    #[test]
    fn not_supported_is_501() {
        let err = Error::NotSupported("feature X".into());
        assert_eq!(err.status_code(), 501);
    }

    // -----------------------------------------------------------------------
    // 3. Error -> ApiError preserves structured details
    // -----------------------------------------------------------------------

    #[test]
    fn api_error_from_download_not_found_has_download_id() {
        let err = Error::Download(DownloadError::NotFound { id: 42 });
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "download_not_found");
        let details = api.error.details.expect("should have details");
        assert_eq!(details["download_id"], 42);
    }

    #[test]
    fn api_error_from_download_files_not_found_has_id_and_path() {
        let err = Error::Download(DownloadError::FilesNotFound {
            id: 7,
            path: PathBuf::from("/data/downloads/test"),
        });
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "files_not_found");
        let details = api.error.details.expect("should have details");
        assert_eq!(details["download_id"], 7);
        assert_eq!(details["path"], "/data/downloads/test");
    }

    #[test]
    fn api_error_from_already_in_state_has_id_and_state() {
        let err = Error::Download(DownloadError::AlreadyInState {
            id: 10,
            state: "paused".into(),
        });
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "already_in_state");
        let details = api.error.details.expect("should have details");
        assert_eq!(details["download_id"], 10);
        assert_eq!(details["state"], "paused");
    }

    #[test]
    fn api_error_from_invalid_state_has_operation_and_current_state() {
        let err = Error::Download(DownloadError::InvalidState {
            id: 3,
            operation: "pause".into(),
            current_state: "completed".into(),
        });
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "invalid_state");
        let details = api.error.details.expect("should have details");
        assert_eq!(details["download_id"], 3);
        assert_eq!(details["operation"], "pause");
        assert_eq!(details["current_state"], "completed");
    }

    #[test]
    fn api_error_from_download_insufficient_space_has_byte_counts() {
        let err = Error::Download(DownloadError::InsufficientSpace {
            required: 5_000_000,
            available: 1_000,
        });
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "insufficient_space");
        let details = api.error.details.expect("should have details");
        assert_eq!(details["required_bytes"], 5_000_000_u64);
        assert_eq!(details["available_bytes"], 1_000_u64);
    }

    #[test]
    fn api_error_from_top_level_insufficient_space_has_byte_counts() {
        let err = Error::InsufficientSpace {
            required: 9_999_999,
            available: 42,
        };
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "insufficient_space");
        let details = api.error.details.expect("should have details");
        assert_eq!(details["required_bytes"], 9_999_999_u64);
        assert_eq!(details["available_bytes"], 42_u64);
    }

    #[test]
    fn api_error_from_all_passwords_failed_has_archive_and_count() {
        let err = Error::PostProcess(PostProcessError::AllPasswordsFailed {
            archive: PathBuf::from("/tmp/secret.rar"),
            count: 15,
        });
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "all_passwords_failed");
        let details = api.error.details.expect("should have details");
        assert_eq!(details["archive"], "/tmp/secret.rar");
        assert_eq!(details["password_count"], 15);
    }

    #[test]
    fn api_error_from_wrong_password_has_archive() {
        let err = Error::PostProcess(PostProcessError::WrongPassword {
            archive: PathBuf::from("/tmp/encrypted.rar"),
        });
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "wrong_password");
        let details = api.error.details.expect("should have details");
        assert_eq!(details["archive"], "/tmp/encrypted.rar");
    }

    #[test]
    fn api_error_from_file_collision_has_path() {
        let err = Error::PostProcess(PostProcessError::FileCollision {
            path: PathBuf::from("/dest/movie.mkv"),
            reason: "file already exists".into(),
        });
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "file_collision");
        let details = api.error.details.expect("should have details");
        assert_eq!(details["path"], "/dest/movie.mkv");
    }

    // -----------------------------------------------------------------------
    // 4. Error -> ApiError produces None details for context-free variants
    // -----------------------------------------------------------------------

    #[test]
    fn api_error_from_io_has_no_details() {
        let err = Error::Io(std::io::Error::other("disk fail"));
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "io_error");
        assert!(
            api.error.details.is_none(),
            "Io errors should not have structured details"
        );
    }

    #[test]
    fn api_error_from_nntp_has_no_details() {
        let err = Error::Nntp("timeout".into());
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "nntp_error");
        assert!(
            api.error.details.is_none(),
            "NNTP errors should not have structured details"
        );
    }

    #[test]
    fn api_error_from_shutting_down_has_no_details() {
        let api: ApiError = Error::ShuttingDown.into();

        assert_eq!(api.error.code, "shutting_down");
        assert!(
            api.error.details.is_none(),
            "ShuttingDown should not have structured details"
        );
    }

    #[test]
    fn api_error_from_config_has_no_details() {
        let err = Error::Config {
            message: "invalid port".into(),
            key: Some("server.port".into()),
        };
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "config_error");
        assert!(
            api.error.details.is_none(),
            "Config errors should not have structured details"
        );
    }

    #[test]
    fn api_error_from_database_has_no_details() {
        let err = Error::Database(DatabaseError::ConnectionFailed("refused".into()));
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "database_error");
        assert!(
            api.error.details.is_none(),
            "Database errors should not have structured details"
        );
    }

    #[test]
    fn api_error_from_not_found_string_has_no_details() {
        let err = Error::NotFound("download 99".into());
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "not_found");
        assert!(
            api.error.details.is_none(),
            "Top-level NotFound(String) should not have structured details"
        );
    }

    #[test]
    fn api_error_from_other_has_no_details() {
        let err = Error::Other("something went wrong".into());
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "internal_error");
        assert!(api.error.details.is_none());
    }

    #[test]
    fn api_error_from_external_tool_has_no_details() {
        let err = Error::ExternalTool("unrar not found".into());
        let api: ApiError = err.into();

        assert_eq!(api.error.code, "external_tool_error");
        assert!(api.error.details.is_none());
    }

    #[test]
    fn api_error_from_postprocess_without_details_has_none() {
        // PostProcessError variants NOT in the details match arm
        let variants: Vec<Error> = vec![
            Error::PostProcess(PostProcessError::VerificationFailed {
                id: 1,
                reason: "corrupt".into(),
            }),
            Error::PostProcess(PostProcessError::RepairFailed {
                id: 1,
                reason: "too damaged".into(),
            }),
            Error::PostProcess(PostProcessError::ExtractionFailed {
                archive: PathBuf::from("test.rar"),
                reason: "crc error".into(),
            }),
            Error::PostProcess(PostProcessError::NoPasswordsAvailable {
                archive: PathBuf::from("secret.rar"),
            }),
            Error::PostProcess(PostProcessError::MoveFailed {
                source_path: PathBuf::from("/a"),
                dest_path: PathBuf::from("/b"),
                reason: "denied".into(),
            }),
            Error::PostProcess(PostProcessError::CleanupFailed {
                id: 1,
                reason: "locked".into(),
            }),
            Error::PostProcess(PostProcessError::InvalidPath {
                path: PathBuf::from("../bad"),
                reason: "traversal".into(),
            }),
        ];

        for err in variants {
            let code = err.error_code().to_string();
            let api: ApiError = err.into();
            assert!(
                api.error.details.is_none(),
                "PostProcessError with code={code} should not have structured details"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 5. ApiError factory methods produce correct codes and messages
    // -----------------------------------------------------------------------

    #[test]
    fn api_error_not_found_factory() {
        let api = ApiError::not_found("Download 123");

        assert_eq!(api.error.code, "not_found");
        assert_eq!(api.error.message, "Download 123 not found");
        assert!(api.error.details.is_none());
    }

    #[test]
    fn api_error_validation_factory() {
        let api = ApiError::validation("name is required");

        assert_eq!(api.error.code, "validation_error");
        assert_eq!(api.error.message, "name is required");
        assert!(api.error.details.is_none());
    }

    #[test]
    fn api_error_conflict_factory() {
        let api = ApiError::conflict("download already exists");

        assert_eq!(api.error.code, "conflict");
        assert_eq!(api.error.message, "download already exists");
        assert!(api.error.details.is_none());
    }

    #[test]
    fn api_error_internal_factory() {
        let api = ApiError::internal("unexpected failure");

        assert_eq!(api.error.code, "internal_error");
        assert_eq!(api.error.message, "unexpected failure");
        assert!(api.error.details.is_none());
    }

    #[test]
    fn api_error_unauthorized_factory() {
        let api = ApiError::unauthorized("invalid token");

        assert_eq!(api.error.code, "unauthorized");
        assert_eq!(api.error.message, "invalid token");
        assert!(api.error.details.is_none());
    }

    #[test]
    fn api_error_service_unavailable_factory() {
        let api = ApiError::service_unavailable("server overloaded");

        assert_eq!(api.error.code, "service_unavailable");
        assert_eq!(api.error.message, "server overloaded");
        assert!(api.error.details.is_none());
    }

    // -----------------------------------------------------------------------
    // 6. ApiError::with_details serializes details correctly
    // -----------------------------------------------------------------------

    #[test]
    fn with_details_preserves_json_object() {
        let details = serde_json::json!({
            "download_id": 42,
            "path": "/tmp/test",
            "retries": 3,
        });
        let api = ApiError::with_details("custom_error", "something broke", details.clone());

        assert_eq!(api.error.code, "custom_error");
        assert_eq!(api.error.message, "something broke");
        let actual_details = api.error.details.expect("details should be present");
        assert_eq!(actual_details, details);
    }

    #[test]
    fn with_details_serializes_to_json_with_details_field() {
        let api = ApiError::with_details(
            "test_code",
            "test message",
            serde_json::json!({"key": "value"}),
        );

        let json_str = serde_json::to_string(&api).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["error"]["code"], "test_code");
        assert_eq!(parsed["error"]["message"], "test message");
        assert_eq!(parsed["error"]["details"]["key"], "value");
    }

    #[test]
    fn api_error_without_details_omits_details_in_json() {
        let api = ApiError::new("test_code", "test message");

        let json_str = serde_json::to_string(&api).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["error"]["code"], "test_code");
        assert_eq!(parsed["error"]["message"], "test message");
        // skip_serializing_if = "Option::is_none" should omit the field entirely
        assert!(
            parsed["error"].get("details").is_none(),
            "details field should be omitted from JSON when None"
        );
    }

    #[test]
    fn api_error_round_trips_through_json() {
        let original = ApiError::with_details(
            "not_found",
            "Download 42 not found",
            serde_json::json!({"download_id": 42}),
        );

        let json_str = serde_json::to_string(&original).unwrap();
        let deserialized: ApiError = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.error.code, original.error.code);
        assert_eq!(deserialized.error.message, original.error.message);
        assert_eq!(deserialized.error.details, original.error.details);
    }

    // -----------------------------------------------------------------------
    // Verify that Error -> ApiError preserves the Display message
    // -----------------------------------------------------------------------

    #[test]
    fn api_error_message_matches_error_display() {
        let err = Error::Download(DownloadError::InvalidState {
            id: 5,
            operation: "resume".into(),
            current_state: "completed".into(),
        });
        let display_msg = err.to_string();
        let api: ApiError = err.into();

        assert_eq!(
            api.error.message, display_msg,
            "ApiError message should match the Error's Display output"
        );
    }

    #[test]
    fn api_error_from_nntp_preserves_display_message_and_maps_to_502() {
        let err = Error::Nntp("connection reset by peer".into());
        let display_msg = err.to_string();
        let status = err.status_code();
        let api: ApiError = err.into();

        assert_eq!(status, 502, "NNTP errors must map to 502 Bad Gateway");
        assert_eq!(api.error.code, "nntp_error");
        assert_eq!(
            api.error.message, display_msg,
            "ApiError message must match Error::Nntp Display output"
        );
        assert!(
            api.error.message.contains("connection reset by peer"),
            "ApiError message must contain the original NNTP error string"
        );
        assert!(
            api.error.details.is_none(),
            "NNTP errors should not have structured details"
        );
    }

    #[test]
    fn api_error_message_for_insufficient_space_includes_byte_counts() {
        let err = Error::InsufficientSpace {
            required: 1_048_576,
            available: 512,
        };
        let api: ApiError = err.into();

        assert!(
            api.error.message.contains("1048576"),
            "message should contain the required bytes"
        );
        assert!(
            api.error.message.contains("512"),
            "message should contain the available bytes"
        );
    }
}
