//! Submission tracker state: scans writer bundles and records confirmed-but-rejected checkpoints.

use std::collections::BTreeMap;

use metrics::{counter, gauge};
use strata_csm_types::CheckpointState;
use strata_db_types::common::L1TxId;
use strata_db_types::l1_writer::BundleIdx;
use strata_db_types::ol_checkpoint::RejectedCheckpointEntry;
use strata_identifiers::{Epoch, EpochCommitment, L1BlockCommitment};
use strata_service::ServiceState;
use tracing::{debug, warn};

use super::classify::{SubmissionVerdict, classify_submission};
use super::context::SubmissionTrackerContext;

/// State of the checkpoint submission tracker.
///
/// Persists a cursor over the L1 writer's bundles: every bundle below it is settled, meaning it
/// carries no checkpoint, its epoch's acceptance is final, or its rejection is recorded. The
/// writer only appends bundles, and reconciling local checkpoint artifacts at startup leaves
/// them in place, so a restart resumes from the cursor and cannot lose a submission.
pub(crate) struct SubmissionTrackerState<C: SubmissionTrackerContext> {
    ctx: C,
    l1_reorg_safe_depth: u32,

    /// Latest checkpoint state the CSM published, re-checked on every tick.
    latest: CheckpointState,

    /// First bundle index not yet settled.
    cursor: BundleIdx,

    /// Recorded rejections, keyed by the transaction that was mined.
    rejected: BTreeMap<L1TxId, RejectedCheckpointEntry>,
}

impl<C: SubmissionTrackerContext> SubmissionTrackerState<C> {
    /// Loads the cursor and recorded rejections from `ctx`.
    pub(crate) fn new(
        ctx: C,
        l1_reorg_safe_depth: u32,
        latest: CheckpointState,
    ) -> anyhow::Result<Self> {
        let cursor = ctx.scan_cursor()?.unwrap_or(0);
        let rejected = ctx
            .rejected_checkpoints()?
            .into_iter()
            .map(|entry| (entry.txid(), entry))
            .collect();
        Ok(Self {
            ctx,
            l1_reorg_safe_depth,
            latest,
            cursor,
            rejected,
        })
    }

    pub(crate) fn set_latest(&mut self, latest: CheckpointState) {
        self.latest = latest;
    }

    pub(crate) fn cursor(&self) -> BundleIdx {
        self.cursor
    }

    /// Recorded rejections whose epoch the ASM has still not accepted.
    pub(crate) fn unresolved_rejections(&self) -> usize {
        let verified_epoch = self.verified_tip().map(|tip| tip.epoch());
        self.rejected
            .values()
            .filter(|entry| verified_epoch.is_none_or(|verified| entry.epoch() > verified))
            .count()
    }

    fn verified_tip(&self) -> Option<EpochCommitment> {
        self.latest.client_state.get_last_epoch()
    }

    /// Last epoch whose acceptance is buried at the reorg-safe depth.
    fn finalized_epoch(&self) -> Option<Epoch> {
        self.latest
            .client_state
            .get_declared_final_epoch()
            .map(|tip| tip.epoch())
    }

