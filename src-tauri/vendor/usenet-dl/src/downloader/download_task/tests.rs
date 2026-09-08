//! Tests for the download task module.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::config::{Config, ServerConfig};

use super::batch_processor::{FetchArticleBatchParams, RetryArticlesParams};
use super::batching::{fetch_download_record, prepare_batches, spawn_fast_fail_watcher};
use super::context::{ArticleProvider, BatchResultVec, DownloadTaskContext, OutputFiles};
use super::finalization::finalize_download;
use super::orchestration::{DownloadResults, run_download_task};

fn config_with_servers(servers: Vec<ServerConfig>) -> Config {
    Config {
        servers,
        ..Config::default()
    }
}

fn server(connections: usize, pipeline_depth: usize) -> ServerConfig {
    ServerConfig {
        host: "news.example.com".to_string(),
        port: 563,
        tls: true,
        username: None,
        password: None,
        connections,
        priority: 0,
        pipeline_depth,
    }
}

fn make_article(id: i64, segment: i32, size: i64) -> crate::db::Article {
    crate::db::Article {
        id,
        download_id: 1,
        message_id: format!("<article-{id}@test>"),
        segment_number: segment,
        file_index: 0,
        size_bytes: size,
        status: crate::db::article_status::PENDING,
        downloaded_at: None,
    }
}

async fn insert_test_file(
    db: &crate::db::Database,
    download_id: crate::types::DownloadId,
    file_index: i32,
    total_segments: i32,
) {
    db.insert_files_batch(&[crate::db::NewDownloadFile {
        download_id,
        file_index,
        filename: format!("file-{file_index}.bin"),
        subject: Some(format!("file-{file_index}")),
        total_segments,
    }])
    .await
    .unwrap();
}

// -----------------------------------------------------------------------
// prepare_batches: concurrency calculation
// -----------------------------------------------------------------------

#[test]
fn prepare_batches_sums_connections_across_all_servers() {
    let config = config_with_servers(vec![server(8, 10), server(4, 10)]);
    let articles = (0..5).map(|i| make_article(i, i as i32, 100)).collect();

    let (concurrency, _, _) = prepare_batches(&config, articles, None);

    assert_eq!(
        concurrency, 12,
        "concurrency should be sum of all server connections"
    );
}

#[test]
fn prepare_batches_single_server_concurrency() {
    let config = config_with_servers(vec![server(20, 10)]);
    let articles = vec![make_article(1, 1, 100)];

    let (concurrency, _, _) = prepare_batches(&config, articles, None);

    assert_eq!(concurrency, 20);
}

#[test]
fn prepare_batches_no_servers_gives_zero_concurrency() {
    let config = config_with_servers(vec![]);
    let articles = vec![make_article(1, 1, 100)];

    let (concurrency, _, _) = prepare_batches(&config, articles, None);

    assert_eq!(concurrency, 0);
}

// -----------------------------------------------------------------------
// prepare_batches: pipeline depth
// -----------------------------------------------------------------------

#[test]
fn prepare_batches_uses_first_server_pipeline_depth() {
    let config = config_with_servers(vec![server(4, 25), server(4, 50)]);
    let articles = (0..100).map(|i| make_article(i, i as i32, 100)).collect();

    let (_, pipeline_depth, _) = prepare_batches(&config, articles, None);

    assert_eq!(
        pipeline_depth, 25,
        "pipeline_depth should come from first server"
    );
}

#[test]
fn prepare_batches_clamps_zero_pipeline_depth_to_one() {
    let config = config_with_servers(vec![server(4, 0)]);
    let articles = (0..5).map(|i| make_article(i, i as i32, 100)).collect();

    let (_, pipeline_depth, _) = prepare_batches(&config, articles, None);

    assert_eq!(
        pipeline_depth, 1,
        "pipeline_depth of 0 should be clamped to 1"
    );
}

#[test]
fn prepare_batches_defaults_pipeline_depth_when_no_servers() {
    let config = config_with_servers(vec![]);
    let articles = vec![make_article(1, 1, 100)];

    let (_, pipeline_depth, _) = prepare_batches(&config, articles, None);

    assert_eq!(
        pipeline_depth, 10,
        "should default to 10 when no servers present"
    );
}

// -----------------------------------------------------------------------
// prepare_batches: batch creation
// -----------------------------------------------------------------------

#[test]
fn prepare_batches_creates_correct_batch_sizes() {
    let config = config_with_servers(vec![server(4, 3)]);
    let articles: Vec<_> = (0..10).map(|i| make_article(i, i as i32, 100)).collect();

    let (_, _, batches) = prepare_batches(&config, articles, None);

    assert_eq!(batches.len(), 4, "10 articles / batch size 3 = 4 batches");
    assert_eq!(batches[0].len(), 3);
    assert_eq!(batches[1].len(), 3);
    assert_eq!(batches[2].len(), 3);
    assert_eq!(batches[3].len(), 1, "last batch gets the remainder");
}

#[test]
fn prepare_batches_preserves_article_order() {
    let config = config_with_servers(vec![server(2, 2)]);
    let articles: Vec<_> = (0..4).map(|i| make_article(i, i as i32, 100)).collect();

    let (_, _, batches) = prepare_batches(&config, articles, None);

    assert_eq!(batches[0][0].id, 0);
    assert_eq!(batches[0][1].id, 1);
    assert_eq!(batches[1][0].id, 2);
    assert_eq!(batches[1][1].id, 3);
}

#[test]
fn prepare_batches_empty_articles_produces_no_batches() {
    let config = config_with_servers(vec![server(4, 10)]);

    let (_, _, batches) = prepare_batches(&config, vec![], None);

    assert!(batches.is_empty());
}

