//! Regression tests for the block-sync exec path ordering.
//!
//! A single-block epoch — the terminal block is also the epoch's first block,
//! as produced when the checkpoint size policy seals an epoch immediately —
//! must persist the terminal block's output before epoch finalization runs:
//! the epoch commitment is stamped onto the indexing row that persisting a
//! block of the epoch creates, and in a single-block epoch no earlier block
//! has created that row.

use std::{collections::HashMap, sync::Mutex};

use strata_acct_types::BitcoinAmount;
use strata_asm_common::AsmManifest;
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_bridge_params::BridgeParams;
use strata_checkpoint_types::EpochSummary;
use strata_db_types::errors::DbError;
use strata_identifiers::{
    Epoch, EpochCommitment, L1BlockCommitment, OLBlockCommitment, OLBlockId, SubjectId,
};
use strata_ol_chain_types::{OLBlock, OLBlockHeader};
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};
use strata_ol_stf::test_utils::{
    EPOCH_RUNNER_TERMINAL_L1_HEIGHT as TERMINAL_L1_HEIGHT, epoch_runner_run_genesis as run_genesis,
    epoch_runner_run_terminal as run_terminal, epoch_runner_seed_accounts as seed_accounts,
    make_deposit_manifest_for_account, make_genesis_state,
};

use crate::{
    WorkerError, WorkerResult, output::OLBlockExecutionOutput, state::exec_block,
    traits::ChainWorkerContext,
};

/// A [`ChainWorkerContext`] that enforces the OL state indexing DB's write
/// ordering contract: stamping an epoch commitment ([`Self::store_summary`])
/// errors unless some block of that epoch had its indexing writes applied
/// first ([`Self::store_block_output`]), mirroring `set_epoch_commitment` on
/// the sled-backed store.
struct OrderEnforcingContext {
    /// Blocks served to [`ChainWorkerContext::fetch_block`].
    blocks: HashMap<OLBlockId, OLBlock>,
    /// Headers served to [`ChainWorkerContext::fetch_header`].
    headers: HashMap<OLBlockId, OLBlockHeader>,
    /// States served to [`ChainWorkerContext::fetch_ol_state`].
    states: HashMap<OLBlockCommitment, OLState>,
    /// Canonical summaries served per epoch index.
    canonical_summaries: HashMap<Epoch, EpochSummary>,
    /// Epochs with at least one block's indexing writes applied.
    indexed_epochs: Mutex<Vec<Epoch>>,
    /// Summaries accepted by [`ChainWorkerContext::store_summary`].
    stored_summaries: Mutex<Vec<EpochSummary>>,
    /// Epochs passed to [`ChainWorkerContext::merge_epoch_data`].
    merged_epochs: Mutex<Vec<EpochCommitment>>,
}