    /// Checks every unsettled bundle against the latest CSM checkpoint state.
    ///
    /// New rejections and the advanced cursor are committed together, and each rejection is
    /// logged once, after the commit. An orderly restart therefore never alerts twice; a crash
    /// before the database flushes can repeat one warning.
    pub(crate) fn evaluate(&mut self) -> anyhow::Result<()> {
        let csm_tip = self.latest.block;
        if !self.latest.has_genesis_occurred() {
            return Ok(());
        }
        // A non-canonical CSM tip means the CSM has not followed a reorg yet; its verified tip
        // could belong to the abandoned branch.
        if !self.is_canonical(&csm_tip)? {
            debug!(%csm_tip, "CSM tip is not canonical; deferring checkpoint submission check");
            return Ok(());
        }
        let verified_tip = self.verified_tip();

        let next_idx = self.ctx.next_bundle_idx()?;
        let mut cursor = self.cursor;
        let mut new_rejections = Vec::new();
        for idx in self.cursor..next_idx {
            let settled = self.check_bundle(idx, csm_tip, verified_tip, &mut new_rejections)?;
            if settled && cursor == idx {
                cursor = idx + 1;
            }
        }

        if cursor != self.cursor || !new_rejections.is_empty() {
            // Re-check right before committing, so a reorg that landed during the scan does not
            // turn into a recorded rejection. Reorgs deeper than the reorg-safe depth violate
            // CSM finality and are out of scope.
            if !new_rejections.is_empty() && !self.is_canonical(&csm_tip)? {
                return Ok(());
            }
            self.ctx.commit_scan(cursor, new_rejections.clone())?;
            self.cursor = cursor;
            for entry in new_rejections {
                warn!(
                    epoch = entry.epoch(),
                    commitment = %entry.commitment(),
                    txid = ?entry.txid(),
                    l1_block = %entry.l1_block(),
                    asm_verified_tip = ?entry.asm_verified_tip(),
                    "checkpoint confirmed on L1 but rejected by ASM"
                );
                counter!("strata_checkpoint_confirmed_rejected_total").increment(1);
                self.rejected.insert(entry.txid(), entry);
            }
        }

        gauge!("strata_checkpoint_confirmed_rejected").set(self.unresolved_rejections() as f64);
        Ok(())
    }

    /// Classifies bundle `idx`, appending a newly found rejection. Returns whether it is settled.
    fn check_bundle(
        &self,
        idx: BundleIdx,
        csm_tip: L1BlockCommitment,
        verified_tip: Option<EpochCommitment>,
        new_rejections: &mut Vec<RejectedCheckpointEntry>,
    ) -> anyhow::Result<bool> {
        let Some(bundle) = self.ctx.checkpoint_bundle(idx)? else {
            return Ok(true);
        };
        let reveal = match bundle.reveal_txid {
            Some(txid) => self.ctx.resolve_reveal(txid)?,
            None => None,
        };
        if reveal.is_some_and(|reveal| self.rejected.contains_key(&reveal.txid)) {
            return Ok(true);
        }

        let mut mined = None;
        if let Some(reveal) = reveal
            && let Some(block) = reveal.inclusion
            && self.is_canonical(&block)?
        {
            mined = Some((reveal.txid, block));
        }

        let verdict = classify_submission(
            bundle.commitment.epoch(),
            mined.map(|(_, block)| block.height()),
            csm_tip.height(),
            verified_tip.map(|tip| tip.epoch()),
            self.l1_reorg_safe_depth,
        );
        Ok(match (verdict, mined) {
            // A shallow reorg can undo an acceptance, so only a final one settles the bundle.
            (SubmissionVerdict::EpochAccepted, _) => self
                .finalized_epoch()
                .is_some_and(|finalized| finalized >= bundle.commitment.epoch()),
            (SubmissionVerdict::ConfirmedRejected, Some((txid, block))) => {
                new_rejections.push(RejectedCheckpointEntry::new(
                    bundle.commitment,
                    txid,
                    block,
                    verified_tip,
                ));
                true
            }
            _ => false,
        })
    }

    fn is_canonical(&self, block: &L1BlockCommitment) -> anyhow::Result<bool> {
        Ok(self.ctx.canonical_l1_block(block.height())? == Some(*block.blkid()))
    }
}

impl<C: SubmissionTrackerContext> ServiceState for SubmissionTrackerState<C> {
    fn name(&self) -> &str {
        "checkpoint_submission_tracker"
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, MutexGuard};

    use strata_asm_checkpoint_types::CheckpointTip;
    use strata_csm_types::{CheckpointL1Ref, ClientState, L1Checkpoint};
    use strata_identifiers::{Buf32, L1BlockId, L1Height, OLBlockCommitment, OLBlockId, RBuf32};

