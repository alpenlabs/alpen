//! Unit tests for the Checkpoint Sync Service (CSS).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use bitcoin::Amount;
use strata_btc_types::GenesisL1View;
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::CheckpointL1Ref;
use strata_csm_worker::CsmWorkerStatus;
use strata_db_types::DbResult;
use strata_identifiers::{
    Buf32, Epoch, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
};
use strata_l1_txfmt::MagicBytes;
use strata_params::{CredRule, ProofPublishMode, RollupParams};
use strata_predicate::PredicateKey;
use strata_primitives::{EpochCommitment, L1Height};
use strata_status::OLSyncStatus;

use crate::checkpoint_sync::{
    context::CheckpointSyncCtx,
    errors::{CheckpointSyncError, CheckpointSyncResult},
    state::{
        find_and_apply_unapplied_epochs, scan_unapplied_epochs, CheckpointSyncState, InnerState,
    },
};

fn make_buf32(v: u8) -> Buf32 {
    Buf32([v; 32])
}

fn make_blkid(v: u8) -> OLBlockId {
    OLBlockId::from(make_buf32(v))
}

fn make_l1_blkid(v: u8) -> L1BlockId {
    L1BlockId::from(make_buf32(v))
}

fn make_epoch(epoch: Epoch, slot: u64, id_byte: u8) -> EpochCommitment {
    EpochCommitment::new(epoch, slot, make_blkid(id_byte))
}

fn make_l1_commitment(height: u32, id_byte: u8) -> L1BlockCommitment {
    L1BlockCommitment::new(height, make_l1_blkid(id_byte))
}

fn make_l1_ref(height: u32) -> CheckpointL1Ref {
    CheckpointL1Ref::new(
        make_l1_commitment(height, height as u8),
        make_buf32(height as u8).0.into(),
        make_buf32(height as u8).0.into(),
    )
}

fn make_epoch_summary(
    epoch: Epoch,
    cur_epoch: EpochCommitment,
    prev_epoch: EpochCommitment,
    l1_height: u32,
) -> EpochSummary {
    EpochSummary::new(
        epoch,
        cur_epoch.to_block_commitment(),
        prev_epoch.to_block_commitment(),
        make_l1_commitment(l1_height, l1_height as u8),
        make_buf32(0xAA),
    )
}

fn test_rollup_params(reorg_safe_depth: u32) -> RollupParams {
    let genesis_l1_view = GenesisL1View {
        blk: make_l1_commitment(0, 0),
        next_target: 0,
        epoch_start_timestamp: 0,
        last_11_timestamps: [0; 11],
    };
    RollupParams {
        magic_bytes: MagicBytes::new(*b"TEST"),
        block_time: 1000,
        cred_rule: CredRule::Unchecked,
        genesis_l1_view,
        operators: vec![],
        evm_genesis_block_hash: Buf32([0; 32]),
        evm_genesis_block_state_root: Buf32([0; 32]),
        l1_reorg_safe_depth: reorg_safe_depth,
        target_l2_batch_size: 64,
        deposit_amount: Amount::from_sat(1_000_000_000),
        recovery_delay: 1008,
        checkpoint_predicate: PredicateKey::never_accept(),
        dispatch_assignment_dur: 64,
        proof_publish_mode: ProofPublishMode::Strict,
        max_deposits_in_block: 16,
        network: bitcoin::Network::Regtest,
    }
}