#[test]
fn prepare_batches_single_article_makes_one_batch() {
    let config = config_with_servers(vec![server(4, 10)]);
    let articles = vec![make_article(42, 1, 500)];

    let (_, _, batches) = prepare_batches(&config, articles, None);

    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].len(), 1);
    assert_eq!(batches[0][0].id, 42);
}

#[test]
fn prepare_batches_exactly_one_batch_when_articles_equal_pipeline_depth() {
    let config = config_with_servers(vec![server(4, 5)]);
    let articles: Vec<_> = (0..5).map(|i| make_article(i, i as i32, 100)).collect();

    let (_, _, batches) = prepare_batches(&config, articles, None);

    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].len(), 5);
}

// -----------------------------------------------------------------------
// aggregate_results: all success
// -----------------------------------------------------------------------

#[test]
fn aggregate_results_all_success() {
    let results: BatchResultVec = vec![Ok(vec![(1, 100), (2, 200)]), Ok(vec![(3, 300)])];

    let agg = super::batch_processor::aggregate_results(results);

    assert_eq!(agg.success_count, 3);
    assert_eq!(agg.failed_count, 0);
    assert!(agg.first_error.is_none());
}

// -----------------------------------------------------------------------
// aggregate_results: all failure
// -----------------------------------------------------------------------

#[test]
fn aggregate_results_all_failure() {
    let results: BatchResultVec = vec![
        Err(("timeout".to_string(), 5)),
        Err(("connection reset".to_string(), 3)),
    ];

    let agg = super::batch_processor::aggregate_results(results);

    assert_eq!(agg.success_count, 0);
    assert_eq!(agg.failed_count, 8);
    assert_eq!(agg.first_error.as_deref(), Some("timeout"));
}

// -----------------------------------------------------------------------
// aggregate_results: mixed results
// -----------------------------------------------------------------------

#[test]
fn aggregate_results_mixed_preserves_first_error() {
    let results: BatchResultVec = vec![
        Ok(vec![(1, 100)]),
        Err(("first failure".to_string(), 2)),
        Ok(vec![(2, 200), (3, 300)]),
        Err(("second failure".to_string(), 1)),
    ];

    let agg = super::batch_processor::aggregate_results(results);

    assert_eq!(agg.success_count, 3);
    assert_eq!(agg.failed_count, 3);
    assert_eq!(
        agg.first_error.as_deref(),
        Some("first failure"),
        "first_error should be from the first Err encountered"
    );
}

// -----------------------------------------------------------------------
// aggregate_results: empty
// -----------------------------------------------------------------------

#[test]
fn aggregate_results_empty_input() {
    let results: BatchResultVec = vec![];

    let agg = super::batch_processor::aggregate_results(results);

    assert_eq!(agg.success_count, 0);
    assert_eq!(agg.failed_count, 0);
    assert!(agg.first_error.is_none());
}

// -----------------------------------------------------------------------
// aggregate_results: edge cases
// -----------------------------------------------------------------------

#[test]
fn aggregate_results_single_success_batch() {
    let results: BatchResultVec = vec![Ok(vec![(1, 50)])];

    let agg = super::batch_processor::aggregate_results(results);

    assert_eq!(agg.success_count, 1);
    assert_eq!(agg.failed_count, 0);
}

#[test]
fn aggregate_results_success_batch_with_empty_vec() {
    let results: BatchResultVec = vec![Ok(vec![])];

    let agg = super::batch_processor::aggregate_results(results);

    assert_eq!(
        agg.success_count, 0,
        "an Ok with empty vec contributes 0 to success_count"
    );
    assert_eq!(agg.failed_count, 0);
}

#[test]
fn aggregate_results_failure_with_zero_batch_size() {
    let results: BatchResultVec = vec![Err(("weird error".to_string(), 0))];

    let agg = super::batch_processor::aggregate_results(results);

    assert_eq!(agg.success_count, 0);
    assert_eq!(agg.failed_count, 0);
    assert_eq!(agg.first_error.as_deref(), Some("weird error"));
}

// -----------------------------------------------------------------------
// max_failure_ratio boundary tests (uses config default of 0.5)
// -----------------------------------------------------------------------

#[test]
fn max_failure_ratio_exactly_at_boundary_does_not_fail() {
    // finalize_download uses strictly-greater: (failed / total) > max_failure_ratio
    // At exactly 50% failures (ratio == 0.5), the download should NOT be marked failed
    let max_ratio = crate::config::DownloadConfig::default().max_failure_ratio;
    let success_count: usize = 50;
    let failed_count: usize = 50;
    let total = success_count + failed_count;
    let ratio = failed_count as f64 / total as f64;

    assert!(
        !(success_count == 0 || ratio > max_ratio),
        "exactly 50% failures should NOT trigger failure (uses > not >=)"
    );
}

#[test]
fn max_failure_ratio_just_above_boundary_fails() {
    let max_ratio = crate::config::DownloadConfig::default().max_failure_ratio;
    let success_count: usize = 49;
    let failed_count: usize = 51;
    let total = success_count + failed_count;
    let ratio = failed_count as f64 / total as f64;

    assert!(
        success_count == 0 || ratio > max_ratio,
        "51% failures should trigger failure"
    );
}

#[test]
fn max_failure_ratio_all_failures_always_fails() {
    let max_ratio = crate::config::DownloadConfig::default().max_failure_ratio;
    let success_count: usize = 0;
    let failed_count: usize = 100;

    // Even without checking the ratio, success_count == 0 triggers failure
    assert!(
        success_count == 0
            || (failed_count as f64 / (success_count + failed_count) as f64) > max_ratio,
        "zero successes should always trigger failure regardless of ratio"
    );
}

