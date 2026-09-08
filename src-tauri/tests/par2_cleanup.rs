//! Regressionstests zur PAR2-Bereinigung nach der Nachverarbeitung.
//!
//! Regel: PAR2-Dateien werden nur entfernt, wenn im Auftrag tatsaechlich ein
//! Archiv entpackt wurde. Bei Direkt-Downloads (ebook/pdf/...) bleiben die
//! Wiederherstellungsdateien neben dem Inhalt erhalten. Laeuft vollstaendig
//! offline gegen die echte Post-Processing-Pipeline.

use std::io::Write;
use std::path::{Path, PathBuf};
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

fn write_zip_archive(path: &Path, entry_name: &str, content: &[u8]) {
    let file = std::fs::File::create(path).expect("create archive");
    let mut archive = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive.start_file(entry_name, options).expect("start entry");
    archive.write_all(content).expect("write entry");
    archive.finish().expect("finish archive");
}

async fn insert_job(database: &Database, name: &str, destination: &Path) -> DownloadId {
    database
        .insert_download(&NewDownload {
            name: name.to_string(),
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
        .expect("insert download")
}

fn processor(database: Arc<Database>, root: &Path) -> PostProcessor {
    let mut config = Config::default();
    config.download.temp_dir = root.join("temporary");
    config.download.download_dir = root.join("usenet");
    config.tools.search_path = false;
    let (event_tx, _rx) = tokio::sync::broadcast::channel(64);
    PostProcessor::new(
        event_tx,
        Arc::new(config),
        Arc::new(NoOpParityHandler),
        database,
    )
}

async fn run_pipeline(root: &Path, name: &str, setup: impl FnOnce(&Path)) -> PathBuf {
    let download_path = root.join("temporary").join("download_1");
    let destination = root.join("usenet").join(name);
    tokio::fs::create_dir_all(&download_path)
        .await
        .expect("create download dir");
    setup(&download_path);

    let database_path = root.join("downloads.sqlite3");
    let database = Arc::new(Database::new(&database_path).await.expect("open db"));
    let id = insert_job(&database, name, &destination).await;
    let final_path = processor(database, root)
        .start_post_processing(
            id,
            download_path,
            PostProcess::UnpackAndCleanup,
            destination.clone(),
        )
        .await
        .expect("post-processing must succeed");
    assert_eq!(final_path, destination);
    destination
}

async fn list_files(folder: &Path) -> Vec<String> {
    let mut entries = tokio::fs::read_dir(folder).await.expect("read dir");
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await.expect("entry") {
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    names
}

#[tokio::test]
async fn unpacked_archive_removes_par2_but_keeps_extracted_content() {
    let root = unique_root("par2-nach-entpacken").await;
    let destination = run_pipeline(&root, "Gepacktes Buch", |download_path| {
        write_zip_archive(
            &download_path.join("gepacktes-buch.zip"),
            "leseprobe.txt",
            b"inhalt aus dem archiv",
        );
        std::fs::write(download_path.join("gepacktes-buch.par2"), b"par2 index")
            .expect("write par2");
        std::fs::write(
            download_path.join("gepacktes-buch.vol00+01.par2"),
            b"par2 volume",
        )
        .expect("write par2 volume");
    })
    .await;

    let names = list_files(&destination).await;
    assert_eq!(
        names,
        vec!["leseprobe.txt"],
        "nach erfolgreichem Entpacken bleiben nur die extrahierten Dateien"
    );
    let content = tokio::fs::read(destination.join("leseprobe.txt"))
        .await
        .expect("read extracted file");
    assert_eq!(content, b"inhalt aus dem archiv");

    let _ = tokio::fs::remove_dir_all(&root).await;
}

#[tokio::test]
async fn direct_download_keeps_par2_next_to_content() {
    let root = unique_root("par2-direkt").await;
    let destination = run_pipeline(&root, "Direktes E-Book", |download_path| {
        std::fs::write(download_path.join("buch.azw3"), b"ebook-inhalt")
            .expect("write content");
        std::fs::write(download_path.join("buch.par2"), b"par2 index").expect("write par2");
        std::fs::write(download_path.join("buch.vol00+01.par2"), b"par2 volume")
            .expect("write par2 volume");
    })
    .await;

    let names = list_files(&destination).await;
    assert_eq!(
        names,
        vec!["buch.azw3", "buch.par2", "buch.vol00+01.par2"],
        "ohne Archiv (nichts ausgepackt) bleiben die PAR2-Dateien erhalten"
    );

    let _ = tokio::fs::remove_dir_all(&root).await;
}

#[tokio::test]
async fn content_only_par2_results_are_left_untouched() {
    let root = unique_root("par2-only").await;
    let destination = run_pipeline(&root, "Nur Paritaet", |download_path| {
        std::fs::write(download_path.join("nur.par2"), b"par2 index").expect("write par2");
    })
    .await;

    assert!(
        destination.join("nur.par2").exists(),
        "ein Ordner mit ausschliesslich PAR2-Daten darf nicht geleert werden"
    );

    let _ = tokio::fs::remove_dir_all(&root).await;
}
