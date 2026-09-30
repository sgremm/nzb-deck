use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use usenet_dl::{Event, Stage};

/// Merkt sich, in welcher Stufe ein Auftrag fehlgeschlagen ist.
///
/// usenet-dl speichert die Fehlerstufe nicht in der Datenbank und meldet bei
/// `Event::Failed` pauschal `Stage::Extract`. Deshalb wird hier die zuletzt
/// gesehene Stufe pro Auftrag verfolgt und beim Fehlschlag in einer kleinen
/// JSON-Datei gesichert, damit sie Reloads und Neustarts übersteht.
pub struct FailureLog {
    path: PathBuf,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    current: HashMap<i64, &'static str>,
    failed: HashMap<i64, String>,
}

impl FailureLog {
    pub async fn load(path: PathBuf) -> Self {
        let failed = tokio::fs::read(&path)
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            inner: Mutex::new(Inner {
                current: HashMap::new(),
                failed,
            }),
        }
    }

    /// Stufe, in der ein fehlgeschlagener Auftrag abgebrochen ist.
    pub fn stage(&self, id: i64) -> Option<String> {
        self.inner.lock().ok()?.failed.get(&id).cloned()
    }

    /// Verarbeitet ein Event; sichert die Datei, wenn sich Fehlerstufen ändern.
    pub async fn observe(&self, event: &Event) {
        if self.apply(event) {
            self.persist().await;
        }
    }

    pub async fn forget(&self, id: i64) {
        let changed = self.inner.lock().is_ok_and(|mut inner| {
            inner.current.remove(&id);
            inner.failed.remove(&id).is_some()
        });
        if changed {
            self.persist().await;
        }
    }

    fn apply(&self, event: &Event) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        match event {
            Event::Queued { id, .. } | Event::Downloading { id, .. } => {
                inner.current.insert(id.0, "download");
                inner.failed.remove(&id.0).is_some()
            }
            Event::Verifying { id } | Event::VerifyComplete { id, .. } => {
                inner.current.insert(id.0, "verify");
                inner.failed.remove(&id.0).is_some()
            }
            Event::Repairing { id, .. }
            | Event::RepairComplete { id, .. }
            | Event::RepairSkipped { id, .. } => {
                inner.current.insert(id.0, "repair");
                inner.failed.remove(&id.0).is_some()
            }
            Event::Extracting { id, .. }
            | Event::ExtractComplete { id }
            | Event::Moving { id, .. }
            | Event::Cleaning { id } => {
                inner.current.insert(id.0, "extract");
                inner.failed.remove(&id.0).is_some()
            }
            Event::DownloadFailed { id, .. } => {
                inner.current.remove(&id.0);
                inner.failed.insert(id.0, "download".to_string());
                true
            }
            Event::Failed { id, stage, .. } => {
                let seen = inner
                    .current
                    .remove(&id.0)
                    .unwrap_or_else(|| stage_key(*stage));
                inner.failed.insert(id.0, seen.to_string());
                true
            }
            Event::Complete { id, .. } | Event::Removed { id } => {
                inner.current.remove(&id.0);
                inner.failed.remove(&id.0).is_some()
            }
            _ => false,
        }
    }

    async fn persist(&self) {
        let bytes = match self.inner.lock() {
            Ok(inner) => serde_json::to_vec(&inner.failed),
            Err(_) => return,
        };
        if let Ok(bytes) = bytes {
            let _ = tokio::fs::write(&self.path, bytes).await;
        }
    }
}

/// Bildet die usenet-dl-Stufe auf die Pipeline-Keys der UI ab.
fn stage_key(stage: Stage) -> &'static str {
    match stage {
        Stage::Download => "download",
        Stage::Verify => "verify",
        Stage::Repair => "repair",
        Stage::Extract | Stage::Move | Stage::Cleanup | Stage::DirectUnpack => "extract",
    }
}

#[cfg(test)]
mod tests {
    use super::FailureLog;
    use usenet_dl::{DownloadId, Event, Stage};

    fn temp_file(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "nzb-deck-failures-{label}-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock must be after epoch")
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn remembers_last_seen_stage_across_reload() {
        let path = temp_file("reload");
        let log = FailureLog::load(path.clone()).await;
        let id = DownloadId(7);
        log.observe(&Event::Repairing {
            id,
            blocks_needed: 1,
            blocks_available: 1,
        })
        .await;
        // usenet-dl meldet pauschal Extract; die gesehene Stufe gewinnt.
        log.observe(&Event::Failed {
            id,
            stage: Stage::Extract,
            error: "kaputt".to_string(),
            files_kept: true,
        })
        .await;
        assert_eq!(log.stage(7).as_deref(), Some("repair"));

        let reloaded = FailureLog::load(path.clone()).await;
        assert_eq!(reloaded.stage(7).as_deref(), Some("repair"));

        reloaded.forget(7).await;
        assert_eq!(FailureLog::load(path.clone()).await.stage(7), None);
        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn download_failure_and_restart_clear_correctly() {
        let path = temp_file("download");
        let log = FailureLog::load(path.clone()).await;
        let id = DownloadId(3);
        log.observe(&Event::DownloadFailed {
            id,
            error: "Artikel fehlen".to_string(),
            articles_succeeded: None,
            articles_failed: None,
            articles_total: None,
        })
        .await;
        assert_eq!(log.stage(3).as_deref(), Some("download"));

        log.observe(&Event::Verifying { id }).await;
        assert_eq!(log.stage(3), None);
        let _ = tokio::fs::remove_file(path).await;
    }
}