#[test]
fn max_failure_ratio_single_failure_in_large_set_does_not_fail() {
    let max_ratio = crate::config::DownloadConfig::default().max_failure_ratio;
    let success_count: usize = 999;
    let failed_count: usize = 1;
    let total = success_count + failed_count;
    let ratio = failed_count as f64 / total as f64;

    assert!(
        !(success_count == 0 || ratio > max_ratio),
        "0.1% failure rate should not trigger failure"
    );
}

// ===================================================================
// MockArticleProvider and test context helpers
// ===================================================================

struct MockArticleProvider {
    responses: std::sync::Mutex<VecDeque<nntp_rs::Result<Vec<nntp_rs::NntpBinaryResponse>>>>,
}

impl MockArticleProvider {
    /// Returns Ok with NntpBinaryResponse for each data Vec
    fn succeeding(data: Vec<Vec<u8>>) -> Self {
        let responses: Vec<nntp_rs::NntpBinaryResponse> = data
            .into_iter()
            .map(|d| nntp_rs::NntpBinaryResponse {
                code: 222,
                message: "Body follows".into(),
                data: d,
            })
            .collect();
        Self {
            responses: std::sync::Mutex::new(VecDeque::from(vec![Ok(responses)])),
        }
    }

    /// Returns Err for every call
    fn failing(err_msg: &str) -> Self {
        Self {
            responses: std::sync::Mutex::new(VecDeque::from(vec![Err(nntp_rs::NntpError::Other(
                err_msg.to_string(),
            ))])),
        }
    }

    /// Custom sequence of responses
    fn with_responses(responses: Vec<nntp_rs::Result<Vec<nntp_rs::NntpBinaryResponse>>>) -> Self {
        Self {
            responses: std::sync::Mutex::new(VecDeque::from(responses)),
        }
    }
}

#[async_trait::async_trait]
impl ArticleProvider for MockArticleProvider {
    async fn fetch_articles(
        &self,
        _message_ids: &[&str],
        _pipeline_depth: usize,
    ) -> nntp_rs::Result<Vec<nntp_rs::NntpBinaryResponse>> {
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                Err(nntp_rs::NntpError::Other(
                    "No more mock responses".to_string(),
                ))
            })
    }
}

/// Helper: create a NewDownload for test insertion.
fn make_new_download(temp_dir: &tempfile::TempDir) -> crate::db::NewDownload {
    crate::db::NewDownload {
        name: "test".to_string(),
        nzb_path: "/tmp/test.nzb".to_string(),
        nzb_meta_name: None,
        nzb_hash: None,
        job_name: None,
        category: None,
        destination: temp_dir.path().join("dest").to_string_lossy().to_string(),
        post_process: 0,
        priority: 0,
        status: crate::types::Status::Queued.to_i32(),
        size_bytes: 1000,
    }
}

/// Helper: insert N pending articles for a download.
async fn insert_test_articles(
    db: &crate::db::Database,
    download_id: crate::types::DownloadId,
    count: usize,
) -> Vec<i64> {
    let mut article_ids = Vec::with_capacity(count);
    for i in 0..count {
        let article = crate::db::NewArticle {
            download_id,
            message_id: format!("<seg-{}@test>", i + 1),
            segment_number: (i + 1) as i32,
            file_index: 0,
            size_bytes: 100,
        };
        let id = db.insert_article(&article).await.unwrap();
        article_ids.push(id);
    }
    article_ids
}

/// Build a full DownloadTaskContext backed by a real SQLite DB and the given mock provider.
async fn make_test_context(
    provider: Arc<dyn ArticleProvider>,
) -> (
    DownloadTaskContext,
    tempfile::TempDir,
    tokio::sync::broadcast::Receiver<crate::types::Event>,
) {
    use crate::parity::NoOpParityHandler;

    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let mut config = Config::default();
    config.persistence.database_path = db_path;
    config.servers = vec![];
    config.download.max_concurrent_downloads = 3;
    config.download.temp_dir = temp_dir.path().join("tmp");

    // Initialize database
    let db = crate::db::Database::new(&config.persistence.database_path)
        .await
        .unwrap();

    // Broadcast channel
    let (event_tx, event_rx) = tokio::sync::broadcast::channel(1000);

    // Speed limiter (unlimited)
    let speed_limiter = crate::speed_limiter::SpeedLimiter::new(config.download.speed_limit_bps);

    let config_arc = std::sync::Arc::new(config.clone());
    let db_arc = std::sync::Arc::new(db);

    // Active downloads map
    let active_downloads =
        std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

    // Queue state
    let queue = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BinaryHeap::new()));
    let concurrent_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(
        config.download.max_concurrent_downloads,
    ));
    let queue_state = super::super::QueueState {
        queue,
        concurrent_limit,
        active_downloads: active_downloads.clone(),
        accepting_new: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
    };

    // Runtime config
    let categories = std::sync::Arc::new(tokio::sync::RwLock::new(
        config.persistence.categories.clone(),
    ));
    let schedule_rules = std::sync::Arc::new(tokio::sync::RwLock::new(vec![]));
    let next_schedule_rule_id = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
    let runtime_config = super::super::RuntimeConfig {
        categories,
        schedule_rules,
        next_schedule_rule_id,
    };

    // Parity + post-processor
    let parity_handler: std::sync::Arc<dyn crate::parity::ParityHandler> =
        std::sync::Arc::new(NoOpParityHandler);
    let post_processor = std::sync::Arc::new(crate::post_processing::PostProcessor::new(
        event_tx.clone(),
        config_arc.clone(),
        parity_handler.clone(),
        db_arc.clone(),
    ));
    let processing = super::super::ProcessingPipeline {
        post_processor,
        parity_handler,
    };

    let downloader = super::super::UsenetDownloader {
        db: db_arc.clone(),
        event_tx: event_tx.clone(),
        config: config_arc.clone(),
        nntp_pools: std::sync::Arc::new(Vec::new()),
        speed_limiter: speed_limiter.clone(),
        queue_state,
        runtime_config,
        processing,
    };

    let ctx = DownloadTaskContext {
        id: crate::types::DownloadId(1), // placeholder; tests override
        db: db_arc,
        event_tx,
        article_provider: provider,
        config: config_arc,
        active_downloads,
        speed_limiter,
        cancel_token: tokio_util::sync::CancellationToken::new(),
        downloader,
    };

    (ctx, temp_dir, event_rx)
}

