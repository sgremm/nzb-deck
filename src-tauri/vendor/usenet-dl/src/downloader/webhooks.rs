//! Webhook and script notification handling.

use crate::types::{DownloadId, Event};
use std::path::PathBuf;
use std::sync::Arc;

use super::UsenetDownloader;

/// Parameters for triggering webhooks
pub struct TriggerWebhooksParams {
    /// The webhook event that occurred
    pub event_type: crate::config::WebhookEvent,
    /// The ID of the download
    pub download_id: DownloadId,
    /// The download name
    pub name: String,
    /// Optional category
    pub category: Option<String>,
    /// Current download status as string
    pub status: String,
    /// Optional destination path (for completed downloads)
    pub destination: Option<PathBuf>,
    /// Optional error message (for failed downloads)
    pub error: Option<String>,
}

/// Parameters for triggering scripts
pub struct TriggerScriptsParams {
    /// The script event that occurred
    pub event_type: crate::config::ScriptEvent,
    /// The ID of the download
    pub download_id: DownloadId,
    /// The download name
    pub name: String,
    /// Optional category
    pub category: Option<String>,
    /// Current download status as string
    pub status: String,
    /// Optional destination path (for completed downloads)
    pub destination: Option<PathBuf>,
    /// Optional error message (for failed downloads)
    pub error: Option<String>,
    /// Size in bytes
    pub size_bytes: u64,
}