/// In-memory mock of [`CheckpointSyncCtx`] replacing storage, Bitcoin RPC, CSM
/// monitor, and chain worker with hash maps and vectors.
///
/// An epoch is considered "applied" iff it has an entry in `epoch_summaries`,
/// mirroring how the real chain worker writes an [`EpochSummary`] after
/// reconstructing the epoch's state.
struct MockCtx {
    rollup_params: RollupParams,
    /// Simulated L1 tip; used to compute reorg-safety of checkpoints.
    l1_tip_height: L1Height,
    /// CSM status returned by `fetch_csm_status`.
    csm_status: CsmWorkerStatus,
    /// Epoch number -> commitment lookup (canonical chain).
    epoch_commitments: HashMap<Epoch, EpochCommitment>,
    /// L1 reference for each finalized epoch's checkpoint tx.
    l1_refs: HashMap<EpochCommitment, CheckpointL1Ref>,
    /// Present iff the epoch has been applied. Behind a `Mutex` so
    /// `apply_checkpoint` can synthesize a summary with only `&self`.
    epoch_summaries: Mutex<HashMap<EpochCommitment, EpochSummary>>,
    /// Epochs passed to `apply_checkpoint`, in call order.
    applied_epochs: Mutex<Vec<EpochCommitment>>,
    /// Tips passed to `update_safe_tip`, in call order.
    safe_tips: Mutex<Vec<OLBlockCommitment>>,
    /// Epochs passed to `finalize_epoch`, in call order.
    finalized_epochs: Mutex<Vec<EpochCommitment>>,
    /// Statuses passed to `publish_ol_sync_status`, in call order.
    published_statuses: Mutex<Vec<OLSyncStatus>>,
}

impl MockCtx {
    fn new(reorg_safe_depth: u32, l1_tip_height: L1Height) -> Self {
        Self {
            rollup_params: test_rollup_params(reorg_safe_depth),
            l1_tip_height,
            csm_status: CsmWorkerStatus {
                cur_block: None,
                last_processed_epoch: None,
                last_confirmed_epoch: None,
                last_finalized_epoch: None,
            },
            epoch_commitments: HashMap::new(),
            l1_refs: HashMap::new(),
            epoch_summaries: Mutex::new(HashMap::new()),
            applied_epochs: Mutex::new(Vec::new()),
            safe_tips: Mutex::new(Vec::new()),
            finalized_epochs: Mutex::new(Vec::new()),
            published_statuses: Mutex::new(Vec::new()),
        }
    }

    fn with_csm_finalized(mut self, epoch: Option<EpochCommitment>) -> Self {
        self.csm_status.last_finalized_epoch = epoch;
        self
    }

    fn add_epoch(
        mut self,
        ec: EpochCommitment,
        l1_ref: CheckpointL1Ref,
        summary: Option<EpochSummary>,
    ) -> Self {
        self.epoch_commitments.insert(ec.epoch(), ec);
        self.l1_refs.insert(ec, l1_ref);
        if let Some(s) = summary {
            self.epoch_summaries.get_mut().unwrap().insert(ec, s);
        }
        self
    }
}

impl CheckpointSyncCtx for MockCtx {
    fn rollup_params(&self) -> &RollupParams {
        &self.rollup_params
    }

    async fn fetch_l1_tip_height(&self) -> CheckpointSyncResult<Option<L1Height>> {
        Ok(Some(self.l1_tip_height))
    }

    async fn fetch_csm_status(&self) -> CheckpointSyncResult<CsmWorkerStatus> {
        Ok(self.csm_status.clone())
    }

    async fn get_canonical_epoch_commitment(&self, ep: Epoch) -> DbResult<Option<EpochCommitment>> {
        Ok(self.epoch_commitments.get(&ep).copied())
    }

    async fn get_checkpoint_l1_ref(
        &self,
        epoch: EpochCommitment,
    ) -> DbResult<Option<CheckpointL1Ref>> {
        Ok(self.l1_refs.get(&epoch).cloned())
    }

    async fn get_epoch_summary(&self, epoch: EpochCommitment) -> DbResult<Option<EpochSummary>> {
        Ok(self.epoch_summaries.lock().unwrap().get(&epoch).copied())
    }

    async fn apply_checkpoint(&self, epoch: EpochCommitment) -> CheckpointSyncResult<()> {
        self.applied_epochs.lock().unwrap().push(epoch);

        // Mimic the real chain worker writing the summary after reconstruction,
        // so a multi-epoch catch-up does not re-collect the same epoch.
        let prev_epoch_num = epoch.epoch().saturating_sub(1);
        let prev_ec = self
            .epoch_commitments
            .get(&prev_epoch_num)
            .copied()
            .unwrap_or(EpochCommitment::null());
        let l1_ref = self.l1_refs.get(&epoch).expect("l1_ref for applied epoch");
        let summary = make_epoch_summary(epoch.epoch(), epoch, prev_ec, l1_ref.block_height());
        self.epoch_summaries.lock().unwrap().insert(epoch, summary);
        Ok(())
    }

