//! Reth node for the Alpen codebase.

use std::{sync::Arc, time::Duration};

use alloy_primitives::{Address, B256, U256};
use alloy_rpc_types::{
    engine::{ForkchoiceState, PayloadAttributes},
    Withdrawal,
};
use alloy_rpc_types_engine::{
    payload, ExecutionData, ExecutionPayload, ExecutionPayloadSidecar, ExecutionPayloadV3,
};
use alpen_chainspec::{chain_value_parser, AlpenChainSpecParser};
use alpen_reth_db::rocksdb::WitnessDB;
use alpen_reth_exex::{ProverWitnessGenerator, StateDiffGenerator};
use alpen_reth_node::{
    args::AlpenNodeArgs,
    payload::{AlpenBuiltPayload, AlpenPayloadBuilderAttributes},
    AlpenEngineTypes, AlpenEthereumNode, AlpenExecutionPayloadEnvelopeV2,
    AlpenExecutionPayloadEnvelopeV4, AlpenPayloadAttributes,
};
// use alpen_reth_rpc::{AlpenRPC, StrataRpcApiServer};
use clap::Parser;
use reth_chain_state::CanonicalInMemoryState;
use reth_chainspec::ChainSpec;
use reth_cli_commands::{launcher::FnLauncher, node::NodeCommand};
use reth_cli_runner::CliRunner;
use reth_node_builder::{
    BeaconConsensusEngineHandle, BuiltPayload, EngineApiMessageVersion, NodeBuilder, NodeHandle,
    PayloadBuilderAttributes, PayloadTypes, WithLaunchContext,
};
use reth_node_core::args::LogArgs;
use reth_payload_builder::{PayloadBuilderHandle, PayloadId};
use tracing::{debug, info, warn};

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

    command.rpc.http = true;

    if let Err(err) = run(
        command,
        |builder: WithLaunchContext<NodeBuilder<Arc<reth_db::DatabaseEnv>, ChainSpec>>,
         ext: AdditionalConfig| async move {
            let datadir = builder.config().datadir().data_dir().to_path_buf();

            let node_args = AlpenNodeArgs {
                sequencer_http: ext.sequencer_http.clone(),
            };

            let mut node_builder = builder.node(AlpenEthereumNode::new(node_args));

            // let mut extend_rpc = None;

            // if ext.enable_witness_gen || ext.enable_state_diff_gen {
            //     let rbdb = db::open_rocksdb_database(datadir.clone()).expect("open rocksdb");
            //     let db = Arc::new(WitnessDB::new(rbdb));
            //     // Add RPC for querying block witness and state diffs.
            //     extend_rpc.replace(AlpenRPC::new(db.clone()));

            //     // Install Prover Input ExEx and persist to DB
            //     if ext.enable_witness_gen {
            //         let witness_db = db.clone();
            //         node_builder = node_builder.install_exex("prover_input", |ctx| async {
            //             Ok(ProverWitnessGenerator::new(ctx, witness_db).start())
            //         });
            //     }

            //     // Install State Diff ExEx and persist to DB
            //     if ext.enable_state_diff_gen {
            //         let state_diff_db = db.clone();
            //         node_builder = node_builder.install_exex("state_diffs", |ctx| async {
            //             Ok(StateDiffGenerator::new(ctx, state_diff_db).start())
            //         });
            //     }
            // }

            // Note: can only add single hook
            // node_builder = node_builder.extend_rpc_modules(|ctx| {
            //     if let Some(rpc) = extend_rpc {
            //         ctx.modules.merge_configured(rpc.into_rpc())?;
            //     }

            //     Ok(())
            // });

            node_builder = node_builder.on_node_started(|node| {
                let task_executor = node.task_executor.clone();

                task_executor.spawn_critical("block builder", async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));

                    // warn!("start sleep");
                    // tokio::time::sleep(Duration::from_secs(10)).await;
                    // warn!("wake up");

                    let canonical_state = node.provider().canonical_in_memory_state();

                    // dbg!(&canonical_state);

                    let initial_state = ForkchoiceState {
                        head_block_hash: canonical_state.chain_info().best_hash,
                        safe_block_hash: canonical_state
                            .get_safe_num_hash()
                            .map_or(B256::ZERO, |numhash| numhash.hash),
                        finalized_block_hash: canonical_state
                            .get_finalized_num_hash()
                            .map_or(B256::ZERO, |numhash| numhash.hash),
                    };

                    let mut state = initial_state.clone();

                    loop {
                        interval.tick().await;

                        dbg!(&state);

                        let payload_attrs =
                            AlpenPayloadAttributes::new_from_eth(PayloadAttributes {
                                timestamp: now_millis(),
                                // IMPORTANT: post cancun will payload build will fail without
                                // parent_beacon_block_root
                                parent_beacon_block_root: Some(B256::ZERO),
                                prev_randao: B256::ZERO,
                                suggested_fee_recipient: Address::ZERO,
                                withdrawals: Some(vec![]),
                            });

                        let payload_builder_attrs = AlpenPayloadBuilderAttributes::try_new(
                            state.head_block_hash,
                            payload_attrs,
                            0,
                        )
                        .unwrap();

                        let payload_id = node
                            .payload_builder_handle
                            .send_new_payload(payload_builder_attrs)
                            .await
                            .expect("should send payload correctly")
                            .unwrap();

                        dbg!(&payload_id);

                        // wait for payload to build
                        let block = node
                            .payload_builder_handle
                            .resolve_kind(
                                payload_id,
                                reth_node_builder::PayloadKind::WaitForPending,
                            )
                            .await
                            .expect("should resolve payload")
                            .expect("should build payload")
                            .block()
                            .to_owned();

                        let payload_status = node
                            .beacon_engine_handle
                            .new_payload(AlpenEngineTypes::block_to_payload(block))
                            .await
                            .expect("should send payload correctly");

                        dbg!(&payload_status);

                        if !payload_status.is_valid() {
                            warn!("payload status invalid");
                            continue;
                        }

                        let next_state = ForkchoiceState {
                            head_block_hash: payload_status.latest_valid_hash.unwrap(),
                            ..state
                        };

                        let res = node
                            .beacon_engine_handle
                            .fork_choice_updated(next_state, None, EngineApiMessageVersion::V4)
                            .await
                            .expect("fcu should succeed");

                        dbg!(res);

                        state = next_state
                    }
                });

                Ok(())
            });

            let NodeHandle {
                node_exit_future, ..
            } = node_builder.launch().await?;

            node_exit_future.await
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

    #[arg(long, default_value_t = false)]
    pub enable_witness_gen: bool,

    #[arg(long, default_value_t = false)]
    pub enable_state_diff_gen: bool,

    /// Rpc of sequener's reth node to forward transactions to.
    #[arg(long, required = false)]
    pub sequencer_http: Option<String>,
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

