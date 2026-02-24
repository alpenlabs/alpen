//! Service framework integration for ASM.

use std::marker;

use bitcoin::hashes::Hash;
use serde::{Deserialize, Serialize};
use strata_primitives::prelude::*;
use strata_service::{Response, Service, SyncService};
use strata_state::asm_state::AsmState;
use tracing::*;

use crate::{AsmWorkerServiceState, traits::WorkerContext};

/// ASM service implementation using the service framework.
#[derive(Debug)]
pub struct AsmWorkerService<W> {
    _phantom: marker::PhantomData<W>,
}

impl<W: WorkerContext + Send + Sync + 'static> Service for AsmWorkerService<W> {
    type State = AsmWorkerServiceState<W>;
    type Msg = L1BlockCommitment;
    type Status = AsmWorkerStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        AsmWorkerStatus {
            is_initialized: state.initialized,
            cur_block: state.blkid,
            cur_state: state.anchor.clone(),
        }
    }
}

impl<W: WorkerContext + Send + Sync + 'static> SyncService for AsmWorkerService<W> {
    fn on_launch(state: &mut AsmWorkerServiceState<W>) -> anyhow::Result<()> {
        Ok(state.load_latest_or_create_genesis()?)
    }

    // TODO(QQ): add tests.
    fn process_input(
        state: &mut AsmWorkerServiceState<W>,
        incoming_block: &L1BlockCommitment,
    ) -> anyhow::Result<Response> {
        let ctx = &state.context;

        // Handle pre-genesis: if the block is before genesis we don't care about it.
        let genesis_height = state.asm_params.l1_view.blk.height();
        let height = incoming_block.height();
        if height < genesis_height {
            warn!(%height, "ignoring unexpected L1 block before genesis");
            return Ok(Response::Continue);
        }

        // Traverse back the chain of l1 blocks until we find an l1 block which has AnchorState.
        // Remember all the blocks along the way and pass it (in the reverse order) to process.
        let pivot_span = info_span!("asm.pivot_lookup",
            target_height = height.to_consensus_u32(),
            target_block = %incoming_block.blkid()
        );
        let pivot_span_guard = pivot_span.enter();

        let mut skipped_blocks = vec![];
        let mut pivot_block = *incoming_block;
        let mut pivot_anchor = ctx.get_anchor_state(&pivot_block);

        while pivot_anchor.is_err() && pivot_block.height() >= genesis_height {
            let block = ctx.get_l1_block(pivot_block.blkid())?;
            let parent_height = pivot_block.height().to_consensus_u32() - 1;
            let parent_block_id = L1BlockCommitment::from_height_u64(
                parent_height as u64,
                block.header.prev_blockhash.into(),
            )
            .expect("parent height should be valid");

            // Push the unprocessed block.
            skipped_blocks.push((block, pivot_block));

            // Update the loop state.
            pivot_anchor = ctx.get_anchor_state(&parent_block_id);
            pivot_block = parent_block_id;
        }

        // We reached the height before genesis (while traversing), but didn't find genesis state.
        if pivot_block.height() < genesis_height {
            warn!("ASM hasn't found pivot anchor state at genesis.");
            return Ok(Response::ShouldExit);
        }

        // Found pivot anchor state - our starting point.
        info!(%pivot_block,
            skipped_blocks = skipped_blocks.len(),
            "ASM found pivot anchor state"
        );

        // Drop pivot span guard before next phase
        drop(pivot_span_guard);

        // Special handling for genesis block - its anchor state was created during init
        // but its manifest wasn't (because Bitcoin block wasn't available yet).
        // We only store the manifest to L1 (for data consumers) but do NOT append it
        // to the external MMR, since the internal compact MMR in AnchorState starts
        // empty with offset = genesis_height + 1. Appending genesis here would shift
        // all external MMR indices by 1 relative to the internal accumulator.
        // Idempotency: skip if the genesis manifest already exists in the L1 database.
        if pivot_block.height() == genesis_height && !ctx.has_l1_manifest(pivot_block.blkid())? {
            let genesis_span = info_span!("asm.genesis_manifest",
                pivot_height = pivot_block.height().to_consensus_u32(),
                pivot_block = %pivot_block.blkid()
            );
            let _genesis_guard = genesis_span.enter();
            // Fetch the genesis block (should work now since L1 reader processed it)
            let genesis_block = ctx.get_l1_block(pivot_block.blkid())?;

            // Compute wtxids_root and create manifest
            let wtxids_root: strata_primitives::Buf32 = genesis_block
                .witness_root()
                .map(|root| root.as_raw_hash().to_byte_array())
                .unwrap_or_else(|| {
                    genesis_block
                        .header
                        .merkle_root
                        .as_raw_hash()
                        .to_byte_array()
                })
                .into();

            let genesis_manifest = strata_asm_common::AsmManifest::new(
                pivot_block.height_u64(),
                *pivot_block.blkid(),
                wtxids_root.into(),
                vec![], // TODO: this is not supposed to be empty right?
            );

            ctx.store_l1_manifest(genesis_manifest)?;

            info!(%pivot_block, "Created genesis manifest");
        } // genesis_span drops here

        state.update_anchor_state(pivot_anchor.unwrap(), pivot_block);

        // Process the whole chain of unprocessed blocks, starting from older blocks till
        // incoming_block.
        for (block, block_id) in skipped_blocks.iter().rev() {
            let transition_span = info_span!("asm.block_transition",
                height = block_id.height().to_consensus_u32(),
                block_id = %block_id.blkid()
            );
            let _transition_guard = transition_span.enter();

            info!(%block_id, "ASM transition attempt");
            match state.transition(block) {
                Ok((asm_stf_out, aux_data)) => {
                    let storage_span = debug_span!("asm.manifest_storage");
                    let _storage_guard = storage_span.enter();

                    // Extract manifest and compute its hash
                    let manifest = asm_stf_out.manifest.clone();
                    let manifest_hash = manifest.compute_hash();

                    // Store manifest to L1 database (for chaintsn and other consumers)
                    state.context.store_l1_manifest(manifest)?;

                    // Append manifest hash to MMR database
                    let leaf_index = state.context.append_manifest_to_mmr(manifest_hash.into())?;

                    // Store auxiliary data for prover consumption
                    state.context.store_aux_data(block_id, &aux_data)?;

                    let new_state = AsmState::from_output(asm_stf_out);
                    // Store and update anchor.
                    state.context.store_anchor_state(block_id, &new_state)?;
                    state.update_anchor_state(new_state, *block_id);

                    info!(%block_id, %height, leaf_index, "ASM transition complete, manifest and state stored");
                }
                Err(e) => {
                    error!(
                        %e,
                        %block_id,
                        height = block_id.height().to_consensus_u32(),
                        "ASM transition error"
                    );
                    // A single transition failure can be transient (for example, temporary
                    // upstream inconsistency while syncing). Keep worker alive so later
                    // inputs can retry from the last good pivot.
                    warn!(
                        %block_id,
                        "ASM transition failed; deferring block and continuing service"
                    );
                    return Ok(Response::Continue);
                }
            }
            info!(%block_id, "ASM transition success");
        } // transition_span drops here

        Ok(Response::Continue)
    }
}