    async fn update_safe_tip(&self, tip: OLBlockCommitment) -> CheckpointSyncResult<()> {
        self.safe_tips.lock().unwrap().push(tip);
        Ok(())
    }

    async fn finalize_epoch(&self, epoch: EpochCommitment) -> CheckpointSyncResult<()> {
        self.finalized_epochs.lock().unwrap().push(epoch);
        Ok(())
    }

    fn publish_ol_sync_status(&self, status: OLSyncStatus) {
        self.published_statuses.lock().unwrap().push(status);
    }
}

// ---- scan_unapplied_epochs ----

#[tokio::test]
async fn scan_walks_to_genesis_when_nothing_applied() {
    let epoch0 = make_epoch(0, 0, 0x00);
    let epoch1 = make_epoch(1, 10, 0x01);
    let epoch2 = make_epoch(2, 20, 0x02);
    let epoch3 = make_epoch(3, 30, 0x03);

    let ctx = MockCtx::new(3, 200)
        .add_epoch(epoch0, make_l1_ref(100), None)
        .add_epoch(epoch1, make_l1_ref(110), None)
        .add_epoch(epoch2, make_l1_ref(120), None)
        .add_epoch(epoch3, make_l1_ref(130), None);

    let (last_applied, unapplied) = scan_unapplied_epochs(&ctx, epoch3, 200, 3).await.unwrap();

    // Genesis is the stopping point; epochs 1..=3 are all unapplied, newest-first.
    assert_eq!(last_applied, Some(epoch0));
    assert_eq!(unapplied, vec![epoch3, epoch2, epoch1]);
}

#[tokio::test]
async fn scan_all_already_applied() {
    let epoch0 = make_epoch(0, 0, 0x00);
    let epoch1 = make_epoch(1, 10, 0x01);
    let epoch2 = make_epoch(2, 20, 0x02);

    let summary1 = make_epoch_summary(1, epoch1, epoch0, 110);
    let summary2 = make_epoch_summary(2, epoch2, epoch1, 120);

    let ctx = MockCtx::new(3, 200)
        .add_epoch(epoch0, make_l1_ref(100), None)
        .add_epoch(epoch1, make_l1_ref(110), Some(summary1))
        .add_epoch(epoch2, make_l1_ref(120), Some(summary2));

    let (last_applied, unapplied) = scan_unapplied_epochs(&ctx, epoch2, 200, 3).await.unwrap();

    assert_eq!(last_applied, Some(epoch2));
    assert!(unapplied.is_empty());
}

#[tokio::test]
async fn scan_collects_unapplied_newest_first_stops_at_applied() {
    let epoch0 = make_epoch(0, 0, 0x00);
    let epoch1 = make_epoch(1, 10, 0x01);
    let epoch2 = make_epoch(2, 20, 0x02);
    let epoch3 = make_epoch(3, 30, 0x03);

    let summary0 = make_epoch_summary(0, epoch0, EpochCommitment::null(), 100);
    let summary1 = make_epoch_summary(1, epoch1, epoch0, 110);

    let ctx = MockCtx::new(3, 200)
        .add_epoch(epoch0, make_l1_ref(100), Some(summary0))
        .add_epoch(epoch1, make_l1_ref(110), Some(summary1)) // applied upto here
        .add_epoch(epoch2, make_l1_ref(120), None)
        .add_epoch(epoch3, make_l1_ref(130), None);

    let (last_applied, unapplied) = scan_unapplied_epochs(&ctx, epoch3, 200, 3).await.unwrap();

    assert_eq!(last_applied, Some(epoch1));
    // Newest-first order.
    assert_eq!(unapplied, vec![epoch3, epoch2]);
}