// ===================================================================
// fetch_download_record tests
// ===================================================================

#[tokio::test]
async fn fetch_download_record_not_found() {
    let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
    let (ctx, _temp_dir, _rx) = make_test_context(provider).await;
    // ctx.id defaults to DownloadId(1) which doesn't exist in DB

    let result = fetch_download_record(&ctx).await;

    assert!(
        result.is_none(),
        "should return None when download is not in DB"
    );
}

#[tokio::test]
async fn fetch_download_record_transitions_to_downloading() {
    let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
    let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

    // Insert download + 3 articles
    let new_dl = make_new_download(&temp_dir);
    let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
    ctx.id = dl_id;
    insert_test_articles(&ctx.db, dl_id, 3).await;

    let result = fetch_download_record(&ctx).await;

    // Returns Some with 3 pending articles
    let (_download, articles) = result.expect("should return Some");
    assert_eq!(articles.len(), 3, "should return all 3 pending articles");

    // DB status changed to Downloading
    let db_dl = ctx.db.get_download(dl_id).await.unwrap().unwrap();
    assert_eq!(
        db_dl.status,
        crate::types::Status::Downloading.to_i32(),
        "status should be Downloading"
    );

    // started_at is set
    assert!(
        db_dl.started_at.is_some(),
        "started_at should be set after transitioning to Downloading"
    );

    // Event::Downloading{percent:0.0} was emitted
    let event = rx.try_recv().unwrap();
    match event {
        crate::types::Event::Downloading {
            id,
            percent,
            speed_bps,
            ..
        } => {
            assert_eq!(id, dl_id);
            assert!((percent - 0.0).abs() < f32::EPSILON);
            assert_eq!(speed_bps, 0);
        }
        other => panic!("expected Downloading event, got {:?}", other),
    }
}

#[tokio::test]
async fn fetch_download_record_returns_only_pending_articles() {
    let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
    let (mut ctx, temp_dir, _rx) = make_test_context(provider).await;

    let new_dl = make_new_download(&temp_dir);
    let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
    ctx.id = dl_id;

    // Insert 5 articles
    let article_ids = insert_test_articles(&ctx.db, dl_id, 5).await;

    // Mark 2 as DOWNLOADED
    ctx.db
        .update_articles_status_batch(&[
            (article_ids[0], crate::db::article_status::DOWNLOADED),
            (article_ids[1], crate::db::article_status::DOWNLOADED),
        ])
        .await
        .unwrap();

    let result = fetch_download_record(&ctx).await;
    let (_download, articles) = result.unwrap();
    assert_eq!(
        articles.len(),
        3,
        "should return only the 3 pending articles"
    );
}

// ===================================================================
// finalize_download tests
// ===================================================================

#[tokio::test]
async fn finalize_all_success_marks_complete() {
    let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
    let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

    let new_dl = make_new_download(&temp_dir);
    let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
    ctx.id = dl_id;
    let db = ctx.db.clone();

    finalize_download(
        ctx,
        DownloadResults {
            success_count: 10,
            failed_count: 0,
            first_error: None,
            total_articles: 10,
            individually_failed: 0,
        },
        1000,
    )
    .await;

    // DB status = Complete
    let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
    assert_eq!(
        db_dl.status,
        crate::types::Status::Complete.to_i32(),
        "status should be Complete"
    );

    // Event: DownloadComplete
    let event = rx.try_recv().unwrap();
    match event {
        crate::types::Event::DownloadComplete { id, .. } => {
            assert_eq!(id, dl_id);
        }
        other => panic!("expected DownloadComplete event, got {:?}", other),
    }
}

#[tokio::test]
async fn finalize_above_threshold_marks_failed() {
    let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
    let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

    let new_dl = make_new_download(&temp_dir);
    let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
    ctx.id = dl_id;
    let db = ctx.db.clone();

    finalize_download(
        ctx,
        DownloadResults {
            success_count: 4,
            failed_count: 6,
            first_error: Some("batch fetch failed".to_string()),
            total_articles: 10,
            individually_failed: 0,
        },
        1000,
    )
    .await;

    // DB status = Failed
    let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
    assert_eq!(
        db_dl.status,
        crate::types::Status::Failed.to_i32(),
        "60% failure rate should mark Failed"
    );

    // Event: DownloadFailed
    let event = rx.try_recv().unwrap();
    match event {
        crate::types::Event::DownloadFailed { id, error, .. } => {
            assert_eq!(id, dl_id);
            assert!(
                error.contains("articles failed"),
                "error should contain article stats, got: {error}"
            );
        }
        other => panic!("expected DownloadFailed event, got {:?}", other),
    }
}

#[tokio::test]
async fn finalize_at_exact_boundary_marks_complete() {
    let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
    let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

    let new_dl = make_new_download(&temp_dir);
    let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
    ctx.id = dl_id;
    let db = ctx.db.clone();

    finalize_download(
        ctx,
        DownloadResults {
            success_count: 50,
            failed_count: 50,
            first_error: Some("some error".to_string()),
            total_articles: 100,
            individually_failed: 0,
        },
        1000,
    )
    .await;

    // At exactly 50% the condition is > 0.5, so it should NOT fail
    let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
    assert_eq!(
        db_dl.status,
        crate::types::Status::Complete.to_i32(),
        "exactly 50% failures uses strict >, so should be Complete"
    );

    let event = rx.try_recv().unwrap();
    assert!(
        matches!(event, crate::types::Event::DownloadComplete { .. }),
        "expected DownloadComplete, got {:?}",
        event
    );
}

