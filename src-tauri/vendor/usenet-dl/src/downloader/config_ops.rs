//! Runtime configuration updates — speed limits, categories, schedule rules.

use super::UsenetDownloader;

impl UsenetDownloader {
    /// Get the current global speed limit
    ///
    /// Returns the current speed limit in bytes per second, or None if unlimited.
    ///
    /// # Returns
    ///
    /// * `Option<u64>` - Speed limit in bytes per second (None = unlimited)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = Config::default();
    /// # let downloader = UsenetDownloader::new(config).await?;
    /// // Get current speed limit
    /// let limit = downloader.get_speed_limit();
    /// if let Some(bps) = limit {
    ///     println!("Current speed limit: {} bytes/sec", bps);
    /// } else {
    ///     println!("No speed limit (unlimited)");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_speed_limit(&self) -> Option<u64> {
        self.speed_limiter.get_limit()
    }

    /// Set the global speed limit
    ///
    /// This changes the download speed limit for all concurrent downloads.
    /// The change takes effect immediately.
    ///
    /// # Arguments
    ///
    /// * `limit_bps` - New speed limit in bytes per second (None = unlimited)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = Config::default();
    /// # let downloader = UsenetDownloader::new(config).await?;
    /// // Set to 10 MB/s
    /// downloader.set_speed_limit(Some(10_000_000)).await;
    ///
    /// // Remove speed limit (unlimited)
    /// downloader.set_speed_limit(None).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_speed_limit(&self, limit_bps: Option<u64>) {
        // Update the speed limiter
        self.speed_limiter.set_limit(limit_bps);

        // Emit event to notify subscribers
        self.emit_event(crate::types::Event::SpeedLimitChanged { limit_bps });

        tracing::info!(
            limit_bps = ?limit_bps,
            "Speed limit changed"
        );
    }

    /// Update runtime-changeable configuration settings
    ///
    /// This method updates configuration settings that can be safely changed while the
    /// downloader is running. Fields requiring restart (like database_path, download_dir,
    /// servers) cannot be updated via this method.
    ///
    /// # Arguments
    ///
    /// * `updates` - Configuration updates to apply
    pub async fn update_config(&self, updates: crate::config::ConfigUpdate) {
        // Update speed limit if provided
        if let Some(speed_limit) = updates.speed_limit_bps {
            self.set_speed_limit(speed_limit).await;
        }
    }

    /// Create or update a category
    ///
    /// This method adds a new category or updates an existing one with the provided configuration.
    /// The change takes effect immediately for new downloads.
    ///
    /// # Arguments
    ///
    /// * `name` - The category name
    /// * `config` - The category configuration
    pub async fn add_or_update_category(&self, name: &str, config: crate::config::CategoryConfig) {
        let mut categories = self.runtime_config.categories.write().await;
        categories.insert(name.to_string(), config);
    }

    /// Remove a category
    ///
    /// This method removes a category from the runtime configuration.
    /// Returns true if the category existed and was removed, false otherwise.
    ///
    /// # Arguments
    ///
    /// * `name` - The category name to remove
    ///
    /// # Returns
    ///
    /// `true` if the category was removed, `false` if it didn't exist
    pub async fn remove_category(&self, name: &str) -> bool {
        let mut categories = self.runtime_config.categories.write().await;
        categories.remove(name).is_some()
    }

    /// Get all categories
    ///
    /// Returns a clone of the current categories HashMap.
    ///
    /// **Performance Note:** This method clones the entire HashMap. For read-only access
    /// from internal code, prefer using `with_categories()` to avoid unnecessary allocations.
    pub async fn get_categories(
        &self,
    ) -> std::collections::HashMap<String, crate::config::CategoryConfig> {
        self.runtime_config.categories.read().await.clone()
    }

    /// Access categories with a read lock without cloning
    ///
    /// This method provides read-only access to the categories HashMap without cloning.
    /// The provided closure receives a reference to the HashMap while holding a read lock.
    ///
    /// # Arguments
    ///
    /// * `f` - Closure that receives a reference to the categories HashMap
    ///
    /// # Returns
    ///
    /// The result of the closure
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = Config::default();
    /// # let downloader = UsenetDownloader::new(config).await?;
    /// // Check if a category exists without cloning
    /// let has_movies = downloader.with_categories(|categories| {
    ///     categories.contains_key("movies")
    /// }).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_categories<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&std::collections::HashMap<String, crate::config::CategoryConfig>) -> R,
    {
        let categories = self.runtime_config.categories.read().await;
        f(&categories)
    }

    // =========================================================================
    // Schedule Rule Management
    // =========================================================================

    /// Get all schedule rules
    ///
    /// Returns a clone of the current schedule rules list.
    ///
    /// **Performance Note:** This method clones the entire Vec. For read-only access
    /// from internal code, prefer using `with_schedule_rules()` to avoid unnecessary allocations.
    pub async fn get_schedule_rules(&self) -> Vec<crate::config::ScheduleRule> {
        self.runtime_config.schedule_rules.read().await.clone()
    }

    /// Access schedule rules with a read lock without cloning
    ///
    /// This method provides read-only access to the schedule rules Vec without cloning.
    /// The provided closure receives a reference to the Vec while holding a read lock.
    ///
    /// # Arguments
    ///
    /// * `f` - Closure that receives a reference to the schedule rules Vec
    ///
    /// # Returns
    ///
    /// The result of the closure
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use usenet_dl::{UsenetDownloader, Config};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = Config::default();
    /// # let downloader = UsenetDownloader::new(config).await?;
    /// // Count rules without cloning
    /// let rule_count = downloader.with_schedule_rules(|rules| {
    ///     rules.len()
    /// }).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_schedule_rules<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Vec<crate::config::ScheduleRule>) -> R,
    {
        let rules = self.runtime_config.schedule_rules.read().await;
        f(&rules)
    }

    /// Add a new schedule rule
    ///
    /// This method adds a new schedule rule to the runtime configuration.
    /// Returns the assigned rule ID.
    pub async fn add_schedule_rule(
        &self,
        rule: crate::config::ScheduleRule,
    ) -> crate::scheduler::RuleId {
        let mut rules = self.runtime_config.schedule_rules.write().await;
        let id = self
            .runtime_config
            .next_schedule_rule_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        rules.push(rule);
        crate::scheduler::RuleId(id)
    }

    /// Update an existing schedule rule by ID.
    ///
    /// Uses the ID as a stable lookup key by searching for the rule's position.
    /// Returns true if the rule was found and updated, false otherwise.
    pub async fn update_schedule_rule(
        &self,
        id: crate::scheduler::RuleId,
        rule: crate::config::ScheduleRule,
    ) -> bool {
        let mut rules = self.runtime_config.schedule_rules.write().await;
        // Safely bounds-check: ID may no longer correspond to a valid index after deletions
        let idx = id.0 as usize;
        if idx < rules.len() {
            rules[idx] = rule;
            true
        } else {
            false
        }
    }

    /// Remove a schedule rule by ID.
    ///
    /// Returns true if the rule was found and removed, false otherwise.
    pub async fn remove_schedule_rule(&self, id: crate::scheduler::RuleId) -> bool {
        let mut rules = self.runtime_config.schedule_rules.write().await;
        let idx = id.0 as usize;
        if idx < rules.len() {
            rules.remove(idx);
            true
        } else {
            false
        }
    }
}