#[tokio::test]
async fn scan_single_unapplied_epoch_above_genesis() {
    let epoch0 = make_epoch(0, 0, 0x00);
    let epoch1 = make_epoch(1, 10, 0x01);

    let ctx = MockCtx::new(3, 200)
        .add_epoch(epoch0, make_l1_ref(100), None)
        .add_epoch(epoch1, make_l1_ref(110), None);

    let (last_applied, unapplied) = scan_unapplied_epochs(&ctx, epoch1, 200, 3).await.unwrap();

    // Genesis is treated as applied.
    assert_eq!(last_applied, Some(epoch0));
    assert_eq!(unapplied, vec![epoch1]);
}

#[tokio::test]
async fn scan_errors_when_not_reorg_safe() {
    // l1_tip=105, l1_ref height=104, reorg_safe_depth=3 => depth=2 < 3.
    let epoch1 = make_epoch(1, 10, 0x01);
    let ctx = MockCtx::new(3, 105).add_epoch(epoch1, make_l1_ref(104), None);

    let err = scan_unapplied_epochs(&ctx, epoch1, 105, 3)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        CheckpointSyncError::NotReorgSafe {
            epoch,
            depth: 2,
            required: 3,
        } if epoch == epoch1
    ));
}

#[tokio::test]
async fn scan_errors_on_missing_l1_ref() {
    let epoch1 = make_epoch(1, 10, 0x01);
    let ctx = MockCtx::new(3, 200);

    let err = scan_unapplied_epochs(&ctx, epoch1, 200, 3)
        .await
        .unwrap_err();
    assert!(matches!(err, CheckpointSyncError::MissingL1Ref(e) if e == epoch1));
}

#[tokio::test]
async fn scan_errors_when_prev_epoch_missing_from_chain() {
    // epoch2 exists but epoch1 does not — broken invariant for a finalized chain.
    let epoch2 = make_epoch(2, 20, 0x02);
    let ctx = MockCtx::new(3, 200).add_epoch(epoch2, make_l1_ref(120), None);

    let err = scan_unapplied_epochs(&ctx, epoch2, 200, 3)
        .await
        .unwrap_err();
    assert!(matches!(err, CheckpointSyncError::MissingPredecessor(1)));
}

// ---- find_and_apply_unapplied_epochs ----

#[tokio::test]
async fn find_and_apply_multi_epoch_catch_up() {
    // Applied prefix ends at epoch1, CSM finalized at epoch3, epoch2 and
    // epoch3 present but unapplied. They must be applied oldest-first.
    let epoch0 = make_epoch(0, 0, 0x00);
    let epoch1 = make_epoch(1, 10, 0x01);
    let epoch2 = make_epoch(2, 20, 0x02);
    let epoch3 = make_epoch(3, 30, 0x03);

    let summary0 = make_epoch_summary(0, epoch0, EpochCommitment::null(), 100);
    let summary1 = make_epoch_summary(1, epoch1, epoch0, 110);

    let ctx = MockCtx::new(3, 200)
        .add_epoch(epoch0, make_l1_ref(100), Some(summary0))
        .add_epoch(epoch1, make_l1_ref(110), Some(summary1))
        .add_epoch(epoch2, make_l1_ref(120), None)
        .add_epoch(epoch3, make_l1_ref(130), None);

    let last_applied = find_and_apply_unapplied_epochs(&ctx, epoch3).await.unwrap();

    assert_eq!(last_applied, Some(epoch3));
    assert_eq!(*ctx.applied_epochs.lock().unwrap(), vec![epoch2, epoch3]);
    assert_eq!(*ctx.finalized_epochs.lock().unwrap(), vec![epoch2, epoch3]);
    assert_eq!(
        *ctx.safe_tips.lock().unwrap(),
        vec![epoch2.to_block_commitment(), epoch3.to_block_commitment()]
    );
}

// ---- handle_new_client_state ----

