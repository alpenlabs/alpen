//! Regression tests for the block-sync exec path ordering.
//!
//! A single-block epoch — the terminal block is also the epoch's first block,
//! as produced when the checkpoint size policy seals an epoch immediately —
//! must persist the terminal block's output before epoch finalization runs:
//! the epoch commitment is stamped onto the indexing row that persisting a
//! block of the epoch creates, and in a single-block epoch no earlier block
//! has created that row.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use strata_acct_types::BitcoinAmount;
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_common::AsmManifest;
use strata_checkpoint_types::EpochSummary;
use strata_codec::{decode_buf_exact, encode_to_vec};
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_db_types::errors::DbError;
use strata_identifiers::{
    Buf32, Epoch, EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
    SubjectId,
};
use strata_ol_block_ancestry_provider::{
    BlockAncestryError, BlockAncestryProvider, EpochBlockAncestry,
};
use strata_ol_chain_types_v1::{BlockFlagsV1, OLBlockHeaderV1, OLBlockV1};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_container::OLStateContainer;
use strata_ol_state_support_types::MemoryStateBaseLayer;
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
use strata_storage::{OLBlockManager, create_node_storage, test_runtime_handle};

use super::{fixture::make_marked_genesis_state, manifests::MockManifests};
use crate::{
    WorkerError, WorkerResult,
    output::OLBlockExecutionOutput,
    state::{exec_block, handle_accepted_block, handle_rejected_block, merge_epoch_state},
    traits::ChainWorkerContext,
};

