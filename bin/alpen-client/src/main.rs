#![allow(unused_crate_dependencies, reason = "temporary")]
//! Reth node for the Alpen codebase.

// Ensure only one database backend is configured at a time
#[cfg(all(
    feature = "sled",
    feature = "rocksdb",
    not(any(test, debug_assertions))
))]
compile_error!(
    "multiple database backends configured: both 'sled' and 'rocksdb' features are enabled"
);

// mod init_db;
mod config;
mod genesis;

use std::sync::Arc;

use alpen_chainspec::{chain_value_parser, AlpenChainSpecParser};
use alpen_reth_exex::{ProverWitnessGenerator, StateDiffGenerator};
use alpen_reth_node::{args::AlpenNodeArgs, AlpenEthereumNode};
use alpen_reth_rpc::{AlpenRPC, StrataRpcApiServer};
use clap::Parser;
// use init_db::init_witness_db;
use reth_chainspec::ChainSpec;
use reth_cli_commands::{launcher::FnLauncher, node::NodeCommand};
use reth_cli_runner::CliRunner;
use reth_node_builder::{NodeBuilder, WithLaunchContext};
use reth_node_core::args::LogArgs;
use strata_acct_types::AccountId;
use strata_identifiers::CredRule;
use tracing::info;

use crate::{
    config::{AlpenEeConfig, AlpenEeParams},
    genesis::ee_genesis_block_info,
};

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    let mut command = NodeCommand::<AlpenChainSpecParser, AdditionalConfig>::parse();

    // use provided alpen chain spec
    command.chain = command.ext.custom_chain.clone();
    // disable peer discovery
    command.network.discovery.disable_discovery = true;
    // enable engine api v4
    command.engine.accept_execution_requests_hash = true;
    // allow chain fork blocks to be created
    command
        .engine
        .always_process_payload_attributes_on_canonical_head = true;

    if let Err(err) = run(
        command,
        |builder: WithLaunchContext<NodeBuilder<Arc<reth_db::DatabaseEnv>, ChainSpec>>,
         ext: AdditionalConfig| async move {
            // let datadir = builder.config().datadir().data_dir().to_path_buf();

            let genesis_info = ee_genesis_block_info(&ext.custom_chain);

            let params = AlpenEeParams {
                account_id: AccountId::new([0; 32]),
                genesis_blockhash: genesis_info.blockhash,
                genesis_stateroot: genesis_info.stateroot,
            };

            let config = AlpenEeConfig {
                params,
                sequencer_credrule: CredRule::Unchecked,
                ee_sequencer_http: ext.sequencer_http,
                ol_client_http: ext.ol_client_http,
            };

            let node_builder = builder
                .node(AlpenEthereumNode::new(AlpenNodeArgs::default()))
                .on_node_started(|node| {
                    // TODO: ...
                    Ok(())
                });

            let handle = node_builder.launch().await?;
            handle.node_exit_future.await
        },
    ) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}

/// Our custom cli args extension that adds one flag to reth default CLI.
#[derive(Debug, clap::Parser)]
pub struct AdditionalConfig {
    #[command(flatten)]
    pub logs: LogArgs,

    /// The chain this node is running.
    ///
    /// Possible values are either a built-in chain or the path to a chain specification file.
    /// Cannot override existing `chain` arg, so this is a workaround.
    #[arg(
        long,
        value_name = "CHAIN_OR_PATH",
        default_value = "testnet",
        value_parser = chain_value_parser,
        required = false,
    )]
    pub custom_chain: Arc<ChainSpec>,

    /// Rpc of sequener's reth node to forward transactions to.
    #[arg(long, required = false)]
    pub sequencer_http: Option<String>,

    /// Rpc of OL node.
    #[arg(long, required = true)]
    pub ol_client_http: String,
}

/// Run node with logging
/// based on reth::cli::Cli::run
fn run<L>(
    mut command: NodeCommand<AlpenChainSpecParser, AdditionalConfig>,
    launcher: L,
) -> eyre::Result<()>
where
    L: std::ops::AsyncFnOnce(
        WithLaunchContext<NodeBuilder<Arc<reth_db::DatabaseEnv>, ChainSpec>>,
        AdditionalConfig,
    ) -> eyre::Result<()>,
{
    command.ext.logs.log_file_directory = command
        .ext
        .logs
        .log_file_directory
        .join(command.chain.chain.to_string());

    let _guard = command.ext.logs.init_tracing()?;
    info!(target: "reth::cli", cmd = %command.ext.logs.log_file_directory, "Initialized tracing, debug log directory");

    let runner = CliRunner::try_default_runtime()?;
    runner.run_command_until_exit(|ctx| {
        command.execute(
            ctx,
            FnLauncher::new::<AlpenChainSpecParser, AdditionalConfig>(launcher),
        )
    })?;

    Ok(())
}
