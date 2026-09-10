use crate::models::{AppSettings, BackendStatus, JobView};
use crate::state::{AppState, server_config};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use usenet_dl::{DownloadId, DownloadOptions, Status, UsenetDownloader};

/// Ergebnis eines Importlaufs: erfolgreiche IDs und die Fehler der übrigen
/// Dateien. Ein Einzelfehler bricht den Lauf nicht mehr ab.
#[derive(Default)]
pub(crate) struct ImportOutcome {
    pub(crate) imported: Vec<i64>,
    pub(crate) errors: Vec<String>,
}

const ENGINE_UNAVAILABLE: &str = "Die Download-Engine ist nicht verfügbar";

impl ImportOutcome {
    fn engine_unavailable() -> Self {
        Self {
            imported: Vec::new(),
            errors: vec![ENGINE_UNAVAILABLE.to_string()],
        }
    }

    /// Deutsche, in der UI zeigbare Zusammenfassung der Fehler.
    pub(crate) fn error_text(&self) -> String {
        let joined = self.errors.join(" · ");
        if self.imported.is_empty() {
            if joined.is_empty() {
                "Es wurden keine NZB-Dateien ausgewählt".to_string()
            } else {
                joined
            }
        } else {
            format!("Teilweise fehlgeschlagen: {joined}")
        }
    }
}

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
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    paths: Vec<String>,
) -> Result<Vec<i64>, String> {
    let outcome = import_paths(&state, paths.into_iter().map(PathBuf::from).collect()).await;
    if !outcome.errors.is_empty() {
        // Teilfehler (und Totalausfall) als Event in die UI melden; der
        // Rückgabeweg deckt nur den Totalausfall als Command-Fehler ab.
        let _ = app.emit("nzb-import-error", outcome.error_text());
    }
    if outcome.imported.is_empty() {
        return Err(outcome.error_text());
    }
    Ok(outcome.imported)
}

