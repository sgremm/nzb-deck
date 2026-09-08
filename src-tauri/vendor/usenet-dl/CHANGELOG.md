# Changelog

All notable changes to the usenet-dl project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2026-04-16

### Added
- Persisted per-file paused state in `download_files` so downstream integrations can implement real file-level pause/resume.
- Database/API helpers for reading and updating file paused state.

### Changed
- Pending article selection now excludes paused files.
- Download completion/finalization logic now keeps partially paused downloads paused instead of falsely completing them.
- Queue restore/resume paths preserve the paused-file invariant.

### Fixed
- Downstream integrations such as `nzbget-rs` can now expose real `listfiles[].Paused` semantics instead of UI-only overlays.

## [0.2.0] - 2026-02-11

### Added

#### DirectUnpack — Extract Archives During Download
- **DirectUnpack coordinator**: Background task that polls for completed files and extracts RAR archives while the download is still in progress. When all articles download without failures and extraction succeeds, the post-processing pipeline skips verify/repair/extract and runs only move + cleanup, significantly reducing total completion time.
- **DirectRename**: Uses PAR2 file metadata to fix obfuscated filenames mid-download. PAR2 files are prioritized for early download so their metadata is available to rename files as they complete.
- **New configuration**: `processing.direct_unpack.enabled` (default: `false`), `processing.direct_unpack.direct_rename` (default: `false`), `processing.direct_unpack.poll_interval_ms` (default: `200`)
- **New events**: `DirectUnpackStarted`, `FileCompleted`, `DirectUnpackExtracting`, `DirectUnpackExtracted`, `DirectUnpackCancelled`, `DirectUnpackComplete`, `DirectRenamed`
- **PAR2 metadata parser**: Pure Rust binary parser for PAR2 File Description packets, used by DirectRename to map 16KB MD5 hashes to real filenames
- **File completion tracker**: Per-file atomic counters with `mpsc` channel notifications so the DirectUnpack coordinator reacts instantly to file completions instead of waiting for the next poll cycle
- **DirectUnpack extracted count tracking**: New `direct_unpack_extracted_count` column persists how many files were actually extracted, preventing vacuous completions (0 extractions) from incorrectly skipping the full post-processing pipeline
- **Database migration v5**: Adds `direct_unpack_state` column to `downloads`, `completed` and `original_filename` columns to `download_files`
- **Database migration v6**: Adds `direct_unpack_extracted_count` column to `downloads`

### Changed
- **yEnc decode + disk I/O offloaded to blocking threads**: `decode_and_write` now runs in `tokio::task::spawn_blocking`, keeping tokio worker threads free for concurrent NNTP fetches across batches
- **Atomic file pre-allocation**: File size pre-allocation uses an `AtomicBool` flag instead of repeated `fstat` + `ftruncate` syscalls on every segment write (~10k syscalls saved per download)
- **Faster article batch updates**: Batch updater interval reduced from 1s to 500ms for more responsive article status persistence
- **Conditional repair stage**: `run_verify_stage` now returns whether damage was found; the repair stage is only invoked when verification reports actual damage, skipping unnecessary PAR2 repair for healthy downloads
- **DirectUnpack coordinator is now event-driven**: Uses `tokio::select!` on both a file completion channel and the poll timer, reacting immediately to completed files instead of only on timer ticks
- **Removed unnecessary `Arc` wrapping in post-processing**: Function signatures for verify, repair, and cleanup stages now accept `&dyn ParityHandler`, `&Config`, and `&Database` directly instead of `&Arc<T>`, reducing indirection where the `Arc` was not being cloned

### Fixed
- DirectUnpack with 0 actual extractions (e.g. no RAR archives present) no longer skips the full verify/repair/extract pipeline — the post-process skip now requires `direct_unpack_extracted_count > 0`

## [0.1.1] - 2026-02-11

### Fixed
- PAR2 verification incorrectly failing when par2 exits non-zero but reports no file damage. The parser now determines completeness from parsed output (damaged blocks, damaged/missing files) rather than the exit code, fixing false failures like "files are damaged (0 blocks) but cannot be repaired (need 0 more recovery blocks)".
- CI build cache causing stale artifacts: cache keys now use `Cargo.toml` hash instead of `Cargo.lock` (which is gitignored).
- `test_health_endpoint` breaking on version bumps due to hardcoded version string.

## [Unreleased]

### Added

