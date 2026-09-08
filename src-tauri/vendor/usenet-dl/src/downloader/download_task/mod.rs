//! Download task execution -- core download lifecycle and article fetching.
//!
//! Split into focused submodules:
//! - [`context`] - Shared state, article provider trait, output file management
//! - [`orchestration`] - Top-level download task lifecycle
//! - [`batching`] - Record fetching, batch preparation, parallel downloading
//! - [`batch_processor`] - Pipelined NNTP fetch, yEnc decode, per-article retry
//! - [`finalization`] - Result evaluation and final status

mod batch_processor;
mod batching;
mod context;
mod finalization;
mod orchestration;

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests;

// Re-export public API so consumers don't need to change
pub(crate) use context::{DownloadTaskContext, NntpArticleProvider};
pub(crate) use orchestration::run_download_task;
