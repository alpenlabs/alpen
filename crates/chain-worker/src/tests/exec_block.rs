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
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_common::AsmManifest;
use strata_checkpoint_types::EpochSummary;
use strata_codec::{decode_buf_exact, encode_to_vec};
use strata_db_types::errors::DbError;
use strata_identifiers::{
    Buf32, Epoch, EpochCommitment, L1BlockCommitment, OLBlockCommitment, OLBlockId, SubjectId,
};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_container::OLStateContainer;
use strata_ol_state_types::{IStateAccessor, IStateAccessorMut, OLSpecId, OLSpecVersions};
use strata_ol_state_types_v1::WriteBatch;
use strata_ol_stf_v1::{
    BlockComponents, BlockInfo,
    test_utils::{
        EPOCH_RUNNER_GENESIS_TIMESTAMP, EPOCH_RUNNER_SLOT_TIMESTAMP_STEP,
        EPOCH_RUNNER_TERMINAL_L1_HEIGHT as TERMINAL_L1_HEIGHT,
        epoch_runner_run_genesis as run_genesis, epoch_runner_run_terminal as run_terminal,
        epoch_runner_seed_accounts as seed_accounts, execute_block,
        make_checkpoint_predicate_enactment_manifest, make_deposit_manifest_for_account,
        tamper_state_root, to_ol_block,
    },
};

use super::fixture::make_marked_genesis_state;
use crate::{
    WorkerError, WorkerResult,
    output::OLBlockExecutionOutput,
    state::{exec_block, merge_epoch_state},
    traits::ChainWorkerContext,
};

/// A [`ChainWorkerContext`] that enforces the OL state indexing DB's write
/// ordering contract: stamping an epoch commitment ([`Self::store_summary`])
/// errors unless some block of that epoch had its indexing writes applied
/// first ([`Self::store_block_output`]), mirroring `set_epoch_commitment` on
/// the sled-backed store.
struct OrderEnforcingContext {
    /// Blocks served to [`ChainWorkerContext::fetch_block`].
    blocks: HashMap<OLBlockId, OLBlockV1>,
    /// Headers served to [`ChainWorkerContext::fetch_header`].
    headers: HashMap<OLBlockId, OLBlockHeaderV1>,
    /// States served to [`ChainWorkerContext::fetch_ol_state`].
    states: HashMap<OLBlockCommitment, OLStateContainer>,
    /// Canonical summaries served per epoch index.
    canonical_summaries: HashMap<Epoch, EpochSummary>,
    /// L1 block genesis anchors to.
    genesis_l1: L1BlockCommitment,
    /// L1 manifests served per height.
    manifests: HashMap<u32, AsmManifest>,
    /// Epochs with at least one block's indexing writes applied.
    indexed_epochs: Mutex<Vec<Epoch>>,
    /// Summaries accepted by [`ChainWorkerContext::store_summary`].
    stored_summaries: Mutex<Vec<EpochSummary>>,
    /// Epochs passed to [`ChainWorkerContext::merge_epoch_data`].
    merged_epochs: Mutex<Vec<EpochCommitment>>,
    /// States accepted by [`ChainWorkerContext::store_toplevel_state`] and
    /// merged by [`ChainWorkerContext::merge_epoch_data`].
    stored_states: Mutex<Vec<(OLBlockCommitment, OLStateContainer)>>,
    /// Write batches accepted by [`ChainWorkerContext::store_block_output`],
    /// encoded with the codec the OL state DB stores them with.
    write_batches: Mutex<HashMap<OLBlockCommitment, Vec<u8>>>,
}

impl OrderEnforcingContext {
    /// Creates a context that serves the given chain data and has stored
    /// nothing yet.
    fn new(
        blocks: HashMap<OLBlockId, OLBlockV1>,
        headers: HashMap<OLBlockId, OLBlockHeaderV1>,
        states: HashMap<OLBlockCommitment, OLStateContainer>,
        canonical_summaries: HashMap<Epoch, EpochSummary>,
        genesis_l1: L1BlockCommitment,
        manifests: HashMap<u32, AsmManifest>,
    ) -> Self {
        Self {
            blocks,
            headers,
            states,
            canonical_summaries,
            genesis_l1,
            manifests,
            indexed_epochs: Mutex::new(Vec::new()),
            stored_summaries: Mutex::new(Vec::new()),
            merged_epochs: Mutex::new(Vec::new()),
            stored_states: Mutex::new(Vec::new()),
            write_batches: Mutex::new(HashMap::new()),
        }
    }
}

