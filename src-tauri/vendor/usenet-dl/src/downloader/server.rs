//! Server connectivity testing.

use crate::config::ServerConfig;
use crate::types::{ServerCapabilities, ServerTestResult};

use super::UsenetDownloader;

impl UsenetDownloader {
    /// Test connectivity and authentication for a server configuration
    ///
    /// This verifies that:
    /// 1. The server is reachable (TCP connection succeeds)
    /// 2. NNTP protocol handshake works
    /// 3. Authentication succeeds (if credentials provided)
    /// 4. Server capabilities can be queried
    ///
    /// This is useful for validating server settings before adding them to production.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use usenet_dl::{UsenetDownloader, Config, config::ServerConfig};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = Config::default();
    ///     let downloader = UsenetDownloader::new(config).await?;
    ///
    ///     let server = ServerConfig {
    ///         host: "news.example.com".to_string(),
    ///         port: 563,
    ///         tls: true,
    ///         username: Some("user".to_string()),
    ///         password: Some("pass".to_string()),
    ///         connections: 10,
    ///         priority: 0,
    ///     };
    ///
    ///     let result = downloader.test_server(&server).await;
    ///     if result.success {
    ///         println!("Server test successful! Latency: {:?}", result.latency);
    ///     } else {
    ///         println!("Server test failed: {:?}", result.error);
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn test_server(&self, server: &ServerConfig) -> ServerTestResult {
        let start = std::time::Instant::now();

        // Try to connect to the server and run capabilities check
        let result = async {
            // Create a temporary NNTP client
            let mut client =
                nntp_rs::NntpClient::connect(std::sync::Arc::new(server.clone().into())).await?;

            // Authenticate if credentials provided
            if server.username.is_some() {
                client.authenticate().await?;
            }

            // Get capabilities
            let caps = client.capabilities().await?;

            Ok::<_, nntp_rs::NntpError>(caps)
        }
        .await;

        let latency = start.elapsed();

        match result {
            Ok(caps) => {
                // Convert nntp-rs Capabilities to our ServerCapabilities
                let server_caps = ServerCapabilities {
                    posting_allowed: caps.has("POST") || caps.has("IHAVE"),
                    max_connections: None, // NNTP doesn't standardize this
                    compression: caps.has("COMPRESS") || caps.has("XZVER"),
                };

                ServerTestResult {
                    success: true,
                    latency: Some(latency),
                    error: None,
                    capabilities: Some(server_caps),
                }
            }
            Err(e) => ServerTestResult {
                success: false,
                latency: Some(latency),
                error: Some(e.to_string()),
                capabilities: None,
            },
        }
    }

    /// Test all configured servers
    ///
    /// Runs connectivity tests on all servers in the configuration.
    /// Returns a list of server names and their test results.
    pub async fn test_all_servers(&self) -> Vec<(String, ServerTestResult)> {
        let mut results = Vec::new();
        for server in self.config.servers.iter() {
            let result = self.test_server(server).await;
            results.push((server.host.clone(), result));
        }
        results
    }
}