/// A [`ChainWorkerContext`] that enforces the OL state indexing DB's write
/// ordering contract: stamping an epoch commitment ([`Self::store_summary`])
/// errors unless some block of that epoch had its indexing writes applied
/// first ([`Self::store_block_output`]), mirroring `set_epoch_commitment` on
/// the sled-backed store.
struct OrderEnforcingContext {
    /// Block storage served to [`ChainWorkerContext::fetch_block`] and
    /// [`ChainWorkerContext::fetch_header`].
    blocks: Arc<OLBlockManager>,
    /// Resolves epoch ancestry from [`Self::blocks`].
    block_ancestry_provider: BlockAncestryProvider,
    /// States served to [`ChainWorkerContext::fetch_ol_state`].
    states: HashMap<OLBlockCommitment, OLStateContainer>,
    /// Stored manifests, served by height to
    /// [`ChainWorkerContext::fetch_l1_manifest`] and by block to
    /// [`ChainWorkerContext::fetch_l1_block_manifest`].
    manifests: MockManifests,
    /// Summaries present before the test runs, served by exact commitment.
    known_summaries: HashMap<EpochCommitment, EpochSummary>,
    /// L1 block genesis anchors to.
    genesis_l1: L1BlockCommitment,
    /// Epochs with at least one block's indexing writes applied.
    indexed_epochs: Mutex<Vec<Epoch>>,
    /// Summaries accepted by [`ChainWorkerContext::store_summary`].
    stored_summaries: Mutex<Vec<EpochSummary>>,
    /// Exact commitments passed to summary deletion.
    deleted_summaries: Mutex<Vec<EpochCommitment>>,
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
    ///
    /// Blocks and header-only terminals go into block storage, so ancestry
    /// walks read them the way the production context does.
    fn new(
        blocks: impl IntoIterator<Item = OLBlockV1>,
        terminal_headers: impl IntoIterator<Item = OLBlockHeaderV1>,
        states: HashMap<OLBlockCommitment, OLStateContainer>,
        known_summaries: impl IntoIterator<Item = EpochSummary>,
        genesis_l1: L1BlockCommitment,
        manifests: MockManifests,
    ) -> Self {
        let storage = create_node_storage(get_test_sled_backend(), test_runtime_handle())
            .expect("test: create storage");
        let block_storage = storage.ol_block().clone();
        for block in blocks {
            block_storage
                .put_block_data_blocking(block)
                .expect("test: store block");
        }
        for header in terminal_headers {
            block_storage
                .put_terminal_header_blocking(header.compute_blkid(), header)
                .expect("test: store terminal header");
        }
        Self {
            block_ancestry_provider: BlockAncestryProvider::new(block_storage.clone()),
            blocks: block_storage,
            states,
            known_summaries: known_summaries
                .into_iter()
                .map(|summary| (summary.get_epoch_commitment(), summary))
                .collect(),
            genesis_l1,
            manifests,
            indexed_epochs: Mutex::new(Vec::new()),
            stored_summaries: Mutex::new(Vec::new()),
            deleted_summaries: Mutex::new(Vec::new()),
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

    fn fetch_l1_manifest(&self, height: u32) -> WorkerResult<Option<AsmManifest>> {
        Ok(self.manifests.canonical_at(height).cloned())
    }

    fn fetch_l1_block_manifest(
        &self,
        block: &L1BlockCommitment,
    ) -> WorkerResult<Option<AsmManifest>> {
        Ok(self.manifests.of_block(block).cloned())
    }

    fn fetch_block(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockV1>> {
        Ok(self.blocks.get_block_data_blocking(*blkid)?)
    }

    fn fetch_header(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockHeaderV1>> {
        Ok(self.blocks.get_ol_header_blocking(*blkid)?)
    }

    fn collect_epoch_block_commitments(
        &self,
        start: &OLBlockHeaderV1,
    ) -> WorkerResult<EpochBlockAncestry> {
        Ok(self
            .block_ancestry_provider
            .collect_epoch_block_commitments_blocking(start)?)
    }

    fn fetch_ol_state(
        &self,
        commitment: OLBlockCommitment,
    ) -> WorkerResult<Option<OLStateContainer>> {
        Ok(self
            .stored_states
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(stored, _)| *stored == commitment)
            .map(|(_, state)| state.clone())
            .or_else(|| self.states.get(&commitment).cloned()))
    }

    fn fetch_summary(&self, epoch: EpochCommitment) -> WorkerResult<Option<EpochSummary>> {
        Ok(self
            .stored_summaries
            .lock()
            .unwrap()
            .iter()
            .find(|summary| summary.get_epoch_commitment() == epoch)
            .copied()
            .or_else(|| self.known_summaries.get(&epoch).copied()))
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
        // merge failure can never announce an incomplete epoch.
        if !self.merged_epochs.lock().unwrap().contains(&commitment) {
            return Err(WorkerError::Database(DbError::Other(format!(
                "summary stored before epoch data merge for epoch {epoch}"
            ))));
        }
        self.stored_summaries.lock().unwrap().push(summary);
        Ok(())
    }

    fn delete_summary(&self, epoch: EpochCommitment) -> WorkerResult<bool> {
        self.deleted_summaries.lock().unwrap().push(epoch);
        let mut summaries = self.stored_summaries.lock().unwrap();
        let old_len = summaries.len();
        summaries.retain(|summary| summary.get_epoch_commitment() != epoch);
        Ok(summaries.len() != old_len)
    }

    fn merge_epoch_data(
        &self,
        summary: &EpochSummary,
        blocks: &[OLBlockCommitment],
    ) -> WorkerResult<()> {
        let merged = merge_epoch_state(self, summary, blocks)?;
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
    run_terminal(&mut state, &mut blocks, &genesis_header, manifest.clone());
    let terminal_block = blocks.pop().expect("terminal block built");
    let terminal_header = terminal_block.header().clone();
    let terminal_commitment =
        OLBlockCommitment::new(terminal_header.slot(), terminal_header.compute_blkid());
    assert!(terminal_header.is_terminal(), "epoch 1 block is terminal");
    assert_eq!(terminal_header.epoch(), 1, "single-block epoch 1");

    // Genesis (epoch 0) commitment and summary; spec selection reads the summary.
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
        [terminal_block],
        [genesis_header],
        HashMap::from([(genesis_commitment, pre_epoch_state)]),
        [genesis_summary],
        genesis_l1_anchor,
        MockManifests::from_iter([genesis_manifest, manifest]),
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

    assert!(ctx.stored_summaries.lock().unwrap().is_empty());
    assert!(ctx.merged_epochs.lock().unwrap().is_empty());
    assert!(ctx.fetch_ol_state(terminal_commitment).unwrap().is_some());

    handle_accepted_block(&ctx, terminal_commitment)
        .expect("FCM requests summary after acceptance");
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
    handle_accepted_block(&ctx, terminal_commitment).expect("create accepted epoch summary");

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

/// A block carrying a deposit log that the canonical manifest at its L1 height
/// lacks is rejected before execution.
#[test]
fn test_exec_block_rejects_noncanonical_manifest() {
    let (mut ctx, terminal_header) = exec_single_block_epoch(false);
    let carried = ctx
        .manifests
        .canonical_at(TERMINAL_L1_HEIGHT)
        .expect("carried manifest");
    let canonical = AsmManifest::new(
        TERMINAL_L1_HEIGHT,
        *carried.blkid(),
        *carried.wtxids_root(),
        Vec::new(),
    )
    .expect("manifest without logs");
    ctx.manifests.insert_canonical(canonical);

    let terminal_commitment =
        OLBlockCommitment::new(terminal_header.slot(), terminal_header.compute_blkid());
    let err = exec_block(&ctx, OLRuntimeParams::test_default(), &terminal_commitment)
        .expect_err("forged manifest must be rejected");
    assert!(matches!(
        err,
        WorkerError::NoncanonicalManifest { height } if height == TERMINAL_L1_HEIGHT
    ));
}

/// A block carrying the manifest of an L1 block that a reorg replaced is
/// rejected, although that manifest stays stored by its block: the check
/// compares with the canonical manifest at the height, not with the stored
/// manifest of the carried block.
#[test]
fn test_exec_block_rejects_manifest_of_a_reorged_block() {
    let (mut ctx, terminal_header) = exec_single_block_epoch(false);
    let carried = ctx
        .manifests
        .canonical_at(TERMINAL_L1_HEIGHT)
        .cloned()
        .expect("carried manifest");
    let replacing = AsmManifest::new(
        TERMINAL_L1_HEIGHT,
        L1BlockId::from(Buf32::from([0xee; 32])),
        *carried.wtxids_root(),
        carried.logs().to_vec(),
    )
    .expect("manifest of the replacing block");
    ctx.manifests.insert_canonical(replacing);
    let carried_block = L1BlockCommitment::new(TERMINAL_L1_HEIGHT, *carried.blkid());
    assert_eq!(
        ctx.fetch_l1_block_manifest(&carried_block)
            .expect("lookup by block"),
        Some(carried),
        "the reorged block's manifest stays stored"
    );

    let terminal_commitment =
        OLBlockCommitment::new(terminal_header.slot(), terminal_header.compute_blkid());
    let err = exec_block(&ctx, OLRuntimeParams::test_default(), &terminal_commitment)
        .expect_err("a reorged block's manifest must be rejected");
    assert!(
        matches!(err, WorkerError::NoncanonicalManifest { height } if height == TERMINAL_L1_HEIGHT),
        "{err}"
    );
}

/// An epoch merge whose result does not hash to the summary's final state root
/// fails instead of producing a terminal state to store.
#[test]
fn test_merge_epoch_state_rejects_root_mismatch() {
    let (ctx, header) = exec_single_block_epoch(false);
    handle_accepted_block(&ctx, header.compute_block_commitment()).expect("create summary");
    let summary = ctx.stored_summaries.lock().unwrap()[0];
    let wrong = EpochSummary::new(
        summary.epoch(),
        *summary.terminal(),
        *summary.prev_terminal(),
        *summary.new_l1(),
        Buf32::from([0xab; 32]),
    );

    let err = merge_epoch_state(&ctx, &wrong, &[*summary.terminal()])
        .expect_err("merged root must match the summary");
    assert!(matches!(
        err,
        WorkerError::MergedStateRootMismatch { merged, .. } if merged == *summary.final_state()
    ));
}

/// Ordinary blocks must not create or delete summaries, or merge epoch data.
#[test]
fn test_handle_non_terminal_block_does_not_change_epoch_summaries() {
    let (ctx, terminal_header) = exec_single_block_epoch(false);
    let header = OLBlockHeaderV1::new(
        0,
        BlockFlagsV1::zero(),
        terminal_header.slot() + 1,
        terminal_header.epoch() + 1,
        terminal_header.compute_blkid(),
        Buf32::zero(),
        Buf32::zero(),
        Buf32::zero(),
    );
    let block = header.compute_block_commitment();
    ctx.blocks
        .put_terminal_header_blocking(*block.blkid(), header)
        .unwrap();
    let stored_state_count = ctx.stored_states.lock().unwrap().len();

    handle_accepted_block(&ctx, block).expect("ordinary accepted block needs no epoch completion");
    handle_rejected_block(&ctx, block).expect("ordinary rejected block needs no summary deletion");

    assert!(ctx.deleted_summaries.lock().unwrap().is_empty());
    assert!(ctx.stored_summaries.lock().unwrap().is_empty());
    assert!(ctx.merged_epochs.lock().unwrap().is_empty());
    assert_eq!(ctx.stored_states.lock().unwrap().len(), stored_state_count);
}

/// Rejection deletes only the exact terminal's summary and tolerates repeated signals.
#[test]
fn test_rejected_terminal_deletes_only_its_summary() {
    let (ctx, header) = exec_single_block_epoch(false);
    let rejected_block = header.compute_block_commitment();
    handle_accepted_block(&ctx, rejected_block)
        .expect("seed summary from the old eager-write flow");
    let original = ctx.stored_summaries.lock().unwrap()[0];
    let replacement = EpochSummary::new(
        original.epoch(),
        OLBlockCommitment::new(
            rejected_block.slot(),
            OLBlockId::from(Buf32::from([0xab; 32])),
        ),
        *original.prev_terminal(),
        *original.new_l1(),
        *original.final_state(),
    );
    ctx.stored_summaries.lock().unwrap().push(replacement);

    handle_rejected_block(&ctx, rejected_block).expect("remove obsolete summary");
    assert_eq!(*ctx.stored_summaries.lock().unwrap(), vec![replacement]);
    handle_rejected_block(&ctx, rejected_block).expect("missing summary is already cleaned up");
    assert_eq!(*ctx.stored_summaries.lock().unwrap(), vec![replacement]);
    assert_eq!(
        *ctx.deleted_summaries.lock().unwrap(),
        vec![original.get_epoch_commitment(); 2]
    );
}

/// Repeated acceptance needs no state reconstruction once the exact summary exists.
#[test]
fn test_accepted_terminal_with_summary_skips_epoch_work() {
    let (mut ctx, header) = exec_single_block_epoch(false);
    let block = header.compute_block_commitment();
    handle_accepted_block(&ctx, block).expect("create missing summary from stored execution");
    let summary = ctx.stored_summaries.lock().unwrap()[0];
    ctx.states.clear();
    ctx.stored_states.lock().unwrap().clear();
    ctx.write_batches.lock().unwrap().clear();

    handle_accepted_block(&ctx, block).expect("existing summary needs no state or write batches");
    assert_eq!(*ctx.stored_summaries.lock().unwrap(), vec![summary]);
    assert_eq!(
        *ctx.merged_epochs.lock().unwrap(),
        vec![summary.get_epoch_commitment()]
    );
    assert!(ctx.stored_states.lock().unwrap().is_empty());
}

/// A summary for another terminal in the same epoch must not suppress recovery.
#[test]
fn test_accepted_terminal_recovers_despite_other_summary_at_same_epoch() {
    let (ctx, header) = exec_single_block_epoch(false);
    let block = header.compute_block_commitment();
    let genesis = ctx
        .fetch_header(header.parent_blkid())
        .unwrap()
        .unwrap()
        .compute_block_commitment();
    let genesis_summary = ctx.known_summaries[&EpochCommitment::from_terminal(0, genesis)];
    let other = EpochSummary::new(
        header.epoch(),
        OLBlockCommitment::new(block.slot(), OLBlockId::from(Buf32::from([0xab; 32]))),
        *genesis_summary.terminal(),
        *genesis_summary.new_l1(),
        *header.state_root(),
    );
    ctx.stored_summaries.lock().unwrap().push(other);

    handle_accepted_block(&ctx, block).expect("recover this terminal's missing summary");
    let summaries = ctx.stored_summaries.lock().unwrap();
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0], other);
    assert_eq!(*summaries[1].terminal(), block);
    assert_eq!(
        ctx.indexed_epochs.lock().unwrap().len(),
        1,
        "block was executed only once"
    );
}

/// Block execution refuses the first block after a terminal block whose V2
/// epoch processed a checkpoint predicate enactment: that block would run
/// V2's successor, which this binary does not implement. Nothing is stored
/// for it.
#[test]
fn test_exec_refuses_the_block_after_a_v2_enactment() {
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

    // Epoch 1 runs V2: one terminal block that processes the enactment. V2's
    // first block takes over the V1 state, and the V1 code the fixture runs
    // leaves the V2 versions as they are, so it builds the epoch as V2 does.
    state.set_spec_versions(OLSpecVersions::uniform(OLSpecId::V2));
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

    // Epoch 2: the block V2 rules would build next.
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
    .expect("V2 rules build the block");
    let next_commitment = next.header().compute_block_commitment();

    let ctx = OrderEnforcingContext::new(
        [to_ol_block(&next)],
        [terminal_header],
        HashMap::from([(terminal_commitment, terminal_state)]),
        [genesis_summary, terminal_summary],
        genesis_l1_anchor,
        MockManifests::from_iter([genesis_manifest, enactment]),
    );

    let err = exec_block(&ctx, OLRuntimeParams::test_default(), &next_commitment)
        .expect_err("no block executes after the V2 enactment");
    let WorkerError::UpgradeRequired(upgrade) = err else {
        panic!("expected an upgrade-required error, got {err}");
    };
    assert_eq!(upgrade.prev_spec(), OLSpecId::V2);
    assert_eq!(upgrade.enactment_l1_height(), TERMINAL_L1_HEIGHT);
    assert!(ctx.stored_states.lock().unwrap().is_empty());
    assert!(ctx.write_batches.lock().unwrap().is_empty());
    assert!(ctx.indexed_epochs.lock().unwrap().is_empty());
}

#[test]
#[should_panic(expected = "epoch block list must not be empty")]
fn test_merge_epoch_state_rejects_empty_blocks() {
    let (ctx, header) = exec_single_block_epoch(false);
    handle_accepted_block(&ctx, header.compute_block_commitment()).expect("accept terminal");
    let summary = ctx.stored_summaries.lock().unwrap()[0];

    let _ = merge_epoch_state(&ctx, &summary, &[]);
}

#[test]
#[should_panic(expected = "epoch block list must end at the summary's terminal")]
fn test_merge_epoch_state_rejects_mismatched_terminal() {
    let (ctx, header) = exec_single_block_epoch(false);
    handle_accepted_block(&ctx, header.compute_block_commitment()).expect("accept terminal");
    let summary = ctx.stored_summaries.lock().unwrap()[0];

    let _ = merge_epoch_state(&ctx, &summary, &[*summary.prev_terminal()]);
}

#[test]
fn test_merge_epoch_state_uses_collected_blocks_without_headers() {
    let (mut ctx, header) = exec_single_block_epoch(false);
    handle_accepted_block(&ctx, header.compute_block_commitment()).expect("accept terminal");
    let summary = ctx.stored_summaries.lock().unwrap()[0];
    let blocks = [*summary.terminal()];

    // Ancestry is already supplied. The merge needs only the base state and write batches.
    let storage = create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap();
    ctx.blocks = storage.ol_block().clone();
    ctx.block_ancestry_provider = BlockAncestryProvider::new(ctx.blocks.clone());

    let merged = merge_epoch_state(&ctx, &summary, &blocks).expect("merge without header reads");
    assert_eq!(merged.compute_state_root(), *summary.final_state());
}

fn build_child_block(
    ctx: &mut OrderEnforcingContext,
    parent: &OLBlockHeaderV1,
    terminal: bool,
) -> OLBlockHeaderV1 {
    let pre_state = ctx
        .fetch_ol_state(parent.compute_block_commitment())
        .unwrap()
        .expect("executed parent state");
    let mut state = MemoryStateBaseLayer::from_container(pre_state);
    let info = BlockInfo::new(
        parent.timestamp() + 1_000,
        parent.slot() + 1,
        parent.epoch() + u32::from(parent.is_terminal()),
    );
    let completed = execute_block(
        &mut state,
        &info,
        Some(parent),
        BlockComponents::new_empty().with_terminal(terminal),
    )
    .expect("build child block");
    let block = to_ol_block(&completed);
    let header = block.header().clone();
    ctx.blocks.put_block_data_blocking(block).unwrap();
    header
}

fn prepare_fork_epoch() -> (OrderEnforcingContext, OLBlockHeaderV1, OLBlockHeaderV1) {
    let (mut ctx, header) = exec_single_block_epoch(false);
    handle_accepted_block(&ctx, header.compute_block_commitment())
        .expect("accept branch A terminal");
    let genesis = ctx.fetch_header(header.parent_blkid()).unwrap().unwrap();

    // Branch A already has an epoch-1 summary. Branch B forks from genesis
    // and produces a different epoch-1 terminal at the same slot.
    let previous_terminal = build_child_block(&mut ctx, &genesis, true);
    exec_block(
        &ctx,
        OLRuntimeParams::test_default(),
        &previous_terminal.compute_block_commitment(),
    )
    .expect("execute competing epoch-1 terminal");
    handle_accepted_block(&ctx, previous_terminal.compute_block_commitment())
        .expect("accept branch B terminal");

    // Epoch 2 has two nonterminal ancestors to traverse before reaching B's terminal.
    let mut parent = previous_terminal.clone();
    for _ in 0..2 {
        parent = build_child_block(&mut ctx, &parent, false);
        exec_block(
            &ctx,
            OLRuntimeParams::test_default(),
            &parent.compute_block_commitment(),
        )
        .expect("execute nonterminal block");
    }
    let terminal = build_child_block(&mut ctx, &parent, true);
    (ctx, terminal, previous_terminal)
}

#[test]
fn test_accepted_terminal_uses_own_branch_previous_terminal() {
    let (mut ctx, terminal, previous_terminal) = prepare_fork_epoch();
    let previous_commitment = previous_terminal.compute_block_commitment();
    assert_ne!(
        ctx.stored_summaries.lock().unwrap()[0].terminal(),
        &previous_commitment
    );

    // Retain only epoch 2's blocks and the preceding terminal header. Its body
    // and all older headers are absent, while the executed states remain available.
    let storage = create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap();
    let retained_blocks = storage.ol_block().clone();
    for slot in previous_commitment.slot() + 1..=terminal.slot() {
        for id in ctx.blocks.get_blocks_at_height_blocking(slot).unwrap() {
            let block = ctx.fetch_block(&id).unwrap().unwrap();
            retained_blocks.put_block_data_blocking(block).unwrap();
        }
    }
    retained_blocks
        .put_terminal_header_blocking(*previous_commitment.blkid(), previous_terminal)
        .unwrap();
    ctx.blocks = retained_blocks;
    ctx.block_ancestry_provider = BlockAncestryProvider::new(ctx.blocks.clone());

    exec_block(
        &ctx,
        OLRuntimeParams::test_default(),
        &terminal.compute_block_commitment(),
    )
    .expect("execute terminal on competing branch");
    handle_accepted_block(&ctx, terminal.compute_block_commitment())
        .expect("accept competing terminal");

    let summaries = ctx.stored_summaries.lock().unwrap();
    let summary = summaries.last().unwrap();
    assert_eq!(summary.epoch(), 2);
    assert_eq!(summary.prev_terminal(), &previous_commitment);
    assert_eq!(summary.terminal(), &terminal.compute_block_commitment());
    let stored_states = ctx.stored_states.lock().unwrap();
    assert_eq!(
        stored_states.last().unwrap().1.compute_state_root(),
        *terminal.state_root()
    );
}

#[test]
fn test_accepted_terminal_rejects_missing_ancestor_header() {
    let (ctx, terminal, previous_terminal) = prepare_fork_epoch();
    let missing_id = previous_terminal.compute_blkid();
    ctx.blocks.del_block_data_blocking(missing_id).unwrap();
    let summaries_before = ctx.stored_summaries.lock().unwrap().len();

    // Other summaries and the requested terminal's parent state still exist;
    // missing ancestry must fail instead of selecting another branch's summary.
    exec_block(
        &ctx,
        OLRuntimeParams::test_default(),
        &terminal.compute_block_commitment(),
    )
    .expect("execution only needs the immediate parent");
    let err = handle_accepted_block(&ctx, terminal.compute_block_commitment())
        .expect_err("missing ancestor must fail epoch finalization");
    assert!(
        matches!(err, WorkerError::BlockAncestry(BlockAncestryError::MissingHeader(id)) if id == missing_id)
    );
    assert_eq!(ctx.stored_summaries.lock().unwrap().len(), summaries_before);
}