impl ChainWorkerContext for OrderEnforcingContext {
    fn runtime_params(&self) -> OLRuntimeParams {
        OLRuntimeParams::test_default()
    }

    fn genesis_l1_block(&self) -> L1BlockCommitment {
        self.genesis_l1
    }

    fn fetch_l1_manifest(&self, block: &L1BlockCommitment) -> WorkerResult<Option<AsmManifest>> {
        Ok(self.manifests.get(&block.height()).cloned())
    }

    fn fetch_block(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockV1>> {
        Ok(self.blocks.get(blkid).cloned())
    }

    fn fetch_header(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockHeaderV1>> {
        Ok(self
            .headers
            .get(blkid)
            .cloned()
            .or_else(|| self.blocks.get(blkid).map(|block| block.header().clone())))
    }

    fn fetch_ol_state(
        &self,
        commitment: OLBlockCommitment,
    ) -> WorkerResult<Option<OLStateContainer>> {
        Ok(self.states.get(&commitment).cloned())
    }

    fn fetch_canonical_epoch_summary_at(&self, epoch: Epoch) -> WorkerResult<Option<EpochSummary>> {
        Ok(self.canonical_summaries.get(&epoch).cloned())
    }

    fn fetch_epoch_summary(&self, epoch: EpochCommitment) -> WorkerResult<Option<EpochSummary>> {
        Ok(self
            .canonical_summaries
            .get(&epoch.epoch())
            .filter(|summary| summary.get_epoch_commitment() == epoch)
            .cloned())
    }

    fn store_block_output(
        &self,
        block: &OLBlockV1,
        commitment: OLBlockCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        let encoded = encode_to_vec(output.write_batch()).expect("write batch encodes");
        self.write_batches
            .lock()
            .unwrap()
            .insert(commitment, encoded);
        self.indexed_epochs
            .lock()
            .unwrap()
            .push(block.header().epoch());
        Ok(())
    }

    fn store_toplevel_state(
        &self,
        commitment: OLBlockCommitment,
        state: OLStateContainer,
    ) -> WorkerResult<()> {
        self.stored_states.lock().unwrap().push((commitment, state));
        Ok(())
    }

    fn store_terminal_header(&self, _id: OLBlockId, _header: OLBlockHeaderV1) -> WorkerResult<()> {
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
        let merged = merge_epoch_state(self, summary)?;
        self.stored_states
            .lock()
            .unwrap()
            .push((*summary.terminal(), merged));
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

    fn fetch_write_batch(&self, commitment: OLBlockCommitment) -> WorkerResult<Option<WriteBatch>> {
        Ok(self
            .write_batches
            .lock()
            .unwrap()
            .get(&commitment)
            .map(|bytes| decode_buf_exact(bytes).expect("stored write batch decodes")))
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

/// Builds epoch 1 as a single terminal block on genesis and executes it
/// through [`exec_block`], returning the context and the terminal header.
///
/// With `v0_parent`, genesis processes the checkpoint predicate enactment
/// that ends V0, and its result is relabelled as the last V0 terminal: the
/// same chainstate as a V0 state, under a header committing to its bare root.
/// The block then runs as the first V1 block and wraps it.
fn exec_single_block_epoch(v0_parent: bool) -> (OrderEnforcingContext, OLBlockHeaderV1) {
    let mut state = make_marked_genesis_state();
    let snark_serial = seed_accounts(&mut state);
    let genesis_l1_anchor = L1BlockCommitment::new(state.last_l1_height(), *state.last_l1_blkid());
    let (genesis, genesis_manifest) = if v0_parent {
        let manifest = make_checkpoint_predicate_enactment_manifest(1, 1);
        let genesis = execute_block(
            &mut state,
            &BlockInfo::new_genesis(EPOCH_RUNNER_GENESIS_TIMESTAMP),
            None,
            BlockComponents::new_manifests(vec![manifest.clone()]).as_terminal(),
        )
        .expect("genesis block");
        (genesis, manifest)
    } else {
        let genesis = run_genesis(&mut state);
        let manifest = genesis
            .body()
            .manifests()
            .and_then(|container| container.manifests().first())
            .expect("genesis carries a manifest")
            .clone();
        (genesis, manifest)
    };
    let mut genesis_header = genesis.header().clone();
    if v0_parent {
        state.set_spec_versions(OLSpecVersions::uniform(OLSpecId::V0));
        let v0_root = state.compute_state_root().expect("V0 root");
        genesis_header = tamper_state_root(&genesis_header, v0_root);
    }
    let pre_epoch_state = state.to_container();
    let genesis_l1 = L1BlockCommitment::new(state.last_l1_height(), *state.last_l1_blkid());

    // Build epoch 1 as a single terminal block directly on genesis.
    let mut blocks: Vec<OLBlockV1> = Vec::new();
    let manifest = make_deposit_manifest_for_account(
        TERMINAL_L1_HEIGHT,
        0,
        snark_serial,
        SubjectId::from([42u8; 32]),
        BitcoinAmount::try_from(150_000_000)
            .expect("amount must not exceed the Bitcoin money supply"),
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
    let genesis_summary = EpochSummary::new(
        0,
        genesis_commitment,
        OLBlockCommitment::null(),
        genesis_l1,
        *genesis_header.state_root(),
    );

    let ctx = OrderEnforcingContext::new(
        HashMap::from([(*terminal_commitment.blkid(), terminal_block)]),
        HashMap::from([(*genesis_commitment.blkid(), genesis_header)]),
        HashMap::from([(genesis_commitment, pre_epoch_state)]),
        HashMap::from([(0, genesis_summary)]),
        genesis_l1_anchor,
        HashMap::from([(genesis_manifest.height(), genesis_manifest)]),
    );

    exec_block(&ctx, OLRuntimeParams::test_default(), &terminal_commitment)
        .expect("single-block epoch executes");
    (ctx, terminal_header)
}

/// Executing a terminal block that is also its epoch's first block must
/// succeed: the block's own indexing persist creates the epoch row that epoch
/// finalization stamps.
#[test]
fn test_exec_single_block_epoch_persists_before_summary() {
    let (ctx, terminal_header) = exec_single_block_epoch(false);
    let terminal_commitment =
        OLBlockCommitment::new(terminal_header.slot(), terminal_header.compute_blkid());

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

/// Block sync of the first V1 block on a V0 terminal persists the wrapped
/// state, whose root is the one the block header commits to. Both the block's
/// post-state and the epoch merge of its codec-decoded write batch reach it.
#[test]
fn test_exec_first_v1_block_on_v0_terminal_persists_wrapped_state() {
    let (ctx, terminal_header) = exec_single_block_epoch(true);
    let terminal_commitment =
        OLBlockCommitment::new(terminal_header.slot(), terminal_header.compute_blkid());

    let stored_states = ctx.stored_states.lock().unwrap();
    let terminal_states: Vec<_> = stored_states
        .iter()
        .filter(|(commitment, _)| *commitment == terminal_commitment)
        .map(|(_, state)| state)
        .collect();
    assert_eq!(
        terminal_states.len(),
        2,
        "post-state and merged state stored"
    );
    for state in terminal_states {
        assert_eq!(state.spec_versions(), OLSpecVersions::uniform(OLSpecId::V1));
        assert_eq!(state.compute_state_root(), *terminal_header.state_root());
    }

    let summaries = ctx.stored_summaries.lock().unwrap();
    assert_eq!(summaries[0].final_state(), terminal_header.state_root());
}

/// An epoch merge whose result does not hash to the summary's final state root
/// fails instead of producing a terminal state to store.
#[test]
fn test_merge_epoch_state_rejects_root_mismatch() {
    let (ctx, _) = exec_single_block_epoch(false);
    let summary = ctx.stored_summaries.lock().unwrap()[0];
    let wrong = EpochSummary::new(
        summary.epoch(),
        *summary.terminal(),
        *summary.prev_terminal(),
        *summary.new_l1(),
        Buf32::from([0xab; 32]),
    );

    let err = merge_epoch_state(&ctx, &wrong).expect_err("merged root must match the summary");
    assert!(matches!(
        err,
        WorkerError::MergedStateRootMismatch { merged, .. } if merged == *summary.final_state()
    ));
}

/// Block execution refuses the first block after a terminal block whose epoch
/// processed a checkpoint predicate enactment: that block would run V1's
/// successor, which this binary does not implement. Nothing is stored for it.
#[test]
fn test_exec_refuses_the_block_after_a_v1_enactment() {
    let mut state = make_marked_genesis_state();
    seed_accounts(&mut state);
    let genesis_l1_anchor = L1BlockCommitment::new(state.last_l1_height(), *state.last_l1_blkid());
    let genesis = run_genesis(&mut state);
    let genesis_header = genesis.header().clone();
    let genesis_manifest = genesis
        .body()
        .manifests()
        .and_then(|container| container.manifests().first())
        .expect("genesis carries a manifest")
        .clone();
    let genesis_commitment = genesis_header.compute_block_commitment();
    let genesis_summary = EpochSummary::new(
        0,
        genesis_commitment,
        OLBlockCommitment::null(),
        L1BlockCommitment::new(state.last_l1_height(), *state.last_l1_blkid()),
        *genesis_header.state_root(),
    );

    // Epoch 1: one terminal block that processes the enactment.
    let enactment = make_checkpoint_predicate_enactment_manifest(TERMINAL_L1_HEIGHT, 1);
    let mut blocks = Vec::new();
    run_terminal(&mut state, &mut blocks, &genesis_header, enactment.clone());
    let terminal_header = blocks.pop().expect("terminal block built").header().clone();
    let terminal_commitment = terminal_header.compute_block_commitment();
    let terminal_state = state.to_container();
    let terminal_summary = genesis_summary.create_next_epoch_summary(
        terminal_commitment,
        L1BlockCommitment::new(state.last_l1_height(), *state.last_l1_blkid()),
        *terminal_header.state_root(),
    );

    // Epoch 2: the block V1 rules would build next.
    let slot = terminal_header.slot() + 1;
    let next = execute_block(
        &mut state,
        &BlockInfo::new(
            EPOCH_RUNNER_GENESIS_TIMESTAMP + slot * EPOCH_RUNNER_SLOT_TIMESTAMP_STEP,
            slot,
            2,
        ),
        Some(&terminal_header),
        BlockComponents::new_empty(),
    )
    .expect("V1 rules build the block");
    let next_commitment = next.header().compute_block_commitment();

    let ctx = OrderEnforcingContext::new(
        HashMap::from([(*next_commitment.blkid(), to_ol_block(&next))]),
        HashMap::from([(*terminal_commitment.blkid(), terminal_header)]),
        HashMap::from([(terminal_commitment, terminal_state)]),
        HashMap::from([(0, genesis_summary), (1, terminal_summary)]),
        genesis_l1_anchor,
        HashMap::from([
            (genesis_manifest.height(), genesis_manifest),
            (enactment.height(), enactment),
        ]),
    );

    let err = exec_block(&ctx, OLRuntimeParams::test_default(), &next_commitment)
        .expect_err("no block executes after the V1 enactment");
    let WorkerError::UpgradeRequired(upgrade) = err else {
        panic!("expected an upgrade-required error, got {err}");
    };
    assert_eq!(upgrade.prev_spec(), OLSpecId::V1);
    assert_eq!(upgrade.enactment_l1_height(), TERMINAL_L1_HEIGHT);
    assert!(ctx.stored_states.lock().unwrap().is_empty());
    assert!(ctx.write_batches.lock().unwrap().is_empty());
    assert!(ctx.indexed_epochs.lock().unwrap().is_empty());
}
