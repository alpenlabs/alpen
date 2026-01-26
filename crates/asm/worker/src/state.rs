use std::sync::Arc;

use bitcoin::{Block, hashes::Hash};
use strata_asm_common::{AnchorState, AsmHistoryAccumulatorState, ChainViewState};
use strata_asm_spec::StrataAsmSpec;
use strata_asm_stf::{AsmStfInput, AsmStfOutput};
use strata_asm_types::HeaderVerificationState;
use strata_params::RollupParams;
use strata_primitives::{Buf32, l1::L1BlockCommitment};
use strata_service::ServiceState;
use strata_state::asm_state::AsmState;
use tracing::field::Empty;

use crate::{WorkerContext, WorkerError, WorkerResult, aux_resolver::AuxDataResolver, constants};

/// Service state for the ASM worker.
///
/// TODO: additional fields and bookkeeping related to STF version and storing aux inputs.
#[derive(Debug)]
pub struct AsmWorkerServiceState<W> {
    /// Params.
    pub(crate) params: Arc<RollupParams>,

    /// Context for the state to interact with outer world.
    pub(crate) context: W,

    /// Whether the service is initialized.
    pub initialized: bool,

    /// Current ASM state.
    pub anchor: Option<AsmState>,

    /// Current anchor block.
    pub blkid: Option<L1BlockCommitment>,

    /// ASM spec for ASM STF.
    asm_spec: StrataAsmSpec,
}

impl<W: WorkerContext + Send + Sync + 'static> AsmWorkerServiceState<W> {
    /// A new (uninitialized) instance of the service state.
    pub fn new(context: W, params: Arc<RollupParams>) -> Self {
        let asm_spec = StrataAsmSpec::from_params(&params);
        Self {
            params,
            context,
            anchor: None,
            blkid: None,
            initialized: false,
            asm_spec,
        }
    }

    /// Loads and sets the latest anchor state.
    ///
    /// If there are no anchor states yet, creates and stores genesis one beforehand.
    pub fn load_latest_or_create_genesis(&mut self) -> WorkerResult<()> {
        match self.context.get_latest_asm_state()? {
            Some((blkid, state)) => {
                self.update_anchor_state(state, blkid);
                Ok(())
            }
            None => {
                // Create genesis anchor state.
                let genesis_l1_view = &self.params.genesis_l1_view;
                let empty_accumulator =
                    AsmHistoryAccumulatorState::new(genesis_l1_view.height_u64());
                let state = AnchorState {
                    chain_view: ChainViewState {
                        pow_state: HeaderVerificationState::new(
                            self.context.get_network()?,
                            genesis_l1_view,
                        ),
                        history_accumulator: empty_accumulator,
                    },
                    sections: vec![],
                };

                // Persist it and update state.
                let state = AsmState::new(state, vec![]);
                self.context
                    .store_anchor_state(&genesis_l1_view.blk, &state)?;
                self.update_anchor_state(state, genesis_l1_view.blk);

                Ok(())
            }
        }
    }

    /// Returns the actual ASM STF results: a Bitcoin block is applied onto current anchor state.
    ///
    /// A caller is responsible for ensuring the current anchor is a parent of a passed block.
    pub fn transition(&self, block: &Block) -> WorkerResult<AsmStfOutput> {
        let cur_state = self.anchor.as_ref().expect("state should be set before");

        // Pre process transition next block against current anchor state.
        let pre_process = {
            let span = tracing::debug_span!("asm.stf.pre_process", protocol_txs = Empty);
            let _guard = span.enter();

            let result = strata_asm_stf::pre_process_asm(&self.asm_spec, cur_state.state(), block)
                .map_err(WorkerError::AsmError)?;

            span.record("protocol_txs", result.txs.len());
            result
        };

        // Resolve auxiliary data requests from subprotocols
        let aux_data = {
            let span = tracing::debug_span!("asm.stf.aux_resolve");
            let _guard = span.enter();

            let resolver = AuxDataResolver::new(&self.context, self.params.clone());
            resolver.resolve(&pre_process.aux_requests)?
        };

        // For blocks without witness data (pre-SegWit or legacy-only transactions),
        // the witness merkle root equals the transaction merkle root per Bitcoin protocol.
        let wtxids_root: Buf32 = block
            .witness_root()
            .map(|root| root.as_raw_hash().to_byte_array())
            .unwrap_or_else(|| block.header.merkle_root.as_raw_hash().to_byte_array())
            .into();

        let stf_input = AsmStfInput {
            protocol_txs: pre_process.txs,
            header: &block.header,
            wtxids_root,
            aux_data,
        };

        // Asm transition.
        let stf_span = tracing::debug_span!("asm.stf.process");
        let _stf_guard = stf_span.enter();

        strata_asm_stf::compute_asm_transition(&self.asm_spec, cur_state.state(), stf_input)
            .map_err(WorkerError::AsmError)
    }

    /// Updates anchor related bookkeping.
    pub(crate) fn update_anchor_state(&mut self, anchor: AsmState, blkid: L1BlockCommitment) {
        self.initialized = true;
        self.anchor = Some(anchor);
        self.blkid = Some(blkid);
    }
}

