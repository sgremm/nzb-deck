use crate::models::{AppSettings, BackendStatus, JobView};
use crate::parity::NativeParityHandler;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, RwLock};
use usenet_dl::config::{DuplicateAction, PostProcess};
use usenet_dl::{Config, Database, ServerConfig, Status, UsenetDownloader};

pub struct AppPaths {
    pub data_dir: PathBuf,
    pub settings_file: PathBuf,
    pub database_file: PathBuf,
    pub nzb_dir: PathBuf,
    pub temp_dir: PathBuf,
}

pub struct AppState {
    pub paths: AppPaths,
    pub settings: RwLock<AppSettings>,
    pub downloader: RwLock<Option<UsenetDownloader>>,
    pub import_lock: Mutex<()>,
    pub database: Arc<Database>,
    pub connection_error: RwLock<Option<String>>,
}

impl AppState {
    pub async fn new(app: &AppHandle, data_dir: PathBuf) -> Result<Arc<Self>, String> {
        let paths = AppPaths {
            settings_file: data_dir.join("settings.json"),
            database_file: data_dir.join("downloads.sqlite3"),
            nzb_dir: data_dir.join("nzbs"),
            temp_dir: data_dir.join("temporary"),
            data_dir,
        };
        tokio::fs::create_dir_all(&paths.nzb_dir)
            .await
            .map_err(|error| error.to_string())?;
        tokio::fs::create_dir_all(&paths.temp_dir)
            .await
            .map_err(|error| error.to_string())?;

        let default_download_dir = paths.data_dir.join("Downloads");
        tokio::fs::create_dir_all(&default_download_dir)
            .await
            .map_err(|error| error.to_string())?;
        let settings = load_settings(&paths.settings_file, &default_download_dir).await;
        let database = Arc::new(
            Database::new(&paths.database_file)
                .await
                .map_err(|error| error.to_string())?,
        );

        let state = Arc::new(Self {
            paths,
            settings: RwLock::new(settings.clone()),
            downloader: RwLock::new(None),
            import_lock: Mutex::new(()),
            database,
            connection_error: RwLock::new(None),
        });
        state.migrate_legacy_destinations().await?;
        state.replace_downloader(app, settings).await;
        Ok(state)
    }

    pub async fn replace_downloader(&self, app: &AppHandle, settings: AppSettings) {
        if let Some(previous) = self.downloader.write().await.take() {
            let _ = previous.shutdown().await;
        }

        let configured = settings.is_configured();
        let config = self.make_config(&settings, configured);
        match UsenetDownloader::new(config).await {
            Ok(downloader) => {
                *self.connection_error.write().await = None;
                self.install_downloader(app, downloader, configured).await;
            }
            Err(error) if configured => {
                *self.connection_error.write().await = Some(error.to_string());
                let offline_config = self.make_config(&settings, false);
                match UsenetDownloader::new(offline_config).await {
                    Ok(downloader) => self.install_downloader(app, downloader, false).await,
                    Err(fallback_error) => {
                        *self.connection_error.write().await = Some(format!(
                            "{error}; offline database initialization failed: {fallback_error}"
                        ));
                    }
                }
            }
            Err(error) => {
                *self.connection_error.write().await = Some(error.to_string());
            }
        }
    }

    async fn install_downloader(
        &self,
        app: &AppHandle,
        downloader: UsenetDownloader,
        start_queue: bool,
    ) {
        let mut events = downloader.subscribe();
        let event_app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        let _ = event_app.emit("download-event", event);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        if start_queue {
            downloader.start_queue_processor();
        }
        *self.downloader.write().await = Some(downloader);
    }

    fn make_config(&self, settings: &AppSettings, include_server: bool) -> Config {
        let mut config = Config::default();
        config.persistence.database_path = self.paths.database_file.clone();
        config.download.download_dir = PathBuf::from(&settings.download_dir);
        config.download.temp_dir = self.paths.temp_dir.clone();
        config.download.max_concurrent_downloads = settings.max_concurrent_downloads.max(1);
        config.download.default_post_process = PostProcess::UnpackAndCleanup;
        config.processing.duplicate.action = DuplicateAction::Allow;
        config.tools.search_path = false;
        config.tools.parity_handler = Some(Arc::new(NativeParityHandler));
        config.notifications.scripts.clear();
        config.servers = if include_server {
            vec![ServerConfig {
                host: settings.host.trim().to_string(),
                port: settings.port,
                tls: settings.tls,
                username: nonempty(&settings.username),
                password: nonempty(&settings.password),
                connections: settings.connections.clamp(1, 50),
                priority: 0,
                pipeline_depth: 10,
            }]
        } else {
            Vec::new()
        };
        config
    }