use std::time;

/// Returns the current time in milliseconds since UNIX_EPOCH.
fn now_millis() -> u64 {
    time::UNIX_EPOCH.elapsed().unwrap().as_millis() as u64
}

trait DepositIntent {
    // unique identifier for deposit
    fn index(&self) -> u64;
    // deposit to address
    fn address(&self) -> Address;
    // deposit amount in gwei
    fn amount(&self) -> u64;
}

trait EnginePayloadAttributes {
    fn parent(&self) -> B256;
    fn timestamp(&self) -> u64;
    fn deposits(&self) -> &[impl DepositIntent];
}

trait EnginePayload {
    type Payload: Clone;

    fn payload(&self) -> &Self::Payload;
}

enum EnginePayloadError {
    Other,
}

trait ExecutionEngine<TBuildAttrs, TEnginePayload>
where
    TBuildAttrs: EnginePayloadAttributes,
    TEnginePayload: EnginePayload,
{
    async fn build_payload(
        &self,
        build_attrs: TBuildAttrs,
    ) -> Result<TEnginePayload, EnginePayloadError>;
    async fn submit_payload(&self, payload: TEnginePayload) -> Result<(), EnginePayloadError>;
    async fn set_head_blockhash(&self, blockhash: B256) -> Result<(), EnginePayloadError>;
    async fn set_safe_blockhash(&self, blockhash: B256) -> Result<(), EnginePayloadError>;
    async fn set_finalized_blockhash(&self, blockhash: B256) -> Result<(), EnginePayloadError>;
}

impl EnginePayload for AlpenBuiltPayload {
    type Payload = Self;

    fn payload(&self) -> &Self::Payload {
        self
    }
}

struct AlpenDepositData {
    index: u64,
    address: Address,
    amount: u64,
}

impl DepositIntent for AlpenDepositData {
    fn index(&self) -> u64 {
        self.index
    }

    fn address(&self) -> Address {
        self.address
    }

    fn amount(&self) -> u64 {
        self.amount
    }
}

struct AlpenEnginePayloadAttributes {
    parent: B256,
    timestamp: u64,
    deposits: Vec<AlpenDepositData>,
}

impl EnginePayloadAttributes for AlpenEnginePayloadAttributes {
    fn parent(&self) -> B256 {
        self.parent
    }

