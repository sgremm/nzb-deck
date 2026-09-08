use tracing::debug;

/// Password list collector for archive extraction
///
/// Collects passwords from multiple sources in priority order:
/// 1. Cached correct password (from previous successful extraction)
/// 2. Per-download password (user-specified)
/// 3. NZB metadata password (embedded in NZB)
/// 4. Global password file (one password per line)
/// 5. Empty password (optional fallback)
#[derive(Debug)]
pub struct PasswordList {
    passwords: Vec<String>,
}

impl PasswordList {
    /// Collect passwords from all sources, de-duplicated, in priority order
    pub async fn collect(
        cached_correct: Option<&str>,
        download_password: Option<&str>,
        nzb_meta_password: Option<&str>,
        global_file: Option<&std::path::Path>,
        try_empty: bool,
    ) -> Self {
        let mut seen = std::collections::HashSet::new();
        let mut passwords = Vec::new();

        // Add in priority order, skip duplicates
        // Use reference in HashSet to avoid double allocation
        for pw in [cached_correct, download_password, nzb_meta_password]
            .into_iter()
            .flatten()
        {
            if seen.insert(pw) {
                passwords.push(pw.to_string());
            }
        }

        // Add from file - need owned strings for file content
        if let Some(path) = global_file
            && let Ok(file_content) = tokio::fs::read_to_string(path).await
        {
            for line in file_content.lines() {
                let pw = line.trim();
                // For file passwords, we need to check if already seen as &str
                // but the HashSet contains &str from the parameters above
                // We need a different approach - collect file passwords separately
                if !pw.is_empty() && !passwords.iter().any(|p| p == pw) {
                    passwords.push(pw.to_string());
                }
            }
        }

        // Empty password last
        if try_empty && !passwords.iter().any(|p| p.is_empty()) {
            passwords.push(String::new());
        }

        debug!(
            "collected {} unique passwords for extraction",
            passwords.len()
        );

        Self { passwords }
    }

    /// Get an iterator over passwords
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.passwords.iter()
    }

    /// Check if there are any passwords to try
    pub fn is_empty(&self) -> bool {
        self.passwords.is_empty()
    }

    /// Get the number of passwords
    pub fn len(&self) -> usize {
        self.passwords.len()
    }
}