impl ChainWorkerContext for OrderEnforcingContext {
    fn fetch_block(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlock>> {
        Ok(self.blocks.get(blkid).cloned())
    }

    fn fetch_header(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockHeader>> {
        Ok(self.headers.get(blkid).cloned())
    }

    fn fetch_ol_state(&self, commitment: OLBlockCommitment) -> WorkerResult<Option<OLState>> {
        Ok(self.states.get(&commitment).cloned())
    }

    fn fetch_canonical_epoch_summary_at(&self, epoch: Epoch) -> WorkerResult<Option<EpochSummary>> {
        Ok(self.canonical_summaries.get(&epoch).cloned())
    }

    fn store_block_output(
        &self,
        block: &OLBlock,
        _commitment: OLBlockCommitment,
        _output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        self.indexed_epochs
            .lock()
            .unwrap()
            .push(block.header().epoch());
        Ok(())
    }

    fn store_toplevel_state(
        &self,
        _commitment: OLBlockCommitment,
        _state: OLState,
    ) -> WorkerResult<()> {
        Ok(())
    }

    fn store_terminal_header(&self, _id: OLBlockId, _header: OLBlockHeader) -> WorkerResult<()> {
        unimplemented!("not used by exec_block")
    }

    fn store_summary(&self, summary: EpochSummary) -> WorkerResult<()> {
        let commitment = summary.get_epoch_commitment();
        let epoch = commitment.epoch();
        if !self.indexed_epochs.lock().unwrap().contains(&epoch) {
            return Err(WorkerError::Database(DbError::Other(format!(
                "no epoch indexing data for epoch {epoch}"
            ))));
        }
        // Pins the intended ordering rather than a DB contract: the summary
        // must be the last durable step, after the epoch data merge, so a
        // merge failure can never leave a summary behind for a block that
        // then gets rejected.
        if !self.merged_epochs.lock().unwrap().contains(&commitment) {
            return Err(WorkerError::Database(DbError::Other(format!(
                "summary stored before epoch data merge for epoch {epoch}"
            ))));
        }
        self.stored_summaries.lock().unwrap().push(summary);
        Ok(())
    }

    fn merge_epoch_data(&self, summary: &EpochSummary) -> WorkerResult<()> {
        self.merged_epochs
            .lock()
            .unwrap()
            .push(summary.get_epoch_commitment());
        Ok(())
    }

    // Methods below are not exercised by the block exec path.

    fn fetch_blocks_at_slot(&self, _slot: u64) -> WorkerResult<Vec<OLBlockId>> {
        unimplemented!("not used by exec_block")
    }

    fn fetch_chain_tip(&self) -> WorkerResult<Option<OLBlockCommitment>> {
        unimplemented!("not used by exec_block")
    }

    fn fetch_write_batch(
        &self,
        _commitment: OLBlockCommitment,
    ) -> WorkerResult<Option<WriteBatch<OLAccountState>>> {
        unimplemented!("not used by exec_block")
    }

    fn prefill_l1_block_refs_mmr(&self) -> WorkerResult<()> {
        unimplemented!("not used by exec_block")
    }

    fn fetch_checkpoint_payload(
        &self,
        _epoch: &EpochCommitment,
    ) -> WorkerResult<Option<CheckpointPayload>> {
        unimplemented!("not used by exec_block")
    }

    fn fetch_l1_manifests(&self, _from: u32, _to: u32) -> WorkerResult<Vec<AsmManifest>> {
        unimplemented!("not used by exec_block")
    }

    fn apply_epoch_indexing(
        &self,
        _epoch: &EpochCommitment,
        _output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        unimplemented!("not used by exec_block")
    }
}

/// Executing a terminal block that is also its epoch's first block must
/// succeed: the block's own indexing persist creates the epoch row that epoch
/// finalization stamps.
#[test]
fn test_exec_single_block_epoch_persists_before_summary() {
    let mut state = make_genesis_state();
    let snark_serial = seed_accounts(&mut state);
    let genesis = run_genesis(&mut state);
    let genesis_header = genesis.header().clone();
    let pre_epoch_state = state.clone().into_inner();

    // Build epoch 1 as a single terminal block directly on genesis.
    let mut blocks: Vec<OLBlock> = Vec::new();
    let manifest = make_deposit_manifest_for_account(
        TERMINAL_L1_HEIGHT,
        0,
        snark_serial,
        SubjectId::from([42u8; 32]),
        BitcoinAmount::from_sat(150_000_000),
    );
    run_terminal(&mut state, &mut blocks, &genesis_header, manifest);
    let terminal_block = blocks.pop().expect("terminal block built");
    let terminal_header = terminal_block.header().clone();
    let terminal_commitment =
        OLBlockCommitment::new(terminal_header.slot(), terminal_header.compute_blkid());
    assert!(terminal_header.is_terminal(), "epoch 1 block is terminal");
    assert_eq!(terminal_header.epoch(), 1, "single-block epoch 1");

    // Genesis (epoch 0) commitment and summary, for `get_prev_terminal`.
    let genesis_commitment =
        OLBlockCommitment::new(genesis_header.slot(), genesis_header.compute_blkid());
    let genesis_epoch_state = pre_epoch_state.epoch_state();
    let genesis_l1 = L1BlockCommitment::new(
        genesis_epoch_state.last_l1_height(),
        *genesis_epoch_state.last_l1_blkid(),
    );
    let genesis_summary = EpochSummary::new(
        0,
        genesis_commitment,
        OLBlockCommitment::null(),
        genesis_l1,
        *genesis_header.state_root(),
    );

    let ctx = OrderEnforcingContext {
        blocks: HashMap::from([(*terminal_commitment.blkid(), terminal_block)]),
        headers: HashMap::from([(*genesis_commitment.blkid(), genesis_header)]),
        states: HashMap::from([(genesis_commitment, pre_epoch_state)]),
        canonical_summaries: HashMap::from([(0, genesis_summary)]),
        indexed_epochs: Mutex::new(Vec::new()),
        stored_summaries: Mutex::new(Vec::new()),
        merged_epochs: Mutex::new(Vec::new()),
    };

    exec_block(&ctx, BridgeParams::default(), &terminal_commitment)
        .expect("single-block epoch executes");

    let summaries = ctx.stored_summaries.lock().unwrap();
    assert_eq!(summaries.len(), 1, "exactly one epoch summary stored");
    let epoch = summaries[0].get_epoch_commitment();
    assert_eq!(epoch.epoch(), 1);
    assert_eq!(epoch.to_block_commitment(), terminal_commitment);
    assert_eq!(
        ctx.merged_epochs.lock().unwrap().as_slice(),
        &[epoch],
        "epoch data merged before the summary was stored"
    );
}