impl UsenetDownloader {
    /// Trigger webhooks for download events
    ///
    /// This method sends HTTP POST requests to all configured webhooks that are
    /// subscribed to the given event type. Webhooks are executed asynchronously
    /// (fire and forget) to avoid blocking the download pipeline.
    pub(crate) fn trigger_webhooks(&self, params: TriggerWebhooksParams) {
        let TriggerWebhooksParams {
            event_type,
            download_id,
            name,
            category,
            status,
            destination,
            error,
        } = params;
        // Filter to only webhooks that match this event type before cloning
        let matching_webhooks: Vec<_> = self
            .config
            .notifications
            .webhooks
            .iter()
            .filter(|w| w.events.contains(&event_type))
            .cloned()
            .collect();

        // Early return if no webhooks are subscribed
        if matching_webhooks.is_empty() {
            return;
        }

        let event_tx = self.event_tx.clone();

        // Spawn async task to send webhooks (fire and forget)
        tokio::spawn(async move {
            let timestamp = chrono::Utc::now().timestamp();

            // Pre-compute event string once (not per webhook)
            let event_str: &'static str = match event_type {
                crate::config::WebhookEvent::OnComplete => "complete",
                crate::config::WebhookEvent::OnFailed => "failed",
                crate::config::WebhookEvent::OnQueued => "queued",
            };

            // Build shared payload once - use Arc to share across webhooks
            let payload = Arc::new(crate::types::WebhookPayload {
                event: event_str.to_string(),
                download_id,
                name,
                category,
                status,
                destination,
                error,
                timestamp,
            });

            for webhook in matching_webhooks {
                // Build HTTP client for this webhook
                let client = reqwest::Client::new();
                let mut request = client
                    .post(&webhook.url)
                    .json(payload.as_ref())
                    .timeout(webhook.timeout);

                // Add authentication header if configured
                if let Some(auth) = &webhook.auth_header {
                    request = request.header("Authorization", auth);
                }

                // url is moved into the async block, only cloned for error reporting if needed
                let url = webhook.url;
                let timeout = webhook.timeout;
                let result = tokio::time::timeout(timeout, request.send()).await;

                // Handle webhook response
                match result {
                    Ok(Ok(response)) => {
                        if !response.status().is_success() {
                            let error_msg = format!(
                                "Webhook returned status {}: {}",
                                response.status(),
                                response.text().await.unwrap_or_default()
                            );
                            tracing::warn!(url = %url, error = %error_msg, "webhook failed");
                            event_tx
                                .send(Event::WebhookFailed {
                                    url,
                                    error: error_msg,
                                })
                                .ok();
                        } else {
                            tracing::debug!(url = %url, "webhook sent successfully");
                        }
                    }
                    Ok(Err(e)) => {
                        let error_msg = format!("Failed to send webhook: {}", e);
                        tracing::warn!(url = %url, error = %error_msg, "webhook failed");
                        event_tx
                            .send(Event::WebhookFailed {
                                url,
                                error: error_msg,
                            })
                            .ok();
                    }
                    Err(_) => {
                        let error_msg = format!("Webhook timed out after {:?}", timeout);
                        tracing::warn!(url = %url, error = %error_msg, "webhook timeout");
                        event_tx
                            .send(Event::WebhookFailed {
                                url,
                                error: error_msg,
                            })
                            .ok();
                    }
                }
            }
        });
    }

    /// Trigger scripts for download events
    ///
    /// This method executes all configured scripts (both global and category-specific)
    /// that are subscribed to the given event type. Scripts are executed asynchronously
    /// (fire and forget) to avoid blocking the download pipeline.
    ///
    /// # Execution Order
    ///
    /// 1. Category-specific scripts (if download has a category)
    /// 2. Global scripts
    pub(crate) fn trigger_scripts(&self, params: TriggerScriptsParams) {
        let TriggerScriptsParams {
            event_type,
            download_id,
            name,
            category,
            status,
            destination,
            error,
            size_bytes,
        } = params;
        use std::collections::HashMap;

        // Build environment variables
        let mut env_vars: HashMap<String, String> = HashMap::new();
        env_vars.insert("USENET_DL_ID".to_string(), download_id.to_string());
        env_vars.insert("USENET_DL_NAME".to_string(), name.clone());
        env_vars.insert("USENET_DL_STATUS".to_string(), status.clone());
        env_vars.insert("USENET_DL_SIZE".to_string(), size_bytes.to_string());

        if let Some(cat) = &category {
            env_vars.insert("USENET_DL_CATEGORY".to_string(), cat.clone());
        }

        if let Some(dest) = &destination {
            env_vars.insert(
                "USENET_DL_DESTINATION".to_string(),
                dest.display().to_string(),
            );
        }

        if let Some(err) = &error {
            env_vars.insert("USENET_DL_ERROR".to_string(), err.clone());
        }

        // Category scripts first
        if let Some(cat_name) = &category
            && let Some(cat_config) = self.config.persistence.categories.get(cat_name)
        {
            // Check if any category scripts match this event before cloning
            let matching_scripts: Vec<_> = cat_config
                .scripts
                .iter()
                .filter(|s| s.events.contains(&event_type))
                .collect();

            if !matching_scripts.is_empty() {
                // Only clone env_vars if we have matching scripts
                let mut cat_env_vars = env_vars.clone();
                cat_env_vars.insert(
                    "USENET_DL_CATEGORY_DESTINATION".to_string(),
                    cat_config.destination.display().to_string(),
                );
                cat_env_vars.insert(
                    "USENET_DL_IS_CATEGORY_SCRIPT".to_string(),
                    "true".to_string(),
                );

                for script in matching_scripts {
                    self.run_script_async(&script.path, script.timeout, &cat_env_vars);
                }
            }
        }

        // Then global scripts - only clone for matching scripts
        let matching_global: Vec<_> = self
            .config
            .notifications
            .scripts
            .iter()
            .filter(|s| s.events.contains(&event_type))
            .collect();

        for script in matching_global {
            self.run_script_async(&script.path, script.timeout, &env_vars);
        }
    }

    /// Execute a script asynchronously (fire and forget)
    ///
    /// This method spawns a tokio task to execute the script with the given
    /// environment variables and timeout. It emits a ScriptFailed event if the
    /// script fails or times out.
    fn run_script_async(
        &self,
        script_path: &std::path::Path,
        timeout: std::time::Duration,
        env_vars: &std::collections::HashMap<String, String>,
    ) {
        let script_path = script_path.to_path_buf();
        let event_tx = self.event_tx.clone();
        let env_vars = env_vars.clone();

        tokio::spawn(async move {
            // Execute the script with timeout
            let result = tokio::time::timeout(
                timeout,
                tokio::process::Command::new(&script_path)
                    .envs(&env_vars)
                    .output(),
            )
            .await;

            // Handle script execution result
            match result {
                Ok(Ok(output)) => {
                    if !output.status.success() {
                        let exit_code = output.status.code();
                        tracing::warn!(
                            script = ?script_path,
                            code = ?exit_code,
                            "notification script failed"
                        );
                        event_tx
                            .send(Event::ScriptFailed {
                                script: script_path.clone(),
                                exit_code,
                            })
                            .ok();
                    } else {
                        tracing::debug!(script = ?script_path, "script executed successfully");
                    }
                }
                Ok(Err(e)) => {
                    tracing::warn!(script = ?script_path, error = %e, "failed to run script");
                    event_tx
                        .send(Event::ScriptFailed {
                            script: script_path.clone(),
                            exit_code: None,
                        })
                        .ok();
                }
                Err(_) => {
                    tracing::warn!(script = ?script_path, timeout = ?timeout, "script timed out");
                    event_tx
                        .send(Event::ScriptFailed {
                            script: script_path.clone(),
                            exit_code: None,
                        })
                        .ok();
                }
            }
        });
    }
}
