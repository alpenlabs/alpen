//! Configuration for the signer, loaded from a TOML file.

use std::{fmt, net::IpAddr, path::PathBuf};

use serde::Deserialize;

use crate::constants::DEFAULT_POLL_INTERVAL_MS;

const DEFAULT_HEALTH_CHECK_HOST: &str = "0.0.0.0";
const DEFAULT_HEALTH_CHECK_PORT: u16 = 8080;

/// Secret configuration value that redacts itself from debug output.
#[derive(Clone, Deserialize)]
#[serde(transparent)]
pub(crate) struct SecretString(String);

impl SecretString {
    /// Returns the underlying secret value.
    pub(crate) fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(***)")
    }
}

/// Top-level signer configuration.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SignerConfig {
    /// Path to the sequencer root key file (xprv).
    pub(crate) sequencer_key: PathBuf,

    /// WebSocket RPC URL of the sequencer node (e.g. ws://127.0.0.1:9944).
    pub(crate) sequencer_admin_endpoint: String,

    /// Bearer token used to authenticate with the sequencer admin RPC.
    pub(crate) sequencer_admin_bearer_token: SecretString,

    /// Duty poll interval in milliseconds.
    #[serde(default = "default_duty_poll_interval")]
    pub(crate) duty_poll_interval: u64,

    /// Host for the HTTP health check endpoint.
    #[serde(default = "default_health_check_host")]
    pub(crate) health_check_host: String,

    /// Port for the HTTP health check endpoint.
    #[serde(default = "default_health_check_port")]
    pub(crate) health_check_port: u16,

    /// Logging configuration.
    #[serde(default)]
    pub(crate) logging: LoggingConfig,
}

/// Logging configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct LoggingConfig {
    /// Service label appended to the service name (e.g. "prod", "dev").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) service_label: Option<String>,

    /// OpenTelemetry OTLP endpoint URL for distributed tracing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) otlp_url: Option<String>,

    /// Directory path for file-based logging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) log_dir: Option<PathBuf>,

    /// Prefix for log file names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) log_file_prefix: Option<String>,

    /// Use JSON format for logs instead of compact format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) json_format: Option<bool>,

    /// Host for the Prometheus `/metrics` HTTP endpoint.
    ///
    /// Defaults to `127.0.0.1` when `metrics_port` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metrics_host: Option<IpAddr>,

    /// Port for the Prometheus `/metrics` HTTP endpoint. Disabled if not set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metrics_port: Option<u16>,
}

const fn default_duty_poll_interval() -> u64 {
    DEFAULT_POLL_INTERVAL_MS
}

fn default_health_check_host() -> String {
    DEFAULT_HEALTH_CHECK_HOST.to_string()
}

const fn default_health_check_port() -> u16 {
    DEFAULT_HEALTH_CHECK_PORT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signer_config_requires_admin_token() {
        let config = r#"
            sequencer_key = "/tmp/sequencer.key"
            sequencer_admin_endpoint = "ws://127.0.0.1:8434"
        "#;

        assert!(toml::from_str::<SignerConfig>(config).is_err());
    }

    #[test]
    fn test_signer_config_parses_admin_endpoint_and_token() {
        let config = r#"
            sequencer_key = "/tmp/sequencer.key"
            sequencer_admin_endpoint = "ws://127.0.0.1:8434"
            sequencer_admin_bearer_token = "test-token"
        "#;

        let config = toml::from_str::<SignerConfig>(config).unwrap();
        assert_eq!(config.sequencer_admin_endpoint, "ws://127.0.0.1:8434");
        assert_eq!(
            config.sequencer_admin_bearer_token.expose_secret(),
            "test-token"
        );
        assert_eq!(config.health_check_host, DEFAULT_HEALTH_CHECK_HOST);
        assert_eq!(config.health_check_port, DEFAULT_HEALTH_CHECK_PORT);
    }

    #[test]
    fn test_signer_config_parses_health_check_addr() {
        let config = r#"
            sequencer_key = "/tmp/sequencer.key"
            sequencer_admin_endpoint = "ws://127.0.0.1:8434"
            sequencer_admin_bearer_token = "test-token"
            health_check_host = "127.0.0.1"
            health_check_port = 18_080
        "#;

        let config = toml::from_str::<SignerConfig>(config).unwrap();
        assert_eq!(config.health_check_host, "127.0.0.1");
        assert_eq!(config.health_check_port, 18_080);
    }

    #[test]
    fn test_signer_config_parses_metrics_port() {
        let config = r#"
            sequencer_key = "/tmp/sequencer.key"
            sequencer_admin_endpoint = "ws://127.0.0.1:8434"
            sequencer_admin_bearer_token = "test-token"

            [logging]
            metrics_host = "0.0.0.0"
            metrics_port = 9615
        "#;

        let config = toml::from_str::<SignerConfig>(config).unwrap();
        assert_eq!(
            config.logging.metrics_host,
            Some(IpAddr::from([0, 0, 0, 0]))
        );
        assert_eq!(config.logging.metrics_port, Some(9615));
    }

    #[test]
    fn test_signer_config_admin_token_debug_redacts_secret() {
        let config = r#"
            sequencer_key = "/tmp/sequencer.key"
            sequencer_admin_endpoint = "ws://127.0.0.1:8434"
            sequencer_admin_bearer_token = "test-token"
        "#;

        let config = toml::from_str::<SignerConfig>(config).unwrap();
        assert!(!format!("{config:?}").contains("test-token"));
        assert!(format!("{config:?}").contains("SecretString(***)"));
    }
}
