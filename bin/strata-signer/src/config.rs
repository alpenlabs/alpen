//! Configuration for the signer, loaded from a TOML file.

use std::path::PathBuf;

use serde::Deserialize;

use crate::constants::DEFAULT_POLL_INTERVAL_MS;

/// Top-level signer configuration.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SignerConfig {
    /// Path to the sequencer root key file (xprv).
    pub(crate) sequencer_key: PathBuf,

    /// WebSocket RPC URL of the sequencer node (e.g. ws://127.0.0.1:9944).
    pub(crate) sequencer_endpoint: String,

    /// Duty poll interval in milliseconds.
    #[serde(default = "default_duty_poll_interval")]
    pub(crate) duty_poll_interval: u64,

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
}

fn default_duty_poll_interval() -> u64 {
    DEFAULT_POLL_INTERVAL_MS
}
