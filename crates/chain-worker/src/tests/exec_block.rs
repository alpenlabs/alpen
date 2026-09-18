//! Regression tests for the block-sync exec path ordering.
//!
//! A single-block epoch — the terminal block is also the epoch's first block,
//! as produced when the checkpoint size policy seals an epoch immediately —
//! must persist the terminal block's output before epoch finalization runs:
//! the epoch commitment is stamped onto the indexing row that persisting a
//! block of the epoch creates, and in a single-block epoch no earlier block
//! has created that row.

use std::{collections::HashMap, slice::from_ref, sync::Mutex};

use strata_acct_types::BitcoinAmount;
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_checkpoint_types::EpochSummary;
use strata_codec::{decode_buf_exact, encode_to_vec};
use strata_db_types::{DbResult, errors::DbError};
use strata_identifiers::{
    Buf32, Buf64, Epoch, EpochCommitment, L1BlockCommitment, L1BlockId, L1Height,
    OLBlockCommitment, OLBlockId, SubjectId, WtxidsRoot,
};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1, SignedOLBlockHeaderV1};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_container::OLStateContainer;
use strata_ol_state_types::{IStateAccessor, IStateAccessorMut, OLSpecId, OLSpecVersions};
use strata_ol_state_types_v1::WriteBatch;
use strata_ol_stf_v1::{
    BlockComponents, BlockInfo, ExecError,
    test_utils::{
        EPOCH_RUNNER_TERMINAL_L1_HEIGHT as TERMINAL_L1_HEIGHT,
        epoch_runner_run_genesis as run_genesis, epoch_runner_run_terminal as run_terminal,
        epoch_runner_seed_accounts as seed_accounts, execute_block,
        make_deposit_manifest_for_account, make_empty_manifest, make_genesis_state,
        tamper_state_root, to_ol_block,
    },
};

use super::fixture::make_marked_genesis_state;
use crate::{
    ManifestPendingReason, WorkerError, WorkerResult,
    output::OLBlockExecutionOutput,
    provenance::validate_manifests,
    state::{exec_block, merge_epoch_state},
    traits::ChainWorkerContext,
};

/// A [`ChainWorkerContext`] that enforces the OL state indexing DB's write
/// ordering contract: stamping an epoch commitment ([`Self::store_summary`])
/// errors unless some block of that epoch had its indexing writes applied
/// first ([`Self::store_block_output`]), mirroring `set_epoch_commitment` on
/// the sled-backed store.
#[derive(Default)]
struct OrderEnforcingContext {
    manifests: HashMap<u32, AsmManifest>,
    tip: Option<u32>,
    depth: u32,
    read_failure: bool,
    /// Blocks served to [`ChainWorkerContext::fetch_block`].
    blocks: HashMap<OLBlockId, OLBlockV1>,
    /// Headers served to [`ChainWorkerContext::fetch_header`].
    headers: HashMap<OLBlockId, OLBlockHeaderV1>,
    /// States served to [`ChainWorkerContext::fetch_ol_state`].
    states: HashMap<OLBlockCommitment, OLStateContainer>,
    /// Canonical summaries served per epoch index.
    canonical_summaries: HashMap<Epoch, EpochSummary>,
    /// Epochs with at least one block's indexing writes applied.
    indexed_epochs: Mutex<Vec<Epoch>>,
    last_indexed_block: Mutex<Option<OLBlockCommitment>>,
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
    /// Calls to the canonical L1 tip and manifest readers.
    canonical_reads: Mutex<usize>,
}

impl ChainWorkerContext for OrderEnforcingContext {
    fn l1_reorg_safe_depth(&self) -> u32 {
        self.depth
    }
    fn canonical_l1_tip_height(&self) -> DbResult<Option<L1Height>> {
        *self.canonical_reads.lock().unwrap() += 1;
        Ok(self.tip)
    }
    fn canonical_manifest(&self, height: L1Height) -> DbResult<Option<AsmManifest>> {
        *self.canonical_reads.lock().unwrap() += 1;
        if self.read_failure {
            return Err(DbError::Other("injected read failure".into()));
        }
        Ok(self.manifests.get(&height).cloned())
    }

    fn runtime_params(&self) -> OLRuntimeParams {
        OLRuntimeParams::test_default()
    }

    fn genesis_l1_block(&self) -> L1BlockCommitment {
        unimplemented!("not used by block execution")
    }

