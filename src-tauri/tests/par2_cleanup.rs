//! Regressionstest: Nach erfolgreicher Verarbeitung werden PAR2-Dateien aus dem
//! Ergebnisordner entfernt, echte Inhalte bleiben erhalten.
//!
//! Laeuft vollstaendig offline: Die Nachverarbeitung erhaelt eine temp-typische
//! Downloadablage (eine Inhaltsdatei + PAR2-Dateien) und bewegt sie ueber die
//! echte Pipeline (verify → extract → move → cleanup → PAR2-Entfernung).

use std::path::PathBuf;
use std::sync::Arc;

use usenet_dl::config::{Config, PostProcess};
use usenet_dl::db::NewDownload;
use usenet_dl::post_processing::PostProcessor;
use usenet_dl::{Database, DownloadId, NoOpParityHandler};

async fn unique_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "nzb-deck-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ))
}

#[tokio::test]
async fn successful_processing_removes_par2_but_keeps_content() {
    let root = unique_root("par2-cleanup").await;
    let download_path = root.join("temporary").join("download_1");
    let destination = root.join("usenet").join("Mein Buch");
    tokio::fs::create_dir_all(&download_path)
        .await
        .expect("create download dir");

    tokio::fs::write(download_path.join("mein-buch.epub"), b"echter buchinhalt")
        .await
        .expect("write content fixture");
    tokio::fs::write(download_path.join("mein-buch.par2"), b"par2 index")
        .await
        .expect("write par2 fixture");
    tokio::fs::write(
        download_path.join("mein-buch.vol00+01.par2"),
        b"par2 volume",
    )
    .await
    .expect("write par2 volume fixture");

    let database_path = root.join("downloads.sqlite3");
    let database = Arc::new(Database::new(&database_path).await.expect("open db"));
    let id = database
        .insert_download(&NewDownload {
            name: "Mein Buch".to_string(),
            nzb_path: "memory:test".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: destination.to_string_lossy().into_owned(),
            post_process: PostProcess::UnpackAndCleanup.to_i32(),
            priority: 0,
            status: 0,
            size_bytes: 0,
        })
        .await
        .expect("insert download");

    let mut config = Config::default();
    config.download.temp_dir = root.join("temporary");
    config.download.download_dir = root.join("usenet");
    config.tools.search_path = false;
    let (event_tx, _rx) = tokio::sync::broadcast::channel(64);
    let processor = PostProcessor::new(
        event_tx,
        Arc::new(config),
        Arc::new(NoOpParityHandler),
        Arc::clone(&database),
    );

    let final_path = processor
        .start_post_processing(
            id,
            download_path.clone(),
            PostProcess::UnpackAndCleanup,
            destination.clone(),
        )
        .await
        .expect("post-processing must succeed");

    assert_eq!(final_path, destination, "result lands in the job folder");

    let content = tokio::fs::read(destination.join("mein-buch.epub"))
        .await
        .expect("content file must be kept");
    assert_eq!(content, b"echter buchinhalt");

    let mut entries = tokio::fs::read_dir(&destination).await.expect("read result dir");
    let names: Vec<String> = {
        let mut found = Vec::new();
        while let Some(entry) = entries.next_entry().await.expect("entry") {
            found.push(
                entry
                    .file_name()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        found
    };
    assert!(
        !names.iter().any(|name| name.ends_with(".par2")),
        "PAR2 files must be removed after success, found: {names:?}"
    );
    assert_eq!(names, vec!["mein-buch.epub"]);

    let _ = tokio::fs::remove_dir_all(&root).await;
}

#[tokio::test]
async fn content_only_par2_results_are_left_untouched() {
    let root = unique_root("par2-only").await;
    let download_path = root.join("temporary").join("download_1");
    let destination = root.join("usenet").join("Nur Paritaet");
    tokio::fs::create_dir_all(&download_path)
        .await
        .expect("create download dir");
    tokio::fs::write(download_path.join("nur.par2"), b"par2 index")
        .await
        .expect("write par2 fixture");

    let database_path = root.join("downloads.sqlite3");
    let database = Arc::new(Database::new(&database_path).await.expect("open db"));
    let id = database
        .insert_download(&NewDownload {
            name: "Nur Paritaet".to_string(),
            nzb_path: "memory:test".to_string(),
            nzb_meta_name: None,
            nzb_hash: None,
            job_name: None,
            category: None,
            destination: destination.to_string_lossy().into_owned(),
            post_process: PostProcess::UnpackAndCleanup.to_i32(),
            priority: 0,
            status: 0,
            size_bytes: 0,
        })
        .await
        .expect("insert download");

    let mut config = Config::default();
    config.download.temp_dir = root.join("temporary");
    config.download.download_dir = root.join("usenet");
    config.tools.search_path = false;
    let (event_tx, _rx) = tokio::sync::broadcast::channel(64);
    let processor = PostProcessor::new(
        event_tx,
        Arc::new(config),
        Arc::new(NoOpParityHandler),
        Arc::clone(&database),
    );

    let _ = processor
        .start_post_processing(
            DownloadId(id.0),
            download_path.clone(),
            PostProcess::UnpackAndCleanup,
            destination.clone(),
        )
        .await
        .expect("post-processing must succeed");

    assert!(
        destination.join("nur.par2").exists(),
        "a folder with only PAR2 data must not be emptied"
    );

    let _ = tokio::fs::remove_dir_all(&root).await;
}
