//! Follows canonical epochs while retaining proof tasks by exact commitment.

use anyhow::{Context, Result};
use strata_identifiers::{Epoch, EpochCommitment};
use strata_status::OLSyncStatusUpdate;
use tokio::{
    sync::watch,
    time::{self, Interval},
};
use tracing::{debug, error, info, warn};

use super::{
    PROVER_RETRY_INTERVAL,
    context::{CheckpointProverContext, MissingCheckpointArtifact, is_waiting_for_state},
    errors::{CheckpointHistoryError, is_history_integrity_failure},
};

pub(super) struct CheckpointRunner<C> {
    ctx: C,
    last_processed: Option<EpochCommitment>,
    history_base: Option<EpochCommitment>,
}

/// Result of waiting for a checkpoint proof.
#[derive(Debug, PartialEq, Eq)]
enum ProofWaitOutcome {
    /// The proving task completed and its proof is stored.
    Complete,

    /// The epoch commitment being proved is no longer canonical.
    Superseded,

    /// Waiting stopped because the sync-status channel closed.
    Closed,
}

impl<C: CheckpointProverContext> CheckpointRunner<C> {
    pub(super) fn new(ctx: C) -> Self {
        Self {
            ctx,
            last_processed: None,
            history_base: None,
        }
    }

    async fn catch_up_target(&self) -> Result<Option<Epoch>> {
        let Some(tip) = self.ctx.get_canonical_tip().await? else {
            return Ok(None);
        };
        let header = self
            .ctx
            .get_block_header(tip)
            .await?
            .with_context(|| format!("missing canonical tip header {tip}"))?;
        Ok(if header.is_terminal() {
            Some(header.epoch())
        } else {
            header.epoch().checked_sub(1)
        })
    }

    async fn initialize(&mut self) -> Result<()> {
        self.history_base = self.ctx.get_history_base().await?;
        self.last_processed = self.history_base;
        let Some(last_payload) = self.ctx.get_last_checkpoint_payload_epoch().await? else {
            return Ok(());
        };
        let Some(target) = self.catch_up_target().await? else {
            return Ok(());
        };
        let floor = self.history_base.map_or(0, |base| base.epoch());
        for epoch in (floor.saturating_add(1)..=target.min(last_payload.epoch())).rev() {
            let Some(commitment) = self.ctx.get_canonical_epoch_commitment_at(epoch).await? else {
                continue;
            };
            if self.ctx.has_payload(commitment).await? {
                self.ctx.get_epoch_summary(commitment).await?;
                self.last_processed = Some(commitment);
                break;
            }
        }
        Ok(())
    }

    async fn is_canonical(&self, commitment: EpochCommitment) -> Result<bool> {
        Ok(self
            .ctx
            .get_canonical_epoch_commitment_at(commitment.epoch())
            .await?
            == Some(commitment))
    }

    async fn reconcile_cursor(&mut self) -> Result<()> {
        if let Some(commitment) = self.last_processed {
            self.last_processed = self
                .ctx
                .find_canonical_resume_point(commitment, self.history_base)
                .await?;
        }
        Ok(())
    }

    /// Advances past reusable proofs and selects the next canonical task.
    async fn next_task(&mut self) -> Result<Option<EpochCommitment>> {
        loop {
            let Some(target) = self.catch_up_target().await? else {
                return Ok(None);
            };
            self.reconcile_cursor().await?;
            let Some(epoch) = self
                .last_processed
                .map_or(Some(1), |c| c.epoch().checked_add(1))
            else {
                return Ok(None);
            };
            if epoch > target {
                return Ok(None);
            }
            let Some(commitment) = self.ctx.get_canonical_epoch_commitment_at(epoch).await? else {
                return Ok(None);
            };
            let summary = self.ctx.get_epoch_summary(commitment).await?;
            if let Some(previous) = self.last_processed
                && *summary.prev_terminal() != previous.to_block_commitment()
            {
                if self
                    .ctx
                    .get_canonical_block_at(previous.last_slot())
                    .await?
                    != Some(previous.to_block_commitment())
                    || !self.is_canonical(commitment).await?
                {
                    continue;
                }
                return Err(CheckpointHistoryError::PredecessorMismatch {
                    commitment,
                    expected: previous,
                    actual: *summary.prev_terminal(),
                }
                .into());
            }
            let has_proof = self.ctx.has_proof(commitment).await?;
            if !self.is_canonical(commitment).await? {
                continue;
            }
            if !has_proof {
                return Ok(Some(commitment));
            }
            self.last_processed = Some(commitment);
        }
    }

