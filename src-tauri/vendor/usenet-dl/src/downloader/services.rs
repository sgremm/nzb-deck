//! Background service starters — folder watcher, RSS scheduler, and time-based scheduler.

use crate::config;
use crate::error::Result;
use crate::folder_watcher;
use crate::rss_manager;
use crate::rss_scheduler;
use crate::scheduler;

use super::UsenetDownloader;

impl UsenetDownloader {
    /// Start the folder watcher background task
    pub fn start_folder_watcher(&self) -> Result<tokio::task::JoinHandle<()>> {
        let watch_folders = self.config.automation.watch_folders.clone();

        if watch_folders.is_empty() {
            tracing::info!("No watch folders configured, skipping folder watcher");
            return Ok(tokio::spawn(async {}));
        }

        let mut watcher =
            folder_watcher::FolderWatcher::new(std::sync::Arc::new(self.clone()), watch_folders)?;

        watcher.start()?;

        let handle = tokio::spawn(async move {
            watcher.run().await;
        });

        tracing::info!("Folder watcher background task started");

        Ok(handle)
    }

    /// Start RSS feed scheduler for automatic feed checking
    pub fn start_rss_scheduler(&self) -> tokio::task::JoinHandle<()> {
        let rss_feeds = self.config.automation.rss_feeds.clone();

        if rss_feeds.is_empty() {
            tracing::info!("No RSS feeds configured, skipping RSS scheduler");
            return tokio::spawn(async {});
        }

        let rss_manager = match rss_manager::RssManager::new(
            self.db.clone(),
            std::sync::Arc::new(self.clone()),
            rss_feeds.clone(),
        ) {
            Ok(manager) => std::sync::Arc::new(manager),
            Err(e) => {
                tracing::error!(error = %e, "Failed to create RSS manager");
                return tokio::spawn(async {});
            }
        };

        let scheduler =
            rss_scheduler::RssScheduler::new(std::sync::Arc::new(self.clone()), rss_manager);

        let handle = tokio::spawn(async move {
            scheduler.run().await;
        });

        tracing::info!("RSS scheduler background task started");

        handle
    }

    /// Start the scheduler task that checks schedule rules every minute
    pub fn start_scheduler(&self) -> tokio::task::JoinHandle<()> {
        let schedule_rules = self.config.persistence.schedule_rules.clone();

        if schedule_rules.is_empty() {
            tracing::info!("No schedule rules configured, skipping scheduler task");
            return tokio::spawn(async {});
        }

        // Convert config::ScheduleRule to scheduler::ScheduleRule
        let scheduler_rules: Vec<scheduler::ScheduleRule> = schedule_rules
            .into_iter()
            .enumerate()
            .filter_map(|(idx, rule)| {
                let start_time =
                    chrono::NaiveTime::parse_from_str(&rule.start_time, "%H:%M").ok()?;
                let end_time = chrono::NaiveTime::parse_from_str(&rule.end_time, "%H:%M").ok()?;

                let days: Vec<scheduler::Weekday> = rule
                    .days
                    .into_iter()
                    .map(|d| match d {
                        config::Weekday::Monday => scheduler::Weekday::Monday,
                        config::Weekday::Tuesday => scheduler::Weekday::Tuesday,
                        config::Weekday::Wednesday => scheduler::Weekday::Wednesday,
                        config::Weekday::Thursday => scheduler::Weekday::Thursday,
                        config::Weekday::Friday => scheduler::Weekday::Friday,
                        config::Weekday::Saturday => scheduler::Weekday::Saturday,
                        config::Weekday::Sunday => scheduler::Weekday::Sunday,
                    })
                    .collect();

                let action = match rule.action {
                    config::ScheduleAction::SpeedLimit { limit_bps } => {
                        scheduler::ScheduleAction::SpeedLimit(limit_bps)
                    }
                    config::ScheduleAction::Unlimited => scheduler::ScheduleAction::Unlimited,
                    config::ScheduleAction::Pause => scheduler::ScheduleAction::Pause,
                };

                Some(scheduler::ScheduleRule {
                    id: scheduler::RuleId(idx as i64),
                    name: rule.name,
                    days,
                    start_time,
                    end_time,
                    action,
                    enabled: rule.enabled,
                })
            })
            .collect();

        let scheduler = std::sync::Arc::new(scheduler::Scheduler::new(scheduler_rules));

        let scheduler_task =
            crate::scheduler_task::SchedulerTask::new(std::sync::Arc::new(self.clone()), scheduler);

        let handle = tokio::spawn(async move {
            scheduler_task.run().await;
        });

        tracing::info!("Scheduler task started, checking rules every minute");

        handle
    }
}
