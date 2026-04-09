use anyhow::anyhow;
use strata_chain_worker_new::FinalizedCkptPayload;
use strata_csm_types::CheckpointL1Ref;
use strata_db_types::DbError;
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::OLL1ManifestContainer;
use strata_ol_state_types::OLState;
use strata_primitives::EpochCommitment;
use strata_service::ServiceState;
use strata_status::OLSyncStatus;
use tracing::{debug, info};

use crate::checkpoint_sync::{
    context::CheckpointSyncCtx, service::find_and_apply_unapplied_epochs,
};

#[derive(Debug, Clone)]
pub struct CheckpointSyncState<C: CheckpointSyncCtx> {
    ctx: C,
    inner: InnerState,
}

#[derive(Clone, Debug)]
pub(crate) struct InnerState {
    last_finalized_and_applied: Option<EpochCommitment>,
}

impl InnerState {
    pub(crate) fn new(last_finalized_epoch: Option<EpochCommitment>) -> Self {
        Self {
            last_finalized_and_applied: last_finalized_epoch,
        }
    }

    pub(crate) fn last_finalized_epoch(&self) -> Option<EpochCommitment> {
        self.last_finalized_and_applied
    }
}

impl<C: CheckpointSyncCtx> CheckpointSyncState<C> {
    pub(crate) fn new(ctx: C, inner: InnerState) -> Self {
        Self { ctx, inner }
    }

    pub(crate) async fn handle_new_client_state(&mut self) -> Result<(), anyhow::Error> {
        let csm_status = self.ctx.fetch_csm_status().await?;
        debug!(?csm_status, "Obtained csm status");
        let new_finalized = csm_status.last_finalized_epoch;
        let new_finalized = match (self.inner.last_finalized_and_applied, new_finalized) {
            (_, None) => {
                debug!("no finalized epoch in CSM status, skipping");
                return Ok(());
            }
            (None, Some(new_fin)) => {
                info!(%new_fin, "first finalized epoch observed");
                new_fin
            }
            (Some(prev), Some(new_fin)) => {
                if prev == new_fin {
                    debug!(%prev, "finalized epoch unchanged, skipping");
                    return Ok(());
                };
                debug!(%prev, %new_fin, "new finalized epoch");
                new_fin
            }
        };

        let l1_ref = self
            .ctx
            .fetch_l1_reference(new_finalized)
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "L1 reference not found for finalized epoch: {}",
                    new_finalized
                )
            })?;

        debug!(
            %new_finalized,
            l1_height = l1_ref.block_height(),
            "checking previous unapplied and applying new finalized checkpoint"
        );

        let last_applied = find_and_apply_unapplied_epochs(&self.ctx, new_finalized).await?;

        // Update internal state
        self.inner.last_finalized_and_applied = last_applied;
        info!(?last_applied, "checkpoint sync advanced");

        Ok(())
    }
}

pub(crate) async fn apply_checkpoint(
    ctx: &impl CheckpointSyncCtx,
    epoch: EpochCommitment,
    l1ref: CheckpointL1Ref,
) -> anyhow::Result<()> {
    debug!(%epoch, "extracting DA and submitting to chain worker");
    extract_checkpoint_and_submit_to_chain_worker(epoch, l1ref, ctx).await?;

    let blk = epoch.to_block_commitment();

    debug!(%epoch, "updating safe tip");
    ctx.update_safe_tip(blk).await?;

    debug!(%epoch, "finalizing epoch");
    ctx.finalize_epoch(epoch).await?;

    debug!(%epoch, "building ol sync status after finalizing epoch");
    let status = build_ol_sync_status(ctx, epoch).await?;
    ctx.publish_ol_sync_status(status);

    info!(%epoch, "checkpoint applied and finalized");

    Ok(())
}

async fn extract_checkpoint_and_submit_to_chain_worker<C: CheckpointSyncCtx>(
    new_epoch: EpochCommitment,
    l1ref: CheckpointL1Ref,
    ctx: &C,
) -> anyhow::Result<()> {
    let prev_epoch_num = new_epoch.epoch().saturating_sub(1);
    let prev_epoch = ctx
        .get_canonical_epoch_commitment(prev_epoch_num)
        .await?
        .ok_or_else(|| anyhow!("Expected epoch not found in db: {}", prev_epoch_num))?;
    let prev_terminal = prev_epoch.to_block_commitment();

    let prev_state: OLState = ctx.get_state_at(prev_terminal).await?;

    let manifest_start = prev_state.last_l1_height().saturating_add(1);
    let manifest_end = l1ref.l1_commitment.height();
    debug!(
        %new_epoch,
        l1_range = %format!("{manifest_start}..={manifest_end}"),
        "fetching ASM manifests"
    );

    let manifests = ctx
        .fetch_asm_manifests_range(manifest_start, manifest_end)
        .await?;

    debug!(
        %new_epoch,
        num_manifests = manifests.len(),
        "fetched ASM manifests, extracting DA"
    );

    let container = OLL1ManifestContainer::new(manifests)?;

    let da = ctx.extract_da_data(&l1ref).await?;
    let (da_payload, terminal_complement) = da.into_parts();

    let payload = FinalizedCkptPayload::new(da_payload, container, new_epoch, terminal_complement);

    debug!(%new_epoch, "submitting DA payload to chain worker");
    ctx.apply_da(&payload).await?;

    Ok(())
}

