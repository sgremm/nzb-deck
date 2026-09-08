use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub username: String,
    pub password: String,
    pub connections: usize,
    pub max_concurrent_downloads: usize,
    pub download_dir: String,
}

impl AppSettings {
    pub fn is_configured(&self) -> bool {
        !self.host.trim().is_empty()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: i64,
    pub name: String,
    pub status: String,
    pub stage: String,
    pub progress: f32,
    pub speed_bps: u64,
    pub size_bytes: u64,
    pub downloaded_bytes: u64,
    pub error: Option<String>,
    pub destination: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    pub configured: bool,
    pub connected: bool,
    pub error: Option<String>,
}
