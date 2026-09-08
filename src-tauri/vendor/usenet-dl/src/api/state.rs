//! Application state for the API server

use crate::{Config, UsenetDownloader};
use std::sync::Arc;

/// Shared application state accessible to all route handlers
///
/// This struct is cloned for each request (cheap Arc clone) and provides
/// access to the downloader instance and configuration.
#[derive(Clone)]
pub struct AppState {
    /// The main UsenetDownloader instance
    pub downloader: Arc<UsenetDownloader>,

    /// Configuration (for read access, runtime updates go through downloader)
    pub config: Arc<Config>,
}

impl AppState {
    /// Create a new AppState
    pub fn new(downloader: Arc<UsenetDownloader>, config: Arc<Config>) -> Self {
        Self { downloader, config }
    }
}
