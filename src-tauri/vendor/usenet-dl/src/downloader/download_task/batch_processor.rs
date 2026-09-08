//! Batch-level article fetching — pipelined NNTP fetch, yEnc decode, and per-article retry.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Cross-platform positional file write.
///
/// Writes `buf` to `file` at the given byte `offset`, equivalent to Unix `pwrite`.
#[cfg(unix)]
fn write_all_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

/// Cross-platform positional file write.
///
/// Writes `buf` to `file` at the given byte `offset`, equivalent to Unix `pwrite`.
#[cfg(windows)]
fn write_all_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut written = 0;
    while written < buf.len() {
        let n = file.seek_write(&buf[written..], offset + written as u64)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "failed to write whole buffer",
            ));
        }
        written += n;
    }
    Ok(())
}

/// Cross-platform positional file write.
///
/// Writes `buf` to `file` at the given byte `offset`, equivalent to Unix `pwrite`.
#[cfg(not(any(unix, windows)))]
fn write_all_at(_file: &std::fs::File, _buf: &[u8], _offset: u64) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "positional writes not supported on this platform",
    ))
}

use crate::types::DownloadId;

use super::context::{ArticleProvider, BatchResultVec, OutputFiles, is_missing_article_error};

/// Result of decoding a single article: Ok(article_id, file_index, segment_number, decoded_bytes) or Err(article_id, error_message).
type DecodeResult = Result<(i64, i32, i32, u64), (i64, String)>;

/// Aggregate batch results into success/failure counts and first error.
pub(super) fn aggregate_results(results: BatchResultVec) -> super::orchestration::DownloadResults {
    let mut success_count = 0;
    let mut failed_count = 0;
    let mut first_error: Option<String> = None;

    for result in results {
        match result {
            Ok(batch_results) => {
                success_count += batch_results.len();
            }
            Err((error_msg, batch_size)) => {
                failed_count += batch_size;
                if first_error.is_none() {
                    first_error = Some(error_msg);
                }
            }
        }
    }

    super::orchestration::DownloadResults {
        success_count,
        failed_count,
        first_error,
        total_articles: 0,      // Set by caller after aggregation
        individually_failed: 0, // Set by caller from atomic counter
    }
}

/// Parameters for fetching a batch of articles
pub(super) struct FetchArticleBatchParams {
    /// Download ID
    pub(super) id: DownloadId,
    /// Articles to fetch in this batch
    pub(super) article_batch: Vec<crate::db::Article>,
    /// Article provider for fetching articles from NNTP servers
    pub(super) article_provider: Arc<dyn ArticleProvider>,
    /// Channel for sending article status updates
    pub(super) batch_tx: tokio::sync::mpsc::Sender<(i64, i32)>,
    /// Speed limiter
    pub(super) speed_limiter: crate::speed_limiter::SpeedLimiter,
    /// Cancellation token
    pub(super) cancel_token: tokio_util::sync::CancellationToken,
    /// Temporary directory for download
    pub(super) download_temp_dir: std::path::PathBuf,
    /// Atomic counter for downloaded bytes
    pub(super) downloaded_bytes: Arc<AtomicU64>,
    /// Atomic counter for downloaded articles
    pub(super) downloaded_articles: Arc<AtomicU64>,
    /// Atomic counter for individually-failed articles (missing/expired)
    pub(super) failed_articles: Arc<AtomicU64>,
    /// Output file handles for DirectWrite
    pub(super) output_files: Arc<OutputFiles>,
    /// Pipeline depth for NNTP commands
    pub(super) pipeline_depth: usize,
    /// Tracker for per-file article completion (DirectUnpack notification)
    pub(super) file_completion_tracker: Arc<super::context::FileCompletionTracker>,
}

