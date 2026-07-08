//! Service state for OL checkpoint builder.

use metrics::{counter, gauge};
use strata_asm_proto_checkpoint_types::{
    CheckpointPayload, CheckpointSidecar, CheckpointTip, OLLog as CheckpointOLLog,
    TerminalHeaderComplement,
};
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Epoch, EpochCommitment};
use strata_service::ServiceState;
use tracing::{debug, info};

use crate::{context::CheckpointWorkerContext, errors::CheckpointNotReady};

/// Service state for OL checkpoint builder.
///
/// Generic over the context to allow testing with mock implementations.
pub(crate) struct OLCheckpointServiceState<C: CheckpointWorkerContext> {
    ctx: C,
    initialized: bool,
    last_processed_epoch: Option<Epoch>,
}

impl<C: CheckpointWorkerContext> OLCheckpointServiceState<C> {
    /// Create a new state with the given context.
    pub(crate) fn new(ctx: C) -> Self {
        Self {
            ctx,
            initialized: false,
            last_processed_epoch: None,
        }
    }

    pub(crate) fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub(crate) fn last_processed_epoch(&self) -> Option<Epoch> {
        self.last_processed_epoch
    }

    pub(crate) fn initialize(&mut self) {
        self.init_cursor_from_db();
        self.initialized = true;
    }

    /// Returns the canonical commitment of the most recently summarized epoch,
    /// or `None` if no epoch has been summarized yet.
    pub(crate) fn last_summarized_commitment(&self) -> anyhow::Result<Option<EpochCommitment>> {
        let Some(epoch_index) = self.ctx.get_last_summarized_epoch()? else {
            return Ok(None);
        };
        self.ctx.get_canonical_epoch_commitment_at(epoch_index)
    }

    /// Handles a completed epoch, catching up from last checkpoint to latest summary.
    ///
    /// The `target` commitment identifies the epoch that was completed. We process
    /// all pending epochs up to and including the latest summarized epoch.
    pub(crate) fn handle_complete_epoch(&mut self, target: EpochCommitment) -> anyhow::Result<()> {
        anyhow::ensure!(self.initialized, "worker not initialized");

        let Some(target_epoch_index) = self.ctx.get_last_summarized_epoch()? else {
            return Ok(());
        };

        // Determine starting epoch index (last processed + 1, or 1 if none, skip genesis epoch)
        let start_epoch_index = self
            .last_processed_epoch
            .map(|e| e.saturating_add(1))
            .unwrap_or(1);

        // Process all epochs from start to target (inclusive)
        for epoch_index in start_epoch_index..=target_epoch_index {
            self.process_epoch(epoch_index)?;
        }

        // Sanity check: verify we processed up to at least the target epoch
        if let Some(last_epoch) = self.last_processed_epoch
            && last_epoch < target.epoch()
        {
            debug!(
                last_processed = last_epoch,
                target_epoch = target.epoch(),
                "processed epochs but not yet caught up to target"
            );
        }

        Ok(())
    }

    /// Process a single epoch, building checkpoint if summary exists.
    ///
    /// Returns error if the epoch index cannot be processed (missing data).
    /// Checkpoints must be built sequentially, so caller should stop on error.
    fn process_epoch(&mut self, epoch_number: Epoch) -> anyhow::Result<()> {
        // Get canonical commitment for this epoch index - must exist to proceed
        let commitment = self
            .ctx
            .get_canonical_epoch_commitment_at(epoch_number)?
            .ok_or(CheckpointNotReady::EpochCommitment(epoch_number))?;

        // Get summary - must exist to proceed
        let summary = self
            .ctx
            .get_epoch_summary(commitment)?
            .ok_or(CheckpointNotReady::EpochSummary(commitment))?;

        let epoch = summary.epoch();

        // Skip if already checkpointed
        if self.ctx.get_checkpoint_payload(commitment)?.is_some() {
            self.last_processed_epoch = Some(epoch);
            return Ok(());
        }

        let payload = build_checkpoint_payload(commitment, &summary, &self.ctx)?;
        self.ctx.put_checkpoint_payload(commitment, payload)?;
        counter!("strata_checkpoint_created_total").increment(1);
        gauge!("strata_checkpoint_last_created_epoch").set(epoch as f64);

        info!(
            component = "ol_checkpoint",
            %epoch,
            l1_height = summary.new_l1().height(),
            l1_block = %summary.new_l1(),
            l2_commitment = %summary.terminal(),
            "stored OL checkpoint entry"
        );
        self.last_processed_epoch = Some(epoch);

        Ok(())
    }