/// Builds an [`OLSyncStatus`] from a finalized epoch.
pub(crate) async fn build_ol_sync_status(
    ctx: &impl CheckpointSyncCtx,
    epoch: EpochCommitment,
) -> anyhow::Result<OLSyncStatus> {
    let summary = ctx
        .get_epoch_summary(epoch)
        .await?
        .ok_or(DbError::NonExistentEntry)?;
    let terminal = *summary.terminal();
    let epoch_num = summary.epoch();
    let new_l1 = *summary.new_l1();
    let prev_epoch = summary
        .get_prev_epoch_commitment()
        .unwrap_or(EpochCommitment::null());

    // checkpoint sync always lands on terminal blocks and
    // confirmed = finalized for checkpoint sync(5th and 6th args)
    Ok(OLSyncStatus::new(
        terminal, epoch_num, true, prev_epoch, epoch, epoch, new_l1,
    ))
}

impl<C> ServiceState for CheckpointSyncState<C>
where
    C: CheckpointSyncCtx + 'static,
{
    fn name(&self) -> &str {
        "checkpoint-sync"
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use bitcoin::Amount;
    use strata_btc_types::GenesisL1View;
    use strata_chain_worker_new::FinalizedCkptPayload;
    use strata_checkpoint_types::EpochSummary;
    use strata_csm_worker::CsmWorkerStatus;
    use strata_db_types::DbResult;
    use strata_identifiers::{
        Buf32, CheckpointL1Ref, Epoch, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
    };
    use strata_l1_txfmt::MagicBytes;
    use strata_params::{CredRule, ProofPublishMode, RollupParams};
    use strata_predicate::PredicateKey;
    use strata_primitives::{EpochCommitment, L1Height};
    use strata_status::OLSyncStatus;

    use super::*;
    use crate::checkpoint_sync::service::scan_unapplied_epochs;

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
            make_buf32(height as u8),
            make_buf32(height as u8),
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
            checkpoint_predicate: PredicateKey::never_accept(),
            dispatch_assignment_dur: 64,
            proof_publish_mode: ProofPublishMode::Strict,
            max_deposits_in_block: 16,
            network: bitcoin::Network::Regtest,
            recovery_delay: 1008,
        }
    }

    /// Records chain worker operations invoked during a test run.
    #[derive(Debug, Default)]
    struct ChainWorkerCalls {
        apply_da_epochs: Vec<EpochCommitment>,
        safe_tips: Vec<OLBlockCommitment>,
        finalized_epochs: Vec<EpochCommitment>,
    }

    /// In-memory mock of [`CheckpointSyncCtx`] that replaces storage, Bitcoin RPC,
    /// CSM monitor, and chain worker with simple hash maps and vectors.
    ///
    /// An epoch is considered "applied" iff it has an entry in `epoch_summaries`,
    /// mirroring how the real system marks application via chain worker inserting
    /// an [`EpochSummary`].
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
        /// Present iff the epoch has been applied (DA executed by chain worker).
        epoch_summaries: HashMap<EpochCommitment, EpochSummary>,
        /// Collects sync statuses published during the test.
        published_statuses: Mutex<Vec<OLSyncStatus>>,
        /// Collects chain worker calls for assertion.
        chain_worker_calls: Mutex<ChainWorkerCalls>,
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
                epoch_summaries: HashMap::new(),
                published_statuses: Mutex::new(Vec::new()),
                chain_worker_calls: Mutex::new(ChainWorkerCalls::default()),
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
                self.epoch_summaries.insert(ec, s);
            }
            self
        }
    }

    impl CheckpointSyncCtx for MockCtx {
        fn rollup_params(&self) -> &RollupParams {
            &self.rollup_params
        }

        async fn fetch_l1_tip_height(&self) -> anyhow::Result<L1Height> {
            Ok(self.l1_tip_height)
        }

        async fn fetch_csm_status(&self) -> anyhow::Result<CsmWorkerStatus> {
            Ok(self.csm_status.clone())
        }

        async fn get_canonical_epoch_commitment(
            &self,
            ep: Epoch,
        ) -> DbResult<Option<EpochCommitment>> {
            Ok(self.epoch_commitments.get(&ep).copied())
        }

        async fn get_checkpoint_l1_ref(
            &self,
            epoch: EpochCommitment,
        ) -> DbResult<Option<CheckpointL1Ref>> {
            Ok(self.l1_refs.get(&epoch).cloned())
        }

        async fn get_epoch_summary(
            &self,
            epoch: EpochCommitment,
        ) -> DbResult<Option<EpochSummary>> {
            Ok(self.epoch_summaries.get(&epoch).copied())
        }

        async fn extract_da_data(
            &self,
            _ckpt_ref: &CheckpointL1Ref,
        ) -> anyhow::Result<strata_ol_da::ExtractedDA> {
            unimplemented!("not needed for scan/status tests")
        }

        async fn get_state_at(
            &self,
            _blkid: OLBlockCommitment,
        ) -> anyhow::Result<strata_ol_state_types::OLState> {
            unimplemented!("not needed for scan/status tests")
        }

        async fn fetch_asm_manifests_range(
            &self,
            _start: L1Height,
            _end: L1Height,
        ) -> anyhow::Result<Vec<strata_asm_common::AsmManifest>> {
            unimplemented!("not needed for scan/status tests")
        }

        fn publish_ol_sync_status(&self, status: OLSyncStatus) {
            self.published_statuses.lock().unwrap().push(status);
        }

        async fn fetch_l1_reference(
            &self,
            epoch: EpochCommitment,
        ) -> anyhow::Result<Option<CheckpointL1Ref>> {
            Ok(self.l1_refs.get(&epoch).cloned())
        }

        async fn apply_da(&self, payload: &FinalizedCkptPayload) -> anyhow::Result<()> {
            self.chain_worker_calls
                .lock()
                .unwrap()
                .apply_da_epochs
                .push(payload.epoch());
            Ok(())
        }

        async fn update_safe_tip(&self, tip: OLBlockCommitment) -> anyhow::Result<()> {
            self.chain_worker_calls.lock().unwrap().safe_tips.push(tip);
            Ok(())
        }

        async fn finalize_epoch(&self, epoch: EpochCommitment) -> anyhow::Result<()> {
            self.chain_worker_calls
                .lock()
                .unwrap()
                .finalized_epochs
                .push(epoch);
            Ok(())
        }
    }

    // ---- InnerState ----

    #[test]
    fn inner_state_none() {
        let s = InnerState::new(None);
        assert!(s.last_finalized_epoch().is_none());
    }

    #[test]
    fn inner_state_some() {
        let ec = make_epoch(5, 50, 0x55);
        let s = InnerState::new(Some(ec));
        assert_eq!(s.last_finalized_epoch(), Some(ec));
    }

    // ---- scan_unapplied_epochs ----

    #[tokio::test]
    async fn scan_stops_at_genesis_epoch() {
        let epoch0 = make_epoch(0, 0, 0x00);
        let epoch0_sum = make_epoch_summary(0, epoch0, EpochCommitment::null(), 110);
        let ctx = MockCtx::new(3, 200).add_epoch(epoch0, make_l1_ref(100), Some(epoch0_sum));

        let (last_applied, unapplied) = scan_unapplied_epochs(&ctx, epoch0, 200, 3).await.unwrap();

        assert_eq!(last_applied, Some(epoch0));
        assert!(unapplied.is_empty());
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

        // epoch1 has summary (applied), epoch2 and epoch3 do not.
        let summary1 = make_epoch_summary(1, epoch1, epoch0, 110);

        let ctx = MockCtx::new(3, 200)
            .add_epoch(epoch0, make_l1_ref(100), None)
            .add_epoch(epoch1, make_l1_ref(110), Some(summary1))
            .add_epoch(epoch2, make_l1_ref(120), None)
            .add_epoch(epoch3, make_l1_ref(130), None);

        let (last_applied, unapplied) = scan_unapplied_epochs(&ctx, epoch3, 200, 3).await.unwrap();

        assert_eq!(last_applied, Some(epoch1));
        // Newest-first order.
        assert_eq!(unapplied.len(), 2);
        assert_eq!(unapplied[0].1, epoch3);
        assert_eq!(unapplied[1].1, epoch2);
    }

    #[tokio::test]
    async fn scan_errors_on_unfinalized_ancestor() {
        let epoch1 = make_epoch(1, 10, 0x01);
        // l1_tip=105, l1_ref height=104, reorg_safe_depth=3 => confs=1 < 3
        let ctx = MockCtx::new(3, 105).add_epoch(epoch1, make_l1_ref(104), None);

        let result = scan_unapplied_epochs(&ctx, epoch1, 105, 3).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unfinalized"));
    }

    #[tokio::test]
    async fn scan_errors_on_missing_l1_ref() {
        let epoch1 = make_epoch(1, 10, 0x01);
        let ctx = MockCtx::new(3, 200);

        let result = scan_unapplied_epochs(&ctx, epoch1, 200, 3).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn scan_errors_when_prev_epoch_missing_from_db() {
        // epoch2 exists but epoch1 does not — broken invariant for finalized chain.
        let epoch2 = make_epoch(2, 20, 0x02);
        let ctx = MockCtx::new(3, 200).add_epoch(epoch2, make_l1_ref(120), None);

        let result = scan_unapplied_epochs(&ctx, epoch2, 200, 3).await;
        assert!(result.is_err());
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
        assert_eq!(unapplied.len(), 1);
        assert_eq!(unapplied[0].1, epoch1);
    }

    // ---- build_ol_sync_status ----

    #[tokio::test]
    async fn build_status_non_genesis() {
        let epoch0 = make_epoch(0, 0, 0x00);
        let epoch1 = make_epoch(1, 10, 0x01);

        let summary = make_epoch_summary(1, epoch1, epoch0, 110);

        let ctx = MockCtx::new(3, 200).add_epoch(epoch1, make_l1_ref(110), Some(summary));

        let status = build_ol_sync_status(&ctx, epoch1).await.unwrap();

        assert_eq!(status.tip, epoch1.to_block_commitment());
        assert_eq!(status.tip_epoch, 1);
        assert!(status.tip_is_terminal);
        // confirmed == finalized == epoch for checkpoint sync
        assert_eq!(status.confirmed_epoch, epoch1);
        assert_eq!(status.finalized_epoch, epoch1);
        assert_eq!(status.prev_epoch.epoch(), 0);
    }

    #[tokio::test]
    async fn build_status_genesis_uses_null_prev() {
        let epoch0 = make_epoch(0, 0, 0x00);
        let summary = make_epoch_summary(0, epoch0, EpochCommitment::null(), 100);

        let ctx = MockCtx::new(3, 200).add_epoch(epoch0, make_l1_ref(100), Some(summary));
        let status = build_ol_sync_status(&ctx, epoch0).await.unwrap();

        assert_eq!(status.tip_epoch, 0);
        assert_eq!(status.prev_epoch, EpochCommitment::null());
    }

    #[tokio::test]
    async fn build_status_errors_on_missing_summary() {
        let epoch1 = make_epoch(1, 10, 0x01);
        let ctx = MockCtx::new(3, 200).add_epoch(epoch1, make_l1_ref(110), None);

        assert!(build_ol_sync_status(&ctx, epoch1).await.is_err());
    }

    // ---- handle_new_client_state ----

    #[tokio::test]
    async fn handle_skips_when_no_finalized_epoch() {
        let ctx = MockCtx::new(3, 200);
        let inner = InnerState::new(None);
        let mut state = CheckpointSyncState::new(ctx, inner);

        state.handle_new_client_state().await.unwrap();
        assert!(state.inner.last_finalized_and_applied.is_none());
    }

    #[tokio::test]
    async fn handle_skips_when_finalized_unchanged() {
        let epoch1 = make_epoch(1, 10, 0x01);
        let ctx = MockCtx::new(3, 200).with_csm_finalized(Some(epoch1));
        let inner = InnerState::new(Some(epoch1));
        let mut state = CheckpointSyncState::new(ctx, inner);

        state.handle_new_client_state().await.unwrap();
        assert_eq!(state.inner.last_finalized_and_applied, Some(epoch1));
    }

    #[tokio::test]
    async fn handle_errors_when_l1_ref_missing() {
        let epoch1 = make_epoch(1, 10, 0x01);
        let ctx = MockCtx::new(3, 200).with_csm_finalized(Some(epoch1));
        let inner = InnerState::new(None);
        let mut state = CheckpointSyncState::new(ctx, inner);

        let result = state.handle_new_client_state().await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("L1 reference not found"));
    }

    // ---- find_and_apply_unapplied_epochs ----

    #[tokio::test]
    async fn find_and_apply_returns_none_when_no_epochs() {
        // Finalized epoch has no l1_ref, so scan will error.
        // But if finalized is genesis, it should return Some(genesis).
        let epoch0 = make_epoch(0, 0, 0x00);
        let ctx = MockCtx::new(3, 200).add_epoch(epoch0, make_l1_ref(100), None);

        let result = find_and_apply_unapplied_epochs(&ctx, epoch0).await.unwrap();
        // Genesis is treated as already applied, nothing to apply.
        assert_eq!(result, Some(epoch0));
    }
}