impl<W: WorkerContext + Send + Sync + 'static> ServiceState for AsmWorkerServiceState<W> {
    fn name(&self) -> &str {
        constants::SERVICE_NAME
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use async_trait::async_trait;
    use bitcoin::{BlockHash, Network, block::Header, hashes::Hash as _};
    use bitcoind_async_client::{
        Client,
        traits::{Reader, Wallet},
    };
    use corepc_node::Node;
    use strata_asm_common::AsmManifest;
    use strata_btc_types::{BitcoinTxid, RawBitcoinTx};
    use strata_primitives::{Buf32, L1BlockId, hash::Hash, l1::GenesisL1View};
    use strata_test_utils_btcio::{get_bitcoind_and_client, mine_blocks};
    use strata_test_utils_l2::gen_params;

    use super::*;

    struct TestEnv {
        pub _node: Node, // Keep node alive
        pub client: Arc<Client>,
        pub service_state: AsmWorkerServiceState<MockWorkerContext>,
    }

    async fn setup_env() -> TestEnv {
        // 1. Setup Bitcoin Regtest
        let (node, client) = get_bitcoind_and_client();
        let client = Arc::new(client);

        // Mine some initial blocks to have funds and chain height.
        let _ = mine_blocks(&node, &client, 101, None)
            .await
            .expect("Failed to mine initial blocks");

        // Pick the current tip as our "genesis" for the ASM.
        let tip_hash = client.get_block_hash(101).await.unwrap();

        // 2. Setup Params
        let mut params = gen_params().rollup;
        params.network = Network::Regtest;

        // Sync parameters with the actual bitcoind state
        let genesis_view = get_genesis_l1_view(&client, &tip_hash)
            .await
            .expect("Failed to fetch genesis view");
        params.genesis_l1_view = genesis_view;

        let params = Arc::new(params);

        // 3. Set worker context and initialize service state
        let context = MockWorkerContext::new();
        let mut service_state = AsmWorkerServiceState::new(context.clone(), params.clone());

        // Initialize: this should create genesis state based on our `genesis_l1_view`
        service_state
            .load_latest_or_create_genesis()
            .expect("Failed to load/create genesis state");

        assert!(service_state.initialized);
        assert!(service_state.anchor.is_some());

        println!("Service initialized with genesis at height 101");

        TestEnv {
            _node: node,
            client,
            service_state,
        }
    }

    /// Helper to construct `GenesisL1View` from a block hash using the client.
    async fn get_genesis_l1_view(
        client: &Client,
        hash: &BlockHash,
    ) -> anyhow::Result<GenesisL1View> {
        let header: Header = client.get_block_header(hash).await?;
        let height = client.get_block_height(hash).await?;

        // Construct L1BlockCommitment
        let blkid: L1BlockId = Buf32::from(header.block_hash().as_raw_hash().to_byte_array()).into();
        let blk_commitment = L1BlockCommitment::from_height_u64(height, blkid)
            .expect("height should fit u32");

        // Create dummy/default values for other fields
        let next_target = header.bits.to_consensus();
        let epoch_start_timestamp = header.time;
        let last_11_timestamps = [header.time - 1; 11]; // simplified: ensure median < tip time by making history older

        Ok(GenesisL1View {
            blk: blk_commitment,
            next_target,
            epoch_start_timestamp,
            last_11_timestamps, // simplified: ensure median < tip time by making history older
        })
    }

    #[derive(Clone, Default)]
    struct MockWorkerContext {
        pub blocks: Arc<Mutex<HashMap<L1BlockId, Block>>>,
        pub asm_states: Arc<Mutex<HashMap<L1BlockCommitment, AsmState>>>,
        pub latest_asm_state: Arc<Mutex<Option<(L1BlockCommitment, AsmState)>>>,
    }

    impl MockWorkerContext {
        fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl WorkerContext for MockWorkerContext {
        fn get_l1_block(&self, blockid: &L1BlockId) -> WorkerResult<Block> {
            self.blocks
                .lock()
                .unwrap()
                .get(blockid)
                .cloned()
                .ok_or(WorkerError::MissingL1Block(*blockid))
        }

        fn get_anchor_state(&self, blockid: &L1BlockCommitment) -> WorkerResult<AsmState> {
            self.asm_states
                .lock()
                .unwrap()
                .get(blockid)
                .cloned()
                .ok_or(WorkerError::MissingAsmState(*blockid.blkid()))
        }

        fn get_latest_asm_state(&self) -> WorkerResult<Option<(L1BlockCommitment, AsmState)>> {
            Ok(self.latest_asm_state.lock().unwrap().clone())
        }

        fn store_anchor_state(
            &self,
            blockid: &L1BlockCommitment,
            state: &AsmState,
        ) -> WorkerResult<()> {
            self.asm_states
                .lock()
                .unwrap()
                .insert(*blockid, state.clone());
            *self.latest_asm_state.lock().unwrap() = Some((*blockid, state.clone()));
            Ok(())
        }

        fn store_l1_manifest(&self, _manifest: AsmManifest) -> WorkerResult<()> {
            // Mock implementation - no-op for tests
            Ok(())
        }

        fn get_network(&self) -> WorkerResult<Network> {
            Ok(Network::Regtest)
        }

        fn get_bitcoin_tx(&self, _txid: &BitcoinTxid) -> WorkerResult<RawBitcoinTx> {
            Err(WorkerError::Unimplemented)
        }

        fn append_manifest_to_mmr(&self, _manifest_hash: Hash) -> WorkerResult<u64> {
            Ok(0)
        }

        fn generate_mmr_proof(&self, _index: u64) -> WorkerResult<strata_merkle::MerkleProofB32> {
            Err(WorkerError::Unimplemented)
        }

        fn get_manifest_hash(&self, _index: u64) -> WorkerResult<Option<Hash>> {
            Ok(None)
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_asm_transition() {
        // 1. Setup Environment
        let env = setup_env().await;
        let client = env.client;
        let node = env._node;
        let service_state = env.service_state;

        // 2. Create a new block to test transition
        // We mine 1 block on top of tip (which is our genesis).
        let address = client.get_new_address().await.unwrap();
        let new_block_hashes = mine_blocks(&node, &client, 1, Some(address)).await.unwrap();
        let new_block_hash = new_block_hashes[0];

        let new_block = client.get_block(&new_block_hash).await.unwrap();

        println!("Mined new block: {}", new_block_hash);

        // 6. Call Transition
        // The transition function expects the block to be a child of the current anchor.
        // Current anchor is at 101. New block is at 102, parent is 101.
        // This should work.

        let result = service_state.transition(&new_block);

        match result {
            Ok(_output) => {
                println!("Transition successful!");
                // Verify output if needed.
                // Since block is empty (coinbase only), `compute_asm_transition` should return a
                // state that reflects an empty transition or just L1 updates.
                // We mainly care that it didn't error.
            }
            Err(e) => {
                panic!("Transition failed: {:?}", e);
            }
        }
    }
}