    use super::*;
    use crate::submission_tracker::context::{CheckpointBundle, RevealStatus};

    const DEPTH: u32 = 3;

    #[derive(Default)]
    struct StubData {
        /// Writer bundles by index; `None` stands for a non-checkpoint bundle.
        bundles: Vec<Option<CheckpointBundle>>,
        reveals: HashMap<L1TxId, RevealStatus>,
        canonical: HashMap<L1Height, L1BlockId>,
        cursor: Option<BundleIdx>,
        rejected: Vec<RejectedCheckpointEntry>,
        commits_with_rejections: usize,
    }

    /// In-memory context; clones share data, so a clone stands in for a restarted node.
    #[derive(Clone, Default)]
    struct StubCtx(Arc<Mutex<StubData>>);

    impl StubCtx {
        fn data(&self) -> MutexGuard<'_, StubData> {
            self.0.lock().unwrap()
        }

        /// Adds a signed checkpoint bundle for `epoch` and returns its reveal txid.
        fn push_checkpoint(&self, epoch: Epoch) -> L1TxId {
            let mut data = self.data();
            let txid = RBuf32::from([data.bundles.len() as u8 + 1; 32]);
            data.bundles.push(Some(CheckpointBundle {
                commitment: commitment(epoch),
                reveal_txid: Some(txid),
            }));
            data.reveals.insert(
                txid,
                RevealStatus {
                    txid,
                    inclusion: None,
                },
            );
            txid
        }

        /// Makes the broadcaster report `txid` in `block` and marks `block` canonical.
        fn mine(&self, txid: L1TxId, block: L1BlockCommitment) {
            let mut data = self.data();
            data.reveals.get_mut(&txid).unwrap().inclusion = Some(block);
            data.canonical.insert(block.height(), *block.blkid());
        }