/// Fetch a single batch of articles via pipelined NNTP commands.
///
/// On missing-article errors, falls back to per-article retry so that available
/// articles in the batch are still downloaded and only truly missing ones are marked failed.
pub(super) async fn fetch_article_batch(
    params: FetchArticleBatchParams,
) -> std::result::Result<Vec<(i32, u64)>, (String, usize)> {
    let FetchArticleBatchParams {
        id,
        article_batch,
        article_provider,
        batch_tx,
        speed_limiter,
        cancel_token,
        download_temp_dir,
        downloaded_bytes,
        downloaded_articles,
        failed_articles,
        output_files,
        pipeline_depth,
        file_completion_tracker,
    } = params;
    let batch_size = article_batch.len();

    // Check if download was cancelled
    if cancel_token.is_cancelled() {
        return Err(("Download cancelled".to_string(), batch_size));
    }

    // Prepare message IDs for pipelined fetch
    let message_ids: Vec<std::borrow::Cow<'_, str>> = article_batch
        .iter()
        .map(|article| {
            if article.message_id.starts_with('<') {
                std::borrow::Cow::Borrowed(article.message_id.as_str())
            } else {
                std::borrow::Cow::Owned(format!("<{}>", article.message_id))
            }
        })
        .collect();

    let message_id_refs: Vec<&str> = message_ids.iter().map(|s| s.as_ref()).collect();

    // Acquire bandwidth tokens before downloading
    let total_batch_size: u64 = article_batch.iter().map(|a| a.size_bytes as u64).sum();
    speed_limiter.acquire(total_batch_size).await;

    // Fetch articles via the article provider
    let responses = match article_provider
        .fetch_articles(&message_id_refs, pipeline_depth)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            // If the error indicates a missing article, retry each article individually
            // so we can salvage the ones that exist
            if is_missing_article_error(&e) {
                tracing::debug!(
                    download_id = id.0,
                    batch_size = batch_size,
                    error = %e,
                    "Batch failed with missing article, retrying individually"
                );
                return retry_articles_individually(RetryArticlesParams {
                    id,
                    article_batch,
                    article_provider,
                    batch_tx,
                    cancel_token,
                    download_temp_dir,
                    downloaded_bytes,
                    downloaded_articles,
                    failed_articles,
                    output_files,
                    file_completion_tracker,
                })
                .await;
            }

            // Non-article errors (connection, timeout) fail the whole batch
            tracing::error!(download_id = id.0, batch_size = batch_size, error = %e, "Batch fetch failed");
            for article in &article_batch {
                if let Err(e) = batch_tx
                    .send((article.id, crate::db::article_status::FAILED))
                    .await
                {
                    tracing::warn!(download_id = id.0, article_id = article.id, error = %e, "Failed to send status update to batch channel");
                }
            }
            return Err((format!("Batch fetch failed: {}", e), batch_size));
        }
    };

    // Offload yEnc decode + disk I/O to a blocking thread so tokio worker threads
    // remain free to drive concurrent NNTP fetches on other batches.
    let output_files_bg = Arc::clone(&output_files);
    let temp_dir_bg = download_temp_dir.clone();
    let decode_results = tokio::task::spawn_blocking(move || {
        let mut results: Vec<DecodeResult> = Vec::with_capacity(article_batch.len());
        for (article, response) in article_batch.iter().zip(responses.iter()) {
            match decode_and_write(article, &response.data, &output_files_bg, &temp_dir_bg) {
                Ok(decoded_size) => {
                    results.push(Ok((article.id, article.file_index, article.segment_number, decoded_size)));
                }
                Err(e) => {
                    results.push(Err((article.id, e)));
                }
            }
        }
        results
    })
    .await
    .map_err(|e| (format!("Decode task panicked: {}", e), batch_size))?;

    // Process results back on the async runtime (channel sends, atomic counter updates)
    let mut batch_results = Vec::with_capacity(batch_size);
    for result in decode_results {
        match result {
            Ok((article_id, file_index, segment_number, decoded_size)) => {
                if let Err(e) = batch_tx
                    .send((article_id, crate::db::article_status::DOWNLOADED))
                    .await
                {
                    tracing::warn!(download_id = id.0, article_id = article_id, error = %e, "Failed to send status update to batch channel");
                }

                downloaded_articles.fetch_add(1, Ordering::Relaxed);
                downloaded_bytes.fetch_add(decoded_size, Ordering::Relaxed);
                file_completion_tracker.article_completed(file_index);

                batch_results.push((segment_number, decoded_size));
            }
            Err((article_id, e)) => {
                tracing::error!(download_id = id.0, article_id = article_id, error = %e, "Failed to decode/write article");
                return Err((format!("Failed to decode/write article: {}", e), batch_size));
            }
        }
    }

    Ok(batch_results)
}

