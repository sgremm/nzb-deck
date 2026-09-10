# Vendor-Patches

Alle Crate-Kopien unter `src-tauri/vendor/` sind modifizierte Versionen der
crates.io-Releases. Diese Datei dokumentiert jede Abweichung, damit Updates
nicht stillschweigend die Fixes verlieren.

Wege, um die Patches gegen eine frische Upstream-Version zu prüfen:

```bash
# Beispiel usenet-dl (entsprechend für nntp-rs, tao):
curl -L https://static.crates.io/crates/usenet-dl/usenet-dl-0.4.0.crate | tar xz
diff -ru usenet-dl-0.4.0/src vendor/usenet-dl/src | less
```

Nach jedem Vendor-Update: `cargo test` in `src-tauri/` (inkl.
`tests/par2_cleanup.rs`) — die Pipeline-Tests decken die usenet-dl-Patches ab.

## usenet-dl (Basis: 0.4.0, Path-Dependency in `../Cargo.toml`)

| Stelle | Änderung | Commit |
|---|---|---|
| `src/downloader/download_task/batch_processor.rs` | NNTP-Header vor `=ybegin` abschneiden; leere Artikelantwort und fehlgeschlagenes yEnc-Depoding als Auftragsfehler statt stiller Datei-Müll; Diagnose-Log beim yEnc-Fehler | `0319bee`, `5ce6e38`, `81f91d0` |
| `src/extraction/shared.rs` | Magic-Header-Erkennung: `detect_archive_type` (~Z. 150), `detect_archive_type_by_magic` (~Z. 210); Pfad-Sanitizer `sanitize_path_component` (~Z. 174) und `sanitize_relative_path` (~Z. 192) ersetzen unanlegbare Zeichen (Unicode-Noncharacters/PUA) | `2b75c92` |
| `src/extraction/rar.rs`, `src/extraction/zip.rs` | Entpacken endungsloser Archive über zentralen Magic-Dispatch | `2b75c92` |
| `src/post_processing/mod.rs` | Entpackfehler beenden den Auftrag (kein stiller Abschluss); Zielordner je Job; fehlende Unterordner anlegen | `b70da75`, `2b75c92` |
| `src/post_processing/verify.rs` | PAR2-Verifikation mit deobfuskierten Originalnamen vor der Prüfung; `is_par2_magic` akzeptiert beide Layouts (RFC-Kopf `PAR2\0PKT` ab Offset 0; QuickPar-Variante mit 8 Nullbytes davor) | `b70da75`, `81f91d0` |

**Nicht entfernt, aber von nzb-deck ungenutzt** (bewusst gelassen, um
Vendor-Updates nicht mit Merge-Konflikten zu erschweren):
`src/rss_manager/`, `src/rss_scheduler.rs`, `src/folder_watcher.rs`,
`src/speed_limiter.rs`.
Ebenso undocumented: `src/extraction/sevenz.rs` nutzt die Pfad-Sanitizer noch
nicht (bekannte Grenze, siehe README).

## nntp-rs (Basis: 0.3.0, Path-Dependency von usenet-dl)

| Stelle | Änderung | Commit |
|---|---|---|
| `src/client/io.rs` (~Z. 300–320, `read_multiline_response_binary_with_timeout`) | Zeilenumbrüche im Artikel-Body bleiben Payload: pro gelesener Zeile wird `\n` angehängt statt die Zeilen ohne Trenner aneinanderzuhängen — yEnc ist zeilenbasiert, Upstream verlor sonst Daten am echten Provider (Kommentar „line breaks are payload“ sichert die Stelle) | `81f91d0` |

## tao (Basis: 0.35.3, via `[patch.crates-io]` in `../Cargo.toml`)

| Stelle | Änderung | Commit |
|---|---|---|
| `src/platform_impl/macos/app_delegate.rs` (~Z. 143, `application:openFile:`/URL-Handling) | `absoluteString()` ist optional — ein nil-Wert darf nicht per unwrap panicieren (abgesichert mit Kommentar „Vendored fix“); behebt den Absturz bei Doppelklick/LaunchServices-Öffnung | `369a151` |