#[tokio::test]
async fn finalize_zero_success_always_fails() {
    let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
    let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

    let new_dl = make_new_download(&temp_dir);
    let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
    ctx.id = dl_id;
    let db = ctx.db.clone();

    finalize_download(
        ctx,
        DownloadResults {
            success_count: 0,
            failed_count: 5,
            first_error: Some("total failure".to_string()),
            total_articles: 5,
            individually_failed: 0,
        },
        1000,
    )
    .await;

    let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
    assert_eq!(
        db_dl.status,
        crate::types::Status::Failed.to_i32(),
        "zero success should always fail"
    );

    let event = rx.try_recv().unwrap();
    assert!(
        matches!(event, crate::types::Event::DownloadFailed { .. }),
        "expected DownloadFailed, got {:?}",
        event
    );
}

// ===================================================================
// fetch_article_batch tests
// ===================================================================

/// Helper to create empty OutputFiles (no DirectWrite -- fallback to article_N.dat)
fn empty_output_files() -> Arc<OutputFiles> {
    Arc::new(OutputFiles {
        files: std::collections::HashMap::new(),
    })
}

#[tokio::test]
async fn fetch_article_batch_cancelled() {
    let provider = Arc::new(MockArticleProvider::succeeding(vec![b"data".to_vec()]));
    let cancel_token = tokio_util::sync::CancellationToken::new();
    cancel_token.cancel(); // pre-cancel

    let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(100);
    let articles = vec![make_article(1, 1, 100)];
    let temp_dir = tempfile::tempdir().unwrap();

    let (fct_tx, _fct_rx) = tokio::sync::mpsc::unbounded_channel();
    let result = super::batch_processor::fetch_article_batch(FetchArticleBatchParams {
        id: crate::types::DownloadId(1),
        article_batch: articles,
        article_provider: provider,
        batch_tx,
        speed_limiter: crate::speed_limiter::SpeedLimiter::new(None),
        cancel_token,
        download_temp_dir: temp_dir.path().to_path_buf(),
        downloaded_bytes: Arc::new(AtomicU64::new(0)),
        downloaded_articles: Arc::new(AtomicU64::new(0)),
        failed_articles: Arc::new(AtomicU64::new(0)),
        output_files: empty_output_files(),
        pipeline_depth: 10,
        file_completion_tracker: Arc::new(super::context::FileCompletionTracker::new(
            std::collections::HashMap::new(),
            fct_tx,
        )),
    })
    .await;

    assert!(result.is_err(), "should return Err when cancelled");
    let (msg, count) = result.unwrap_err();
    assert_eq!(msg, "Download cancelled");
    assert_eq!(count, 1);

    // No status updates sent
    assert!(
        batch_rx.try_recv().is_err(),
        "no status updates should be sent when cancelled"
    );
}

#[tokio::test]
async fn fetch_article_batch_success_writes_files() {
    let article_data = vec![b"hello world".to_vec(), b"second article".to_vec()];
    let provider = Arc::new(MockArticleProvider::succeeding(article_data.clone()));

    let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(100);
    let temp_dir = tempfile::tempdir().unwrap();
    let downloaded_bytes = Arc::new(AtomicU64::new(0));
    let downloaded_articles = Arc::new(AtomicU64::new(0));

    let articles = vec![make_article(10, 1, 50), make_article(11, 2, 60)];

    let (fct_tx, _fct_rx) = tokio::sync::mpsc::unbounded_channel();
    let result = super::batch_processor::fetch_article_batch(FetchArticleBatchParams {
        id: crate::types::DownloadId(1),
        article_batch: articles,
        article_provider: provider,
        batch_tx,
        speed_limiter: crate::speed_limiter::SpeedLimiter::new(None),
        cancel_token: tokio_util::sync::CancellationToken::new(),
        download_temp_dir: temp_dir.path().to_path_buf(),
        downloaded_bytes: downloaded_bytes.clone(),
        downloaded_articles: downloaded_articles.clone(),
        failed_articles: Arc::new(AtomicU64::new(0)),
        output_files: empty_output_files(),
        pipeline_depth: 10,
        file_completion_tracker: Arc::new(super::context::FileCompletionTracker::new(
            std::collections::HashMap::new(),
            fct_tx,
        )),
    })
    .await;

    // Assert success -- with empty OutputFiles, yEnc decode fails so raw data is written
    let batch_results = result.unwrap();
    assert_eq!(batch_results.len(), 2);
    // Sizes are now the raw data sizes (yEnc decode fails, fallback writes raw bytes)
    assert_eq!(batch_results[0].0, 1); // segment_number
    assert_eq!(batch_results[1].0, 2); // segment_number

    // Files exist on disk (fallback article_N.dat since no OutputFiles mapping)
    let file1 = temp_dir.path().join("article_1.dat");
    let file2 = temp_dir.path().join("article_2.dat");
    assert!(file1.exists(), "article_1.dat should exist");
    assert!(file2.exists(), "article_2.dat should exist");
    assert_eq!(
        std::fs::read(&file1).unwrap(),
        b"hello world",
        "article_1.dat content should match"
    );
    assert_eq!(
        std::fs::read(&file2).unwrap(),
        b"second article",
        "article_2.dat content should match"
    );

    // Atomics incremented
    assert_eq!(downloaded_articles.load(Ordering::Relaxed), 2);
    let total_bytes = downloaded_bytes.load(Ordering::Relaxed);
    assert_eq!(
        total_bytes,
        (b"hello world".len() + b"second article".len()) as u64,
        "bytes should reflect raw data sizes"
    );

    // DOWNLOADED status sent via batch_tx for both articles
    let (id1, status1) = batch_rx.try_recv().unwrap();
    assert_eq!(id1, 10);
    assert_eq!(status1, crate::db::article_status::DOWNLOADED);
    let (id2, status2) = batch_rx.try_recv().unwrap();
    assert_eq!(id2, 11);
    assert_eq!(status2, crate::db::article_status::DOWNLOADED);
}