pub async fn import_paths(state: &AppState, paths: Vec<PathBuf>) -> ImportOutcome {
    let _import_guard = state.import_lock.lock().await;
    let downloader_guard = state.downloader.read().await;
    let Some(downloader) = downloader_guard.as_ref() else {
        return ImportOutcome::engine_unavailable();
    };

    let mut outcome = ImportOutcome::default();
    for path in paths {
        if !is_nzb(&path) {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("Usenet-Download");
        let destination = match state.next_job_destination(name).await {
            Ok(destination) => destination,
            Err(error) => {
                outcome.errors.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        match import_single(downloader, &state.paths.nzb_dir, destination, &path, name).await {
            Ok(id) => outcome.imported.push(id),
            Err(error) => outcome.errors.push(error),
        }
    }
    outcome
}

/// Importiert genau eine NZB-Datei; Fehler betreffen nur diesen Auftrag.
async fn import_single(
    downloader: &UsenetDownloader,
    nzb_dir: &Path,
    destination: PathBuf,
    path: &Path,
    name: &str,
) -> Result<i64, String> {
    let content = tokio::fs::read(path)
        .await
        .map_err(|error| format!("{} konnte nicht gelesen werden: {error}", path.display()))?;
    let options = DownloadOptions {
        destination: Some(destination),
        ..DownloadOptions::default()
    };
    let id = downloader
        .add_nzb_content(&content, name, options)
        .await
        .map_err(|error| format!("Import von {} fehlgeschlagen: {error}", path.display()))?;
    tokio::fs::write(nzb_dir.join(format!("{}.nzb", id.0)), content)
        .await
        .map_err(|error| {
            format!("NZB für erneuten Download konnte nicht konserviert werden: {error}")
        })?;
    Ok(id.0)
}

#[tauri::command]
pub async fn pause_job(state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    let guard = state.downloader.read().await;
    let downloader = guard
        .as_ref()
        .ok_or_else(|| ENGINE_UNAVAILABLE.to_string())?;
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
        .ok_or_else(|| ENGINE_UNAVAILABLE.to_string())?;
    let job = state
        .database
        .get_download(DownloadId(id))
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Download {id} wurde nicht gefunden"))?;
    match Status::from_i32(job.status) {
        Status::Paused => downloader.resume(DownloadId(id)).await,
        Status::Failed => downloader.resume_download(DownloadId(id)).await,
        status => Err(usenet_dl::Error::Other(format!(
            "Ein Download im Status {status:?} kann nicht fortgesetzt werden"
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
        .map_err(|_| "Die konservierte NZB-Datei ist nicht verfügbar; dieser Auftrag kann nicht erneut gestartet werden".to_string())?;
    let original = state
        .database
        .get_download(DownloadId(id))
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Download {id} wurde nicht gefunden"))?;
    let destination = state.next_job_destination(&original.name).await?;
    let options = DownloadOptions {
        destination: Some(destination),
        ..DownloadOptions::default()
    };
    let downloader_guard = state.downloader.read().await;
    let downloader = downloader_guard
        .as_ref()
        .ok_or_else(|| ENGINE_UNAVAILABLE.to_string())?;
    let new_id = downloader
        .add_nzb_content(&content, &original.name, options)
        .await
        .map_err(|error| error.to_string())?;
    tokio::fs::write(
        state.paths.nzb_dir.join(format!("{}.nzb", new_id.0)),
        content,
    )
    .await
    .map_err(|error| {
        format!("NZB für erneuten Download konnte nicht konserviert werden: {error}")
    })?;
    Ok(new_id.0)
}

#[tauri::command]
pub async fn reprocess_job(state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    let guard = state.downloader.read().await;
    let downloader = guard
        .as_ref()
        .ok_or_else(|| ENGINE_UNAVAILABLE.to_string())?;
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
        .ok_or_else(|| ENGINE_UNAVAILABLE.to_string())?;
    let result = downloader.test_server(&server_config(&settings)).await;
    if result.success {
        let latency = result
            .latency
            .map(|value| format!(" in {} ms", value.as_millis()))
            .unwrap_or_default();
        Ok(format!("Verbindung und Anmeldung erfolgreich{latency}"))
    } else {
        Err(result
            .error
            .unwrap_or_else(|| "Servertest fehlgeschlagen".to_string()))
    }
}

pub(crate) fn is_nzb(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("nzb"))
}

/// Status, unter dem ein Auftrag entfernt werden darf (kein aktiver Lauf).
fn is_removable(status: Status) -> bool {
    matches!(status, Status::Complete | Status::Failed)
}

/// Entfernt die konservierte NZB und den Temp-Ordner eines Auftrags
/// (best-effort; Dateien im Zielordner bleiben bewusst erhalten).
async fn remove_job_artifacts(state: &AppState, id: i64) {
    let _ = tokio::fs::remove_file(state.paths.nzb_dir.join(format!("{id}.nzb"))).await;
    let _ = tokio::fs::remove_dir_all(state.paths.temp_dir.join(format!("download_{id}"))).await;
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
    remove_job_artifacts(&state, id).await;
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
    let mut errors: Vec<String> = Vec::new();
    for download in downloads {
        if !is_removable(Status::from_i32(download.status)) {
            continue;
        }
        // Einzelfehler nicht abbrechen lassen: Rest aufarbeiten, am Ende melden.
        match state
            .database
            .delete_download(DownloadId(download.id))
            .await
        {
            Ok(()) => {
                remove_job_artifacts(&state, download.id).await;
                removed += 1;
            }
            Err(error) => errors.push(format!("Auftrag {}: {error}", download.id)),
        }
    }
    if removed == 0 && !errors.is_empty() {
        return Err(errors.join(" · "));
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::{ImportOutcome, import_single};
    use usenet_dl::{Config, UsenetDownloader};

    const SAMPLE_NZB: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file poster="test@example.invalid" date="0" subject="&quot;sample.zip&quot; yEnc (1/1)">
    <groups><group>alt.binaries.test</group></groups>
    <segments><segment bytes="16" number="1">sample@example.invalid</segment></segments>
  </file>
</nzb>"#;

    #[tokio::test]
    async fn unreadable_file_fails_only_itself() {
        let root = std::env::temp_dir().join(format!(
            "nzb-deck-import-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock must be after epoch")
                .as_nanos()
        ));
        let nzb_dir = root.join("nzbs");
        let good_path = root.join("gute.nzb");
        // Ein Verzeichnis mit .nzb-Endung ist als Datei nicht lesbar.
        let broken_path = root.join("kaputt.nzb");
        tokio::fs::create_dir_all(&nzb_dir).await.unwrap();
        tokio::fs::write(&good_path, SAMPLE_NZB).await.unwrap();
        tokio::fs::create_dir_all(&broken_path).await.unwrap();

        let mut config = Config::default();
        config.download.temp_dir = root.join("temporary");
        config.download.download_dir = root.join("downloads");
        config.persistence.database_path = root.join("downloads.sqlite3");
        config.tools.search_path = false;
        let downloader = UsenetDownloader::new(config)
            .await
            .expect("offline downloader must initialize");

        let good = import_single(
            &downloader,
            &nzb_dir,
            root.join("downloads/gute"),
            &good_path,
            "gute",
        )
        .await
        .expect("a broken sibling must not block a valid NZB");
        assert!(
            nzb_dir.join(format!("{good}.nzb")).exists(),
            "preserved NZB must exist for rerun"
        );

        let broken = import_single(
            &downloader,
            &nzb_dir,
            root.join("downloads/kaputt"),
            &broken_path,
            "kaputt",
        )
        .await
        .expect_err("unreadable NZB must report an error");
        assert!(broken.contains("konnte nicht gelesen werden"));

        let _ = downloader.shutdown().await;
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[test]
    fn error_text_distinguishes_total_and_partial_failure() {
        let total = ImportOutcome {
            imported: Vec::new(),
            errors: vec!["A".to_string(), "B".to_string()],
        };
        assert_eq!(total.error_text(), "A · B");

        let empty = ImportOutcome::default();
        assert_eq!(empty.error_text(), "Es wurden keine NZB-Dateien ausgewählt");

        let partial = ImportOutcome {
            imported: vec![1],
            errors: vec!["B".to_string()],
        };
        assert_eq!(partial.error_text(), "Teilweise fehlgeschlagen: B");
    }
}
