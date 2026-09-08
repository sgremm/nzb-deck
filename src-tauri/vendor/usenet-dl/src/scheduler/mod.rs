//! Time-based scheduler for applying speed limits and pause/resume based on schedules.
//!
//! The scheduler allows users to define rules that automatically adjust download behavior
//! based on the time of day and day of week. Common use cases include:
//! - Limiting speed during work hours to preserve bandwidth
//! - Running unlimited during off-peak hours (nights/weekends)
//! - Pausing downloads during specific time windows
//!
//! # Example
//!
//! ```rust
//! use usenet_dl::scheduler::{ScheduleRule, ScheduleAction, RuleId, Weekday};
//! use chrono::NaiveTime;
//!
//! // Unlimited at night (midnight to 6 AM)
//! let night_rule = ScheduleRule {
//!     id: RuleId::new(1),
//!     name: "Night owl".into(),
//!     days: vec![],  // All days
//!     start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
//!     end_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
//!     action: ScheduleAction::Unlimited,
//!     enabled: true,
//! };
//!
//! // Limited during work hours (weekdays only)
//! let work_rule = ScheduleRule {
//!     id: RuleId::new(2),
//!     name: "Work hours".into(),
//!     days: vec![Weekday::Monday, Weekday::Tuesday, Weekday::Wednesday,
//!                Weekday::Thursday, Weekday::Friday],
//!     start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
//!     end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
//!     action: ScheduleAction::SpeedLimit(1_000_000),  // 1 MB/s
//!     enabled: true,
//! };
//! ```

use chrono::{Datelike, NaiveTime};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use utoipa::ToSchema;

/// Unique identifier for a schedule rule
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(transparent)]
pub struct RuleId(pub i64);

impl RuleId {
    /// Create a new RuleId
    pub fn new(id: i64) -> Self {
        Self(id)
    }

    /// Get the inner i64 value
    pub fn get(&self) -> i64 {
        self.0
    }
}

impl From<i64> for RuleId {
    fn from(id: i64) -> Self {
        Self(id)
    }
}

impl From<RuleId> for i64 {
    fn from(id: RuleId) -> Self {
        id.0
    }
}