/// Decode a yEnc-encoded article and write the decoded data to the correct output file.
///
/// If `output_files` has a mapping for the article's `file_index`, uses DirectWrite
/// (positional write via `write_all_at`). Otherwise falls back to writing the raw data
/// as `article_{segment}.dat` (for legacy downloads without file metadata).
///
/// Returns the number of decoded bytes written.
pub(super) fn decode_and_write(
    article: &crate::db::Article,
    data: &[u8],
    output_files: &OutputFiles,
    download_temp_dir: &std::path::Path,
) -> std::result::Result<u64, String> {
    // Try yEnc decode
    match nntp_rs::yenc_decode(data) {
        Ok(decoded) => {
            let decoded_size = decoded.data.len() as u64;

            if let Some((file_handle, _filename, allocated)) = output_files.files.get(&article.file_index) {
                // Calculate byte offset (yEnc begin is 1-based)
                let offset = decoded
                    .part
                    .as_ref()
                    .map(|p| p.begin - 1) // multi-part: write at byte offset
                    .unwrap_or(0); // single-part: write at start

                // Pre-allocate file to full size on first segment write (creates sparse file).
                // AtomicBool avoids repeated fstat+ftruncate syscalls (~10k saved per download).
                if decoded.header.size > 0
                    && !allocated.swap(true, std::sync::atomic::Ordering::Relaxed)
                {
                    file_handle
                        .set_len(decoded.header.size)
                        .map_err(|e| format!("Failed to pre-allocate file: {}", e))?;
                }

                // Write decoded data at correct offset (lock-free via pwrite/seek_write)
                write_all_at(file_handle, &decoded.data, offset)
                    .map_err(|e| format!("Failed to write at offset {}: {}", offset, e))?;
            } else {
                // Fallback: no output file mapping -- write raw decoded data as article file
                let article_file =
                    download_temp_dir.join(format!("article_{}.dat", article.segment_number));
                std::fs::write(&article_file, &decoded.data)
                    .map_err(|e| format!("Failed to write article file: {}", e))?;
            }

            Ok(decoded_size)
        }
        Err(_) => {
            // yEnc decode failed -- write raw data as fallback
            let article_file =
                download_temp_dir.join(format!("article_{}.dat", article.segment_number));
            let raw_size = data.len() as u64;
            std::fs::write(&article_file, data)
                .map_err(|e| format!("Failed to write raw article file: {}", e))?;
            Ok(raw_size)
        }
    }
}

/// Parameters for retrying articles individually after a batch failure.
pub(super) struct RetryArticlesParams {
    pub(super) id: DownloadId,
    pub(super) article_batch: Vec<crate::db::Article>,
    pub(super) article_provider: Arc<dyn ArticleProvider>,
    pub(super) batch_tx: tokio::sync::mpsc::Sender<(i64, i32)>,
    pub(super) cancel_token: tokio_util::sync::CancellationToken,
    pub(super) download_temp_dir: std::path::PathBuf,
    pub(super) downloaded_bytes: Arc<AtomicU64>,
    pub(super) downloaded_articles: Arc<AtomicU64>,
    pub(super) failed_articles: Arc<AtomicU64>,
    pub(super) output_files: Arc<OutputFiles>,
    /// Tracker for per-file article completion (DirectUnpack notification)
    pub(super) file_completion_tracker: Arc<super::context::FileCompletionTracker>,
}

