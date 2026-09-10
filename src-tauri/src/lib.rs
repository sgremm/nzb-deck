mod commands;
mod models;
mod parity;
mod state;

use commands::{
    backend_status, clear_jobs, delete_job, get_settings, import_nzbs, is_nzb, list_jobs,
    pause_job, reprocess_job, rerun_job, resume_job, save_settings, test_server,
};
use state::AppState;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{Manager, RunEvent};

/// NZB-Pfade, die über "Oeffnen mit"/Doppelklick eintreffen, bevor der
/// App-Zustand in `setup` verwaltet wird. Werden nach dem Setup importiert.
static PENDING_OPEN_PATHS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();

fn pending_open_paths() -> &'static Mutex<Vec<PathBuf>> {
    PENDING_OPEN_PATHS.get_or_init(|| Mutex::new(Vec::new()))
}

fn remember_open_paths(paths: Vec<PathBuf>) {
    if let Ok(mut pending) = pending_open_paths().lock() {
        pending.extend(paths);
    }
}

fn drain_open_paths() -> Vec<PathBuf> {
    if let Ok(mut pending) = pending_open_paths().lock() {
        std::mem::take(&mut pending)
    } else {
        Vec::new()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .try_init();

    let builder = tauri::Builder::default();

    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
        let paths = paths_from_strings(args.into_iter().skip(1));
        if !paths.is_empty()
            && let Some(state) = app.try_state::<Arc<AppState>>()
        {
            import_in_background(app.clone(), state.inner().clone(), paths);
        }
    }));

    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| Box::<dyn std::error::Error>::from(error.to_string()))?;
            let state = tauri::async_runtime::block_on(AppState::new(app.handle(), data_dir))
                .map_err(Box::<dyn std::error::Error>::from)?;
            app.manage(state.clone());

            // Fruehe "Oeffnen mit"-Ereignisse (vor manage) jetzt importieren.
            let pending = drain_open_paths();
            if !pending.is_empty() {
                import_in_background(app.handle().clone(), state.clone(), pending);
            }

            let startup_paths = paths_from_strings(std::env::args().skip(1));
            if !startup_paths.is_empty() {
                import_in_background(app.handle().clone(), state, startup_paths);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            backend_status,
            list_jobs,
            import_nzbs,
            pause_job,
            resume_job,
            rerun_job,
            reprocess_job,
            delete_job,
            clear_jobs,
            test_server
        ]);

    let app = builder
        .build(tauri::generate_context!())
        .expect("NZB Deck konnte nicht initialisiert werden");

    app.run(|app_handle, event| match event {
        RunEvent::Opened { urls } => {
            let paths = urls
                .into_iter()
                .filter_map(|url| url.to_file_path().ok())
                .filter(|path| is_nzb(path))
                .collect::<Vec<_>>();
            if !paths.is_empty()
                && let Some(state) = app_handle.try_state::<Arc<AppState>>()
            {
                import_in_background(app_handle.clone(), state.inner().clone(), paths);
            } else if !paths.is_empty() {
                remember_open_paths(paths);
            }
        }
        RunEvent::Exit => {
            let state = app_handle.state::<Arc<AppState>>().inner().clone();
            tauri::async_runtime::block_on(async move {
                if let Some(downloader) = state.downloader.write().await.take() {
                    let _ = downloader.shutdown().await;
                }
            });
        }
        _ => {}
    });
}

fn import_in_background(app: tauri::AppHandle, state: Arc<AppState>, paths: Vec<PathBuf>) {
    tauri::async_runtime::spawn(async move {
        let outcome = commands::import_paths(&state, paths).await;
        use tauri::Emitter;
        if !outcome.errors.is_empty() {
            let _ = app.emit("nzb-import-error", outcome.error_text());
        }
        if !outcome.imported.is_empty() {
            let _ = app.emit("nzb-imported", outcome.imported);
        }
    });
}

fn paths_from_strings(values: impl IntoIterator<Item = String>) -> Vec<PathBuf> {
    values
        .into_iter()
        .filter_map(|value| {
            if let Ok(url) = url::Url::parse(&value)
                && url.scheme() == "file"
            {
                return url.to_file_path().ok();
            }
            Some(PathBuf::from(value))
        })
        .filter(|path| is_nzb(path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::paths_from_strings;
    use std::path::PathBuf;

    #[test]
    fn accepts_multiple_plain_nzb_paths() {
        let paths =
            paths_from_strings(["/tmp/first.nzb".to_string(), "/tmp/second.NZB".to_string()]);

        assert_eq!(
            paths,
            vec![
                PathBuf::from("/tmp/first.nzb"),
                PathBuf::from("/tmp/second.NZB")
            ]
        );
    }

    #[test]
    fn decodes_file_urls_and_rejects_other_files() {
        let paths = paths_from_strings([
            "file:///tmp/A%20Release.nzb".to_string(),
            "file:///tmp/readme.txt".to_string(),
        ]);

        assert_eq!(paths, vec![PathBuf::from("/tmp/A Release.nzb")]);
    }

    #[tokio::test]
    async fn failed_extraction_keeps_downloaded_files() {
        use usenet_dl::{Config, Database, DownloadOptions, Status, UsenetDownloader};

        let root = std::env::temp_dir().join(format!(
            "nzb-deck-postprocess-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock must be after epoch")
                .as_nanos()
        ));
        let temp_dir = root.join("temporary");
        let destination = root.join("downloads").join("Broken release");
        let database_path = root.join("downloads.sqlite3");
        let mut config = Config::default();
        config.download.temp_dir = temp_dir.clone();
        config.download.download_dir = root.join("downloads");
        config.persistence.database_path = database_path.clone();
        config.tools.search_path = false;

        let downloader = UsenetDownloader::new(config)
            .await
            .expect("offline downloader must initialize");
        let nzb = br#"<?xml version="1.0" encoding="UTF-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <file poster="test@example.invalid" date="0" subject="&quot;broken.zip&quot; yEnc (1/1)">
    <groups><group>alt.binaries.test</group></groups>
    <segments><segment bytes="16" number="1">broken@example.invalid</segment></segments>
  </file>
</nzb>"#;
        let id = downloader
            .add_nzb_content(
                nzb,
                "Broken release",
                DownloadOptions {
                    destination: Some(destination.clone()),
                    ..DownloadOptions::default()
                },
            )
            .await
            .expect("NZB must import");
        let job_temp = temp_dir.join(format!("download_{}", id.0));
        tokio::fs::create_dir_all(&job_temp)
            .await
            .expect("job directory must be created");
        let broken_archive = job_temp.join("broken.zip");
        tokio::fs::write(&broken_archive, b"not a zip archive")
            .await
            .expect("fixture must be written");

        assert!(downloader.start_post_processing(id).await.is_err());
        assert!(
            broken_archive.exists(),
            "failed processing must preserve the downloaded archive"
        );
        assert!(
            !destination.exists(),
            "an empty extraction result must not be published"
        );

        let database = Database::new(&database_path)
            .await
            .expect("database must reopen");
        let job = database
            .get_download(id)
            .await
            .expect("job query must succeed")
            .expect("job must remain visible");
        assert_eq!(Status::from_i32(job.status), Status::Failed);
        assert!(job.error_message.is_some());

        let _ = downloader.shutdown().await;
        database.close().await;
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