impl PartialEq<i64> for RuleId {
    fn eq(&self, other: &i64) -> bool {
        self.0 == *other
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for RuleId {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

/// A time-based schedule rule that applies an action during specific time windows
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ScheduleRule {
    /// Unique identifier for this rule
    pub id: RuleId,

    /// Human-readable name for this rule
    pub name: String,

    /// Days this rule applies (empty = all days)
    pub days: Vec<Weekday>,

    /// Start time (HH:MM:SS, 24-hour format)
    #[serde(with = "time_format")]
    pub start_time: NaiveTime,

    /// End time (HH:MM:SS, 24-hour format)
    #[serde(with = "time_format")]
    pub end_time: NaiveTime,

    /// Action to take during this time window
    pub action: ScheduleAction,

    /// Whether this rule is currently active
    pub enabled: bool,
}

/// Action to take when a schedule rule is active
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value")]
pub enum ScheduleAction {
    /// Set speed limit in bytes per second
    SpeedLimit(u64),
    /// Remove speed limit (unlimited speed)
    Unlimited,
    /// Pause all downloads
    Pause,
}

/// Days of the week for schedule rules
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Weekday {
    /// Monday
    Monday,
    /// Tuesday
    Tuesday,
    /// Wednesday
    Wednesday,
    /// Thursday
    Thursday,
    /// Friday
    Friday,
    /// Saturday
    Saturday,
    /// Sunday
    Sunday,
}

impl Weekday {
    /// Convert from chrono::Weekday to our Weekday
    pub fn from_chrono(wd: chrono::Weekday) -> Self {
        use chrono::Weekday as ChronoWd;
        match wd {
            ChronoWd::Mon => Weekday::Monday,
            ChronoWd::Tue => Weekday::Tuesday,
            ChronoWd::Wed => Weekday::Wednesday,
            ChronoWd::Thu => Weekday::Thursday,
            ChronoWd::Fri => Weekday::Friday,
            ChronoWd::Sat => Weekday::Saturday,
            ChronoWd::Sun => Weekday::Sunday,
        }
    }

    /// Convert to chrono::Weekday
    pub fn to_chrono(self) -> chrono::Weekday {
        use chrono::Weekday as ChronoWd;
        match self {
            Weekday::Monday => ChronoWd::Mon,
            Weekday::Tuesday => ChronoWd::Tue,
            Weekday::Wednesday => ChronoWd::Wed,
            Weekday::Thursday => ChronoWd::Thu,
            Weekday::Friday => ChronoWd::Fri,
            Weekday::Saturday => ChronoWd::Sat,
            Weekday::Sunday => ChronoWd::Sun,
        }
    }
}

/// Scheduler manages time-based rules for controlling download behavior
///
/// The Scheduler maintains a list of schedule rules and provides methods
/// to evaluate which action should be active at any given time.
#[derive(Clone, Debug)]
pub struct Scheduler {
    /// List of schedule rules (order matters - first match wins)
    rules: Vec<ScheduleRule>,
}

impl Scheduler {
    /// Create a new Scheduler with the given rules
    ///
    /// Rules are evaluated in order - the first matching enabled rule wins.
    /// For best results, order rules from most specific to least specific.
    ///
    /// # Example
    ///
    /// ```rust
    /// use usenet_dl::scheduler::{Scheduler, ScheduleRule, ScheduleAction, RuleId, Weekday};
    /// use chrono::NaiveTime;
    ///
    /// let rules = vec![
    ///     ScheduleRule {
    ///         id: RuleId::new(1),
    ///         name: "Work hours".into(),
    ///         days: vec![Weekday::Monday, Weekday::Tuesday, Weekday::Wednesday,
    ///                    Weekday::Thursday, Weekday::Friday],
    ///         start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
    ///         end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
    ///         action: ScheduleAction::SpeedLimit(1_000_000),
    ///         enabled: true,
    ///     },
    /// ];
    ///
    /// let scheduler = Scheduler::new(rules);
    /// ```
    pub fn new(rules: Vec<ScheduleRule>) -> Self {
        Self { rules }
    }

    /// Get the list of all rules
    pub fn rules(&self) -> &[ScheduleRule] {
        &self.rules
    }

    /// Update the list of rules
    ///
    /// This replaces all existing rules with the new list.
    pub fn set_rules(&mut self, rules: Vec<ScheduleRule>) {
        self.rules = rules;
    }

    /// Add a new rule to the scheduler
    ///
    /// The rule is added to the end of the list (lowest priority).
    pub fn add_rule(&mut self, rule: ScheduleRule) {
        self.rules.push(rule);
    }

    /// Remove a rule by ID
    ///
    /// Returns true if a rule was removed, false if no rule with that ID exists.
    pub fn remove_rule(&mut self, id: RuleId) -> bool {
        let original_len = self.rules.len();
        self.rules.retain(|r| r.id != id);
        self.rules.len() < original_len
    }

    /// Update an existing rule
    ///
    /// Returns true if the rule was found and updated, false otherwise.
    pub fn update_rule(&mut self, rule: ScheduleRule) -> bool {
        if let Some(existing) = self.rules.iter_mut().find(|r| r.id == rule.id) {
            *existing = rule;
            true
        } else {
            false
        }
    }

    /// Get the current effective action based on the current time
    ///
    /// Evaluates all rules and returns the action of the first matching rule.
    /// Returns None if no rules match the current time.
    ///
    /// Rules are evaluated in order:
    /// 1. Rule must be enabled
    /// 2. Rule must match the current day (empty days = all days)
    /// 3. Current time must be >= start_time and < end_time
    /// 4. First matching rule wins
    ///
    /// # Example
    ///
    /// ```rust
    /// use usenet_dl::scheduler::{Scheduler, ScheduleRule, ScheduleAction, RuleId, Weekday};
    /// use chrono::{NaiveTime, Local};
    ///
    /// let rules = vec![
    ///     ScheduleRule {
    ///         id: RuleId::new(1),
    ///         name: "Work hours".into(),
    ///         days: vec![Weekday::Monday, Weekday::Tuesday, Weekday::Wednesday,
    ///                    Weekday::Thursday, Weekday::Friday],
    ///         start_time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
    ///         end_time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
    ///         action: ScheduleAction::SpeedLimit(1_000_000),
    ///         enabled: true,
    ///     },
    /// ];
    ///
    /// let scheduler = Scheduler::new(rules);
    /// if let Some(action) = scheduler.get_current_action(Local::now()) {
    ///     // Apply the action
    /// }
    /// ```
    pub fn get_current_action(
        &self,
        now: chrono::DateTime<chrono::Local>,
    ) -> Option<ScheduleAction> {
        let weekday = Weekday::from_chrono(now.weekday());
        let time = now.time();

        self.rules
            .iter()
            .find(|r| {
                if !r.enabled {
                    return false;
                }
                if !r.days.is_empty() && !r.days.contains(&weekday) {
                    return false;
                }
                // Handle midnight-crossing rules (e.g., 22:00 to 06:00)
                if r.start_time <= r.end_time {
                    // Normal case: start < end (same day)
                    time >= r.start_time && time < r.end_time
                } else {
                    // Midnight crossing: start > end (e.g., 22:00 to 06:00)
                    time >= r.start_time || time < r.end_time
                }
            })
            .map(|r| r.action.clone())
    }
}

impl Default for Scheduler {
    /// Create a scheduler with no rules
    fn default() -> Self {
        Self { rules: Vec::new() }
    }
}

/// Serde module for serializing/deserializing NaiveTime as HH:MM:SS strings
mod time_format {
    use chrono::NaiveTime;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(time: &NaiveTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = time.format("%H:%M:%S").to_string();
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<NaiveTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        NaiveTime::parse_from_str(&s, "%H:%M:%S").map_err(serde::de::Error::custom)
    }
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests;
