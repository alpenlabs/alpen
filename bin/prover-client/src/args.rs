use std::{collections::HashMap, fs, path::PathBuf};

use argh::FromArgs;
use serde_json::from_str;
use strata_primitives::{params::RollupParams, proof::ProofZkVm};

use crate::config::ProverConfig;

/// Command-line arguments used to configure the prover-client in both development and production
/// modes.
///
/// Values specified here will override the values in the config file.
#[derive(Debug, FromArgs)]
pub(crate) struct Args {
    /// The JSON-RPC port used when running in development mode.
    #[argh(option, description = "JSON-RPC port")]
    pub rpc_port: Option<usize>,

    /// The base URL for the JSON-RPC endpoint in development mode.
    #[argh(option, description = "base JSON-RPC URL")]
    pub rpc_url: Option<String>,

    /// The directory path for storing databases and related data.
    #[argh(option, short = 'd', description = "datadir path containing databases")]
    pub datadir: PathBuf,

    /// The URL of the Sequencer RPC endpoint.
    ///
    /// Typically in the format `host:port`.
    #[argh(option, description = "sequencer rpc host:port")]
    pub sequencer_rpc: String,

    /// The URL of the Reth RPC endpoint.
    ///
    /// Typically in the format `host:port`.
    #[argh(option, description = "reth rpc host:port")]
    pub reth_rpc: String,

    /// The host address of the bitcoind RPC endpoint.
    ///
    /// Provide the host (and optionally port) for connecting to a running bitcoind instance.
    #[argh(option, description = "bitcoind RPC host")]
    pub bitcoind_url: String,

    /// The username for the bitcoind RPC authentication.
    #[argh(option, description = "bitcoind RPC user")]
    pub bitcoind_user: String,

    /// The password for the bitcoind RPC authentication.
    #[argh(option, description = "bitcoind RPC password")]
    pub bitcoind_password: String,

    /// Max retries for Bitcoin RPC calls.
    #[argh(option, description = "max retries for bitcoin RPC")]
    pub bitcoin_retry_count: Option<u8>,

    /// Timeout duration for btc request retries in ms.
    #[argh(option, description = "max interval between bitcoin RPC retries in ms")]
    pub bitcoin_retry_interval: Option<u64>,

    /// Path to the custom rollup configuration file.
    #[argh(option, short = 'p', description = "custom rollup config path")]
    pub rollup_params: PathBuf,

    /// The number of Risc0 prover workers to spawn.
    ///
    /// This setting is only available if the `risc0` feature is enabled.
    #[cfg(feature = "risc0")]
    #[argh(option, description = "number of risc0 prover workers to spawn")]
    pub risc0_workers: Option<usize>,

    /// The number of SP1 prover workers to spawn.
    ///
    /// This setting is only available if the `sp1` feature is enabled.
    #[cfg(feature = "sp1")]
    #[argh(option, description = "number of sp1 prover workers to spawn")]
    pub sp1_workers: Option<usize>,

    /// The number of native prover workers to spawn.
    ///
    /// Overrides the value from the config file if provided.
    #[argh(option, description = "number of native prover workers to spawn")]
    pub native_workers: Option<usize>,

    /// The wait time, in milliseconds, for the prover manager loop.
    ///
    /// This value determines how frequently the prover manager checks for available jobs.
    /// Adjust it to balance responsiveness and resource usage.
    #[argh(
        option,
        description = "wait time in milliseconds for the prover manager loop"
    )]
    pub polling_interval: Option<u64>,

    /// Enables or disables development RPC endpoints.
    ///
    /// Set this to `true` to expose additional RPC endpoints for debugging during development.
    #[argh(option, description = "enable prover client dev rpc")]
    pub enable_dev_rpcs: Option<bool>,

    /// Controls the checkpoint proof runner service.
    ///
    /// When enabled, prover will automatically generate and submit proofs for checkpoints.
    #[argh(option, description = "enable prover client checkpoint runner")]
    pub enable_checkpoint_runner: Option<bool>,
}

impl Args {
    /// Load and merge configuration from TOML file with command-line arguments.
    pub fn resolve_config(&self) -> anyhow::Result<ResolvedConfig> {
        // Load base config from file if provided, otherwise use defaults
        let base_config = if let Some(config_path) = &self.config {
            ProverConfig::from_file(config_path)?
        } else {
            ProverConfig::default()
        };

        // Apply command-line overrides
        let config = ResolvedConfig {
            // RPC configuration with CLI overrides
            rpc_port: self.rpc_port.unwrap_or(base_config.rpc.dev_port),
            rpc_url: self.rpc_url.clone().unwrap_or(base_config.rpc.dev_url),

            // Worker configuration with CLI overrides
            native_workers: self.native_workers.unwrap_or(base_config.workers.native),
            #[cfg(feature = "sp1")]
            sp1_workers: self.sp1_workers.unwrap_or(base_config.workers.sp1),
            #[cfg(feature = "risc0")]
            risc0_workers: self.risc0_workers.unwrap_or(base_config.workers.risc0),

            // Timing configuration with CLI overrides
            polling_interval: self
                .polling_interval
                .unwrap_or(base_config.timing.polling_interval_ms),
            checkpoint_poll_interval: base_config.timing.checkpoint_poll_interval_s,

            // Feature flags with CLI overrides
            enable_dev_rpcs: self
                .enable_dev_rpcs
                .unwrap_or(base_config.features.enable_dev_rpcs),
            enable_checkpoint_runner: self
                .enable_checkpoint_runner
                .unwrap_or(base_config.features.enable_checkpoint_runner),

            // Retry configuration with CLI overrides
            bitcoin_retry_count: self
                .bitcoin_retry_count
                .unwrap_or(base_config.retry.bitcoin_retry_count),
            bitcoin_retry_interval: self
                .bitcoin_retry_interval
                .unwrap_or(base_config.retry.bitcoin_retry_interval_ms),
            max_retry_counter: base_config.retry.max_retry_counter,

            // Pass through non-configurable args
            datadir: self.datadir.clone(),
            sequencer_rpc: self.sequencer_rpc.clone(),
            reth_rpc: self.reth_rpc.clone(),
            bitcoind_url: self.bitcoind_url.clone(),
            bitcoind_user: self.bitcoind_user.clone(),
            bitcoind_password: self.bitcoind_password.clone(),
            rollup_params: self.rollup_params.clone(),
        };

        Ok(config)
    }

