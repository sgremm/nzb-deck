# NZB Deck

Nativer Usenet-NZB-Downloader für **macOS und iOS** im Stil von SABnzbd – gebaut mit
**Tauri 2 + Svelte 5 + Rust**. Keine externen Binaries: PAR2-Reparatur und
Archivextraktion laufen vollständig über Rust-Crates.

Repo: `~/Projekte/nzb-deck`

## Funktionen

- **Eine Fortschrittskarte je NZB** mit Pipeline-Anzeige:
  Download → Prüfen → Reparieren → Entpacken → Fertig
- **NZB-Import** per Dateiauswahl, Finder-„Öffnen mit"/Doppelklick (macOS `RunEvent::Opened`)
  und Kaltstart mit übergebenen Dateien
- **Aufträge persistent** in SQLite; Wiederverwenden („Neu herunterladen") und
  erneutes Verarbeiten vorhandener Downloads möglich
- **Einzeln/alle löschen** abgeschlossener oder fehlgeschlagener Aufträge aus der Liste
  (Dateien im Zielordner bleiben erhalten; „Alle löschen" verlangt einen zweiten
  Bestätigungsklick)
- **Magic-Header-Erkennung** von Archiven ohne Dateiendung (z. B. `.nfo`-Endung auf
  einem RAR); nie bei echten Nicht-Archiven (`.epub`, Büro-Dokumenten, …)
- **PAR2-Verifikation/-Reparatur** (`rust-par2`, `NativeParityHandler`); nach
  erfolgreicher Verarbeitung werden `.par2`-Dateien gelöscht (SABnzbd-Verhalten)
- **Deobfuskation** von Dateinamen aus PAR2-Sätzen vor der Verifikation
- **Fehlerhafte Artikel → Auftrag fehlgeschlagen** (keine stillen „leeren Erfolge")
- Versionsanzeige unten links in der App (aus `CARGO_PKG_VERSION`)

## Tech-Stack

| Schicht | Technologie |
|---|---|
| Shell | Tauri 2 (tao, wry; iOS-Support via `gen/apple`) |
| Backend | Rust (Edition 2024), `tokio`, `sqlx`/SQLite, `usenet-dl` (vendored) |
| Frontend | Svelte 5 (Runes), Vite, `@tauri-apps/api` |
| Downloads | `nntp-rs` (vendored), `rust-par2`, `unrar`, `zip`, `sevenz-rust` |
| UI-Sprache | Deutsch |

## Voraussetzungen

- macOS (arm64 getestet), Xcode Command Line Tools
- Rust (stable, edition 2024) und Node.js/npm
- iOS: Xcode; die Plattform wird über `src-tauri/gen/apple` gebaut (SwiftPM, kein CocoaPods)

## Build & Start

```bash
cd ~/Projekte/nzb-deck
npm install                 # einmalig (unter src-tauri/ auch: cargo build)
npm run check               # Svelte-/TS-Prüfung (0 Fehler)
cd src-tauri && cargo test  # Tests: 4 lib-Tests + 6 par2_cleanup-Pipeline-Tests
cd .. && npm run tauri build -- --debug --bundles app
open "src-tauri/target/debug/bundle/macos/NZB Deck.app"
```

Testläufe mit Logging direkt aus dem Terminal:

```bash
RUST_LOG=usenet_dl=info,nzb_deck=debug \
  "src-tauri/target/debug/bundle/macos/NZB Deck.app" <datei.nzb>
```

Kaltstart-/Finder-Import-Test (LaunchServices löst `RunEvent::Opened` aus, kein argv):

```bash
pkill -f "NZB Deck.app"; open -a "src-tauri/target/debug/bundle/macos/NZB Deck.app" "<datei.nzb>"
```

## Projektstruktur

```
nzb-deck/
├── src/                        # Svelte-Frontend (routes/, lib/components/, lib/jobs.ts)
├── src-tauri/
│   ├── src/
│   │   ├── main.rs             # Tauri-Einstieg
│   │   ├── lib.rs              # Setup, Import-Puffer, invoke_handler, RunEvent
│   │   ├── commands.rs         # Tauri-Commands (siehe unten)
│   │   ├── state.rs            # AppState, Einstellungen, Job-Ansichten, Status
│   │   ├── models.rs           # JobView/BackendStatus/AppSettings (serde)
│   │   └── parity.rs           # PAR2-Handler-Anbindung (NativeParityHandler)
│   ├── vendor/
│   │   ├── usenet-dl/          # gepatchte Kopie von usenet-dl 0.4.0
│   │   ├── nntp-rs/            # gepatcht (Zeilenumbrüche im Article-Body)
│   │   └── tao/                # gepatcht (unwrap-Panic beim Öffnen)
│   ├── tests/par2_cleanup.rs   # Pipeline-Tests (PAR2-Löschung, Magic-Erkennung)
│   ├── Cargo.toml              # enthält [patch.crates-io] tao = { path = "vendor/tao" }
│   └── tauri.conf.json
└── package.json
```

## Tauri-Commands (Backend → Frontend)

| Command | Zweck |
|---|---|
| `get_settings` / `save_settings` | Server- und Speicherort-Konfiguration |
| `backend_status` | Konfiguriert/Verbunden-Flag, Fehlertext, **Version** |
| `list_jobs` | Auftragsliste (`JobView[]`) |
| `import_nzbs(paths)` | NZB-Dateien importieren und einreihen |
| `pause_job` / `resume_job` | Lauf pausieren/fortsetzen |
| `rerun_job` | Download erneut ausführen (nur Complete/Failed; konservierte NZB) |
| `reprocess_job` | Verarbeitung vorhandener Dateien wiederholen |
| `delete_job(id)` | Auftrag einzeln löschen (nur Complete/Failed) |
| `clear_jobs()` | Alle abschließbaren Aufträge löschen |
| `test_server(settings)` | Verbindung/Auth gegen den Newsserver prüfen |

Status-Events: `downloading`, `download_complete`, `verifying`, `verify_complete`,
`repairing`, `repair_complete`/`repair_skipped`, `extracting`, `extract_complete`,
`moving`, `cleaning`, `complete`, `download_failed`, `failed`.

## Daten & Pfade

| Was | Pfad |
|---|---|
| Projekt | `~/Projekte/nzb-deck` |
| Debug-App | `src-tauri/target/debug/bundle/macos/NZB Deck.app` |
| App-Daten | `~/Library/Application Support/de.sgremm.nzbdeck/` |
| Datenbank | `…/downloads.sqlite3` (Tabellen `downloads`, `download_files`, `download_articles`) |
| Temporäre Downloads | `…/temporary/download_<id>/` |
| Konservierte NZBs | `…/nzbs/<id>.nzb` (für „Neu herunterladen") |
| Einstellungen | `…/settings.json` (Server-Zugangsdaten – **nicht committen**) |
| Zielordner | `~/Downloads/usenet/<Jobname>[ (n)]` (Suffix bei Namenskonflikt) |

Einstellungen enthalten Zugangsdaten des Newsservers; die Datei ist über
`.gitignore` vom Repo ausgeschlossen.

## Vendoring & bekannte Patches

Alle Patches liegen in `src-tauri/vendor/`; usenet-dl referenziert nntp-rs als
Path-Dependency, tao ist via `[patch.crates-io]` umgelenkt.

| Crate | Patch | Grund |
|---|---|---|
| `usenet-dl` | Extraktion via `unrar`/`zip`, Magic-Header-Erkennung in `extraction/shared.rs` (`detect_archive_type`, `sanitize_relative_path`/`sanitize_path_component`), PAR2-Originalnamen vor Verify, Ordner je Job | Endungslose Archive entpacken; unanlegbare Zeichen (Unicode-Noncharacters/PUA) ersetzen; fehlende Unterordner anlegen |
| `nntp-rs` | Zeilenumbrüche im Article-Body erhalten | Verbindungs-/Datenverlust am echten Provider |
| `tao` | unwrap-Panic bei `Opened`-Event | Doppelklick-Crash auf macOS |

Provider-Eigenheiten: Der Test-Provider liefert Posts teils nur mit Verzögerung
oder mit `430 No Such Article` (DMCA-artiger Abbau); Artikelabrufe ohne `GROUP`
funktionieren bei ihm, mit `GROUP` teils nicht.

## Bekannte Grenzen

- Live-Tests gegen den Provider sind zeitweise unzuverlässig (siehe oben);
  fürs Debuggen liegen Test-RARs in den Temp-Ordnern vor (Offline-Reproduktion).
- `extraction/sevenz.rs` nutzt noch keinen Pfad-Sanitizer (nur zentraler
  `detect_archive_type`-Dispatch).

## Git

Konvention: ein Commit je abgeschlossener Arbeit. `.gitignore` schließt
`node_modules`, `target`, App-Daten und lokale Tools aus.
