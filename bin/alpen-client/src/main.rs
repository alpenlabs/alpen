#![allow(
    unused_crate_dependencies,
    clippy::allow_attributes,
    reason = "tempfile use is feature gated; remove after db consolidation"
)]
//! Reth node for the Alpen codebase.

// mod init_db;
mod db;
mod engine_control;
mod genesis;
mod ol_client;
mod ol_tracker;

use std::sync::Arc;

use alpen_chainspec::{chain_value_parser, AlpenChainSpecParser};
use alpen_ee_common::traits::ol_client::chain_status_checked;
use alpen_ee_config::{AlpenEeConfig, AlpenEeParams};
use alpen_reth_node::{args::AlpenNodeArgs, AlpenEthereumNode};
use clap::Parser;
use ol_client::DummyOlClient;
use reth_chainspec::ChainSpec;
use reth_cli_commands::{launcher::FnLauncher, node::NodeCommand};
use reth_cli_runner::CliRunner;
use reth_node_builder::{NodeBuilder, WithLaunchContext};
use reth_node_core::args::LogArgs;
use strata_acct_types::AccountId;
use strata_identifiers::{CredRule, OLBlockId};
use tokio::sync::broadcast;
use tracing::info;

use crate::{
    db::init_db_storage,
    engine_control::{create_engine_control_task, AlpenRethExecEngine},
    genesis::ee_genesis_block_info,
    ol_tracker::{init_ol_tracker_state, OlTrackerBuilder},
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
            let datadir = builder.config().datadir().data_dir().to_path_buf();

            let genesis_info = ee_genesis_block_info(&ext.custom_chain);

            let params = AlpenEeParams::new(
                AccountId::new([0; 32]),
                genesis_info.blockhash(),
                genesis_info.stateroot(),
                0,
                OLBlockId::null(), // TODO
            );

            let config = Arc::new(AlpenEeConfig::new(
                params,
                CredRule::Unchecked,
                ext.ol_client_http,
                ext.sequencer_http,
                ext.db_retry_count,
            ));

            let storage: Arc<_> = init_db_storage(&datadir, config.db_retry_count())
                .expect("failed to load alpen database")
                .into();

            // TODO: real ol client
            let ol_client = Arc::new(DummyOlClient::default());

            // TODO: startup consistency check
            let ol_chain_status = builder
                .task_executor()
                .handle()
                .block_on(chain_status_checked(ol_client.as_ref()))
                .expect("cannot fetch OL chain status");

            let ol_tracker_state = builder
                .task_executor()
                .handle()
                .block_on(init_ol_tracker_state(
                    config.clone(),
                    ol_chain_status,
                    storage.clone(),
                ))
                .expect("ol tracker state initialization should not fail");

            let node_builder = builder
                .node(AlpenEthereumNode::new(AlpenNodeArgs::default()))
                .on_node_started(move |node| {
                    let (ol_tracker, ol_tracker_task) = OlTrackerBuilder::new(
                        ol_tracker_state,
                        config.params().clone(),
                        storage,
                        ol_client,
                    )
                    .build();

                    // TODO: p2p head block gossip
                    let (_preconf_tx, preconf_rx) = broadcast::channel(1);

                    let engine_control_task = create_engine_control_task(
                        preconf_rx,
                        ol_tracker.consensus_watcher(),
                        node.provider.clone(),
                        AlpenRethExecEngine::new(node.beacon_engine_handle.clone()),
                    );

                    node.task_executor
                        .spawn_critical("ol_tracker_task", ol_tracker_task);
                    node.task_executor
                        .spawn_critical("engine_control", engine_control_task);

                    // sequencer specific tasks
                    // TODO: block assembly
                    // TODO: batch assembly
                    // TODO: proof generation
                    // TODO: post update to OL

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

    /// Rpc of sequencer's reth node to forward transactions to.
    #[arg(long, required = false)]
    pub sequencer_http: Option<String>,

    /// Rpc of OL node.
    #[arg(long, required = true)]
    pub ol_client_http: String,

    #[arg(long, required = false)]
    pub db_retry_count: Option<u16>,
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