    /// Resolves the rollup params file to use, from a path, and validates
    /// it to ensure it passes sanity checks.
    pub fn resolve_and_validate_rollup_params(&self) -> anyhow::Result<RollupParams> {
        let json = fs::read_to_string(&self.rollup_params)?;
        let rollup_params = from_str::<RollupParams>(&json)?;
        rollup_params.check_well_formed()?;
        Ok(rollup_params)
    }
}

/// Resolved configuration that combines TOML config with CLI argument overrides
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Base URL for the JSON-RPC endpoint in development mode.
    pub rpc_url: String,

    /// JSON-RPC port used when running in development mode.
    pub rpc_port: usize,

    /// Number of native prover workers to spawn.
    pub native_workers: usize,

    /// Number of SP1 prover workers to spawn.
    #[cfg(feature = "sp1")]
    pub sp1_workers: usize,

    /// Number of Risc0 prover workers to spawn.
    #[cfg(feature = "risc0")]
    pub risc0_workers: usize,

    /// Wait time in milliseconds for the prover manager loop.
    pub polling_interval: u64,

    /// Checkpoint polling interval in seconds.
    pub checkpoint_poll_interval: u64,

    /// Enables or disables development RPC endpoints.
    pub enable_dev_rpcs: bool,

    /// Controls the checkpoint proof runner service.
    pub enable_checkpoint_runner: bool,

    /// Max retries for Bitcoin RPC calls.
    pub bitcoin_retry_count: u8,

    /// Timeout duration for btc request retries in ms.
    pub bitcoin_retry_interval: u64,

    /// Maximum number of retries for transient failures.
    pub max_retry_counter: u64,

    /// Path to the custom rollup configuration file.
    pub datadir: PathBuf,

    /// URL of the Sequencer RPC endpoint.
    pub sequencer_rpc: String,

    /// URL of the Reth RPC endpoint.
    pub reth_rpc: String,

    /// Host address of the bitcoind RPC endpoint.
    pub bitcoind_url: String,

    /// Username for the bitcoind RPC authentication.
    pub bitcoind_user: String,

    /// Password for the bitcoind RPC authentication.
    pub bitcoind_password: String,

    /// Path to the custom rollup configuration file.
    #[expect(dead_code)] // Part of public API, may be used in future
    pub rollup_params: PathBuf,
}

impl ResolvedConfig {
    /// Constructs the complete development JSON-RPC URL by combining `rpc_url` and `rpc_port`.
    ///
    /// This is used for configuring the client’s RPC interface in development mode.
    pub(crate) fn get_dev_rpc_url(&self) -> String {
        format!("{}:{}", self.rpc_url, self.rpc_port)
    }

    /// Returns the Sequencer RPC URL as a `String`.
    ///
    /// Useful for configuring communication with the Sequencer service.
    pub(crate) fn get_sequencer_rpc_url(&self) -> String {
        self.sequencer_rpc.to_string()
    }

    /// Returns the Reth RPC URL as a `String`.
    ///
    /// Useful for configuring communication with the Reth service.
    pub(crate) fn get_reth_rpc_url(&self) -> String {
        self.reth_rpc.to_string()
    }

    /// Formats and returns the `bitcoind` RPC URL prefixed with `http://`.
    ///
    /// Useful for establishing a connection to the bitcoind RPC endpoint.
    pub(crate) fn get_btc_rpc_url(&self) -> String {
        format!("http://{}", self.bitcoind_url)
    }

    /// Returns a map of proof VMs to the number of workers assigned to each, depending on enabled
    /// features.
    ///
    /// This function populates the `HashMap` based on which features are enabled at compile time.
    /// For example, if the `sp1` or `risc0` features are enabled, corresponding entries will be
    /// included with their configured number of worker threads.
    pub(crate) fn get_workers(&self) -> HashMap<ProofZkVm, usize> {
        let mut workers = HashMap::new();
        workers.insert(ProofZkVm::Native, self.native_workers);

        #[cfg(feature = "sp1")]
        {
            workers.insert(ProofZkVm::SP1, self.sp1_workers);
        }

        #[cfg(feature = "risc0")]
        {
            workers.insert(ProofZkVm::Risc0, self.risc0_workers);
        }

        workers
    }

    /// Resolves the rollup params file to use, from a path, and validates
    /// it to ensure it passes sanity checks.
    pub(crate) fn resolve_and_validate_rollup_params(&self) -> anyhow::Result<RollupParams> {
        let json = fs::read_to_string(&self.rollup_params)?;
        let rollup_params = from_str::<RollupParams>(&json)?;
        rollup_params.check_well_formed()?;
        Ok(rollup_params)
    }
}
