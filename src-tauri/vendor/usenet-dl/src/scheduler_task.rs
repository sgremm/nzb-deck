//! Scheduler task execution for time-based automation
//!
//! This module provides the background task that evaluates schedule rules and applies
//! actions (speed limits, pauses) based on the current time and day of week.
//!
//! # Features
//!
//! - Minute-level rule evaluation
//! - Action change tracking to avoid redundant operations
//! - Graceful shutdown handling
//! - Automatic revert to defaults when no rules match
//!
//! # Example
//!
//! ```no_run
//! use usenet_dl::{UsenetDownloader, config::Config};
//! use usenet_dl::scheduler::Scheduler;
//! use usenet_dl::scheduler_task::SchedulerTask;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = Config::default();
//! let downloader = Arc::new(UsenetDownloader::new(config).await?);
//! let scheduler = Arc::new(Scheduler::new(downloader.config.schedule_rules.clone()));
//!
//! let task = SchedulerTask::new(downloader.clone(), scheduler);
//!
//! // Run scheduler task (blocks until shutdown)
//! tokio::spawn(async move {
//!     task.run().await;
//! });
//! # Ok(())
//! # }
//! ```

use crate::{
    UsenetDownloader,
    scheduler::{ScheduleAction, Scheduler},
};
use chrono::Local;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::time::{Duration, sleep};
use tracing::{debug, info};

/// Scheduler task that periodically checks schedule rules and applies actions
///
/// The scheduler task runs every minute to evaluate schedule rules and apply
/// the appropriate action (speed limit, unlimited, or pause) based on the
/// current time and day of week.
pub struct SchedulerTask {
    /// Reference to the scheduler for rule evaluation
    scheduler: Arc<Scheduler>,

    /// Reference to downloader for applying actions and checking shutdown status
    downloader: Arc<UsenetDownloader>,
}

impl SchedulerTask {
    /// Creates a new scheduler task
    ///
    /// # Parameters
    /// - `downloader`: Reference to the UsenetDownloader for action application
    /// - `scheduler`: Reference to the Scheduler for rule evaluation
    pub fn new(downloader: Arc<UsenetDownloader>, scheduler: Arc<Scheduler>) -> Self {
        Self {
            scheduler,
            downloader,
        }
    }

    /// Starts the scheduler task
    ///
    /// This runs in a loop checking schedule rules every minute.
    /// The task will:
    /// 1. Check if shutdown was requested (via downloader.queue_state.accepting_new flag)
    /// 2. Get current time and evaluate schedule rules
    /// 3. Apply the appropriate action if a rule matches
    /// 4. Sleep for 1 minute before the next check
    ///
    /// The task respects the shutdown signal and will exit gracefully when
    /// the downloader stops accepting new downloads.
    pub async fn run(self) {
        info!("Scheduler task started");

        // Track the last applied action to avoid redundant operations
        let mut last_action: Option<ScheduleAction> = None;

        loop {
            // Check for shutdown signal via downloader's accepting_new flag
            if !self
                .downloader
                .queue_state
                .accepting_new
                .load(Ordering::SeqCst)
            {
                info!("Scheduler task shutting down");
                break;
            }

            // Get current time
            let now = Local::now();

            // Evaluate schedule rules
            let current_action = self.scheduler.get_current_action(now);

            // Apply action if it changed
            if current_action != last_action {
                match &current_action {
                    Some(action) => {
                        debug!(?action, "Schedule action changed, applying new action");
                        self.apply_action(action).await;
                    }
                    None => {
                        debug!("No schedule rules active, clearing any previous actions");
                        // When no rules match, revert to default behavior (unlimited)
                        self.clear_schedule_actions().await;
                    }
                }
                last_action = current_action;
            } else {
                debug!(
                    ?current_action,
                    "Schedule action unchanged, no action needed"
                );
            }

            // Sleep for 1 minute before next check
            sleep(Duration::from_secs(60)).await;
        }

        info!("Scheduler task stopped");
    }

    /// Apply a schedule action
    async fn apply_action(&self, action: &ScheduleAction) {
        match action {
            ScheduleAction::SpeedLimit(limit_bps) => {
                info!(limit_bps = %limit_bps, "Applying scheduled speed limit");
                self.downloader.set_speed_limit(Some(*limit_bps)).await;
            }
            ScheduleAction::Unlimited => {
                info!("Applying scheduled unlimited speed");
                self.downloader.set_speed_limit(None).await;
            }
            ScheduleAction::Pause => {
                info!("Applying scheduled pause");
                // Pause all downloads via the queue
                if let Err(e) = self.downloader.pause_all().await {
                    tracing::warn!(error = %e, "Failed to pause downloads for scheduled action");
                }
            }
        }
    }

