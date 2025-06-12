//! Configuration for the prover client.
//!
//! This module contains the configuration for the prover client, including the RPC configuration,
//! worker configuration, timing configuration, retry configuration, and feature flags.
//!
//! The configuration is loaded from a TOML file, and can be overridden by command-line arguments.
//!
//! The configuration is used to configure the prover client, including the RPC configuration,
//! worker configuration, timing configuration, retry configuration, and feature flags.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Default development RPC host to listen on.
const DEFAULT_DEV_RPC_HOST: &str = "0.0.0.0";

/// Default development RPC port to listen on.
const DEFAULT_DEV_RPC_PORT: usize = 4844;

/// Default number of workers for each proving backend.
const DEFAULT_WORKERS: usize = 20;

/// Default polling interval for the prover manager loop.
const DEFAULT_POLLING_INTERVAL_MS: u64 = 1_000;

/// Default checkpoint polling interval in seconds.
const DEFAULT_CHECKPOINT_POLL_INTERVAL_S: u64 = 10;

/// Default maximum number of retries for transient failures.
const DEFAULT_MAX_RETRY_COUNTER: u64 = 15;

/// Default number of retries for Bitcoin RPC calls.
const DEFAULT_BITCOIN_RETRY_COUNT: u8 = 3;

/// Default Bitcoin RPC retry interval in milliseconds.
const DEFAULT_BITCOIN_RETRY_INTERVAL_MS: u64 = 1_000;

/// Prover client configuration loaded from TOML file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct ProverConfig {
    /// RPC configuration
    #[serde(default)]
    pub(crate) rpc: RpcConfig,

    /// Worker configuration for different proving backends.
    #[serde(default)]
    pub(crate) workers: WorkerConfig,

    /// Polling and timing configuration.
    #[serde(default)]
    pub(crate) timing: TimingConfig,

    /// Retry policy configuration.
    #[serde(default)]
    pub(crate) retry: RetryConfig,

    /// Feature flags.  
    #[serde(default)]
    pub(crate) features: FeatureConfig,
}

/// RPC configuration for the prover client.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct RpcConfig {
    /// The JSON-RPC port for development mode.
    #[serde(default = "default_values::dev_rpc_port")]
    pub dev_port: usize,

    /// The base URL for JSON-RPC endpoint in development mode
    #[serde(default = "default_values::dev_rpc_url")]
    pub dev_url: String,
}

/// Worker configuration for the prover client.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct WorkerConfig {
    /// Number of native prover workers.
    #[serde(default = "default_values::workers")]
    pub(crate) native: usize,

    /// Number of SP1 prover workers.
    #[serde(default = "default_values::workers")]
    pub(crate) sp1: usize,

    /// Number of Risc0 prover workers.
    #[serde(default = "default_values::workers")]
    pub(crate) risc0: usize,
}

