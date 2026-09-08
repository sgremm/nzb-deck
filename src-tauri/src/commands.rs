use crate::models::{AppSettings, BackendStatus, JobView};
use crate::state::AppState;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, State};
use usenet_dl::{DownloadId, DownloadOptions, ServerConfig, Status};

#[tauri::command]
pub async fn get_settings(state: State<'_, Arc<AppState>>) -> Result<AppSettings, String> {
    Ok(state.settings.read().await.clone())
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    settings: AppSettings,
) -> Result<(), String> {
    state.save_settings(&app, settings).await
}

#[tauri::command]
pub async fn backend_status(state: State<'_, Arc<AppState>>) -> Result<BackendStatus, String> {
    Ok(state.status().await)
}

#[tauri::command]
pub async fn list_jobs(state: State<'_, Arc<AppState>>) -> Result<Vec<JobView>, String> {
    state.jobs().await
}

#[tauri::command]
pub async fn import_nzbs(
    state: State<'_, Arc<AppState>>,
    paths: Vec<String>,
) -> Result<Vec<i64>, String> {
    import_paths(&state, paths.into_iter().map(PathBuf::from).collect()).await
}

pub async fn import_paths(state: &AppState, paths: Vec<PathBuf>) -> Result<Vec<i64>, String> {
    let _import_guard = state.import_lock.lock().await;
    let downloader_guard = state.downloader.read().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or_else(|| "The download engine is not available".to_string())?;
    let mut imported = Vec::with_capacity(paths.len());

    for path in paths {
        if !is_nzb(&path) {
            continue;
        }
        let content = tokio::fs::read(&path)
            .await
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("Usenet download");
        let destination = state.next_job_destination(name).await?;
        let options = DownloadOptions {
            destination: Some(destination),
            ..DownloadOptions::default()
        };
        let id = downloader
            .add_nzb_content(&content, name, options)
            .await
            .map_err(|error| format!("Could not import {}: {error}", path.display()))?;
        tokio::fs::write(state.paths.nzb_dir.join(format!("{}.nzb", id.0)), content)
            .await
            .map_err(|error| format!("Could not preserve NZB for rerun: {error}"))?;
        imported.push(id.0);
    }

    if imported.is_empty() {
        return Err("No NZB files were selected".to_string());
    }
    Ok(imported)
}

#[tauri::command]
pub async fn pause_job(state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    let guard = state.downloader.read().await;
    let downloader = guard
        .as_ref()
        .ok_or_else(|| "The download engine is not available".to_string())?;
    downloader
        .pause(DownloadId(id))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn resume_job(state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    let downloader_guard = state.downloader.read().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or_else(|| "The download engine is not available".to_string())?;
    let job = state
        .database
        .get_download(DownloadId(id))
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Download {id} was not found"))?;
    match Status::from_i32(job.status) {
        Status::Paused => downloader.resume(DownloadId(id)).await,
        Status::Failed => downloader.resume_download(DownloadId(id)).await,
        status => Err(usenet_dl::Error::Other(format!(
            "Cannot resume a {status:?} download"
        ))),
    }
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn rerun_job(state: State<'_, Arc<AppState>>, id: i64) -> Result<i64, String> {
    let _import_guard = state.import_lock.lock().await;
    let source_path = state.paths.nzb_dir.join(format!("{id}.nzb"));
    let content = tokio::fs::read(&source_path)
        .await
        .map_err(|_| "The preserved NZB is unavailable; this job cannot be rerun".to_string())?;
    let original = state
        .database
        .get_download(DownloadId(id))
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Download {id} was not found"))?;
    let destination = state.next_job_destination(&original.name).await?;
    let options = DownloadOptions {
        destination: Some(destination),
        ..DownloadOptions::default()
    };
    let downloader_guard = state.downloader.read().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or_else(|| "The download engine is not available".to_string())?;
    let new_id = downloader
        .add_nzb_content(&content, &original.name, options)
        .await
        .map_err(|error| error.to_string())?;
    tokio::fs::write(
        state.paths.nzb_dir.join(format!("{}.nzb", new_id.0)),
        content,
    )
    .await
    .map_err(|error| format!("Could not preserve NZB for rerun: {error}"))?;
    Ok(new_id.0)
}

#[tauri::command]
pub async fn reprocess_job(state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    let guard = state.downloader.read().await;
    let downloader = guard
        .as_ref()
        .ok_or_else(|| "The download engine is not available".to_string())?;
    downloader
        .reprocess(DownloadId(id))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn test_server(
    state: State<'_, Arc<AppState>>,
    settings: AppSettings,
) -> Result<String, String> {
    let downloader_guard = state.downloader.read().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or_else(|| "The download engine is not available".to_string())?;
    let result = downloader
        .test_server(&ServerConfig {
            host: settings.host.trim().to_string(),
            port: settings.port,
            tls: settings.tls,
            username: option(&settings.username),
            password: option(&settings.password),
            connections: settings.connections.clamp(1, 50),
            priority: 0,
            pipeline_depth: 10,
        })
        .await;
    if result.success {
        let latency = result
            .latency
            .map(|value| format!(" in {} ms", value.as_millis()))
            .unwrap_or_default();
        Ok(format!("Connection and authentication succeeded{latency}"))
    } else {
        Err(result
            .error
            .unwrap_or_else(|| "Server test failed".to_string()))
    }
}

fn is_nzb(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("nzb"))
}

/// Status, unter dem ein Auftrag entfernt werden darf (kein aktiver Lauf).
fn is_removable(status: Status) -> bool {
    matches!(status, Status::Complete | Status::Failed)
}

/// Entfernt den konservierten NZB-Export eines Auftrags (Wiederverwenden
/// wäre sonst später nicht mehr möglich – wird nur beim Löschen gemacht).
async fn remove_preserved_nzb(state: &AppState, id: i64) {
    let _ = tokio::fs::remove_file(state.paths.nzb_dir.join(format!("{id}.nzb"))).await;
}

#[tauri::command]
pub async fn delete_job(state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    let download = state
        .database
        .get_download(DownloadId(id))
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Download {id} wurde nicht gefunden"))?;
    if !is_removable(Status::from_i32(download.status)) {
        return Err(
            "Nur abgeschlossene oder fehlgeschlagene Downloads können gelöscht werden".into(),
        );
    }
    state
        .database
        .delete_download(DownloadId(id))
        .await
        .map_err(|error| error.to_string())?;
    remove_preserved_nzb(&state, id).await;
    Ok(())
}

#[tauri::command]
pub async fn clear_jobs(state: State<'_, Arc<AppState>>) -> Result<usize, String> {
    let downloads = state
        .database
        .list_downloads()
        .await
        .map_err(|error| error.to_string())?;
    let mut removed = 0;
    for download in downloads {
        if !is_removable(Status::from_i32(download.status)) {
            continue;
        }
        state
            .database
            .delete_download(DownloadId(download.id))
            .await
            .map_err(|error| error.to_string())?;
        remove_preserved_nzb(&state, download.id).await;
        removed += 1;
    }
    Ok(removed)
}

fn option(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}