        fn set_canonical(&self, block: L1BlockCommitment) {
            self.data().canonical.insert(block.height(), *block.blkid());
        }
    }

    impl SubmissionTrackerContext for StubCtx {
        fn scan_cursor(&self) -> anyhow::Result<Option<BundleIdx>> {
            Ok(self.data().cursor)
        }

        fn rejected_checkpoints(&self) -> anyhow::Result<Vec<RejectedCheckpointEntry>> {
            Ok(self.data().rejected.clone())
        }

        fn commit_scan(
            &self,
            cursor: BundleIdx,
            rejected: Vec<RejectedCheckpointEntry>,
        ) -> anyhow::Result<()> {
            let mut data = self.data();
            data.cursor = Some(cursor);
            if !rejected.is_empty() {
                data.commits_with_rejections += 1;
            }
            data.rejected.extend(rejected);
            Ok(())
        }

        fn next_bundle_idx(&self) -> anyhow::Result<BundleIdx> {
            Ok(self.data().bundles.len() as BundleIdx)
        }

        fn checkpoint_bundle(&self, idx: BundleIdx) -> anyhow::Result<Option<CheckpointBundle>> {
            Ok(self.data().bundles[idx as usize])
        }

        fn resolve_reveal(&self, txid: L1TxId) -> anyhow::Result<Option<RevealStatus>> {
            Ok(self.data().reveals.get(&txid).copied())
        }

        fn canonical_l1_block(&self, height: L1Height) -> anyhow::Result<Option<L1BlockId>> {
            Ok(self.data().canonical.get(&height).copied())
        }
    }

    /// Epochs of the recorded rejections, in txid order.
    fn rejected_epochs(state: &SubmissionTrackerState<StubCtx>) -> Vec<Epoch> {
        state.rejected.values().map(|entry| entry.epoch()).collect()
    }

    fn commitment(epoch: Epoch) -> EpochCommitment {
        EpochCommitment::new(
            epoch,
            u64::from(epoch) * 4,
            OLBlockId::from(Buf32::from([epoch as u8; 32])),
        )
    }

    fn block(height: L1Height, fork: u8) -> L1BlockCommitment {
        L1BlockCommitment::new(height, L1BlockId::from(Buf32::from([fork; 32])))
    }

    fn l1_checkpoint(epoch: Epoch, observed: L1BlockCommitment) -> L1Checkpoint {
        let terminal = commitment(epoch);
        let tip = CheckpointTip {
            epoch,
            l1_height: observed.height() - 10,
            l2_commitment: OLBlockCommitment::new(terminal.last_slot(), *terminal.last_blkid()),
        };
        L1Checkpoint::new(
            tip,
            CheckpointL1Ref::new(observed, RBuf32::zero(), RBuf32::zero()),
        )
    }

    /// CSM state at `tip` (made canonical) with the last accepted and last finalized epochs.
    fn csm_state(
        ctx: &StubCtx,
        tip: L1Height,
        verified_epoch: Option<Epoch>,
        finalized_epoch: Option<Epoch>,
    ) -> CheckpointState {
        let tip = block(tip, 0xAA);
        ctx.set_canonical(tip);
        let finalized = finalized_epoch.map(|epoch| l1_checkpoint(epoch, tip));
        let last_seen = verified_epoch.map(|epoch| l1_checkpoint(epoch, tip));
        CheckpointState::new(ClientState::new(finalized, last_seen), tip)
    }

    /// Checks at `tip` with `verified_epoch` accepted and already final.
    fn evaluate_at(
        state: &mut SubmissionTrackerState<StubCtx>,
        ctx: &StubCtx,
        tip: L1Height,
        verified_epoch: Option<Epoch>,
    ) {
        state.set_latest(csm_state(ctx, tip, verified_epoch, verified_epoch));
        state.evaluate().unwrap();
    }

    fn new_state(ctx: &StubCtx) -> SubmissionTrackerState<StubCtx> {
        SubmissionTrackerState::new(ctx.clone(), DEPTH, csm_state(ctx, 1, None, None)).unwrap()
    }

    #[test]
    fn rejection_recorded_once_across_repeated_observations() {
        let ctx = StubCtx::default();
        let txid = ctx.push_checkpoint(2);
        ctx.mine(txid, block(100, 1));
        let mut state = new_state(&ctx);

        evaluate_at(&mut state, &ctx, 101, Some(1));
        assert!(
            ctx.data().rejected.is_empty(),
            "two confirmations is short of depth"
        );
        assert_eq!(state.cursor(), 0);

        for tip in [102, 102, 103, 110] {
            evaluate_at(&mut state, &ctx, tip, Some(1));
        }
        let data = ctx.data();
        assert_eq!(data.commits_with_rejections, 1);
        assert_eq!(
            data.rejected,
            vec![RejectedCheckpointEntry::new(
                commitment(2),
                txid,
                block(100, 1),
                Some(commitment(1)),
            )]
        );
        assert_eq!(data.cursor, Some(1));
        drop(data);
        assert_eq!(state.unresolved_rejections(), 1);

        // A later transaction getting the epoch accepted resolves it without forgetting it.
        evaluate_at(&mut state, &ctx, 111, Some(2));
        assert_eq!(state.unresolved_rejections(), 0);
        assert_eq!(rejected_epochs(&state), vec![2]);
    }

    #[test]
    fn reorg_before_depth_reclassifies_from_the_new_block() {
        let ctx = StubCtx::default();
        let txid = ctx.push_checkpoint(2);
        ctx.mine(txid, block(100, 1));
        let mut state = new_state(&ctx);
        evaluate_at(&mut state, &ctx, 101, Some(1));

        // The inclusion block is reorged out, but the broadcaster still reports it.
        ctx.set_canonical(block(100, 2));
        evaluate_at(&mut state, &ctx, 104, Some(1));
        assert!(
            ctx.data().rejected.is_empty(),
            "orphaned inclusion must not count"
        );

        // The transaction is mined again in a new block; depth counts from there. The second
        // check reuses the same CSM state, as a retry tick does.
        ctx.mine(txid, block(104, 3));
        state.evaluate().unwrap();
        evaluate_at(&mut state, &ctx, 105, Some(1));
        assert!(ctx.data().rejected.is_empty());
        evaluate_at(&mut state, &ctx, 106, Some(1));
        assert_eq!(ctx.data().rejected[0].l1_block(), block(104, 3));
    }

    #[test]
    fn acceptance_undone_by_a_reorg_is_checked_again() {
        let ctx = StubCtx::default();
        let txid = ctx.push_checkpoint(2);
        ctx.mine(txid, block(100, 1));
        let mut state = new_state(&ctx);

        // Accepted but not final yet, so the bundle stays in the scan window.
        state.set_latest(csm_state(&ctx, 101, Some(2), Some(1)));
        state.evaluate().unwrap();
        assert_eq!(state.cursor(), 0);

        // A shallow reorg drops the acceptance and the reveal lands in a new block, where the
        // ASM rejects it.
        ctx.mine(txid, block(101, 2));
        evaluate_at(&mut state, &ctx, 103, Some(1));
        assert_eq!(ctx.data().rejected[0].l1_block(), block(101, 2));
        assert_eq!(state.cursor(), 1);
    }

    #[test]
    fn restart_keeps_recorded_rejection_without_realerting() {
        let ctx = StubCtx::default();
        let txid = ctx.push_checkpoint(2);
        ctx.mine(txid, block(100, 1));
        let mut state = new_state(&ctx);
        evaluate_at(&mut state, &ctx, 105, Some(1));
        assert_eq!(ctx.data().commits_with_rejections, 1);

        // A new bundle above the cursor makes the restarted tracker scan again.
        ctx.push_checkpoint(3);
        let mut restarted = new_state(&ctx);
        assert_eq!(rejected_epochs(&restarted), vec![2]);
        evaluate_at(&mut restarted, &ctx, 106, Some(1));
        assert_eq!(ctx.data().commits_with_rejections, 1);
        assert_eq!(restarted.unresolved_rejections(), 1);
        assert_eq!(
            restarted.cursor(),
            1,
            "the unmined epoch 3 bundle stays unsettled"
        );
    }

    #[test]
    fn rejection_behind_a_newer_accepted_bundle_is_found() {
        // A rebuild can submit a lower epoch after a higher one. The newer bundle's epoch being
        // accepted settles it without a record, and must not hide the older mined rejection.
        let ctx = StubCtx::default();
        let stale = ctx.push_checkpoint(12);
        ctx.mine(stale, block(100, 1));
        let rebuilt = ctx.push_checkpoint(11);
        ctx.mine(rebuilt, block(101, 1));
        ctx.data().bundles.insert(1, None);

        let mut state = new_state(&ctx);
        evaluate_at(&mut state, &ctx, 110, Some(11));
        assert_eq!(rejected_epochs(&state), vec![12]);
        assert_eq!(state.cursor(), 3);
    }

    #[test]
    fn cursor_waits_for_an_unmined_bundle() {
        let ctx = StubCtx::default();
        let unmined = ctx.push_checkpoint(2);
        let mined = ctx.push_checkpoint(3);
        ctx.mine(mined, block(100, 1));
        let mut state = new_state(&ctx);

        evaluate_at(&mut state, &ctx, 110, Some(1));
        assert_eq!(rejected_epochs(&state), vec![3]);
        assert_eq!(state.cursor(), 0);

        ctx.mine(unmined, block(111, 1));
        evaluate_at(&mut state, &ctx, 113, Some(1));
        assert_eq!(rejected_epochs(&state), vec![2, 3]);
        assert_eq!(state.cursor(), 2);
        assert_eq!(ctx.data().commits_with_rejections, 2);
    }
}