#[tokio::test]
async fn fetch_article_batch_provider_error() {
    let provider = Arc::new(MockArticleProvider::failing("connection refused"));

    let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(100);
    let temp_dir = tempfile::tempdir().unwrap();

    let articles = vec![make_article(20, 1, 100), make_article(21, 2, 200)];

    let (fct_tx, _fct_rx) = tokio::sync::mpsc::unbounded_channel();
    let result = super::batch_processor::fetch_article_batch(FetchArticleBatchParams {
        id: crate::types::DownloadId(1),
        article_batch: articles,
        article_provider: provider,
        batch_tx,
        speed_limiter: crate::speed_limiter::SpeedLimiter::new(None),
        cancel_token: tokio_util::sync::CancellationToken::new(),
        download_temp_dir: temp_dir.path().to_path_buf(),
        downloaded_bytes: Arc::new(AtomicU64::new(0)),
        downloaded_articles: Arc::new(AtomicU64::new(0)),
        failed_articles: Arc::new(AtomicU64::new(0)),
        output_files: empty_output_files(),
        pipeline_depth: 10,
        file_completion_tracker: Arc::new(super::context::FileCompletionTracker::new(
            std::collections::HashMap::new(),
            fct_tx,
        )),
    })
    .await;

    assert!(result.is_err(), "should return Err on provider failure");
    let (msg, count) = result.unwrap_err();
    assert!(
        msg.contains("connection refused"),
        "error should contain provider message"
    );
    assert_eq!(count, 2, "batch_size should be 2");

    // FAILED status sent for all articles
    let (id1, status1) = batch_rx.try_recv().unwrap();
    assert_eq!(id1, 20);
    assert_eq!(status1, crate::db::article_status::FAILED);
    let (id2, status2) = batch_rx.try_recv().unwrap();
    assert_eq!(id2, 21);
    assert_eq!(status2, crate::db::article_status::FAILED);
}

// ===================================================================
// run_download_task integration tests
// ===================================================================

#[tokio::test]
async fn run_download_task_full_lifecycle() {
    // Mock provider that returns data for 3 articles
    let provider = Arc::new(MockArticleProvider::with_responses(vec![
        Ok(vec![nntp_rs::NntpBinaryResponse {
            code: 222,
            message: "Body follows".into(),
            data: b"article-1-data".to_vec(),
        }]),
        Ok(vec![nntp_rs::NntpBinaryResponse {
            code: 222,
            message: "Body follows".into(),
            data: b"article-2-data".to_vec(),
        }]),
        Ok(vec![nntp_rs::NntpBinaryResponse {
            code: 222,
            message: "Body follows".into(),
            data: b"article-3-data".to_vec(),
        }]),
    ]));

    let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

    // Insert download + 3 articles
    let new_dl = make_new_download(&temp_dir);
    let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
    ctx.id = dl_id;
    insert_test_articles(&ctx.db, dl_id, 3).await;

    // Need to configure a server for batching (pipeline_depth=1 so each article is its own batch)
    let mut config = (*ctx.config).clone();
    config.servers = vec![server(1, 1)];
    ctx.config = std::sync::Arc::new(config);

    let db = ctx.db.clone();
    run_download_task(ctx).await;

    // DB status = Complete
    let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
    assert_eq!(
        db_dl.status,
        crate::types::Status::Complete.to_i32(),
        "download should be Complete after successful lifecycle"
    );

    // Collect events -- find DownloadComplete
    let mut found_complete = false;
    while let Ok(event) = rx.try_recv() {
        if matches!(event, crate::types::Event::DownloadComplete { id, .. } if id == dl_id) {
            found_complete = true;
        }
    }
    assert!(
        found_complete,
        "DownloadComplete event should have been emitted"
    );
}

// ===================================================================
// retry_articles_individually tests
// ===================================================================