    pub async fn save_settings(
        &self,
        app: &AppHandle,
        settings: AppSettings,
    ) -> Result<(), String> {
        if settings.download_dir.trim().is_empty() {
            return Err("Ein Download-Ordner ist erforderlich".to_string());
        }
        tokio::fs::create_dir_all(&settings.download_dir)
            .await
            .map_err(|error| format!("Download-Ordner konnte nicht angelegt werden: {error}"))?;
        let bytes = serde_json::to_vec_pretty(&settings).map_err(|error| error.to_string())?;
        tokio::fs::write(&self.paths.settings_file, bytes)
            .await
            .map_err(|error| error.to_string())?;
        *self.settings.write().await = settings.clone();
        self.replace_downloader(app, settings).await;
        if let Some(error) = self.connection_error.read().await.clone() {
            return Err(error);
        }
        Ok(())
    }

    pub async fn jobs(&self) -> Result<Vec<JobView>, String> {
        let mut rows = self
            .database
            .list_downloads()
            .await
            .map_err(|error| error.to_string())?;
        rows.sort_by_key(|job| std::cmp::Reverse(job.created_at));
        Ok(rows
            .into_iter()
            .map(|job| {
                let status = Status::from_i32(job.status);
                JobView {
                    id: job.id,
                    name: job.name,
                    status: status_name(status).to_string(),
                    stage: stage_name(status).to_string(),
                    progress: job.progress.clamp(0.0, 100.0),
                    speed_bps: job.speed_bps.max(0) as u64,
                    size_bytes: job.size_bytes.max(0) as u64,
                    downloaded_bytes: job.downloaded_bytes.max(0) as u64,
                    error: job.error_message,
                    destination: job.destination,
                    created_at: job.created_at,
                    completed_at: job.completed_at,
                }
            })
            .collect())
    }

    pub async fn next_job_destination(&self, name: &str) -> Result<PathBuf, String> {
        let root = PathBuf::from(&self.settings.read().await.download_dir);
        let folder = safe_job_folder(name);
        let downloads = self
            .database
            .list_downloads()
            .await
            .map_err(|error| error.to_string())?;
        let occupied = downloads
            .into_iter()
            .map(|download| PathBuf::from(download.destination))
            .collect::<std::collections::HashSet<_>>();

        for suffix in 1usize.. {
            let folder_name = if suffix == 1 {
                folder.clone()
            } else {
                format!("{folder} ({suffix})")
            };
            let candidate = root.join(folder_name);
            let exists = tokio::fs::try_exists(&candidate)
                .await
                .map_err(|error| format!("Zielordner konnte nicht geprüft werden: {error}"))?;
            if !exists && !occupied.contains(&candidate) {
                return Ok(candidate);
            }
        }
        unreachable!("der numerische Zielordner-Suffix ist unbeschränkt")
    }

    async fn migrate_legacy_destinations(&self) -> Result<(), String> {
        let root = PathBuf::from(&self.settings.read().await.download_dir);
        let downloads = self
            .database
            .list_downloads()
            .await
            .map_err(|error| error.to_string())?;
        for download in downloads {
            if PathBuf::from(&download.destination) != root {
                continue;
            }
            let destination = self.next_job_destination(&download.name).await?;
            self.database
                .update_destination(
                    usenet_dl::DownloadId(download.id),
                    &destination.to_string_lossy(),
                )
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub async fn status(&self) -> BackendStatus {
        let settings = self.settings.read().await;
        let error = self.connection_error.read().await.clone();
        BackendStatus {
            configured: settings.is_configured(),
            connected: settings.is_configured() && error.is_none(),
            error,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

async fn load_settings(path: &Path, default_download_dir: &Path) -> AppSettings {
    if let Ok(bytes) = tokio::fs::read(path).await
        && let Ok(settings) = serde_json::from_slice(&bytes)
    {
        return settings;
    }
    AppSettings {
        host: String::new(),
        port: 563,
        tls: true,
        username: String::new(),
        password: String::new(),
        connections: 8,
        max_concurrent_downloads: 2,
        download_dir: default_download_dir.to_string_lossy().into_owned(),
    }
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Queued => "queued",
        Status::Downloading => "downloading",
        Status::Paused => "paused",
        Status::Processing => "processing",
        Status::Complete => "complete",
        Status::Failed => "failed",
    }
}

fn stage_name(status: Status) -> &'static str {
    match status {
        Status::Queued => "queue",
        Status::Downloading => "download",
        Status::Paused => "paused",
        Status::Processing => "processing",
        Status::Complete => "complete",
        Status::Failed => "failed",
    }
}

fn safe_job_folder(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_control() || matches!(character, '/' | '\\' | ':') {
                '_'
            } else {
                character
            }
        })
        .take(120)
        .collect::<String>();
    let sanitized = sanitized.trim_matches(|character| character == ' ' || character == '.');
    if sanitized.is_empty() {
        "Usenet-Download".to_string()
    } else {
        sanitized.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::safe_job_folder;

    #[test]
    fn sanitizes_job_folder_names() {
        assert_eq!(
            safe_job_folder("  release:part/one.nzb. "),
            "release_part_one.nzb"
        );
        assert_eq!(safe_job_folder("..."), "Usenet-Download");
    }
}