    /// Clear schedule actions and return to default behavior
    async fn clear_schedule_actions(&self) {
        // When no schedule is active, revert to unlimited speed
        // and resume the queue if it was paused by a schedule
        info!("Clearing schedule actions, reverting to default behavior");
        self.downloader.set_speed_limit(None).await;

        // Note: We don't automatically resume here because the pause might have been
        // manual (not from a schedule). A future enhancement could track whether
        // the pause was from a schedule or manual.
    }
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::{ScheduleRule, Weekday};
    use chrono::{Datelike, NaiveTime, Timelike};

    async fn create_test_downloader() -> (UsenetDownloader, tempfile::TempDir) {
        crate::downloader::test_helpers::create_test_downloader().await
    }

    #[tokio::test]
    async fn test_scheduler_task_shutdown_on_signal() {
        let (downloader, _temp_dir) = create_test_downloader().await;
        let scheduler = Scheduler::new(vec![]);

        let downloader_arc = Arc::new(downloader);

        // Set shutdown signal immediately
        downloader_arc
            .queue_state
            .accepting_new
            .store(false, Ordering::SeqCst);

        let task = SchedulerTask::new(downloader_arc.clone(), Arc::new(scheduler));

        // Start the task
        let handle = tokio::spawn(async move {
            task.run().await;
        });

        // Task should exit gracefully without waiting the full minute
        let result = tokio::time::timeout(Duration::from_secs(1), handle).await;

        assert!(
            result.is_ok(),
            "Scheduler task should exit on shutdown signal"
        );
    }

    #[tokio::test]
    async fn test_scheduler_task_applies_speed_limit() {
        let (downloader, _temp_dir) = create_test_downloader().await;

        // Create a rule that matches current time (always active)
        let now = Local::now();
        let current_weekday = Weekday::from_chrono(now.weekday());
        let start_time = NaiveTime::from_hms_opt(now.hour().saturating_sub(1), 0, 0)
            .unwrap_or(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let end_time = NaiveTime::from_hms_opt((now.hour() + 1) % 24, 59, 59)
            .unwrap_or(NaiveTime::from_hms_opt(23, 59, 59).unwrap());

        let rule = ScheduleRule {
            id: crate::scheduler::RuleId(1),
            name: "Test Speed Limit".into(),
            days: vec![current_weekday],
            start_time,
            end_time,
            action: ScheduleAction::SpeedLimit(1_000_000), // 1 MB/s
            enabled: true,
        };

        let scheduler = Scheduler::new(vec![rule]);
        let downloader_arc = Arc::new(downloader);
        let task = SchedulerTask::new(downloader_arc.clone(), Arc::new(scheduler));

        // Verify initial speed limit is None (unlimited)
        assert_eq!(downloader_arc.get_speed_limit(), None);

        // Apply the action manually (simulating what run() would do)
        task.apply_action(&ScheduleAction::SpeedLimit(1_000_000))
            .await;

        // Verify speed limit was applied
        assert_eq!(downloader_arc.get_speed_limit(), Some(1_000_000));
    }

    #[tokio::test]
    async fn test_scheduler_task_applies_unlimited() {
        let (downloader, _temp_dir) = create_test_downloader().await;
        let downloader_arc = Arc::new(downloader);

        // First set a speed limit
        downloader_arc.set_speed_limit(Some(500_000)).await;
        assert_eq!(downloader_arc.get_speed_limit(), Some(500_000));

        let scheduler = Scheduler::new(vec![]);
        let task = SchedulerTask::new(downloader_arc.clone(), Arc::new(scheduler));

        // Apply unlimited action
        task.apply_action(&ScheduleAction::Unlimited).await;

        // Verify speed limit was removed
        assert_eq!(downloader_arc.get_speed_limit(), None);
    }

    #[tokio::test]
    async fn test_scheduler_task_clears_actions_when_no_rules_match() {
        let (downloader, _temp_dir) = create_test_downloader().await;
        let downloader_arc = Arc::new(downloader);

        // Set a speed limit
        downloader_arc.set_speed_limit(Some(1_000_000)).await;
        assert_eq!(downloader_arc.get_speed_limit(), Some(1_000_000));

        let scheduler = Scheduler::new(vec![]);
        let task = SchedulerTask::new(downloader_arc.clone(), Arc::new(scheduler));

        // Clear actions (simulating no rules matching)
        task.clear_schedule_actions().await;

        // Verify speed limit was cleared (reverted to unlimited)
        assert_eq!(downloader_arc.get_speed_limit(), None);
    }
}