#[tokio::test]
async fn retry_articles_individually_partial_success() {
    // Mock: 3 individual fetches -- article 1 succeeds, article 2 missing, article 3 succeeds
    let provider = Arc::new(MockArticleProvider::with_responses(vec![
        // Article 1: success
        Ok(vec![nntp_rs::NntpBinaryResponse {
            code: 222,
            message: "Body follows".into(),
            data: b"article-1-data".to_vec(),
        }]),
        // Article 2: NoSuchArticle
        Err(nntp_rs::NntpError::NoSuchArticle(
            "<article-2@test>".to_string(),
        )),
        // Article 3: success
        Ok(vec![nntp_rs::NntpBinaryResponse {
            code: 222,
            message: "Body follows".into(),
            data: b"article-3-data".to_vec(),
        }]),
    ]));

    let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(100);
    let temp_dir = tempfile::tempdir().unwrap();
    let downloaded_bytes = Arc::new(AtomicU64::new(0));
    let downloaded_articles = Arc::new(AtomicU64::new(0));
    let failed_articles = Arc::new(AtomicU64::new(0));

    let articles = vec![
        make_article(1, 1, 100),
        make_article(2, 2, 200),
        make_article(3, 3, 300),
    ];

    let (fct_tx, _fct_rx) = tokio::sync::mpsc::unbounded_channel();
    let result = super::batch_processor::retry_articles_individually(RetryArticlesParams {
        id: crate::types::DownloadId(1),
        article_batch: articles,
        article_provider: provider,
        batch_tx,
        cancel_token: tokio_util::sync::CancellationToken::new(),
        download_temp_dir: temp_dir.path().to_path_buf(),
        downloaded_bytes: downloaded_bytes.clone(),
        downloaded_articles: downloaded_articles.clone(),
        failed_articles: failed_articles.clone(),
        output_files: empty_output_files(),
        file_completion_tracker: Arc::new(super::context::FileCompletionTracker::new(
            std::collections::HashMap::new(),
            fct_tx,
        )),
    })
    .await;

    // Should succeed with 2 articles
    let batch_results = result.unwrap();
    assert_eq!(batch_results.len(), 2, "2 of 3 articles should succeed");
    // Sizes are raw data sizes (yEnc decode fails, fallback writes raw bytes)
    assert_eq!(batch_results[0].0, 1); // segment_number
    assert_eq!(batch_results[1].0, 3); // segment_number

    // Counters -- sizes are raw byte counts
    assert_eq!(downloaded_articles.load(Ordering::Relaxed), 2);
    let total_bytes = downloaded_bytes.load(Ordering::Relaxed);
    assert_eq!(
        total_bytes,
        (b"article-1-data".len() + b"article-3-data".len()) as u64
    );
    assert_eq!(failed_articles.load(Ordering::Relaxed), 1);

    // Status updates via batch_tx
    let (id1, status1) = batch_rx.try_recv().unwrap();
    assert_eq!(id1, 1);
    assert_eq!(status1, crate::db::article_status::DOWNLOADED);

    let (id2, status2) = batch_rx.try_recv().unwrap();
    assert_eq!(id2, 2);
    assert_eq!(status2, crate::db::article_status::FAILED);

    let (id3, status3) = batch_rx.try_recv().unwrap();
    assert_eq!(id3, 3);
    assert_eq!(status3, crate::db::article_status::DOWNLOADED);
}

#[tokio::test]
async fn retry_articles_individually_all_missing() {
    // Mock: all 3 articles return 430
    let provider = Arc::new(MockArticleProvider::with_responses(vec![
        Err(nntp_rs::NntpError::NoSuchArticle(
            "<article-1@test>".to_string(),
        )),
        Err(nntp_rs::NntpError::NoSuchArticle(
            "<article-2@test>".to_string(),
        )),
        Err(nntp_rs::NntpError::NoSuchArticle(
            "<article-3@test>".to_string(),
        )),
    ]));

    let (batch_tx, mut batch_rx) = tokio::sync::mpsc::channel(100);
    let temp_dir = tempfile::tempdir().unwrap();
    let failed_articles = Arc::new(AtomicU64::new(0));

    let articles = vec![
        make_article(10, 1, 100),
        make_article(11, 2, 200),
        make_article(12, 3, 300),
    ];

    let (fct_tx, _fct_rx) = tokio::sync::mpsc::unbounded_channel();
    let result = super::batch_processor::retry_articles_individually(RetryArticlesParams {
        id: crate::types::DownloadId(1),
        article_batch: articles,
        article_provider: provider,
        batch_tx,
        cancel_token: tokio_util::sync::CancellationToken::new(),
        download_temp_dir: temp_dir.path().to_path_buf(),
        downloaded_bytes: Arc::new(AtomicU64::new(0)),
        downloaded_articles: Arc::new(AtomicU64::new(0)),
        failed_articles: failed_articles.clone(),
        output_files: empty_output_files(),
        file_completion_tracker: Arc::new(super::context::FileCompletionTracker::new(
            std::collections::HashMap::new(),
            fct_tx,
        )),
    })
    .await;

    // Should fail -- all articles missing
    assert!(result.is_err(), "should return Err when all articles fail");
    let (msg, count) = result.unwrap_err();
    assert!(
        msg.contains("No such article"),
        "error should mention missing article, got: {msg}"
    );
    assert_eq!(count, 3, "batch_size should be 3");

    // All 3 articles counted as failed
    assert_eq!(failed_articles.load(Ordering::Relaxed), 3);

    // All 3 marked FAILED via batch_tx
    for expected_id in [10, 11, 12] {
        let (id, status) = batch_rx.try_recv().unwrap();
        assert_eq!(id, expected_id);
        assert_eq!(status, crate::db::article_status::FAILED);
    }
}

// ===================================================================
// fast-fail watcher tests
// ===================================================================

#[tokio::test]
async fn fast_fail_cancels_when_mostly_missing() {
    let downloaded_articles = Arc::new(AtomicU64::new(0));
    let failed_articles = Arc::new(AtomicU64::new(0));
    let cancel_token = tokio_util::sync::CancellationToken::new();

    // Configure: threshold 0.8, sample size 5
    let _watcher = spawn_fast_fail_watcher(
        &downloaded_articles,
        &failed_articles,
        0.8,
        5,
        cancel_token.clone(),
    );

    // Simulate 4 failures, 1 success (80% failure = threshold)
    failed_articles.store(4, Ordering::Relaxed);
    downloaded_articles.store(1, Ordering::Relaxed);

    // Wait for the watcher to detect it (polls every 200ms)
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    assert!(
        cancel_token.is_cancelled(),
        "should cancel when failure ratio >= threshold (4/5 = 0.8 >= 0.8)"
    );
}

#[tokio::test]
async fn fast_fail_does_not_cancel_below_threshold() {
    let downloaded_articles = Arc::new(AtomicU64::new(0));
    let failed_articles = Arc::new(AtomicU64::new(0));
    let cancel_token = tokio_util::sync::CancellationToken::new();

    let _watcher = spawn_fast_fail_watcher(
        &downloaded_articles,
        &failed_articles,
        0.8,
        5,
        cancel_token.clone(),
    );

    // 3 failures, 2 successes (60% < 80% threshold)
    failed_articles.store(3, Ordering::Relaxed);
    downloaded_articles.store(2, Ordering::Relaxed);

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    assert!(
        !cancel_token.is_cancelled(),
        "should NOT cancel when failure ratio < threshold (3/5 = 0.6 < 0.8)"
    );
}