    fn timestamp(&self) -> u64 {
        self.timestamp
    }

    fn deposits(&self) -> &[impl DepositIntent] {
        &self.deposits
    }
}

struct AlpenRethExecEngine {
    payload_builder_handle: PayloadBuilderHandle<AlpenEngineTypes>,
    beacon_engine_handle: BeaconConsensusEngineHandle<AlpenEngineTypes>,
    canonical_in_memory_state: CanonicalInMemoryState,
}

impl AlpenRethExecEngine {
    fn latest_forkchoice_state(&self) -> ForkchoiceState {
        ForkchoiceState {
            head_block_hash: self.canonical_in_memory_state.chain_info().best_hash,
            safe_block_hash: self
                .canonical_in_memory_state
                .get_safe_num_hash()
                .map_or(B256::ZERO, |numhash| numhash.hash),
            finalized_block_hash: self
                .canonical_in_memory_state
                .get_finalized_num_hash()
                .map_or(B256::ZERO, |numhash| numhash.hash),
        }
    }
}

impl ExecutionEngine<AlpenEnginePayloadAttributes, AlpenBuiltPayload> for AlpenRethExecEngine {
    async fn build_payload(
        &self,
        build_attrs: AlpenEnginePayloadAttributes,
    ) -> Result<AlpenBuiltPayload, EnginePayloadError> {
        let payload_attrs = AlpenPayloadAttributes::new_from_eth(PayloadAttributes {
            timestamp: build_attrs.timestamp(),
            // IMPORTANT: post cancun will payload build will fail without
            // parent_beacon_block_root
            parent_beacon_block_root: Some(B256::ZERO),
            prev_randao: B256::ZERO,
            // TODO: get from config
            suggested_fee_recipient: Address::ZERO,
            withdrawals: Some(
                build_attrs
                    .deposits()
                    .iter()
                    .map(|deposit| Withdrawal {
                        index: deposit.index(),
                        validator_index: 0,
                        address: deposit.address(),
                        amount: deposit.amount(),
                    })
                    .collect(),
            ),
        });

        let payload_builder_attrs =
            AlpenPayloadBuilderAttributes::try_new(build_attrs.parent(), payload_attrs, 0).unwrap();

        let payload_id = self
            .payload_builder_handle
            .send_new_payload(payload_builder_attrs)
            .await
            .expect("should send payload correctly")
            .unwrap();

        let payload = self
            .payload_builder_handle
            .resolve_kind(payload_id, reth_node_builder::PayloadKind::WaitForPending)
            .await
            .expect("should resolve payload")
            .expect("should build payload");

        Ok(payload)
    }

    async fn submit_payload(&self, payload: AlpenBuiltPayload) -> Result<(), EnginePayloadError> {
        let payload_status = self
            .beacon_engine_handle
            .new_payload(AlpenEngineTypes::block_to_payload(
                payload.block().to_owned(),
            ))
            .await
            .expect("should send payload correctly");

        // match payload_status.status {
        //     payload::PayloadStatusEnum::Valid => todo!(),
        //     payload::PayloadStatusEnum::Invalid { validation_error } => todo!(),
        //     payload::PayloadStatusEnum::Syncing => todo!(),
        //     payload::PayloadStatusEnum::Accepted => todo!(),
        // }

        Ok(())
    }

    async fn set_head_blockhash(&self, blockhash: B256) -> Result<(), EnginePayloadError> {
        let mut forkchoice_state = self.latest_forkchoice_state();
        forkchoice_state.head_block_hash = blockhash;

        self.beacon_engine_handle
            .fork_choice_updated(forkchoice_state, None, EngineApiMessageVersion::V4)
            .await
            .unwrap();

        Ok(())
    }

    async fn set_safe_blockhash(&self, blockhash: B256) -> Result<(), EnginePayloadError> {
        let mut forkchoice_state = self.latest_forkchoice_state();
        forkchoice_state.safe_block_hash = blockhash;

        self.beacon_engine_handle
            .fork_choice_updated(forkchoice_state, None, EngineApiMessageVersion::V4)
            .await
            .unwrap();

        Ok(())
    }

    async fn set_finalized_blockhash(&self, blockhash: B256) -> Result<(), EnginePayloadError> {
        let mut forkchoice_state = self.latest_forkchoice_state();
        forkchoice_state.finalized_block_hash = blockhash;

        self.beacon_engine_handle
            .fork_choice_updated(forkchoice_state, None, EngineApiMessageVersion::V4)
            .await
            .unwrap();

        Ok(())
    }
}

trait OlClient {}
