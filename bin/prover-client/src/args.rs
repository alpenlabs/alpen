use std::{collections::HashMap, fs, path::PathBuf};

use argh::FromArgs;
use serde_json::from_str;
use strata_primitives::{params::RollupParams, proof::ProofZkVm};

pub(super) const DEV_RPC_PORT: usize = 4844;
pub(super) const DEV_RPC_URL: &str = "0.0.0.0";

/// Command-line arguments used to configure the prover-client in both development and production
/// modes.
#[derive(Debug, FromArgs)]
pub(crate) struct Args {
    /// The JSON-RPC port used when running in development mode.
    ///
    /// This port defaults to `DEV_RPC_PORT` and determines the local endpoint port
    /// where the client’s RPC interface is exposed for debugging.
    #[argh(option, description = "JSON-RPC port", default = "DEV_RPC_PORT")]
    pub rpc_port: usize,

    /// The base URL for the JSON-RPC endpoint in development mode.
    ///
    /// Defaults to `DEV_RPC_URL`. When combined with `rpc_port`, it forms the full
    /// RPC endpoint URL for debugging during development.
    #[argh(
        option,
        description = "base JSON-RPC URL",
        default = "DEV_RPC_URL.to_string()"
    )]
    pub rpc_url: String,

    /// The directory path for storing databases and related data.
    ///
    /// This path determines where the client maintains its persistent state.
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
    #[argh(option, description = "max retries for bitcoin RPC (default: 3)")]
    pub bitcoin_retry_count: Option<u8>,

    /// Timeout duration for btc request retries in ms. Defaults to `1000`.
    #[argh(
        option,
        description = "max interval between bitcoin RPC retries in ms (default: 1000)"
    )]
    pub bitcoin_retry_interval: Option<u64>,

    /// Path to the custom rollup configuration file.
    #[argh(option, short = 'p', description = "custom rollup config path")]
    pub rollup_params: PathBuf,

    /// The number of Risc0 prover workers to spawn.
    ///
    /// This setting is only available if the `risc0` feature is enabled.
    /// Defaults to `20`.
    #[cfg(feature = "risc0")]
    #[argh(
        option,
        description = "number of risc0 prover workers to spawn",
        default = "20"
    )]
    pub risc0_workers: usize,

    /// The number of SP1 prover workers to spawn.
    ///
    /// This setting is only available if the `sp1` feature is enabled.
    /// Defaults to `20` to ensure sufficient prover capacity.
    ///
    /// Rationale: We produce EVM EE and CL STF blocks approximately every 5 seconds, and proof
    /// generation for these blocks in SP1 takes roughly 40 seconds. Over a 40-second period, this
    /// results in the need to generate 16 proofs for 8 EVM EE blocks and 8 CL STF blocks.
    /// Additionally, 2 BTC blocks are generated in the same timeframe, requiring 2 more proofs.
    ///
    /// To handle this workload and account for catching up in cases of backlog, a minimum of 18
    /// workers is required. Setting the default to `20` provides a small buffer to ensure smooth
    /// operation under normal and catch-up scenarios.
    #[cfg(feature = "sp1")]
    #[argh(
        option,
        description = "number of sp1 prover workers to spawn",
        default = "20"
    )]
    pub sp1_workers: usize,

    /// The number of native prover workers to spawn.
    ///
    /// Defaults to `20`.
    #[argh(
        option,
        description = "number of native prover workers to spawn",
        default = "20"
    )]
    pub native_workers: usize,

    /// The wait time, in milliseconds, for the prover manager loop.
    ///
    /// This value determines how frequently the prover manager checks for available jobs.
    /// Adjust it to balance responsiveness and resource usage. The default value is 1,000
    /// milliseconds.
    #[argh(
        option,
        description = "wait time in milliseconds for the prover manager loop",
        default = "1_000"
    )]
    pub polling_interval: u64,

    /// Enables or disables development RPC endpoints.
    ///
    /// Set this to `true` to expose additional RPC endpoints for debugging during development.
    /// Defaults to `true`.
    #[argh(option, description = "enable prover client dev rpc", default = "true")]
    pub enable_dev_rpcs: bool,

    /// Controls the checkpoint proof runner service.
    ///
    /// When enabled, prover will automatically generate and submit proofs for checkpoints.
    /// Defaults to `false`.
    #[argh(
        option,
        description = "enable prover client checkpoint runner",
        default = "false"
    )]
    pub enable_checkpoint_runner: bool,
}

impl Args {
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

    /// Formats and returns the bitcoind RPC URL prefixed with `http://`.
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