/// Retry each article in a failed batch individually (pipeline_depth=1).
///
/// Articles that succeed are written to disk and marked DOWNLOADED.
/// Articles that fail are marked FAILED and counted in the `failed_articles` atomic.
/// Returns Ok with successful results if any articles succeeded, Err if ALL failed.
pub(super) async fn retry_articles_individually(
    params: RetryArticlesParams,
) -> std::result::Result<Vec<(i32, u64)>, (String, usize)> {
    let RetryArticlesParams {
        id,
        article_batch,
        article_provider,
        batch_tx,
        cancel_token,
        download_temp_dir,
        downloaded_bytes,
        downloaded_articles,
        failed_articles,
        output_files,
        file_completion_tracker,
    } = params;
    let batch_size = article_batch.len();
    let mut successful_results = Vec::new();
    let mut first_error: Option<String> = None;

    for article in &article_batch {
        if cancel_token.is_cancelled() {
            break;
        }

        let msg_id = if article.message_id.starts_with('<') {
            article.message_id.clone()
        } else {
            format!("<{}>", article.message_id)
        };

        match article_provider.fetch_articles(&[&msg_id], 1).await {
            Ok(mut responses) if !responses.is_empty() => {
                let response_data = responses.swap_remove(0).data;
                let article_clone = article.clone();
                let of = Arc::clone(&output_files);
                let td = download_temp_dir.clone();

                let decode_result = tokio::task::spawn_blocking(move || {
                    decode_and_write(&article_clone, &response_data, &of, &td)
                })
                .await
                .unwrap_or_else(|e| Err(format!("Decode task panicked: {}", e)));

                match decode_result {
                    Ok(decoded_size) => {
                        let _ = batch_tx
                            .send((article.id, crate::db::article_status::DOWNLOADED))
                            .await;
                        downloaded_articles.fetch_add(1, Ordering::Relaxed);
                        downloaded_bytes.fetch_add(decoded_size, Ordering::Relaxed);
                        file_completion_tracker.article_completed(article.file_index);
                        successful_results.push((article.segment_number, decoded_size));
                    }
                    Err(e) => {
                        tracing::debug!(
                            download_id = id.0,
                            article_id = article.id,
                            error = %e,
                            "Failed to decode/write article during individual retry"
                        );
                        failed_articles.fetch_add(1, Ordering::Relaxed);
                        if first_error.is_none() {
                            first_error = Some(format!("Failed to decode/write article: {}", e));
                        }
                        let _ = batch_tx
                            .send((article.id, crate::db::article_status::FAILED))
                            .await;
                        continue;
                    }
                }
            }
            Ok(_) => {
                // Empty response = article missing
                tracing::debug!(
                    download_id = id.0,
                    article_id = article.id,
                    message_id = %article.message_id,
                    "Article missing (empty response)"
                );
                failed_articles.fetch_add(1, Ordering::Relaxed);
                if first_error.is_none() {
                    first_error = Some(format!("No such article: {}", article.message_id));
                }
                let _ = batch_tx
                    .send((article.id, crate::db::article_status::FAILED))
                    .await;
            }
            Err(e) => {
                tracing::debug!(
                    download_id = id.0,
                    article_id = article.id,
                    message_id = %article.message_id,
                    error = %e,
                    "Article fetch failed during individual retry"
                );
                failed_articles.fetch_add(1, Ordering::Relaxed);
                if first_error.is_none() {
                    first_error = Some(format!("No such article: {}", article.message_id));
                }
                let _ = batch_tx
                    .send((article.id, crate::db::article_status::FAILED))
                    .await;
            }
        }
    }

    if successful_results.is_empty() {
        Err((
            first_error.unwrap_or_else(|| "All articles in batch failed".to_string()),
            batch_size,
        ))
    } else {
        // Partial success: return the articles we got, failures are already tracked
        // via the failed_articles atomic and batch_tx
        Ok(successful_results)
    }
}