    fn fetch_l1_manifest(&self, _height: u32) -> WorkerResult<Option<AsmManifest>> {
        unimplemented!("not used by block execution")
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

    fn store_block_output(
        &self,
        block: &OLBlockV1,
        commitment: OLBlockCommitment,
        output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        let mut last_indexed = self.last_indexed_block.lock().unwrap();
        if *last_indexed == Some(commitment) {
            return Err(DbError::BlockIndexingConflict {
                epoch: block.header().epoch(),
                attempted: commitment,
                last_applied: commitment,
            }
            .into());
        }
        *last_indexed = Some(commitment);
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

/// Builds epoch 1 as a single terminal block on genesis without executing it,
/// returning the context and the terminal header.
///
/// With `v0_parent`, genesis's result is first relabelled as the last V0
/// terminal: the same chainstate as a V0 state, under a header committing to
/// its bare root. The block then runs as the first V1 block and wraps it.
fn single_block_epoch_fixture(v0_parent: bool) -> (OrderEnforcingContext, OLBlockHeaderV1) {
    let mut state = make_marked_genesis_state();
    let snark_serial = seed_accounts(&mut state);
    let genesis = run_genesis(&mut state);
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

    let canonical_manifest = terminal_block.body().manifests().unwrap().manifests()[0].clone();
    let ctx = OrderEnforcingContext {
        manifests: HashMap::from([(canonical_manifest.height(), canonical_manifest)]),
        tip: Some(100),
        depth: 1,
        read_failure: false,
        blocks: HashMap::from([(*terminal_commitment.blkid(), terminal_block)]),
        headers: HashMap::from([(*genesis_commitment.blkid(), genesis_header)]),
        states: HashMap::from([(genesis_commitment, pre_epoch_state)]),
        canonical_summaries: HashMap::from([(0, genesis_summary)]),
        indexed_epochs: Mutex::new(Vec::new()),
        last_indexed_block: Mutex::new(None),
        stored_summaries: Mutex::new(Vec::new()),
        merged_epochs: Mutex::new(Vec::new()),
        stored_states: Mutex::new(Vec::new()),
        write_batches: Mutex::new(HashMap::new()),
        canonical_reads: Mutex::new(0),
    };

    (ctx, terminal_header)
}

/// Executes the epoch built by [`single_block_epoch_fixture`] through [`exec_block`].
fn exec_single_block_epoch(v0_parent: bool) -> (OrderEnforcingContext, OLBlockHeaderV1) {
    let (ctx, terminal_header) = single_block_epoch_fixture(v0_parent);
    let terminal_commitment =
        OLBlockCommitment::new(terminal_header.slot(), terminal_header.compute_blkid());
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
#[test]
fn terminal_execution_recovers_when_predecessor_summary_arrives() {
    let (mut ctx, terminal_header) = single_block_epoch_fixture(false);
    let terminal_commitment =
        OLBlockCommitment::new(terminal_header.slot(), terminal_header.compute_blkid());
    let predecessor = ctx.canonical_summaries.remove(&0).unwrap();

    assert!(matches!(
        exec_block(&ctx, OLRuntimeParams::test_default(), &terminal_commitment),
        Err(WorkerError::MissingSummaryForEpoch(0))
    ));
    assert_eq!(*ctx.indexed_epochs.lock().unwrap(), vec![1]);
    assert!(ctx.stored_summaries.lock().unwrap().is_empty());
    assert!(ctx.merged_epochs.lock().unwrap().is_empty());

    ctx.canonical_summaries.insert(0, predecessor);
    exec_block(&ctx, OLRuntimeParams::test_default(), &terminal_commitment)
        .expect("terminal execution resumes after its predecessor summary arrives");
    assert_eq!(*ctx.indexed_epochs.lock().unwrap(), vec![1]);
    assert_eq!(
        *ctx.last_indexed_block.lock().unwrap(),
        Some(terminal_commitment)
    );
    let summaries = ctx.stored_summaries.lock().unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(*summaries[0].terminal(), terminal_commitment);
}

fn manifest_context(manifests: &[AsmManifest]) -> OrderEnforcingContext {
    OrderEnforcingContext {
        manifests: manifests
            .iter()
            .map(|mf| (mf.height(), mf.clone()))
            .collect(),
        tip: Some(100),
        depth: 6,
        ..Default::default()
    }
}

#[test]
fn full_manifest_authentication_covers_ids_roots_and_ordered_log_bytes() {
    let original = AsmManifest::new(
        2,
        L1BlockId::from(Buf32::from([1; 32])),
        WtxidsRoot::from(Buf32::from([2; 32])),
        vec![
            AsmLogEntry::from_raw(vec![1, 2, 3]).unwrap(),
            AsmLogEntry::from_raw(vec![4, 5]).unwrap(),
        ],
    )
    .unwrap();
    let ctx = manifest_context(from_ref(&original));
    validate_manifests(&ctx, 1, from_ref(&original)).unwrap();
    let mut variants = vec![
        AsmManifest::new(
            2,
            L1BlockId::from(Buf32::from([9; 32])),
            *original.wtxids_root(),
            original.logs().to_vec(),
        )
        .unwrap(),
        AsmManifest::new(
            2,
            *original.blkid(),
            WtxidsRoot::from(Buf32::from([9; 32])),
            original.logs().to_vec(),
        )
        .unwrap(),
    ];
    for logs in [
        vec![],
        original.logs()[..1].to_vec(),
        original.logs().iter().cloned().rev().collect(),
        vec![
            AsmLogEntry::from_raw(vec![1, 2, 9]).unwrap(),
            original.logs()[1].clone(),
        ],
        vec![
            original.logs()[0].clone(),
            original.logs()[1].clone(),
            original.logs()[0].clone(),
        ],
    ] {
        variants
            .push(AsmManifest::new(2, *original.blkid(), *original.wtxids_root(), logs).unwrap());
    }
    for forged in variants {
        assert!(matches!(
            validate_manifests(&ctx, 1, &[forged]),
            Err(WorkerError::ManifestContentMismatch { height: 2 })
        ));
    }
}

#[test]
fn provenance_checks_order_before_dependencies_and_exact_burial_boundary() {
    let first = make_empty_manifest(2, 1);
    let second = make_empty_manifest(3, 2);
    let mut ctx = manifest_context(&[first.clone(), second.clone()]);
    for malformed in [
        vec![second.clone()],
        vec![first.clone(), first.clone()],
        vec![second.clone(), first.clone()],
    ] {
        assert!(matches!(
            validate_manifests(&ctx, 1, &malformed),
            Err(WorkerError::StfExecution(_))
        ));
    }
    ctx.tip = None;
    validate_manifests(&ctx, u32::MAX, &[]).unwrap();
    assert!(matches!(
        validate_manifests(&ctx, u32::MAX, from_ref(&first)),
        Err(WorkerError::StfExecution(_))
    ));
    assert!(matches!(
        validate_manifests(&ctx, 1, from_ref(&first)),
        Err(WorkerError::ManifestPending {
            reason: ManifestPendingReason::MissingTip,
            ..
        })
    ));
    ctx.tip = Some(6); // h=2 has five confirmations; six are required.
    assert!(matches!(
        validate_manifests(&ctx, 1, from_ref(&first)),
        Err(WorkerError::ManifestPending {
            reason: ManifestPendingReason::NotBuried,
            ..
        })
    ));
    ctx.tip = Some(7);
    validate_manifests(&ctx, 1, from_ref(&first)).unwrap();
    // The later manifest cannot pass just because the first one is buried.
    assert!(matches!(
        validate_manifests(&ctx, 1, &[first.clone(), second]),
        Err(WorkerError::ManifestPending {
            height: 3,
            reason: ManifestPendingReason::NotBuried
        })
    ));
    ctx.manifests.clear();
    assert!(matches!(
        validate_manifests(&ctx, 1, from_ref(&first)),
        Err(WorkerError::ManifestPending {
            reason: ManifestPendingReason::MissingManifest,
            ..
        })
    ));
    ctx.read_failure = true;
    assert!(matches!(
        validate_manifests(&ctx, 1, &[first]),
        Err(WorkerError::ManifestStorage(_))
    ));
    ctx.tip = Some(0);
    assert!(matches!(
        validate_manifests(&ctx, 0, &[make_empty_manifest(1, 1)]),
        Err(WorkerError::ManifestPending {
            reason: ManifestPendingReason::NotBuried,
            ..
        })
    ));
}

#[test]
fn manifest_heights_are_checked_before_any_canonical_read() {
    let mut ctx = manifest_context(&[]);
    ctx.read_failure = true;
    let gapped = [make_empty_manifest(1, 1), make_empty_manifest(3, 1)];
    assert!(matches!(
        validate_manifests(&ctx, 0, &gapped),
        Err(WorkerError::StfExecution(
            ExecError::AsmManifestHeightMismatch {
                expected: 2,
                actual: 3,
                index: 1,
            }
        ))
    ));
    assert!(matches!(
        validate_manifests(&ctx, L1Height::MAX, &[make_empty_manifest(1, 1)]),
        Err(WorkerError::StfExecution(
            ExecError::AsmManifestHeightOverflow
        ))
    ));
}

#[test]
fn discontinuous_header_is_rejected_before_any_canonical_read() {
    /// Parent slot, child slot, child epoch, and a matcher for the expected rejection.
    type Case = (u64, u64, u32, fn(&ExecError) -> bool);
    const HUGE_SLOT: u64 = 1 << 63;
    let cases: [Case; 3] = [
        (0, 2, 1, |e| matches!(e, ExecError::SkipTooManySlots(0, 2))),
        (0, 1, 2, |e| matches!(e, ExecError::SkipEpochs(0, 2))),
        // Must not overflow the shared verifier's signed slot arithmetic.
        (1, HUGE_SLOT, 1, |e| {
            matches!(e, ExecError::SkipTooManySlots(1, HUGE_SLOT))
        }),
    ];
    for (parent_slot, child_slot, child_epoch, expected) in cases {
        let mut state = make_genesis_state();
        let genesis = run_genesis(&mut state);
        let mut parent_header = genesis.header().clone();
        let mut next_l1_height = 2;
        if parent_slot == 1 {
            let parent = execute_block(
                &mut state,
                &BlockInfo::new(1_001, 1, 1),
                Some(&parent_header),
                BlockComponents::new_manifests(vec![make_empty_manifest(2, 7)]),
            )
            .unwrap();
            parent_header = parent.header().clone();
            next_l1_height = 3;
        }
        let parent = parent_header.compute_block_commitment();
        let pre_state = state.clone().into_container();
        let completed = execute_block(
            &mut state,
            &BlockInfo::new(2_001, parent_slot + 1, 1),
            Some(&parent_header),
            BlockComponents::new_manifests(vec![make_empty_manifest(next_l1_height, 7)]),
        )
        .unwrap();
        let header = completed.header();
        let discontinuous = OLBlockHeaderV1::new(
            header.timestamp(),
            header.flags(),
            child_slot,
            child_epoch,
            *header.parent_blkid(),
            *header.body_root(),
            *header.state_root(),
            *header.logs_root(),
        );
        let block = OLBlockV1::new(
            SignedOLBlockHeaderV1::new(discontinuous, Buf64::zero()),
            completed.body().clone(),
        );
        let commitment = block.header().compute_block_commitment();

        // The canonical tip is below the carried manifest, so any canonical lookup would
        // defer the child as not yet buried.
        let mut ctx = manifest_context(&[]);
        ctx.tip = Some(1);
        ctx.blocks.insert(*commitment.blkid(), block);
        ctx.headers.insert(*parent.blkid(), parent_header);
        ctx.states.insert(parent, pre_state);
        let error = exec_block(&ctx, OLRuntimeParams::test_default(), &commitment)
            .expect_err("discontinuous header must be rejected");
        assert!(
            matches!(&error, WorkerError::StfExecution(actual) if expected(actual)),
            "unexpected error: {error}"
        );
        assert_eq!(*ctx.canonical_reads.lock().unwrap(), 0);
        assert!(ctx.indexed_epochs.lock().unwrap().is_empty());
        assert!(ctx.stored_summaries.lock().unwrap().is_empty());
    }
}

#[test]
fn forged_deposit_never_reaches_worker_persistence() {
    for terminal in [false, true] {
        let mut state = make_genesis_state();
        let account = seed_accounts(&mut state);
        let genesis = run_genesis(&mut state);
        let parent_header = genesis.header().clone();
        let parent = parent_header.compute_block_commitment();
        let pre_state = state.clone().into_container();
        let canonical = make_empty_manifest(2, 7);
        let deposit = make_deposit_manifest_for_account(
            2,
            7,
            account,
            SubjectId::from([1; 32]),
            BitcoinAmount::try_from(100).unwrap(),
        );
        // Keep the real L1 ID and witness root: only the logs are forged.
        let forged = AsmManifest::new(
            2,
            *canonical.blkid(),
            *canonical.wtxids_root(),
            deposit.logs().to_vec(),
        )
        .unwrap();
        let components = BlockComponents::new_manifests(vec![forged]);
        let components = if terminal {
            components.as_terminal()
        } else {
            components
        };
        let completed = execute_block(
            &mut state,
            &BlockInfo::new(1_001, 1, 1),
            Some(&parent_header),
            components,
        )
        .unwrap();
        let block = to_ol_block(&completed);
        let commitment = block.header().compute_block_commitment();
        let mut ctx = manifest_context(&[canonical]);
        ctx.blocks.insert(*commitment.blkid(), block);
        ctx.headers.insert(*parent.blkid(), parent_header);
        ctx.states.insert(parent, pre_state);
        assert!(matches!(
            exec_block(&ctx, OLRuntimeParams::test_default(), &commitment),
            Err(WorkerError::ManifestContentMismatch { height: 2 })
        ));
        assert!(ctx.indexed_epochs.lock().unwrap().is_empty());
        assert!(ctx.stored_summaries.lock().unwrap().is_empty());
        assert!(ctx.merged_epochs.lock().unwrap().is_empty());
    }
}