/// Status information for the ASM worker service.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AsmWorkerStatus {
    pub is_initialized: bool,
    pub cur_block: Option<L1BlockCommitment>,
    pub cur_state: Option<AsmState>,
}

impl AsmWorkerStatus {
    /// Get the logs from the current ASM state.
    ///
    /// Returns an empty slice if the state is not initialized.
    pub fn logs(&self) -> &[strata_asm_common::AsmLogEntry] {
        self.cur_state
            .as_ref()
            .map(|s| s.logs().as_slice())
            .unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use bitcoin::{
        Block, CompactTarget, Network, TxMerkleNode,
        block::{Header, Version},
        hashes::Hash as BitcoinHash,
    };
    use strata_asm_common::AuxData;
    use strata_asm_params::AsmParams;
    use strata_btc_types::BitcoinTxid;
    use strata_primitives::{
        L1BlockId,
        hash::Hash,
        l1::{L1BlockCommitment, RawBitcoinTx},
    };
    use strata_service::{Response, SyncService};
    use strata_state::asm_state::AsmState;
    use strata_test_utils::ArbitraryGenerator;

    use super::AsmWorkerService;
    use crate::{AsmWorkerServiceState, WorkerContext, WorkerError, WorkerResult};

    #[derive(Clone, Default)]
    struct MockWorkerContext {
        blocks: Arc<Mutex<HashMap<L1BlockId, Block>>>,
        asm_states: Arc<Mutex<HashMap<L1BlockCommitment, AsmState>>>,
        latest_asm_state: Arc<Mutex<Option<(L1BlockCommitment, AsmState)>>>,
        forced_missing_anchors: Arc<Mutex<HashSet<L1BlockCommitment>>>,
    }

    impl MockWorkerContext {
        fn insert_block(&self, block: Block) -> L1BlockId {
            let block_id: L1BlockId = block.block_hash().into();
            self.blocks
                .lock()
                .expect("poisoned lock")
                .insert(block_id, block);
            block_id
        }

        fn mark_anchor_missing(&self, blockid: L1BlockCommitment) {
            self.forced_missing_anchors
                .lock()
                .expect("poisoned lock")
                .insert(blockid);
        }
    }

    #[async_trait]
    impl WorkerContext for MockWorkerContext {
        fn get_l1_block(&self, blockid: &L1BlockId) -> WorkerResult<Block> {
            self.blocks
                .lock()
                .expect("poisoned lock")
                .get(blockid)
                .cloned()
                .ok_or(WorkerError::MissingL1Block(*blockid))
        }

        fn get_anchor_state(&self, blockid: &L1BlockCommitment) -> WorkerResult<AsmState> {
            if self
                .forced_missing_anchors
                .lock()
                .expect("poisoned lock")
                .contains(blockid)
            {
                return Err(WorkerError::MissingAsmState(*blockid.blkid()));
            }

            self.asm_states
                .lock()
                .expect("poisoned lock")
                .get(blockid)
                .cloned()
                .or_else(|| {
                    self.latest_asm_state
                        .lock()
                        .expect("poisoned lock")
                        .as_ref()
                        .map(|(_, state)| state.clone())
                })
                .ok_or(WorkerError::MissingAsmState(*blockid.blkid()))
        }

        fn get_latest_asm_state(&self) -> WorkerResult<Option<(L1BlockCommitment, AsmState)>> {
            Ok(self.latest_asm_state.lock().expect("poisoned lock").clone())
        }

        fn store_anchor_state(
            &self,
            blockid: &L1BlockCommitment,
            state: &AsmState,
        ) -> WorkerResult<()> {
            self.asm_states
                .lock()
                .expect("poisoned lock")
                .insert(*blockid, state.clone());
            *self.latest_asm_state.lock().expect("poisoned lock") = Some((*blockid, state.clone()));
            Ok(())
        }

        fn store_l1_manifest(&self, _manifest: strata_asm_common::AsmManifest) -> WorkerResult<()> {
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

        fn store_aux_data(
            &self,
            _blockid: &L1BlockCommitment,
            _data: &AuxData,
        ) -> WorkerResult<()> {
            Ok(())
        }

        fn get_aux_data(&self, _blockid: &L1BlockCommitment) -> WorkerResult<Option<AuxData>> {
            Ok(None)
        }

        fn has_l1_manifest(&self, _blockid: &L1BlockId) -> WorkerResult<bool> {
            Ok(true)
        }
    }

    fn setup_state() -> (AsmWorkerServiceState<MockWorkerContext>, MockWorkerContext) {
        let mut asm_params: AsmParams = ArbitraryGenerator::new().generate();
        let genesis_block_id = *asm_params.l1_view.blk.blkid();
        asm_params.l1_view.blk = L1BlockCommitment::from_height_u64(100, genesis_block_id)
            .expect("valid genesis height");
        let asm_params = Arc::new(asm_params);

        let context = MockWorkerContext::default();
        let mut state = AsmWorkerServiceState::new(context.clone(), asm_params);
        state
            .load_latest_or_create_genesis()
            .expect("genesis state must initialize");
        (state, context)
    }

    #[test]
    fn process_input_keeps_worker_alive_on_transition_error() {
        let (mut state, _) = setup_state();
        let genesis = state.blkid.expect("genesis block must be set");

        // Build a synthetic child block that points to genesis but does not satisfy
        // consensus validation. This should force `state.transition()` to return an error.
        let header = Header {
            version: Version::ONE,
            prev_blockhash: (*genesis.blkid()).into(),
            merkle_root: TxMerkleNode::all_zeros(),
            time: 0,
            bits: CompactTarget::from_consensus(0),
            nonce: 0,
        };
        let invalid_block = Block {
            header,
            txdata: vec![],
        };

        // Ensure the parent commitment derived from the synthetic header is present in the
        // anchor-state map, regardless of any hash-byte-order conversion details.
        let parent_id: L1BlockId = invalid_block.header.prev_blockhash.into();
        let parent_commitment = L1BlockCommitment::from_height_u64(genesis.height_u64(), parent_id)
            .expect("valid parent height");
        if parent_commitment != genesis {
            let anchor_state = state.anchor.clone().expect("anchor state should exist");
            state
                .context
                .store_anchor_state(&parent_commitment, &anchor_state)
                .expect("parent anchor should be insertable");
        }

        let invalid_block_id = state.context.insert_block(invalid_block);
        assert!(
            state.context.get_l1_block(&invalid_block_id).is_ok(),
            "synthetic block should be retrievable from mock context"
        );
        let incoming =
            L1BlockCommitment::from_height_u64(genesis.height_u64() + 1, invalid_block_id)
                .expect("valid incoming height");
        state.context.mark_anchor_missing(incoming);

        let response = <AsmWorkerService<MockWorkerContext> as SyncService>::process_input(
            &mut state, &incoming,
        )
        .expect("service should handle transition error");

        assert!(matches!(response, Response::Continue));
        assert!(state.initialized);
        assert_eq!(state.blkid, Some(genesis));
    }
}