#### Core Download Features
- Core types: `Config`, `DownloadId`, `DownloadInfo`, `HistoryEntry`, `Status`, `Priority`, and `Stage` enums
- SQLite persistence with automatic migrations for downloads, articles, history, RSS feeds, and scheduler
- Article-level download tracking for granular resume support
- Event system using `tokio::broadcast` channels with 20+ event types
- NNTP integration via nntp-rs library for protocol operations
- Article download with real-time progress tracking, speed calculation, and ETA estimation
- Priority-based queue ordering (Force > High > Normal > Low)
- Download pause/resume support with queue-wide controls
- Automatic resume on startup with crash recovery detection
- Token bucket algorithm for global speed limiting across downloads
- Runtime speed limit changes with burst capacity support
- Exponential backoff retry logic with jitter
- Graceful shutdown with SIGTERM/SIGINT signal handling
- Parallel article downloads with automatic concurrency scaling

#### Post-Processing Pipeline
- Five-stage post-processing pipeline: Verify → Repair → Extract → Move → Cleanup
- Multi-part RAR archive extraction with password support
- 7-Zip, ZIP, TAR/GZ/BZ2 archive extraction
- Recursive archive extraction with configurable depth
- Password caching for successful extractions
- Multi-source password collection (cached, per-download, NZB metadata, global file)
- Obfuscated filename detection and cleanup using entropy analysis
- Multi-source name resolution (job name, NZB metadata, archive comments, largest file)
- File collision handling (Rename, Overwrite, Skip)
- Category-based destination routing
- Intermediate file cleanup (.par2, .nzb, .sfv, .srr)
- Sample folder detection and removal

#### REST API
- Axum-based HTTP server with CORS support
- Optional API key authentication
- OpenAPI 3.1 specification with Swagger UI at `/swagger-ui`
- Real-time event streaming via Server-Sent Events at `/events`
- Queue management endpoints (list, add, pause, resume, cancel, priority changes)
- NZB upload via multipart/form-data and URL fetching
- Configuration management with runtime updates
- Category management (create, update, delete)
- Download history endpoints
- Post-processing re-run endpoints (reprocess, reextract)
- Rate limiting with configurable requests per second
- Health check endpoint

#### Automation Features
- Automatic NZB import from watched directories
- RSS/Atom feed monitoring with automatic download
- Regex-based feed filtering (include/exclude patterns)
- Size and age-based filtering for RSS items
- Time-based scheduler with day-of-week rules
- Speed limit scheduling
- Pause/resume scheduling
- Duplicate detection via NZB hash, filename, or job name
- Configurable duplicate actions (Block, Warn, Allow)

#### Notifications
- HTTP webhook support with event-based triggers
- External script execution with standard environment variables
- Category-specific notification handlers
- JSON payload delivery with authentication headers

#### Utilities
- Pre-download disk space validation with extraction overhead calculation
- NNTP server health checks (connectivity, TLS, authentication, capabilities)
- Latency measurement for server selection

### Dependencies
- tokio 1.0 - Async runtime
- sqlx 0.7 - Database operations
- axum 0.7 - HTTP server
- utoipa 4.0 - OpenAPI documentation
- serde 1.0 - Serialization
- tracing 0.1 - Logging
- thiserror 1.0 - Error handling
- reqwest 0.11 - HTTP client
- notify 6.0 - File system watching
- rss 2.0 - RSS feed parsing
- futures 0.3 - Async stream utilities

### Architecture
- Event-driven design with tokio::broadcast channels
- SQLite persistence for queue state and article tracking
- Article-level granularity for resume support
- Token bucket algorithm for global speed limiting
- Exponential backoff for retry logic
- Five-stage post-processing pipeline
- OpenAPI 3.1 compliant REST API
- Real-time updates via Server-Sent Events

### Known Limitations
- Archive extraction requires external tools (unrar, 7z)
- Password-protected archives use trial-and-error approach
- Sample folder detection is heuristic-based

## [0.1.0] - Initial Development

### Added
- Initial project structure and core architecture
- Database schema design
- REST API endpoint specification
- Configuration system design

---

## Release Notes Format

### Version Numbering
- **Major version** (1.x.x): Breaking API changes
- **Minor version** (x.1.x): New features, backwards compatible
- **Patch version** (x.x.1): Bug fixes, backwards compatible

### Release Categories
- **Added**: New features
- **Changed**: Changes in existing functionality
- **Deprecated**: Soon-to-be removed features
- **Removed**: Removed features
- **Fixed**: Bug fixes
- **Security**: Security fixes

---

*This changelog is automatically maintained and follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) conventions.*