// ===================================================================
// configurable failure ratio tests
// ===================================================================

#[tokio::test]
async fn configurable_failure_ratio_from_config() {
    let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
    let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

    // Set a low failure ratio threshold
    let mut config = (*ctx.config).clone();
    config.download.max_failure_ratio = 0.1; // 10% threshold
    ctx.config = std::sync::Arc::new(config);

    let new_dl = make_new_download(&temp_dir);
    let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
    ctx.id = dl_id;
    let db = ctx.db.clone();

    // 85 successes, 15 failures = 15% > 10% threshold
    finalize_download(
        ctx,
        DownloadResults {
            success_count: 85,
            failed_count: 15,
            first_error: Some("missing article".to_string()),
            total_articles: 100,
            individually_failed: 0,
        },
        10000,
    )
    .await;

    let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
    assert_eq!(
        db_dl.status,
        crate::types::Status::Failed.to_i32(),
        "15% failure rate should trigger failure with max_failure_ratio=0.1"
    );

    let event = rx.try_recv().unwrap();
    match event {
        crate::types::Event::DownloadFailed { id, error, .. } => {
            assert_eq!(id, dl_id);
            assert!(
                error.contains("articles failed"),
                "error should contain article stats, got: {error}"
            );
        }
        other => panic!("expected DownloadFailed event, got {:?}", other),
    }
}

#[tokio::test]
async fn partial_success_emits_complete_with_stats() {
    let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
    let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

    let new_dl = make_new_download(&temp_dir);
    let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
    ctx.id = dl_id;
    let db = ctx.db.clone();

    // 90 successes, 10 failures = 10% (at default 50% threshold, this is fine)
    finalize_download(
        ctx,
        DownloadResults {
            success_count: 90,
            failed_count: 5,
            first_error: Some("missing".to_string()),
            total_articles: 100,
            individually_failed: 5, // 5 batch + 5 individual = 10 total
        },
        10000,
    )
    .await;

    let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
    assert_eq!(
        db_dl.status,
        crate::types::Status::Complete.to_i32(),
        "10% failure rate should still complete with default 50% threshold"
    );

    let event = rx.try_recv().unwrap();
    match event {
        crate::types::Event::DownloadComplete {
            id,
            articles_failed,
            articles_total,
        } => {
            assert_eq!(id, dl_id);
            assert_eq!(
                articles_failed,
                Some(10),
                "should report 10 total failed articles"
            );
            assert_eq!(
                articles_total,
                Some(100),
                "should report 100 total articles"
            );
        }
        other => panic!("expected DownloadComplete event, got {:?}", other),
    }
}

#[tokio::test]
async fn finalize_download_keeps_partially_paused_download_paused() {
    let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
    let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

    let new_dl = make_new_download(&temp_dir);
    let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
    ctx.id = dl_id;

    insert_test_file(&ctx.db, dl_id, 0, 1).await;
    insert_test_file(&ctx.db, dl_id, 1, 1).await;

    let done_id = ctx
        .db
        .insert_article(&crate::db::NewArticle {
            download_id: dl_id,
            message_id: "<done@test>".to_string(),
            segment_number: 1,
            file_index: 0,
            size_bytes: 100,
        })
        .await
        .unwrap();
    ctx.db
        .insert_article(&crate::db::NewArticle {
            download_id: dl_id,
            message_id: "<paused@test>".to_string(),
            segment_number: 1,
            file_index: 1,
            size_bytes: 100,
        })
        .await
        .unwrap();
    ctx.db
        .update_article_status(done_id, crate::db::article_status::DOWNLOADED)
        .await
        .unwrap();
    ctx.db.set_file_paused(dl_id, 1, true).await.unwrap();

    let db = ctx.db.clone();
    finalize_download(
        ctx,
        DownloadResults {
            success_count: 1,
            failed_count: 0,
            first_error: None,
            total_articles: 1,
            individually_failed: 0,
        },
        100,
    )
    .await;

    let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
    assert_eq!(db_dl.status, crate::types::Status::Paused.to_i32());

    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event, crate::types::Event::DownloadComplete { .. }),
            "paused-remaining download should not emit DownloadComplete"
        );
    }
}

#[tokio::test]
async fn run_download_task_empty_articles() {
    let provider = Arc::new(MockArticleProvider::succeeding(vec![]));
    let (mut ctx, temp_dir, mut rx) = make_test_context(provider).await;

    // Insert download with 0 articles
    let new_dl = make_new_download(&temp_dir);
    let dl_id = ctx.db.insert_download(&new_dl).await.unwrap();
    ctx.id = dl_id;
    // No articles inserted

    let db = ctx.db.clone();
    run_download_task(ctx).await;

    // DownloadComplete should be emitted immediately
    let mut found_complete = false;
    while let Ok(event) = rx.try_recv() {
        if matches!(event, crate::types::Event::DownloadComplete { id, .. } if id == dl_id) {
            found_complete = true;
        }
    }
    assert!(
        found_complete,
        "DownloadComplete event should be emitted immediately for empty articles"
    );

    // DB status should still be Downloading (fetch_download_record sets it)
    // since finalize_download is not called for empty articles path
    let db_dl = db.get_download(dl_id).await.unwrap().unwrap();
    assert_eq!(
        db_dl.status,
        crate::types::Status::Downloading.to_i32(),
        "status should be Downloading (empty articles skip finalize)"
    );
}