/// Timing configuration for the prover client.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct TimingConfig {
    /// Polling interval for prover manager loop in milliseconds.
    #[serde(default = "default_values::polling_interval_ms")]
    pub(crate) polling_interval_ms: u64,

    /// Checkpoint polling interval in seconds.
    #[serde(default = "default_values::checkpoint_poll_interval_s")]
    pub(crate) checkpoint_poll_interval_s: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct RetryConfig {
    /// Maximum number of retries for transient failures.
    #[serde(default = "default_values::max_retry_counter")]
    pub(crate) max_retry_counter: u64,

    /// Default number of Bitcoin RPC retries.
    #[serde(default = "default_values::bitcoin_retry_count")]
    pub(crate) bitcoin_retry_count: u8,

    /// Default Bitcoin RPC retry interval in milliseconds.
    #[serde(default = "default_values::bitcoin_retry_interval_ms")]
    pub(crate) bitcoin_retry_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct FeatureConfig {
    /// Enable development RPC endpoints.
    #[serde(default = "default_values::enable_dev_rpcs")]
    pub(crate) enable_dev_rpcs: bool,

    /// Enable checkpoint proof runner.
    #[serde(default = "default_values::enable_checkpoint_runner")]
    pub(crate) enable_checkpoint_runner: bool,
}

/// Default value functions to make [`serde`] happy and make the [`super`] code mess easy to read.
mod default_values {
    use super::*;

    pub(super) fn dev_rpc_port() -> usize {
        DEFAULT_DEV_RPC_PORT
    }

    pub(super) fn dev_rpc_url() -> String {
        DEFAULT_DEV_RPC_HOST.to_string()
    }

    pub(super) fn workers() -> usize {
        DEFAULT_WORKERS
    }

    pub(super) fn polling_interval_ms() -> u64 {
        DEFAULT_POLLING_INTERVAL_MS
    }

    pub(super) fn checkpoint_poll_interval_s() -> u64 {
        DEFAULT_CHECKPOINT_POLL_INTERVAL_S
    }

    pub(super) fn max_retry_counter() -> u64 {
        DEFAULT_MAX_RETRY_COUNTER
    }

    pub(super) fn bitcoin_retry_count() -> u8 {
        DEFAULT_BITCOIN_RETRY_COUNT
    }

    pub(super) fn bitcoin_retry_interval_ms() -> u64 {
        DEFAULT_BITCOIN_RETRY_INTERVAL_MS
    }

    pub(super) fn enable_dev_rpcs() -> bool {
        true
    }

    pub(super) fn enable_checkpoint_runner() -> bool {
        false
    }
}

impl ProverConfig {
    /// Loads configuration from a TOML file.
    pub(crate) fn from_file(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config = toml::from_str(&content)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_config_roundtrip_serialization() {
        // Sample config content as string - matches prover-client.sample.toml
        let sample_config_toml = r#"
# Prover Client Configuration
# This file contains all configurable parameters for the prover client

[rpc]
# RPC server configuration for development mode
dev_port = 4844
dev_url = "0.0.0.0"

[workers]
# Number of worker threads for different proving backends
# Adjust these values based on your hardware capabilities and workload
native = 20
sp1 = 20
risc0 = 20

[timing]
# Polling and timing configuration (in milliseconds and seconds)
polling_interval_ms = 1000      # How often the prover manager checks for new tasks
checkpoint_poll_interval_s = 10 # How often to check for new checkpoints

[retry]
# Retry policy configuration
max_retry_counter = 15           # Maximum retries for transient failures
bitcoin_retry_count = 3          # Default Bitcoin RPC retry count
bitcoin_retry_interval_ms = 1000 # Bitcoin RPC retry interval in milliseconds

[features]
# Feature flags to enable/disable functionality
enable_dev_rpcs = true           # Enable development RPC endpoints
enable_checkpoint_runner = false # Enable automatic checkpoint proving
"#;

        // Deserialize the sample config
        let original_config: ProverConfig =
            toml::from_str(sample_config_toml).expect("Failed to deserialize sample config");

        // Serialize it back to TOML
        let serialized_toml =
            toml::to_string(&original_config).expect("Failed to serialize config back to TOML");

        // Deserialize the serialized TOML again
        let roundtrip_config: ProverConfig =
            toml::from_str(&serialized_toml).expect("Failed to deserialize roundtrip config");

        // Compare original and roundtrip configs - they should be identical
        assert_eq!(
            format!("{original_config:?}"),
            format!("{roundtrip_config:?}"),
            "Roundtrip serialization failed: configs differ"
        );

        // Verify the values match expected defaults
        assert_eq!(original_config.rpc.dev_port, DEFAULT_DEV_RPC_PORT);
        assert_eq!(original_config.rpc.dev_url, DEFAULT_DEV_RPC_HOST);
        assert_eq!(original_config.workers.native, DEFAULT_WORKERS);
        assert_eq!(original_config.workers.sp1, DEFAULT_WORKERS);
        assert_eq!(original_config.workers.risc0, DEFAULT_WORKERS);
        assert_eq!(
            original_config.timing.polling_interval_ms,
            DEFAULT_POLLING_INTERVAL_MS
        );
        assert_eq!(
            original_config.timing.checkpoint_poll_interval_s,
            DEFAULT_CHECKPOINT_POLL_INTERVAL_S
        );
        assert_eq!(
            original_config.retry.max_retry_counter,
            DEFAULT_MAX_RETRY_COUNTER
        );
        assert_eq!(
            original_config.retry.bitcoin_retry_count,
            DEFAULT_BITCOIN_RETRY_COUNT
        );
        assert_eq!(
            original_config.retry.bitcoin_retry_interval_ms,
            DEFAULT_BITCOIN_RETRY_INTERVAL_MS
        );
        assert!(original_config.features.enable_dev_rpcs);
        assert!(!original_config.features.enable_checkpoint_runner);
    }
}
