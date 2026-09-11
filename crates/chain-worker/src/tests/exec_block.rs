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
use strata_db_types::{DbResult, errors::DbError};
use strata_identifiers::{
    Buf32, Epoch, EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
    SubjectId, WtxidsRoot,
};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_types_v1::{OLStateV1, WriteBatch};
use strata_ol_stf_v1::{
    BlockComponents, BlockInfo,
    test_utils::{
        EPOCH_RUNNER_TERMINAL_L1_HEIGHT as TERMINAL_L1_HEIGHT,
        epoch_runner_run_genesis as run_genesis, epoch_runner_run_terminal as run_terminal,
        epoch_runner_seed_accounts as seed_accounts, execute_block,
        make_deposit_manifest_for_account, make_empty_manifest, make_genesis_state, to_ol_block,
    },
};

use crate::{
    ManifestPendingReason, WorkerError, WorkerResult, output::OLBlockExecutionOutput,
    provenance::validate_manifests, state::exec_block, traits::ChainWorkerContext,
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
    states: HashMap<OLBlockCommitment, OLStateV1>,
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
    fn l1_reorg_safe_depth(&self) -> u32 {
        self.depth
    }
    fn canonical_l1_tip_height(&self) -> DbResult<Option<u32>> {
        Ok(self.tip)
    }
    fn canonical_manifest(&self, height: u32) -> DbResult<Option<AsmManifest>> {
        if self.read_failure {
            return Err(DbError::Other("injected read failure".into()));
        }
        Ok(self.manifests.get(&height).cloned())
    }

    fn runtime_params(&self) -> OLRuntimeParams {
        OLRuntimeParams::test_default()
    }

    fn fetch_block(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockV1>> {
        Ok(self.blocks.get(blkid).cloned())
    }

    fn fetch_header(&self, blkid: &OLBlockId) -> WorkerResult<Option<OLBlockHeaderV1>> {
        Ok(self.headers.get(blkid).cloned())
    }

    fn fetch_ol_state(&self, commitment: OLBlockCommitment) -> WorkerResult<Option<OLStateV1>> {
        Ok(self.states.get(&commitment).cloned())
    }

    fn fetch_canonical_epoch_summary_at(&self, epoch: Epoch) -> WorkerResult<Option<EpochSummary>> {
        Ok(self.canonical_summaries.get(&epoch).cloned())
    }

    fn store_block_output(
        &self,
        block: &OLBlockV1,
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
        _state: OLStateV1,
    ) -> WorkerResult<()> {
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
    ) -> WorkerResult<Option<WriteBatch>> {
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
        stored_summaries: Mutex::new(Vec::new()),
        merged_epochs: Mutex::new(Vec::new()),
    };

    exec_block(&ctx, OLRuntimeParams::test_default(), &terminal_commitment)
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
            Err(WorkerError::ManifestPending {
                reason: ManifestPendingReason::ContentMismatch,
                ..
            })
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
fn forged_deposit_never_reaches_worker_persistence() {
    for terminal in [false, true] {
        let mut state = make_genesis_state();
        let account = seed_accounts(&mut state);
        let genesis = run_genesis(&mut state);
        let parent_header = genesis.header().clone();
        let parent = parent_header.compute_block_commitment();
        let pre_state = state.clone().into_inner();
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
            Err(WorkerError::ManifestPending {
                reason: ManifestPendingReason::ContentMismatch,
                ..
            })
        ));
        assert!(ctx.indexed_epochs.lock().unwrap().is_empty());
        assert!(ctx.stored_summaries.lock().unwrap().is_empty());
        assert!(ctx.merged_epochs.lock().unwrap().is_empty());
    }
}