    async fn wait_for_proof(
        &self,
        commitment: EpochCommitment,
        sync_rx: &mut watch::Receiver<Option<OLSyncStatusUpdate>>,
        retry_tick: &mut Interval,
    ) -> Result<ProofWaitOutcome> {
        let proving = self.ctx.prove(commitment);
        tokio::pin!(proving);
        loop {
            tokio::select! {
                result = &mut proving => {
                    if !self.is_canonical(commitment).await? {
                        return Ok(ProofWaitOutcome::Superseded);
                    }
                    result?;
                    anyhow::ensure!(self.ctx.has_proof(commitment).await?, "completed checkpoint task has no proof for {commitment}");
                    return Ok(ProofWaitOutcome::Complete);
                }
                changed = sync_rx.changed() => {
                    if changed.is_err() { return Ok(ProofWaitOutcome::Closed); }
                }
                _ = retry_tick.tick() => {}
            }
            if !self.is_canonical(commitment).await? {
                return Ok(ProofWaitOutcome::Superseded);
            }
        }
    }

    pub(super) async fn run(
        mut self,
        mut sync_rx: watch::Receiver<Option<OLSyncStatusUpdate>>,
    ) -> Result<()> {
        let mut retry_tick = time::interval(PROVER_RETRY_INTERVAL);
        retry_tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        let mut initialized = false;
        let mut waiting: Option<String> = None;
        loop {
            let result: Result<bool> = async {
                if !initialized {
                    self.initialize().await?;
                    initialized = true;
                }
                while let Some(commitment) = self.next_task().await? {
                    match self
                        .wait_for_proof(commitment, &mut sync_rx, &mut retry_tick)
                        .await?
                    {
                        ProofWaitOutcome::Closed => return Ok(false),
                        ProofWaitOutcome::Superseded => {
                            debug!(%commitment, "checkpoint proof wait superseded")
                        }
                        ProofWaitOutcome::Complete => {
                            info!(%commitment, "checkpoint proof completed")
                        }
                    }
                    // Reconcile immediately after a cancelled wait, even if signals coalesced.
                }
                Ok(true)
            }
            .await;
            match result {
                Ok(false) => return Ok(()),
                Ok(true) => waiting = None,
                Err(failure) => {
                    if is_history_integrity_failure(&failure) {
                        return Err(failure);
                    }
                    let reason = failure.to_string();
                    if waiting.as_ref() != Some(&reason) {
                        if failure.is::<MissingCheckpointArtifact>() {
                            error!(%failure, "checkpoint proof awaits operator configuration");
                        } else if is_waiting_for_state(&failure) {
                            debug!(%failure, "checkpoint proof awaits state");
                        } else {
                            warn!(%failure, "checkpoint proof runner will retry");
                        }
                        waiting = Some(reason);
                    }
                }
            }
            // Status is only a wake-up; its epoch fields may describe another branch.
            tokio::select! {
                changed = sync_rx.changed() => { if changed.is_err() { return Ok(()); } }
                _ = retry_tick.tick() => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet, VecDeque},
        sync::{Arc, Mutex},
        time::Duration,
    };

    use strata_checkpoint_types::EpochSummary;
    use strata_db_types::DbError;
    use strata_identifiers::{Buf32, L1BlockCommitment, OLBlockCommitment, Slot};
    use strata_ol_chain_types_v1::{BlockFlagsV1, OLBlockHeaderV1};
    use strata_ol_checkpoint::CanonicalEpochReconcileError;
    use tokio::{
        sync::{Notify, mpsc},
        time::timeout,
    };

    use super::*;

    #[derive(Default)]
    struct Records {
        headers: BTreeMap<OLBlockCommitment, OLBlockHeaderV1>,
        canonical: BTreeMap<Epoch, EpochCommitment>,
        summaries: BTreeMap<EpochCommitment, EpochSummary>,
        proofs: BTreeSet<EpochCommitment>,
        payloads: BTreeSet<EpochCommitment>,
        tip: Option<OLBlockCommitment>,
        base: Option<EpochCommitment>,
        resume_points: BTreeMap<EpochCommitment, Result<Option<EpochCommitment>, &'static str>>,
        reconcile_calls: Vec<(EpochCommitment, Option<EpochCommitment>)>,
        reconcile_errors: VecDeque<CanonicalEpochReconcileError>,
        fail_proof_read: bool,
    }

    #[derive(Clone)]
    struct TestContext {
        records: Arc<Mutex<Records>>,
        submitted: mpsc::UnboundedSender<EpochCommitment>,
        proof_ready: Arc<Notify>,
    }

    impl TestContext {
        fn new() -> (Self, mpsc::UnboundedReceiver<EpochCommitment>) {
            let (submitted, receiver) = mpsc::unbounded_channel();
            (
                Self {
                    records: Arc::default(),
                    submitted,
                    proof_ready: Arc::default(),
                },
                receiver,
            )
        }

        fn branch(&self, parent: EpochCommitment, count: Epoch, seed: u8) -> Vec<EpochCommitment> {
            let mut records = self.records.lock().unwrap();
            let mut previous = parent;
            (1..=count)
                .map(|offset| {
                    let mut flags = BlockFlagsV1::zero();
                    flags.set_is_terminal(true);
                    let header = OLBlockHeaderV1::new(
                        u64::from(offset),
                        flags,
                        parent.last_slot() + u64::from(offset),
                        parent.epoch() + offset,
                        *previous.last_blkid(),
                        Buf32::from([seed; 32]),
                        Buf32::zero(),
                        Buf32::zero(),
                    );
                    let block = header.compute_block_commitment();
                    let summary = EpochSummary::new(
                        header.epoch(),
                        block,
                        previous.to_block_commitment(),
                        L1BlockCommitment::default(),
                        *header.state_root(),
                    );
                    let commitment = summary.get_epoch_commitment();
                    records.headers.insert(block, header);
                    records.summaries.insert(commitment, summary);
                    previous = commitment;
                    commitment
                })
                .collect()
        }

        fn publish(&self, branch: &[EpochCommitment]) {
            let mut records = self.records.lock().unwrap();
            records
                .canonical
                .retain(|epoch, _| *epoch < branch[0].epoch());
            for commitment in branch {
                records.canonical.insert(commitment.epoch(), *commitment);
            }
            records.tip = branch.last().map(|c| c.to_block_commitment());
        }
    }

    impl CheckpointProverContext for TestContext {
        async fn get_canonical_tip(&self) -> Result<Option<OLBlockCommitment>> {
            Ok(self.records.lock().unwrap().tip)
        }
        async fn get_block_header(
            &self,
            block: OLBlockCommitment,
        ) -> Result<Option<OLBlockHeaderV1>> {
            Ok(self.records.lock().unwrap().headers.get(&block).cloned())
        }
        async fn get_canonical_epoch_commitment_at(
            &self,
            epoch: Epoch,
        ) -> Result<Option<EpochCommitment>> {
            Ok(self.records.lock().unwrap().canonical.get(&epoch).copied())
        }
        async fn get_canonical_block_at(&self, slot: Slot) -> Result<Option<OLBlockCommitment>> {
            Ok(self
                .records
                .lock()
                .unwrap()
                .canonical
                .values()
                .find(|c| c.last_slot() == slot)
                .map(|c| c.to_block_commitment()))
        }
        async fn find_canonical_resume_point(
            &self,
            last_processed: EpochCommitment,
            history_base: Option<EpochCommitment>,
        ) -> Result<Option<EpochCommitment>> {
            let mut records = self.records.lock().unwrap();
            records.reconcile_calls.push((last_processed, history_base));
            if let Some(error) = records.reconcile_errors.pop_front() {
                return Err(error.into());
            }
            records
                .resume_points
                .get(&last_processed)
                .copied()
                .unwrap_or(Ok(Some(last_processed)))
                .map_err(anyhow::Error::msg)
        }
        async fn get_history_base(&self) -> Result<Option<EpochCommitment>> {
            Ok(self.records.lock().unwrap().base)
        }
        async fn get_epoch_summary(&self, commitment: EpochCommitment) -> Result<EpochSummary> {
            self.records
                .lock()
                .unwrap()
                .summaries
                .get(&commitment)
                .copied()
                .context("missing epoch summary")
        }
        async fn get_last_checkpoint_payload_epoch(&self) -> Result<Option<EpochCommitment>> {
            Ok(self.records.lock().unwrap().payloads.last().copied())
        }
        async fn has_payload(&self, commitment: EpochCommitment) -> Result<bool> {
            Ok(self.records.lock().unwrap().payloads.contains(&commitment))
        }
        async fn has_proof(&self, commitment: EpochCommitment) -> Result<bool> {
            let records = self.records.lock().unwrap();
            anyhow::ensure!(!records.fail_proof_read, "injected proof read failure");
            Ok(records.proofs.contains(&commitment))
        }
        async fn prove(&self, commitment: EpochCommitment) -> Result<()> {
            self.submitted.send(commitment).unwrap();
            loop {
                let notified = self.proof_ready.notified();
                if self.has_proof(commitment).await? {
                    return Ok(());
                }
                notified.await;
            }
        }
    }

    #[tokio::test]
    async fn test_startup_ignores_orphan_payload_and_reuses_canonical_proofs() {
        let (ctx, _) = TestContext::new();
        let branch = ctx.branch(EpochCommitment::null(), 3, 1);
        let orphan = ctx.branch(EpochCommitment::null(), 5, 2);
        ctx.publish(&branch);
        {
            let mut records = ctx.records.lock().unwrap();
            records.payloads.extend([branch[0], orphan[4]]);
            records.proofs.insert(branch[1]);
        }
        let mut runner = CheckpointRunner::new(ctx);
        runner.initialize().await.unwrap();
        assert_eq!(runner.last_processed, Some(branch[0]));
        assert_eq!(runner.next_task().await.unwrap(), Some(branch[2]));
        assert_eq!(runner.last_processed, Some(branch[1]));
    }

    #[tokio::test]
    async fn test_tip_becoming_terminal_enables_same_epoch() {
        let (ctx, _) = TestContext::new();
        let branch = ctx.branch(EpochCommitment::null(), 1, 1);
        let tip = OLBlockHeaderV1::new(
            10,
            BlockFlagsV1::zero(),
            1,
            1,
            *EpochCommitment::null().last_blkid(),
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        {
            let mut records = ctx.records.lock().unwrap();
            records.tip = Some(tip.compute_block_commitment());
            records.headers.insert(tip.compute_block_commitment(), tip);
        }
        let mut runner = CheckpointRunner::new(ctx.clone());
        runner.initialize().await.unwrap();
        assert_eq!(runner.next_task().await.unwrap(), None);
        ctx.publish(&branch);
        assert_eq!(runner.next_task().await.unwrap(), Some(branch[0]));
    }

    #[tokio::test]
    async fn test_selects_next_task_from_context_resume_point() {
        let (ctx, _) = TestContext::new();
        let branch = ctx.branch(EpochCommitment::null(), 4, 1);
        ctx.publish(&branch);
        let mut runner = CheckpointRunner::new(ctx.clone());
        runner.last_processed = Some(branch[3]);
        runner.history_base = Some(branch[0]);

        // The shared reconciler owns the walk; supply its result through the context.
        let replacement = ctx.branch(branch[1], 2, 2);
        ctx.publish(&replacement);
        ctx.records
            .lock()
            .unwrap()
            .resume_points
            .insert(branch[3], Ok(Some(branch[1])));
        assert_eq!(runner.next_task().await.unwrap(), Some(replacement[0]));
        assert_eq!(runner.last_processed, Some(branch[1]));
        assert_eq!(
            ctx.records.lock().unwrap().reconcile_calls,
            [(branch[3], Some(branch[0]))]
        );
    }

    #[tokio::test]
    async fn test_history_floor_not_proved() {
        let (ctx, _) = TestContext::new();
        let branch = ctx.branch(EpochCommitment::null(), 3, 1);
        ctx.publish(&branch);
        ctx.records.lock().unwrap().base = Some(branch[1]);
        let mut runner = CheckpointRunner::new(ctx.clone());
        runner.initialize().await.unwrap();
        assert_eq!(runner.next_task().await.unwrap(), Some(branch[2]));
        assert_eq!(
            ctx.records.lock().unwrap().reconcile_calls,
            [(branch[1], Some(branch[1]))]
        );
    }

    #[tokio::test]
    async fn test_reconciliation_and_proof_read_errors_preserve_progress() {
        let (ctx, _) = TestContext::new();
        let branch = ctx.branch(EpochCommitment::null(), 2, 1);
        ctx.publish(&branch);
        ctx.records.lock().unwrap().fail_proof_read = true;
        let mut runner = CheckpointRunner::new(ctx.clone());
        runner.initialize().await.unwrap();
        assert_eq!(
            runner.next_task().await.unwrap_err().to_string(),
            "injected proof read failure"
        );
        assert_eq!(runner.last_processed, None);
        {
            let mut records = ctx.records.lock().unwrap();
            records.fail_proof_read = false;
            records.proofs.extend(branch.iter().copied());
        }
        assert_eq!(runner.next_task().await.unwrap(), None);
        ctx.records
            .lock()
            .unwrap()
            .resume_points
            .insert(branch[1], Err("injected reconciliation failure"));
        assert_eq!(
            runner.next_task().await.unwrap_err().to_string(),
            "injected reconciliation failure"
        );
        assert_eq!(runner.last_processed, Some(branch[1]));
    }

    #[tokio::test]
    async fn test_run_stops_on_history_integrity_errors() {
        let commitment = EpochCommitment::null();
        let failures = [
            CanonicalEpochReconcileError::MissingSummary(commitment),
            CanonicalEpochReconcileError::SummaryMismatch {
                expected: commitment,
                actual: commitment,
            },
            CanonicalEpochReconcileError::InvalidPredecessor {
                commitment,
                previous: commitment,
            },
            CanonicalEpochReconcileError::HistoryBaseConflict(commitment),
        ];
        for failure in failures {
            let (ctx, mut submitted) = TestContext::new();
            let branch = ctx.branch(commitment, 2, 1);
            ctx.publish(&branch);
            let expected = failure.to_string();
            {
                let mut records = ctx.records.lock().unwrap();
                records.base = Some(branch[0]);
                // A shared-reconciler failure must exit even while wake-ups remain available.
                records.reconcile_errors.push_back(failure);
            }
            let (_sender, receiver) = watch::channel(None);
            let error = timeout(
                Duration::from_secs(2),
                CheckpointRunner::new(ctx.clone()).run(receiver),
            )
            .await
            .unwrap()
            .unwrap_err();
            assert!(error.is::<CanonicalEpochReconcileError>());
            assert_eq!(error.to_string(), expected);
            assert_eq!(ctx.records.lock().unwrap().reconcile_calls.len(), 1);
            assert_eq!(submitted.try_recv(), Err(mpsc::error::TryRecvError::Empty));
        }
    }

    #[tokio::test]
    async fn test_run_retries_reconciler_storage_failure() {
        let (ctx, mut submitted) = TestContext::new();
        let branch = ctx.branch(EpochCommitment::null(), 2, 1);
        ctx.publish(&branch);
        {
            let mut records = ctx.records.lock().unwrap();
            records.base = Some(branch[0]);
            records
                .reconcile_errors
                .push_back(CanonicalEpochReconcileError::Storage(DbError::Busy));
        }
        let (sender, receiver) = watch::channel(None);
        let runner = tokio::spawn(CheckpointRunner::new(ctx.clone()).run(receiver));
        // The retry succeeds and starts proof work without resetting progress to genesis.
        assert_eq!(
            timeout(Duration::from_secs(2), submitted.recv())
                .await
                .unwrap(),
            Some(branch[1])
        );
        assert_eq!(
            ctx.records.lock().unwrap().reconcile_calls,
            [(branch[0], Some(branch[0])), (branch[0], Some(branch[0])),]
        );
        drop(sender);
        timeout(Duration::from_secs(2), runner)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn test_run_stops_on_canonical_summary_predecessor_mismatch() {
        let (ctx, mut submitted) = TestContext::new();
        let branch = ctx.branch(EpochCommitment::null(), 1, 1);
        let other_branch = ctx.branch(EpochCommitment::null(), 1, 2);
        let successor = ctx.branch(other_branch[0], 1, 3);
        // Both rows are canonical, but the successor summary belongs to another ancestry.
        ctx.publish(&branch);
        ctx.publish(&successor);
        ctx.records.lock().unwrap().base = Some(branch[0]);
        let (_sender, receiver) = watch::channel(None);
        let error = timeout(
            Duration::from_secs(2),
            CheckpointRunner::new(ctx).run(receiver),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(matches!(error.downcast_ref::<CheckpointHistoryError>(),
            Some(CheckpointHistoryError::PredecessorMismatch { commitment, expected, actual })
                if *commitment == successor[0] && *expected == branch[0]
                    && *actual == other_branch[0].to_block_commitment()));
        assert_eq!(
            submitted.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        );
    }

    #[tokio::test]
    async fn test_runner_switches_pending_proof_on_status_update() {
        let (ctx, mut submitted) = TestContext::new();
        let branch = ctx.branch(EpochCommitment::null(), 3, 1);
        let replacement = ctx.branch(EpochCommitment::null(), 3, 2);
        ctx.publish(&branch);
        ctx.records
            .lock()
            .unwrap()
            .proofs
            .extend(branch[..2].iter().copied());
        let (sender, receiver) = watch::channel(None);
        let runner = tokio::spawn(CheckpointRunner::new(ctx.clone()).run(receiver));
        // Startup catch-up runs before any signal. The third epoch's proof never arrives.
        assert_eq!(
            timeout(Duration::from_secs(2), submitted.recv())
                .await
                .unwrap(),
            Some(branch[2])
        );
        ctx.records
            .lock()
            .unwrap()
            .resume_points
            .insert(branch[1], Ok(None));
        ctx.publish(&replacement);
        sender.send_replace(None);
        assert_eq!(
            timeout(Duration::from_secs(2), submitted.recv())
                .await
                .unwrap(),
            Some(replacement[0])
        );
        drop(sender);
        timeout(Duration::from_secs(2), runner)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn test_wait_rechecks_on_tick_without_notification() {
        let (ctx, mut submitted) = TestContext::new();
        let branch = ctx.branch(EpochCommitment::null(), 1, 1);
        let replacement = ctx.branch(EpochCommitment::null(), 1, 2);
        ctx.publish(&branch);
        let runner = CheckpointRunner::new(ctx.clone());
        let (_sender, mut receiver) = watch::channel(None);
        let mut tick = time::interval(Duration::from_millis(1));
        let (outcome, ()) = timeout(Duration::from_secs(2), async {
            tokio::join!(
                runner.wait_for_proof(branch[0], &mut receiver, &mut tick),
                async {
                    assert_eq!(submitted.recv().await, Some(branch[0]));
                    ctx.publish(&replacement);
                    // No status or receipt signal: only the retry tick can end the wait.
                }
            )
        })
        .await
        .unwrap();
        assert_eq!(outcome.unwrap(), ProofWaitOutcome::Superseded);
    }

    #[tokio::test]
    async fn test_receipt_racing_reorg_does_not_complete_old_branch() {
        let (ctx, mut submitted) = TestContext::new();
        let branch = ctx.branch(EpochCommitment::null(), 1, 1);
        let replacement = ctx.branch(EpochCommitment::null(), 1, 2);
        ctx.publish(&branch);
        let runner = CheckpointRunner::new(ctx.clone());
        let (_sender, mut receiver) = watch::channel(None);
        let mut tick = time::interval(Duration::from_secs(60));
        tick.tick().await;
        let (outcome, ()) = timeout(Duration::from_secs(2), async {
            tokio::join!(
                runner.wait_for_proof(branch[0], &mut receiver, &mut tick),
                async {
                    assert_eq!(submitted.recv().await, Some(branch[0]));
                    ctx.publish(&replacement);
                    ctx.records.lock().unwrap().proofs.insert(branch[0]);
                    ctx.proof_ready.notify_one();
                }
            )
        })
        .await
        .unwrap();
        assert_eq!(outcome.unwrap(), ProofWaitOutcome::Superseded);
        assert_eq!(runner.last_processed, None);
    }
}