    fn init_cursor_from_db(&mut self) {
        let Ok(Some(last_checkpoint_commitment)) = self.ctx.get_last_checkpoint_payload_epoch()
        else {
            return;
        };

        let Ok(Some(last_summarized_index)) = self.ctx.get_last_summarized_epoch() else {
            return;
        };

        for epoch_index in (0..=last_summarized_index).rev() {
            let Ok(Some(commitment)) = self.ctx.get_canonical_epoch_commitment_at(epoch_index)
            else {
                continue;
            };
            let Ok(Some(summary)) = self.ctx.get_epoch_summary(commitment) else {
                continue;
            };

            if summary.get_epoch_commitment() == last_checkpoint_commitment {
                self.last_processed_epoch = Some(last_checkpoint_commitment.epoch());
                break;
            }
        }
    }
}

impl<C: CheckpointWorkerContext> ServiceState for OLCheckpointServiceState<C> {
    fn name(&self) -> &str {
        "ol_checkpoint"
    }
}

fn build_checkpoint_payload<C: CheckpointWorkerContext>(
    commitment: EpochCommitment,
    summary: &EpochSummary,
    ctx: &C,
) -> anyhow::Result<CheckpointPayload> {
    let l1_height = summary.new_l1().height();
    let l2_commitment = *summary.terminal();
    let new_tip = CheckpointTip::new(summary.epoch(), l1_height, l2_commitment);

    let (state_bytes, ol_logs) = ctx.fetch_da_for_epoch(summary)?;
    let ol_logs: Vec<CheckpointOLLog> = ol_logs
        .into_iter()
        .map(|log| CheckpointOLLog::new(log.account_serial(), log.payload().to_vec()))
        .collect();

    let terminal_header = ctx
        .get_block_header(summary.terminal())?
        .ok_or_else(|| anyhow::anyhow!("missing terminal block for epoch summary {:?}", summary))?;
    let terminal_header_complement = TerminalHeaderComplement::new(
        terminal_header.timestamp(),
        *terminal_header.parent_blkid(),
        *terminal_header.body_root(),
        *terminal_header.logs_root(),
    );

    let sidecar = CheckpointSidecar::new(state_bytes, ol_logs, terminal_header_complement)?;
    let proof = ctx.get_proof(&commitment)?;

    Ok(CheckpointPayload::new(new_tip, sidecar, proof)?)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;
    use strata_acct_types::{BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL, BitcoinAmount};
    use strata_asm_proto_checkpoint_types::{
        CheckpointPayload, CheckpointSidecar, CheckpointTip, OLLog as CheckpointOLLog,
        TerminalHeaderComplement,
    };
    use strata_bridge_params::BridgeParams;
    use strata_checkpoint_types::EpochSummary;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{
        AccountSerial, Buf64, Epoch, L1BlockCommitment, OLBlockCommitment,
        test_utils::{
            buf32_strategy, l1_block_commitment_strategy, ol_block_commitment_strategy,
            ol_block_id_strategy,
        },
    };
    use strata_ledger_types::{IAccountState, IStateAccessor};
    use strata_ol_chain_types::{
        BlockFlags, OLBlock, OLBlockBody, OLBlockHeader, OLBlockId, OLLog, OLTransaction,
        OLTransactionData, OLTxSegment, SignedOLBlockHeader, SimpleWithdrawalIntentLogData,
        TxProofs,
    };
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_ol_state_types::OLState;
    use strata_ol_stf::{
        BlockComponents,
        test_utils::{
            EPOCH_RUNNER_TERMINAL_L1_HEIGHT as TERMINAL_L1_HEIGHT, InboxMmrTracker,
            SnarkUpdateBuilder, TEST_SNARK_ACCOUNT_ID, epoch_runner_run_block as run_block,
            epoch_runner_run_genesis as run_genesis, epoch_runner_run_terminal as run_terminal,
            epoch_runner_seed_accounts as seed_accounts, get_snark_state_expect, make_account_id,
            make_empty_manifest, make_genesis_state, make_p2wpkh_bosd_descriptor, make_state_root,
            make_withdrawal_payload, snark_inbox_msg, to_ol_block,
        },
    };
    use strata_primitives::epoch::EpochCommitment;
    use strata_storage::create_node_storage;

    use super::OLCheckpointServiceState;
    use crate::context::{CheckpointWorkerContext, CheckpointWorkerContextImpl, StateDiffRaw};

    fn state_diff_strategy() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(any::<u8>(), 0..1024)
    }

    fn ol_logs_strategy() -> impl Strategy<Value = Vec<OLLog>> {
        prop::collection::vec(
            (
                any::<u32>().prop_map(AccountSerial::from),
                prop::collection::vec(any::<u8>(), 0..=512),
            )
                .prop_map(|(account_serial, payload)| OLLog::new(account_serial, payload)),
            0..10,
        )
    }

    fn terminal_header_complement_strategy() -> impl Strategy<Value = TerminalHeaderComplement> {
        (
            any::<u64>(),
            ol_block_id_strategy(),
            buf32_strategy(),
            buf32_strategy(),
        )
            .prop_map(|(timestamp, parent_blkid, body_root, logs_root)| {
                TerminalHeaderComplement::new(timestamp, parent_blkid, body_root, logs_root)
            })
    }

    /// Generates checkpoint OL logs whose *total* payload stays within the sidecar's
    /// `MAX_TOTAL_LOG_PAYLOAD_BYTES` cap (at most 10 logs of up to 512 bytes each).
    fn checkpoint_ol_logs_strategy() -> impl Strategy<Value = Vec<CheckpointOLLog>> {
        prop::collection::vec(
            (
                any::<u32>().prop_map(AccountSerial::from),
                prop::collection::vec(any::<u8>(), 0..=512),
            )
                .prop_map(|(account_serial, payload)| {
                    CheckpointOLLog::new(account_serial, payload)
                }),
            0..10,
        )
    }

    // TODO(STR-3804): drop this once https://github.com/alpenlabs/asm/pull/154 lands and we bump
    // to a tag that includes it, then go back to
    // `strata_asm_proto_checkpoint_types::test_utils::checkpoint_sidecar_strategy`. The upstream
    // strategy can emit ~40 KiB of logs and `CheckpointSidecar::new` rejects anything over the
    // 16 KiB cap, so it panics. The local `checkpoint_ol_logs_strategy` stays well under the cap.
    fn checkpoint_sidecar_strategy() -> impl Strategy<Value = CheckpointSidecar> {
        (
            state_diff_strategy(),
            checkpoint_ol_logs_strategy(),
            terminal_header_complement_strategy(),
        )
            .prop_map(|(state_diff, ol_logs, terminal_header_complement)| {
                CheckpointSidecar::new(state_diff, ol_logs, terminal_header_complement)
                    .expect("valid sidecar")
            })
    }

    /// Test context that delegates everything to the real impl but stubs out
    /// `fetch_da_for_epoch` with provided DA data. This avoids needing a full
    /// replay chain (prev terminal block, OL state, etc.) in structural tests.
    struct TestCheckpointContext {
        inner: CheckpointWorkerContextImpl,
        stub_state_diff: StateDiffRaw,
        stub_ol_logs: Vec<OLLog>,
    }

    impl TestCheckpointContext {
        fn new(
            storage: Arc<strata_storage::NodeStorage>,
            stub_state_diff: StateDiffRaw,
            stub_ol_logs: Vec<OLLog>,
        ) -> Self {
            Self {
                inner: CheckpointWorkerContextImpl::new(storage, BridgeParams::default()),
                stub_state_diff,
                stub_ol_logs,
            }
        }
    }

    impl CheckpointWorkerContext for TestCheckpointContext {
        fn get_last_summarized_epoch(&self) -> anyhow::Result<Option<Epoch>> {
            self.inner.get_last_summarized_epoch()
        }

        fn get_canonical_epoch_commitment_at(
            &self,
            index: Epoch,
        ) -> anyhow::Result<Option<EpochCommitment>> {
            self.inner.get_canonical_epoch_commitment_at(index)
        }

        fn get_epoch_summary(
            &self,
            commitment: EpochCommitment,
        ) -> anyhow::Result<Option<EpochSummary>> {
            self.inner.get_epoch_summary(commitment)
        }

        fn get_checkpoint_payload(
            &self,
            commitment: EpochCommitment,
        ) -> anyhow::Result<Option<CheckpointPayload>> {
            self.inner.get_checkpoint_payload(commitment)
        }

        fn get_last_checkpoint_payload_epoch(&self) -> anyhow::Result<Option<EpochCommitment>> {
            self.inner.get_last_checkpoint_payload_epoch()
        }

        fn put_checkpoint_payload(
            &self,
            commitment: EpochCommitment,
            payload: CheckpointPayload,
        ) -> anyhow::Result<()> {
            self.inner.put_checkpoint_payload(commitment, payload)
        }

        fn get_proof(&self, epoch: &EpochCommitment) -> anyhow::Result<Vec<u8>> {
            self.inner.get_proof(epoch)
        }

        fn get_block_header(
            &self,
            blkid: &OLBlockCommitment,
        ) -> anyhow::Result<Option<OLBlockHeader>> {
            self.inner.get_block_header(blkid)
        }

        fn get_block(&self, id: &OLBlockId) -> anyhow::Result<Option<OLBlock>> {
            self.inner.get_block(id)
        }

        fn get_ol_state(&self, commitment: &OLBlockCommitment) -> anyhow::Result<Option<OLState>> {
            self.inner.get_ol_state(commitment)
        }

        fn fetch_da_for_epoch(
            &self,
            _summary: &EpochSummary,
        ) -> anyhow::Result<(StateDiffRaw, Vec<OLLog>)> {
            Ok((self.stub_state_diff.clone(), self.stub_ol_logs.clone()))
        }
    }

    proptest! {
        #[test]
        fn init_cursor_from_db_uses_last_checkpoint_payload_epoch(
            len in 1usize..=5,
            terminals in prop::collection::vec(ol_block_commitment_strategy(), 1..=5),
            l1s in prop::collection::vec(l1_block_commitment_strategy(), 1..=5),
            finals in prop::collection::vec(buf32_strategy(), 1..=5),
            sidecars in prop::collection::vec(checkpoint_sidecar_strategy(), 1..=5),
            last_checkpoint in 0usize..=4,
        ) {
            let len = len.min(terminals.len())
                .min(l1s.len())
                .min(finals.len())
                .min(sidecars.len());
            prop_assume!(len > 0);
            let last_checkpoint = last_checkpoint.min(len.saturating_sub(1));

            let backend = get_test_sled_backend();
            let storage = Arc::new(
                create_node_storage(backend, strata_storage::test_runtime_handle())
                    .expect("test storage"),
            );
            let checkpoint_mgr = storage.ol_checkpoint();

            let mut prev_terminal = OLBlockCommitment::null();
            let mut summaries = Vec::with_capacity(len);
            for i in 0..len {
                let epoch = i as Epoch;
                let terminal = terminals[i];
                let new_l1 = l1s[i];
                let summary = EpochSummary::new(
                    epoch,
                    terminal,
                    prev_terminal,
                    new_l1,
                    finals[i],
                );
                prev_terminal = terminal;
                checkpoint_mgr
                    .insert_epoch_summary_blocking(summary)
                    .expect("insert summary");
                summaries.push(summary);
            }

            for i in 0..=last_checkpoint {
                let summary = &summaries[i];
                let tip = CheckpointTip::new(summary.epoch(), summary.new_l1().height(), *summary.terminal());
                let payload = CheckpointPayload::new(tip, sidecars[i].clone(), Vec::new())
                    .expect("payload");
                checkpoint_mgr
                    .put_checkpoint_payload_entry_blocking(summary.get_epoch_commitment(), payload)
                    .expect("put checkpoint");
            }

            let ctx = CheckpointWorkerContextImpl::new(storage, BridgeParams::default());
            let mut state = OLCheckpointServiceState::new(ctx);
            state.initialize();

            assert_eq!(state.last_processed_epoch(), Some(last_checkpoint as Epoch));
        }
    }

    proptest! {
        #[test]
        fn builds_checkpoint_from_epoch_summary(
            prev_terminal in ol_block_commitment_strategy(),
            slot_offset in 1..u64::MAX,
            body_root in buf32_strategy(),
            logs_root in buf32_strategy(),
            genesis_l1 in l1_block_commitment_strategy(),
            new_l1 in l1_block_commitment_strategy(),
            final_state in buf32_strategy(),
            state_diff in state_diff_strategy(),
            ol_logs in ol_logs_strategy(),
        ) {
            let backend = get_test_sled_backend();
            let storage = Arc::new(
                create_node_storage(backend, strata_storage::test_runtime_handle()).expect("test storage"),
            );
            let checkpoint_mgr = storage.ol_checkpoint();
            let ol_block_mgr = storage.ol_block();

            let epoch: Epoch = 1;
            let prev_terminal: OLBlockCommitment = prev_terminal;
            let terminal_slot = prev_terminal.slot().saturating_add(slot_offset);
            let terminal_header = OLBlockHeader::new(
                1_700_000_000,
                BlockFlags::zero(),
                terminal_slot,
                epoch,
                *prev_terminal.blkid(),
                body_root,
                final_state,
                logs_root,
            );

            let terminal_block = OLBlock::new(
                SignedOLBlockHeader::new(terminal_header.clone(), Buf64::zero()),
                OLBlockBody::new_common(
                    OLTxSegment::new(vec![])
                        .expect("empty tx segment construction is infallible"),
                ),
            );
            ol_block_mgr
                .put_block_data_blocking(terminal_block)
                .expect("insert terminal block");

            let terminal = terminal_header.compute_block_commitment();
            let genesis_summary =
                EpochSummary::new(0, prev_terminal, OLBlockCommitment::null(), genesis_l1, final_state);
            checkpoint_mgr
                .insert_epoch_summary_blocking(genesis_summary)
                .expect("insert genesis summary");
            let summary = EpochSummary::new(epoch, terminal, prev_terminal, new_l1, final_state);
            let commitment = summary.get_epoch_commitment();
            checkpoint_mgr
                .insert_epoch_summary_blocking(summary)
                .expect("insert summary");

            let ctx = TestCheckpointContext::new(Arc::clone(&storage), state_diff, ol_logs);
            let mut state = OLCheckpointServiceState::new(ctx);
            state.initialize();

            state
                .handle_complete_epoch(commitment)
                .expect("build checkpoint");

            let stored = checkpoint_mgr
                .get_checkpoint_payload_entry_blocking(commitment)
                .expect("get checkpoint")
                .expect("checkpoint should be stored");
            let sidecar_terminal_subset = stored.sidecar().terminal_header_complement();
            let stored_tip = stored.new_tip();

            prop_assert_eq!(stored_tip.epoch, epoch);
            prop_assert_eq!(stored_tip.l1_height(), new_l1.height());
            prop_assert_eq!(*stored_tip.l2_commitment(), terminal);
            prop_assert_eq!(sidecar_terminal_subset.timestamp(), terminal_header.timestamp());
            prop_assert_eq!(*sidecar_terminal_subset.parent_blkid(), *terminal_header.parent_blkid());
            prop_assert_eq!(*sidecar_terminal_subset.body_root(), *terminal_header.body_root());
            prop_assert_eq!(*sidecar_terminal_subset.logs_root(), *terminal_header.logs_root());
            prop_assert!(stored.proof().is_empty());
        }
    }

    /// Exercises the real (non-stubbed) `fetch_da_for_epoch` replay, which the
    /// fixture-derived chain-worker tests cannot reach.
    #[test]
    fn build_checkpoint_payload_includes_withdrawal_log_from_real_replay() {
        let withdraw_sats: u64 = 100_000_000;
        let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
        let withdrawal_dest = make_p2wpkh_bosd_descriptor(0x14);

        // Build an epoch whose snark update emits a withdrawal, then seal it.
        let mut sim_state = make_genesis_state();
        seed_accounts(&mut sim_state);
        let genesis = run_genesis(&mut sim_state);
        let pre_epoch_state = sim_state.clone().into_inner();

        let mut blocks = Vec::new();
        let inbox_msg = snark_inbox_msg();
        let gam = OLTransaction::new(
            OLTransactionData::from_gam_bytes(snark_id, inbox_msg.payload().data().to_vec())
                .expect("gam payload"),
            TxProofs::new_empty(),
        );
        let prev = run_block(
            &mut sim_state,
            &mut blocks,
            genesis.header(),
            BlockComponents::new_txs_from_ol_transactions(vec![gam]),
        );

        let mut tracker = InboxMmrTracker::new();
        let proof = tracker.add_message(&inbox_msg);
        let (_, snark_state) = get_snark_state_expect(&sim_state, snark_id);
        let withdrawal_update = SnarkUpdateBuilder::from_snark_state(snark_state.clone())
            .with_processed_msgs(vec![inbox_msg.clone()])
            .with_inbox_proofs(vec![proof])
            .with_output_message(
                BRIDGE_GATEWAY_ACCT_ID,
                withdraw_sats,
                make_withdrawal_payload(withdrawal_dest.clone()),
            )
            .build(snark_id, make_state_root(2), vec![0u8; 32]);
        let prev = run_block(
            &mut sim_state,
            &mut blocks,
            &prev,
            BlockComponents::new_txs_from_ol_transactions(vec![withdrawal_update]),
        );
        let terminal = run_terminal(
            &mut sim_state,
            &mut blocks,
            &prev,
            make_empty_manifest(TERMINAL_L1_HEIGHT, 0),
        );

        let post_epoch_state = sim_state.into_inner();
        let terminal_header = terminal.header().clone();

        // The real replay reads the genesis terminal block and its state plus
        // every epoch block from storage, so persist all of them.
        let backend = get_test_sled_backend();
        let storage = Arc::new(
            create_node_storage(backend, strata_storage::test_runtime_handle())
                .expect("test storage"),
        );
        let ol_block_mgr = storage.ol_block();
        let ol_state_mgr = storage.ol_state();
        let checkpoint_mgr = storage.ol_checkpoint();

        ol_block_mgr
            .put_block_data_blocking(to_ol_block(&genesis))
            .expect("insert genesis block");
        for block in &blocks {
            ol_block_mgr
                .put_block_data_blocking(block.clone())
                .expect("insert epoch block");
        }

        let genesis_commitment =
            OLBlockCommitment::new(genesis.header().slot(), genesis.header().compute_blkid());
        ol_state_mgr
            .put_toplevel_ol_state_blocking(genesis_commitment, pre_epoch_state.clone())
            .expect("insert pre-epoch state");

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
            *genesis.header().state_root(),
        );
        checkpoint_mgr
            .insert_epoch_summary_blocking(genesis_summary)
            .expect("insert genesis summary");

        let terminal_commitment =
            OLBlockCommitment::new(terminal_header.slot(), terminal_header.compute_blkid());
        let post_epoch_l1 = L1BlockCommitment::new(
            post_epoch_state.epoch_state().last_l1_height(),
            *post_epoch_state.epoch_state().last_l1_blkid(),
        );
        let summary = EpochSummary::new(
            terminal_header.epoch(),
            terminal_commitment,
            genesis_commitment,
            post_epoch_l1,
            *terminal_header.state_root(),
        );
        let commitment = summary.get_epoch_commitment();
        checkpoint_mgr
            .insert_epoch_summary_blocking(summary)
            .expect("insert summary");

        let ctx = CheckpointWorkerContextImpl::new(Arc::clone(&storage), BridgeParams::default());
        let mut state = OLCheckpointServiceState::new(ctx);
        state.initialize();
        state
            .handle_complete_epoch(commitment)
            .expect("build checkpoint");

        let stored = checkpoint_mgr
            .get_checkpoint_payload_entry_blocking(commitment)
            .expect("get checkpoint")
            .expect("checkpoint should be stored");

        // The sidecar must carry the withdrawal-intent log, and it must decode
        // to the amount and destination the update emitted.
        let withdrawal = stored
            .sidecar()
            .ol_logs()
            .iter()
            .find(|log| log.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL)
            .map(|log| {
                OLLog::new(log.account_serial(), log.payload().to_vec())
                    .try_into_log::<SimpleWithdrawalIntentLogData>()
                    .expect("bridge-gateway log must decode as a withdrawal intent")
            })
            .expect("sidecar must contain the bridge-gateway withdrawal log");
        assert_eq!(
            withdrawal.amt, withdraw_sats,
            "withdrawal amount must match"
        );
        assert_eq!(
            withdrawal.dest.as_slice(),
            withdrawal_dest.as_slice(),
            "withdrawal destination must match the emitted descriptor"
        );
        assert_eq!(
            withdrawal.selected_operator,
            u32::MAX,
            "withdrawal must preserve the any-operator sentinel"
        );

        // The withdrawal must have debited the account, not routed to limbo.
        assert_eq!(
            MemoryStateBaseLayer::new(post_epoch_state.clone())
                .get_account_state(snark_id)
                .unwrap()
                .unwrap()
                .balance(),
            BitcoinAmount::from_sat(0),
            "withdrawal must debit the full seeded balance"
        );
    }
}