#[tokio::test]
async fn handle_skips_when_finalized_unchanged() {
    let epoch1 = make_epoch(1, 10, 0x01);
    let summary1 = make_epoch_summary(1, epoch1, make_epoch(0, 0, 0x00), 110);
    let ctx = Arc::new(
        MockCtx::new(3, 200)
            .add_epoch(epoch1, make_l1_ref(110), Some(summary1))
            .with_csm_finalized(Some(epoch1)),
    );
    let inner = InnerState::new(Some(epoch1));
    let mut state = CheckpointSyncState::new(ctx.clone(), inner);

    state.handle_new_client_state().await.unwrap();

    // No-op: inner state unchanged, no recorder activity.
    assert_eq!(state.last_finalized_and_applied(), Some(epoch1));
    assert!(ctx.applied_epochs.lock().unwrap().is_empty());
    assert!(ctx.safe_tips.lock().unwrap().is_empty());
    assert!(ctx.finalized_epochs.lock().unwrap().is_empty());
    assert!(ctx.published_statuses.lock().unwrap().is_empty());
}

#[tokio::test]
async fn handle_errors_on_chain_hole_leaves_state_unadvanced() {
    // CSM jumps finalized to epoch3 but epoch2 is missing from the canonical
    // chain. handle_new_client_state must error, leave inner state on epoch1,
    // and produce no chain-worker side effects.
    let epoch0 = make_epoch(0, 0, 0x00);
    let epoch1 = make_epoch(1, 10, 0x01);
    let epoch3 = make_epoch(3, 30, 0x03);

    let summary0 = make_epoch_summary(0, epoch0, EpochCommitment::null(), 100);
    let summary1 = make_epoch_summary(1, epoch1, epoch0, 110);

    let ctx = Arc::new(
        MockCtx::new(3, 200)
            .add_epoch(epoch0, make_l1_ref(100), Some(summary0))
            .add_epoch(epoch1, make_l1_ref(110), Some(summary1))
            .add_epoch(epoch3, make_l1_ref(130), None)
            .with_csm_finalized(Some(epoch3)),
    );
    let inner = InnerState::new(Some(epoch1));
    let mut state = CheckpointSyncState::new(ctx.clone(), inner);

    let err = state.handle_new_client_state().await.unwrap_err();
    assert!(matches!(err, CheckpointSyncError::MissingPredecessor(2)));

    assert_eq!(state.last_finalized_and_applied(), Some(epoch1));
    assert!(ctx.applied_epochs.lock().unwrap().is_empty());
    assert!(ctx.safe_tips.lock().unwrap().is_empty());
    assert!(ctx.finalized_epochs.lock().unwrap().is_empty());
}

// ---- restart / re-run ----

#[tokio::test]
async fn rerun_after_partial_drain_applies_nothing() {
    // First run applies epoch2 and epoch3 (mock synthesizes their summaries).
    // A second run with the same finalized epoch must apply nothing because
    // the scan now stops at the already-applied summaries.
    let epoch0 = make_epoch(0, 0, 0x00);
    let epoch1 = make_epoch(1, 10, 0x01);
    let epoch2 = make_epoch(2, 20, 0x02);
    let epoch3 = make_epoch(3, 30, 0x03);

    let summary0 = make_epoch_summary(0, epoch0, EpochCommitment::null(), 100);
    let summary1 = make_epoch_summary(1, epoch1, epoch0, 110);

    let ctx = MockCtx::new(3, 200)
        .add_epoch(epoch0, make_l1_ref(100), Some(summary0))
        .add_epoch(epoch1, make_l1_ref(110), Some(summary1))
        .add_epoch(epoch2, make_l1_ref(120), None)
        .add_epoch(epoch3, make_l1_ref(130), None);

    let last_applied = find_and_apply_unapplied_epochs(&ctx, epoch3).await.unwrap();
    assert_eq!(last_applied, Some(epoch3));
    let applied_after_first = ctx.applied_epochs.lock().unwrap().len();
    assert_eq!(applied_after_first, 2);

    let last_applied = find_and_apply_unapplied_epochs(&ctx, epoch3).await.unwrap();
    assert_eq!(last_applied, Some(epoch3));
    assert_eq!(
        ctx.applied_epochs.lock().unwrap().len(),
        applied_after_first
    );
}
