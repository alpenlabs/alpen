//! Service state for OL checkpoint builder.

use metrics::{counter, gauge};
use strata_asm_checkpoint_types::{
    CheckpointPayload, CheckpointSidecar, CheckpointTip, OLLog as CheckpointOLLog,
    TerminalHeaderComplement,
};
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Epoch, EpochCommitment};
use strata_service::ServiceState;
use tracing::info;

use crate::context::{CheckpointWorkerContext, ProofWaitOutcome};
#[cfg(feature = "debug-utils")]
use crate::debug_faults::apply_requested_proof_corruption;
use crate::errors::CheckpointNotReady;

/// Service state for OL checkpoint builder.
///
/// Generic over the context to allow testing with mock implementations.
pub(crate) struct OLCheckpointServiceState<C: CheckpointWorkerContext> {
    ctx: C,
    initialized: bool,
    last_processed: Option<EpochCommitment>,
    history_base: Option<EpochCommitment>,
}

#[derive(Debug, PartialEq, Eq)]
enum EpochProcessingOutcome {
    Complete,
    Superseded,
}

impl<C: CheckpointWorkerContext> OLCheckpointServiceState<C> {
    pub(crate) fn new(ctx: C) -> Self {
        Self {
            ctx,
            initialized: false,
            last_processed: None,
            history_base: None,
        }
    }

    pub(crate) fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub(crate) fn last_processed_epoch(&self) -> Option<Epoch> {
        self.last_processed.map(|commitment| commitment.epoch())
    }

    pub(crate) fn initialize(&mut self) -> anyhow::Result<()> {
        self.history_base = self.ctx.get_history_base()?;
        self.last_processed = self.history_base;
        self.init_cursor_from_db()?;
        self.initialized = true;
        Ok(())
    }

    fn latest_completed_epoch(&self) -> anyhow::Result<Option<Epoch>> {
        let Some(tip) = self.ctx.get_canonical_tip()? else {
            return Ok(None);
        };
        let header = self
            .ctx
            .get_block_header(&tip)?
            .ok_or_else(|| anyhow::anyhow!("missing canonical tip header {tip}"))?;
        Ok(if header.is_terminal() {
            Some(header.epoch())
        } else {
            header.epoch().checked_sub(1)
        })
    }

    fn epoch_summary(&self, commitment: EpochCommitment) -> anyhow::Result<EpochSummary> {
        let summary = self
            .ctx
            .get_epoch_summary(commitment)?
            .ok_or(CheckpointNotReady::EpochSummary(commitment))?;
        anyhow::ensure!(
            summary.get_epoch_commitment() == commitment,
            "epoch summary does not match commitment {commitment}"
        );
        Ok(summary)
    }

    /// Reconciles the processed branch and catches up using current canonical reads.
    pub(crate) fn handle_chain_update(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(self.initialized, "worker not initialized");
        loop {
            let Some(target) = self.latest_completed_epoch()? else {
                return Ok(());
            };
            self.reconcile_cursor()?;
            let next = match self.last_processed {
                Some(commitment) => commitment.epoch().checked_add(1),
                None => Some(1),
            };
            let Some(next) = next else {
                return Ok(());
            };
            let mut superseded = false;
            for epoch in next..=target {
                if self.process_epoch(epoch)? == EpochProcessingOutcome::Superseded {
                    superseded = true;
                    break;
                }
            }
            if !superseded {
                return Ok(());
            }
            // A proof wait may consume the time during which a new signal arrived.
            // Reconcile immediately instead of requiring another notification.
        }
    }

    fn reconcile_cursor(&mut self) -> anyhow::Result<()> {
        if let Some(commitment) = self.last_processed {
            self.last_processed = self
                .ctx
                .find_canonical_resume_point(commitment, self.history_base)?;
        }
        Ok(())
    }

    fn process_epoch(&mut self, epoch: Epoch) -> anyhow::Result<EpochProcessingOutcome> {
        let commitment = self
            .ctx
            .get_canonical_epoch_commitment_at(epoch)?
            .ok_or(CheckpointNotReady::EpochCommitment(epoch))?;
        let summary = self.epoch_summary(commitment)?;
        if let Some(previous) = self.last_processed
            && *summary.prev_terminal() != previous.to_block_commitment()
        {
            if self.ctx.get_canonical_block_at(previous.last_slot())?
                != Some(previous.to_block_commitment())
                || self.ctx.get_canonical_epoch_commitment_at(epoch)? != Some(commitment)
            {
                return Ok(EpochProcessingOutcome::Superseded);
            }
            anyhow::bail!("epoch summary predecessor does not match processed epoch {previous}");
        }

        if self.ctx.get_checkpoint_payload(commitment)?.is_none() {
            let Some(payload) = build_checkpoint_payload(commitment, &summary, &self.ctx)? else {
                return Ok(EpochProcessingOutcome::Superseded);
            };
            if self.ctx.get_canonical_epoch_commitment_at(epoch)? != Some(commitment) {
                return Ok(EpochProcessingOutcome::Superseded);
            }
            self.ctx.put_checkpoint_payload(commitment, payload)?;
            counter!("strata_checkpoint_created_total").increment(1);
            gauge!("strata_checkpoint_last_created_epoch").set(epoch as f64);
            info!(%commitment, "stored OL checkpoint entry");
        } else if self.ctx.get_canonical_epoch_commitment_at(epoch)? != Some(commitment) {
            return Ok(EpochProcessingOutcome::Superseded);
        }
        self.last_processed = Some(commitment);
        Ok(EpochProcessingOutcome::Complete)
    }

    fn init_cursor_from_db(&mut self) -> anyhow::Result<()> {
        let Some(last_payload) = self.ctx.get_last_checkpoint_payload_epoch()? else {
            return Ok(());
        };
        let Some(target) = self.latest_completed_epoch()? else {
            return Ok(());
        };
        let upper = target.min(last_payload.epoch());
        for epoch in (0..=upper)
            .rev()
            .take_while(|epoch| self.history_base.is_none_or(|base| *epoch > base.epoch()))
        {
            let Some(commitment) = self.ctx.get_canonical_epoch_commitment_at(epoch)? else {
                continue;
            };
            if self.ctx.get_checkpoint_payload(commitment)?.is_some() {
                self.epoch_summary(commitment)?;
                self.last_processed = Some(commitment);
                break;
            }
        }
        Ok(())
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
) -> anyhow::Result<Option<CheckpointPayload>> {
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
    match ctx.get_proof(&commitment)? {
        ProofWaitOutcome::Ready(proof) => {
            #[cfg(feature = "debug-utils")]
            let proof = apply_requested_proof_corruption(commitment, proof);
            Ok(Some(CheckpointPayload::new(new_tip, sidecar, proof)?))
        }
        ProofWaitOutcome::Superseded => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use proptest::prelude::*;
    use strata_acct_types::BitcoinAmount;
    use strata_asm_checkpoint_types::test_utils::{
        checkpoint_sidecar_strategy, create_test_checkpoint_payload,
    };
    use strata_asm_checkpoint_types::{
        AsmManifestRangeHash, CheckpointPayload, CheckpointTip, PendingPredicateTransition,
    };
    use strata_asm_common::AsmLogEntry;
    use strata_asm_logs::CheckpointPredicateEnacted;
    use strata_asm_params::CheckpointInitConfig;
    use strata_checkpoint_types::EpochSummary;
    use strata_checkpoint_verification::{CheckpointL1Range, CheckpointState, verify_progression};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::DbError;
    use strata_db_types::backend::DatabaseBackend;
    use strata_db_types::ol_checkpoint::OLCheckpointDatabase;
    use strata_identifiers::test_utils::{
        buf32_strategy, l1_block_commitment_strategy, ol_block_commitment_strategy,
    };
    use strata_identifiers::{
        AccountSerial, BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL, Buf32, Buf64, Epoch,
        L1BlockCommitment, L1BlockId, OLBlockCommitment, Slot,
    };
    use strata_ol_block_ancestry_provider::EpochBlockAncestry;
    use strata_ol_chain_types_v1::{
        AsmManifest, BlockFlagsV1, OLBlockBodyV1, OLBlockHeaderV1, OLBlockId, OLBlockV1, OLLog,
        OLTxSegmentV1, SignedOLBlockHeaderV1, SimpleWithdrawalIntentLogData,
    };
    use strata_ol_params::OLRuntimeParams;
    use strata_ol_state_container::OLStateContainer;
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_ol_state_types::{
        IAccountState, IStateAccessor, IStateAccessorMut, OLSpecId, OLSpecVersions,
    };
    use strata_ol_stf_v1::test_utils::{
        EPOCH_RUNNER_GENESIS_TIMESTAMP, EPOCH_RUNNER_TERMINAL_L1_HEIGHT as TERMINAL_L1_HEIGHT,
        FixtureAsmManifestBuilder, InboxMmrTracker, OLStfFixture, SnarkUpdateBuilder,
        TEST_SNARK_ACCOUNT_ID, epoch_runner_run_block as run_block,
        epoch_runner_run_genesis as run_genesis, epoch_runner_run_terminal as run_terminal,
        epoch_runner_seed_accounts as seed_accounts, execute_block, get_snark_state_expect,
        make_account_id, make_checkpoint_predicate_enactment_manifest, make_empty_manifest,
        make_genesis_state, make_p2wpkh_bosd_descriptor, make_state_root, make_withdrawal_payload,
        snark_inbox_msg, tamper_state_root, to_ol_block,
    };
    use strata_ol_stf_v1::{BlockComponents, BlockInfo};
    use strata_ol_tx_types_v1::{OLTransactionDataV1, OLTransactionV1, TxProofsV1};
    use strata_predicate::PredicateKey;
    use strata_primitives::epoch::EpochCommitment;
    use strata_service::SyncService;
    use strata_status::{OLSyncStatus, OLSyncStatusUpdate};
    use strata_storage::create_node_storage;

    use super::OLCheckpointServiceState;
    use crate::context::{
        CheckpointWorkerContext, CheckpointWorkerContextImpl, ProofWaitOutcome, StateDiffRaw,
    };
    use crate::errors::CheckpointNotReady;
    use crate::service::OLCheckpointService;

    type ProofResponse =
        Box<dyn Fn(&EpochCommitment) -> anyhow::Result<ProofWaitOutcome> + Send + Sync>;

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

    /// Test context that delegates everything to the real impl but stubs out
    /// `fetch_da_for_epoch` with provided DA data. This avoids needing a full
    /// replay chain (prev terminal block, OL state, etc.) in structural tests.
    struct TestCheckpointContext {
        inner: CheckpointWorkerContextImpl,
        stub_state_diff: StateDiffRaw,
        stub_ol_logs: Vec<OLLog>,
        proof_requests: Mutex<Vec<EpochCommitment>>,
        proof_response: Option<ProofResponse>,
    }

    impl TestCheckpointContext {
        fn new(
            storage: Arc<strata_storage::NodeStorage>,
            stub_state_diff: StateDiffRaw,
            stub_ol_logs: Vec<OLLog>,
        ) -> Self {
            Self {
                inner: CheckpointWorkerContextImpl::new(
                    storage,
                    OLRuntimeParams::test_default(),
                    L1BlockCommitment::default(),
                ),
                stub_state_diff,
                stub_ol_logs,
                proof_requests: Mutex::new(Vec::new()),
                proof_response: None,
            }
        }
    }

    impl CheckpointWorkerContext for TestCheckpointContext {
        fn find_canonical_resume_point(
            &self,
            last_processed: EpochCommitment,
            history_base: Option<EpochCommitment>,
        ) -> anyhow::Result<Option<EpochCommitment>> {
            self.inner
                .find_canonical_resume_point(last_processed, history_base)
        }

        fn get_canonical_tip(&self) -> anyhow::Result<Option<OLBlockCommitment>> {
            self.inner.get_canonical_tip()
        }

        fn get_canonical_block_at(&self, slot: Slot) -> anyhow::Result<Option<OLBlockCommitment>> {
            self.inner.get_canonical_block_at(slot)
        }

        fn get_canonical_epoch_commitment_at(
            &self,
            epoch: Epoch,
        ) -> anyhow::Result<Option<EpochCommitment>> {
            self.inner.get_canonical_epoch_commitment_at(epoch)
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

        fn get_history_base(&self) -> anyhow::Result<Option<EpochCommitment>> {
            self.inner.get_history_base()
        }

        fn put_checkpoint_payload(
            &self,
            commitment: EpochCommitment,
            payload: CheckpointPayload,
        ) -> anyhow::Result<()> {
            self.inner.put_checkpoint_payload(commitment, payload)
        }

        fn get_proof(&self, epoch: &EpochCommitment) -> anyhow::Result<ProofWaitOutcome> {
            self.proof_requests.lock().unwrap().push(*epoch);
            match &self.proof_response {
                Some(response) => response(epoch),
                None => self.inner.get_proof(epoch),
            }
        }

        fn get_block_header(
            &self,
            blkid: &OLBlockCommitment,
        ) -> anyhow::Result<Option<OLBlockHeaderV1>> {
            self.inner.get_block_header(blkid)
        }

        fn collect_epoch_block_commitments(
            &self,
            start: &OLBlockHeaderV1,
        ) -> anyhow::Result<EpochBlockAncestry> {
            self.inner.collect_epoch_block_commitments(start)
        }

        fn get_block(&self, id: &OLBlockId) -> anyhow::Result<Option<OLBlockV1>> {
            self.inner.get_block(id)
        }

        fn get_ol_state(
            &self,
            commitment: &OLBlockCommitment,
        ) -> anyhow::Result<Option<OLStateContainer>> {
            self.inner.get_ol_state(commitment)
        }

        fn genesis_l1_block(&self) -> L1BlockCommitment {
            self.inner.genesis_l1_block()
        }

        fn get_l1_manifest(
            &self,
            block: &L1BlockCommitment,
        ) -> anyhow::Result<Option<AsmManifest>> {
            self.inner.get_l1_manifest(block)
        }

        fn fetch_da_for_epoch(
            &self,
            _summary: &EpochSummary,
        ) -> anyhow::Result<(StateDiffRaw, Vec<OLLog>)> {
            Ok((self.stub_state_diff.clone(), self.stub_ol_logs.clone()))
        }
    }

    fn create_cursor_test_storage() -> Arc<strata_storage::NodeStorage> {
        let backend = get_test_sled_backend();
        Arc::new(
            create_node_storage(backend, strata_storage::test_runtime_handle())
                .expect("test storage"),
        )
    }

    fn initialize_cursor(
        storage: Arc<strata_storage::NodeStorage>,
    ) -> OLCheckpointServiceState<CheckpointWorkerContextImpl> {
        let ctx = CheckpointWorkerContextImpl::new(
            storage,
            OLRuntimeParams::test_default(),
            L1BlockCommitment::default(),
        );
        let mut state = OLCheckpointServiceState::new(ctx);
        state.initialize().expect("initialize checkpoint cursor");
        state
    }

    /// Processes an epoch with a checkpoint worker and returns its stored payload.
    fn process_epoch_and_load_payload<C: CheckpointWorkerContext>(
        storage: &strata_storage::NodeStorage,
        ctx: C,
        commitment: EpochCommitment,
    ) -> CheckpointPayload {
        let mut state = OLCheckpointServiceState::new(ctx);
        state.initialize().expect("initialize checkpoint cursor");
        state.handle_chain_update().expect("build checkpoint");
        storage
            .ol_checkpoint()
            .get_checkpoint_payload_entry_blocking(commitment)
            .expect("get checkpoint")
            .expect("checkpoint should be stored")
    }

    /// Decodes withdrawal intents emitted by the bridge gateway in a sidecar.
    fn decode_withdrawal_logs(payload: &CheckpointPayload) -> Vec<SimpleWithdrawalIntentLogData> {
        payload
            .sidecar()
            .ol_logs()
            .iter()
            .filter(|log| log.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL)
            .map(|log| {
                OLLog::new(log.account_serial(), log.payload().to_vec())
                    .try_into_log::<SimpleWithdrawalIntentLogData>()
                    .expect("bridge-gateway log must decode as a withdrawal intent")
            })
            .collect()
    }

    fn store_test_terminal(
        storage: &strata_storage::NodeStorage,
        epoch: Epoch,
        slot: u64,
        parent: OLBlockCommitment,
        seed: Buf32,
    ) -> OLBlockHeaderV1 {
        let mut flags = BlockFlagsV1::zero();
        flags.set_is_terminal(true);
        let header = OLBlockHeaderV1::new(
            1_000 + slot,
            flags,
            slot,
            epoch,
            *parent.blkid(),
            seed,
            seed,
            seed,
        );
        storage
            .ol_block()
            .put_terminal_header_blocking(header.compute_blkid(), header.clone())
            .expect("store test terminal header");
        header
    }

    fn set_canonical_tip(storage: &strata_storage::NodeStorage, tip: OLBlockCommitment) {
        storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(tip.slot(), vec![*tip.blkid()])
            .expect("set canonical tip");
    }

    fn set_history_base(storage: &strata_storage::NodeStorage, epoch: Epoch) -> EpochCommitment {
        let header = store_test_terminal(
            storage,
            epoch,
            u64::from(epoch),
            OLBlockCommitment::null(),
            Buf32::zero(),
        );
        let commitment = EpochCommitment::from_terminal(epoch, header.compute_block_commitment());
        storage
            .ol_block()
            .promote_to_history_anchor_blocking(commitment)
            .expect("set history base");
        commitment
    }

    fn put_checkpoint_payload(
        storage: &strata_storage::NodeStorage,
        epoch: Epoch,
    ) -> EpochCommitment {
        let payload = create_test_checkpoint_payload(epoch);
        let header = store_test_terminal(
            storage,
            epoch,
            u64::from(epoch),
            OLBlockCommitment::null(),
            Buf32::zero(),
        );
        let terminal = header.compute_block_commitment();
        set_canonical_tip(storage, terminal);
        let payload = CheckpointPayload::new(
            CheckpointTip::new(epoch, payload.new_tip().l1_height(), terminal),
            payload.sidecar().clone(),
            payload.proof().to_vec(),
        )
        .unwrap();
        let commitment = EpochCommitment::from_terminal(epoch, terminal);
        let summary = EpochSummary::new(
            epoch,
            terminal,
            OLBlockCommitment::null(),
            L1BlockCommitment::new(0, L1BlockId::default()),
            Buf32::zero(),
        );
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(summary)
            .expect("insert epoch summary");
        storage
            .ol_checkpoint()
            .put_checkpoint_payload_entry_blocking(commitment, payload)
            .expect("put checkpoint payload");
        commitment
    }

    #[test]
    fn init_cursor_without_history_base_or_payload_remains_none() {
        let state = initialize_cursor(create_cursor_test_storage());

        assert_eq!(state.last_processed_epoch(), None);
    }

    #[test]
    fn init_cursor_with_history_base_and_no_payload_uses_floor() {
        let storage = create_cursor_test_storage();
        set_history_base(&storage, 5);

        let state = initialize_cursor(storage);

        assert_eq!(state.last_processed_epoch(), Some(5));
    }

    #[test]
    fn init_cursor_with_payload_after_history_base_uses_payload() {
        let storage = create_cursor_test_storage();
        set_history_base(&storage, 5);
        put_checkpoint_payload(&storage, 6);

        let state = initialize_cursor(storage);

        assert_eq!(state.last_processed_epoch(), Some(6));
    }

    #[test]
    fn init_cursor_with_payload_before_history_base_uses_floor() {
        let storage = create_cursor_test_storage();
        put_checkpoint_payload(&storage, 4);
        set_history_base(&storage, 5);

        let state = initialize_cursor(storage);

        assert_eq!(state.last_processed_epoch(), Some(5));
    }

    #[test]
    fn init_cursor_uses_history_base_after_payload_is_deleted() {
        let storage = create_cursor_test_storage();
        set_history_base(&storage, 5);
        let payload_commitment = put_checkpoint_payload(&storage, 6);
        storage
            .db()
            .ol_checkpoint_db()
            .del_checkpoint_payload_entry(payload_commitment)
            .expect("delete checkpoint payload");

        let state = initialize_cursor(storage);

        assert_eq!(state.last_processed_epoch(), Some(5));
    }

    fn store_epoch(
        storage: &strata_storage::NodeStorage,
        previous: EpochCommitment,
        branch: u8,
    ) -> EpochCommitment {
        let epoch = previous.epoch() + 1;
        let header = store_test_terminal(
            storage,
            epoch,
            previous.last_slot() + 1,
            previous.to_block_commitment(),
            Buf32::from([branch; 32]),
        );
        let summary = EpochSummary::new(
            epoch,
            header.compute_block_commitment(),
            previous.to_block_commitment(),
            L1BlockCommitment::new(epoch, L1BlockId::default()),
            *header.state_root(),
        );
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(summary)
            .unwrap();
        summary.get_epoch_commitment()
    }

    fn publish_epochs(storage: &strata_storage::NodeStorage, epochs: &[EpochCommitment]) {
        storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(
                epochs[0].last_slot(),
                epochs.iter().map(|epoch| *epoch.last_blkid()).collect(),
            )
            .unwrap();
    }

    fn new_test_state(
        storage: &Arc<strata_storage::NodeStorage>,
    ) -> OLCheckpointServiceState<TestCheckpointContext> {
        OLCheckpointServiceState::new(TestCheckpointContext::new(
            Arc::clone(storage),
            vec![1, 2, 3],
            vec![],
        ))
    }

    fn wake_builder(
        state: &mut OLCheckpointServiceState<TestCheckpointContext>,
        advertised: EpochCommitment,
    ) {
        let status = OLSyncStatus::new(
            advertised.to_block_commitment(),
            advertised.epoch(),
            true,
            advertised,
            advertised,
            advertised,
            L1BlockCommitment::new(0, L1BlockId::default()),
        );
        OLCheckpointService::process_input(state, Some(OLSyncStatusUpdate::new(status))).unwrap();
    }

    #[test]
    fn test_waits_for_index_publication_and_ignores_notification_commitments() {
        let storage = create_cursor_test_storage();
        let genesis = set_history_base(&storage, 0);
        let first = store_epoch(&storage, genesis, 1);
        let second = store_epoch(&storage, first, 1);
        let fork = store_epoch(&storage, genesis, 2);
        let mut higher_fork = fork;
        for _ in 0..3 {
            higher_fork = store_epoch(&storage, higher_fork, 2);
        }
        let mut state = new_test_state(&storage);
        OLCheckpointService::on_launch(&mut state).unwrap();
        assert_eq!(state.last_processed, Some(genesis));
        assert_eq!(*state.ctx.proof_requests.lock().unwrap(), vec![]);

        // One wake-up covers both completed epochs. Its fields deliberately
        // describe a higher fork; only the published index selects candidates.
        publish_epochs(&storage, &[first, second]);
        wake_builder(&mut state, higher_fork);
        assert_eq!(state.last_processed, Some(second));
        assert_eq!(
            *state.ctx.proof_requests.lock().unwrap(),
            vec![first, second]
        );
        let payload = state.ctx.get_checkpoint_payload(first).unwrap().unwrap();
        assert_eq!(
            *payload.new_tip().l2_commitment(),
            first.to_block_commitment()
        );
        assert_eq!(state.ctx.get_checkpoint_payload(fork).unwrap(), None);
        assert_eq!(state.ctx.get_checkpoint_payload(higher_fork).unwrap(), None);

        wake_builder(&mut state, higher_fork);
        assert_eq!(
            *state.ctx.proof_requests.lock().unwrap(),
            vec![first, second]
        );
    }

    #[test]
    fn test_retries_unfinished_epoch_without_tip_advancing() {
        let storage = create_cursor_test_storage();
        let genesis = set_history_base(&storage, 0);
        let first = store_epoch(&storage, genesis, 1);
        let summary = storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(first)
            .unwrap()
            .unwrap();
        storage
            .ol_checkpoint()
            .del_epoch_summary_blocking(first)
            .unwrap();
        publish_epochs(&storage, &[first]);
        let mut state = new_test_state(&storage);
        OLCheckpointService::on_launch(&mut state).unwrap();
        assert_eq!(state.last_processed, Some(genesis));

        // The index and completed-epoch target remain unchanged when the
        // missing summary becomes available before the next wake-up.
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(summary)
            .unwrap();
        wake_builder(&mut state, genesis);
        assert_eq!(state.last_processed, Some(first));
        assert_eq!(*state.ctx.proof_requests.lock().unwrap(), vec![first]);
    }

    #[test]
    fn test_proof_failure_does_not_advance_cursor_or_store_payload() {
        let storage = create_cursor_test_storage();
        let genesis = set_history_base(&storage, 0);
        let first = store_epoch(&storage, genesis, 1);
        publish_epochs(&storage, &[first]);
        let mut state = new_test_state(&storage);
        state.ctx.proof_response = Some(Box::new(|_| {
            Err(DbError::Other("injected proof read failure".into()).into())
        }));
        let err = OLCheckpointService::on_launch(&mut state).unwrap_err();
        assert!(
            matches!(err.downcast_ref::<DbError>(), Some(DbError::Other(reason)) if reason == "injected proof read failure")
        );
        assert_eq!(state.last_processed, Some(genesis));
        assert_eq!(state.ctx.get_checkpoint_payload(first).unwrap(), None);
    }

    #[test]
    fn test_startup_catches_up_and_reuses_exact_canonical_payload() {
        let storage = create_cursor_test_storage();
        let genesis = set_history_base(&storage, 0);
        let first = store_epoch(&storage, genesis, 1);
        publish_epochs(&storage, &[first]);
        let mut state = new_test_state(&storage);
        OLCheckpointService::on_launch(&mut state).unwrap();
        assert_eq!(state.last_processed, Some(first));

        let second = store_epoch(&storage, first, 1);
        publish_epochs(&storage, &[second]);
        // Restart catches up without a notification and skips the cached first epoch.
        let mut restarted = new_test_state(&storage);
        OLCheckpointService::on_launch(&mut restarted).unwrap();
        assert_eq!(restarted.last_processed, Some(second));
        assert_eq!(*restarted.ctx.proof_requests.lock().unwrap(), vec![second]);
    }

    #[test]
    fn test_restart_ignores_payloads_from_a_higher_replaced_branch() {
        let storage = create_cursor_test_storage();
        let genesis = set_history_base(&storage, 0);
        let first = store_epoch(&storage, genesis, 1);
        let second = store_epoch(&storage, first, 1);
        let third = store_epoch(&storage, second, 1);
        publish_epochs(&storage, &[first, second, third]);
        OLCheckpointService::on_launch(&mut new_test_state(&storage)).unwrap();

        // The highest stored payload belongs to the old branch. Neither it nor
        // the old payloads at matching epoch numbers may advance the new cursor.
        let fork_first = store_epoch(&storage, genesis, 2);
        let fork_second = store_epoch(&storage, fork_first, 2);
        publish_epochs(&storage, &[fork_first, fork_second]);
        let mut restarted = new_test_state(&storage);
        OLCheckpointService::on_launch(&mut restarted).unwrap();
        assert_eq!(restarted.last_processed, Some(fork_second));
        assert_eq!(
            *restarted.ctx.proof_requests.lock().unwrap(),
            vec![fork_first, fork_second]
        );
        assert!(
            restarted
                .ctx
                .get_checkpoint_payload(third)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn test_same_epoch_becoming_terminal_triggers_checkpoint() {
        let storage = create_cursor_test_storage();
        let genesis = set_history_base(&storage, 0);
        let header = OLBlockHeaderV1::new(
            1_001,
            BlockFlagsV1::zero(),
            1,
            1,
            *genesis.last_blkid(),
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        let nonterminal = header.compute_block_commitment();
        storage
            .ol_block()
            .put_block_data_blocking(OLBlockV1::new(
                SignedOLBlockHeaderV1::new(header, Buf64::zero()),
                OLBlockBodyV1::new_common(OLTxSegmentV1::new(vec![]).unwrap()),
            ))
            .unwrap();
        set_canonical_tip(&storage, nonterminal);
        let mut state = new_test_state(&storage);
        OLCheckpointService::on_launch(&mut state).unwrap();
        assert_eq!(state.last_processed, Some(genesis));

        let terminal = store_test_terminal(&storage, 1, 2, nonterminal, Buf32::from([1; 32]));
        let summary = EpochSummary::new(
            1,
            terminal.compute_block_commitment(),
            genesis.to_block_commitment(),
            L1BlockCommitment::new(1, L1BlockId::default()),
            *terminal.state_root(),
        );
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(summary)
            .unwrap();
        let epoch = summary.get_epoch_commitment();
        publish_epochs(&storage, &[epoch]);
        wake_builder(&mut state, genesis);
        assert_eq!(state.last_processed, Some(epoch));
        assert_eq!(*state.ctx.proof_requests.lock().unwrap(), vec![epoch]);
    }

    #[test]
    fn test_reorg_rewinds_processed_branch_and_preserves_signed_payload() {
        let storage = create_cursor_test_storage();
        let floor = set_history_base(&storage, 5);
        let first = store_epoch(&storage, floor, 1);
        let second = store_epoch(&storage, first, 1);
        publish_epochs(&storage, &[first, second]);
        let mut state = new_test_state(&storage);
        OLCheckpointService::on_launch(&mut state).unwrap();
        let old_payload = state.ctx.get_checkpoint_payload(second).unwrap().unwrap();
        storage
            .ol_checkpoint()
            .put_checkpoint_signing_entry_blocking(second, 42)
            .unwrap();

        // Replace only the last completed epoch first, then replace both epochs.
        let replacement = store_epoch(&storage, first, 2);
        publish_epochs(&storage, &[replacement]);
        wake_builder(&mut state, second);
        assert_eq!(state.last_processed, Some(replacement));
        assert_eq!(
            *state.ctx.proof_requests.lock().unwrap(),
            vec![first, second, replacement]
        );
        assert_eq!(
            state.ctx.get_checkpoint_payload(second).unwrap(),
            Some(old_payload)
        );
        assert_eq!(
            storage
                .ol_checkpoint()
                .get_checkpoint_signing_entry_blocking(second)
                .unwrap(),
            Some(42)
        );

        let fork_first = store_epoch(&storage, floor, 3);
        let fork_second = store_epoch(&storage, fork_first, 3);
        publish_epochs(&storage, &[fork_first, fork_second]);
        wake_builder(&mut state, second);
        assert_eq!(state.last_processed, Some(fork_second));
        assert_eq!(
            *state.ctx.proof_requests.lock().unwrap(),
            vec![first, second, replacement, fork_first, fork_second]
        );

        // Returning to the old branch reuses only its exact payloads.
        publish_epochs(&storage, &[first, second]);
        let requests = state.ctx.proof_requests.lock().unwrap().clone();
        wake_builder(&mut state, fork_second);
        assert_eq!(state.last_processed, Some(second));
        assert_eq!(*state.ctx.proof_requests.lock().unwrap(), requests);
        assert_eq!(state.ctx.get_checkpoint_payload(floor).unwrap(), None);
    }

    #[test]
    fn test_reorg_during_proof_wait_retries_without_another_notification() {
        // Cover explicit cancellation and a proof that arrives just as its
        // branch is replaced. Neither may store the superseded payload.
        for proof_arrives in [false, true] {
            let storage = create_cursor_test_storage();
            let genesis = set_history_base(&storage, 0);
            let first = store_epoch(&storage, genesis, 1);
            let replacement = store_epoch(&storage, genesis, 2);
            publish_epochs(&storage, &[first]);
            let mut state = new_test_state(&storage);
            let proof_storage = Arc::clone(&storage);
            state.ctx.proof_response = Some(Box::new(move |epoch| {
                if *epoch == first {
                    publish_epochs(&proof_storage, &[replacement]);
                    if !proof_arrives {
                        return Ok(ProofWaitOutcome::Superseded);
                    }
                }
                Ok(ProofWaitOutcome::Ready(vec![]))
            }));
            OLCheckpointService::on_launch(&mut state).unwrap();
            assert_eq!(state.last_processed, Some(replacement));
            assert_eq!(
                *state.ctx.proof_requests.lock().unwrap(),
                vec![first, replacement]
            );
            assert_eq!(state.ctx.get_checkpoint_payload(first).unwrap(), None);
            assert!(
                state
                    .ctx
                    .get_checkpoint_payload(replacement)
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[test]
    fn test_rewind_rejects_missing_processed_summary() {
        let storage = create_cursor_test_storage();
        let genesis = set_history_base(&storage, 0);
        let first = store_epoch(&storage, genesis, 1);
        let second = store_epoch(&storage, first, 3);
        publish_epochs(&storage, &[first, second]);
        let mut state = new_test_state(&storage);
        OLCheckpointService::on_launch(&mut state).unwrap();
        let replacement = store_epoch(&storage, genesis, 2);
        publish_epochs(&storage, &[replacement]);
        storage
            .ol_checkpoint()
            .del_epoch_summary_blocking(first)
            .unwrap();

        let err = state.handle_chain_update().unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("missing processed epoch summary for {first}")
        );
        assert!(!err.is::<CheckpointNotReady>());
        // A later read failed after walking past the second epoch; progress stays intact.
        assert_eq!(state.last_processed, Some(second));
        assert_eq!(state.ctx.get_checkpoint_payload(replacement).unwrap(), None);
    }

    #[test]
    fn test_rewind_rejects_conflicting_history_anchor() {
        let storage = create_cursor_test_storage();
        let floor = set_history_base(&storage, 5);
        let first = store_epoch(&storage, floor, 1);
        publish_epochs(&storage, &[first]);
        let mut state = new_test_state(&storage);
        OLCheckpointService::on_launch(&mut state).unwrap();
        // Remove the processed suffix and replace the anchor's canonical row.
        let different_anchor = store_test_terminal(
            &storage,
            floor.epoch(),
            floor.last_slot(),
            OLBlockCommitment::null(),
            Buf32::from([9; 32]),
        );
        set_canonical_tip(&storage, different_anchor.compute_block_commitment());
        let err = state.handle_chain_update().unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("canonical history conflicts with retained anchor {floor}")
        );
    }

    #[test]
    fn test_inconsistent_summary_predecessor_errors() {
        let storage = create_cursor_test_storage();
        let floor = set_history_base(&storage, 5);
        let unrelated = EpochCommitment::from_terminal(
            floor.epoch(),
            OLBlockCommitment::new(floor.last_slot(), OLBlockId::from(Buf32::from([9; 32]))),
        );
        let first = store_epoch(&storage, unrelated, 1);
        publish_epochs(&storage, &[first]);
        let mut state = new_test_state(&storage);
        state.initialize().unwrap();
        // The retained anchor need not have an epoch-summary record. A mismatched
        // predecessor must still fail, rather than retrying it as a reorg forever.
        let err = state.handle_chain_update().unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("epoch summary predecessor does not match processed epoch {floor}")
        );
        assert_eq!(state.ctx.get_checkpoint_payload(first).unwrap(), None);
    }

    proptest! {
        #[test]
        fn init_cursor_from_db_uses_last_checkpoint_payload_epoch(
            len in 1usize..=5,
            l1s in prop::collection::vec(l1_block_commitment_strategy(), 1..=5),
            finals in prop::collection::vec(buf32_strategy(), 1..=5),
            sidecars in prop::collection::vec(checkpoint_sidecar_strategy(), 1..=5),
            last_checkpoint in 0usize..=4,
        ) {
            let len = len.min(l1s.len())
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
                let header = store_test_terminal(&storage, epoch, i as u64, prev_terminal, finals[i]);
                let terminal = header.compute_block_commitment();
                set_canonical_tip(&storage, terminal);
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

            let ctx = CheckpointWorkerContextImpl::new(
                storage,
                OLRuntimeParams::test_default(),
                L1BlockCommitment::default(),
            );
            let mut state = OLCheckpointServiceState::new(ctx);
            state.initialize().expect("initialize checkpoint cursor");

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
            let mut flags = BlockFlagsV1::zero();
            flags.set_is_terminal(true);
            let terminal_header = OLBlockHeaderV1::new(
                1_700_000_000,
                flags,
                terminal_slot,
                epoch,
                *prev_terminal.blkid(),
                body_root,
                final_state,
                logs_root,
            );

            let terminal_block = OLBlockV1::new(
                SignedOLBlockHeaderV1::new(terminal_header.clone(), Buf64::zero()),
                OLBlockBodyV1::new_common(
                    OLTxSegmentV1::new(vec![])
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

            set_canonical_tip(&storage, terminal);
            let ctx = TestCheckpointContext::new(Arc::clone(&storage), state_diff, ol_logs);
            let stored = process_epoch_and_load_payload(&storage, ctx, commitment);
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

    /// Checks the L1 range produced by OL execution and checkpoint DA replay
    /// against ASM. The checkpoint ends at the predicate boundary `B`, and the
    /// worker replays the epoch after it under V2. That epoch ends at another
    /// enactment, and the worker builds no checkpoint for the epoch after it:
    /// that epoch runs V2's successor, which this binary does not implement.
    #[test]
    fn checkpoint_payload_ends_at_predicate_boundary_and_stops_after_v2() {
        let boundary = 2;
        let v2_boundary = boundary + 1;
        let new_predicate = PredicateKey::never_accept();
        let enactment =
            AsmLogEntry::from_log(&CheckpointPredicateEnacted::new(new_predicate.clone()))
                .expect("enactment log encodes");
        let mut fixture = OLStfFixture::builder().execute_genesis();
        let storage = create_cursor_test_storage();
        // Genesis processes no manifests, so it still ends on the L1 anchor.
        let genesis_l1 = L1BlockCommitment::new(
            fixture.state().last_l1_height(),
            *fixture.state().last_l1_blkid(),
        );
        let genesis = fixture.last_completed_block();
        let genesis_commitment =
            OLBlockCommitment::new(genesis.header().slot(), genesis.header().compute_blkid());
        let mut previous_terminal = OLBlockCommitment::null();

        // Persist genesis and three terminal blocks with their executed states,
        // and the non-terminal block that starts epoch 2. The checkpoint worker
        // replays each epoch from the preceding terminal state.
        for epoch in 0..=3 {
            if epoch == 1 {
                let boundary_manifest = FixtureAsmManifestBuilder::new_at_height(boundary)
                    .with_logs(vec![enactment.clone()])
                    .build();
                storage
                    .l1()
                    .put_block_data(boundary_manifest.clone())
                    .unwrap();
                fixture
                    .child_block()
                    .with_manifest(FixtureAsmManifestBuilder::new_at_height(1).build())
                    .with_manifest(boundary_manifest)
                    .terminal()
                    .execute();
            } else if epoch == 2 {
                // The epoch after B runs V2, whose first block takes over the
                // V1 state. The fixture runs the V1 code, which leaves the V2
                // versions as they are, so it builds the epoch as V2 does.
                fixture
                    .state_mut()
                    .set_spec_versions(OLSpecVersions::uniform(OLSpecId::V2));
                // A non-terminal first block, whose root commits V2, makes a
                // replay of the epoch under V1 fail its root check.
                fixture.child_block().execute();
                storage
                    .ol_block()
                    .put_block_data_blocking(to_ol_block(fixture.last_completed_block()))
                    .unwrap();
                let v2_boundary_manifest = FixtureAsmManifestBuilder::new_at_height(v2_boundary)
                    .with_logs(vec![enactment.clone()])
                    .build();
                storage
                    .l1()
                    .put_block_data(v2_boundary_manifest.clone())
                    .unwrap();
                fixture
                    .child_block()
                    .with_manifest(v2_boundary_manifest)
                    .terminal()
                    .execute();
            } else if epoch == 3 {
                fixture
                    .child_block()
                    .with_manifest(
                        FixtureAsmManifestBuilder::new_at_height(v2_boundary + 1).build(),
                    )
                    .terminal()
                    .execute();
            }
            let block = fixture.last_completed_block();
            let header = block.header();
            let terminal = OLBlockCommitment::new(header.slot(), header.compute_blkid());
            let last_l1 = L1BlockCommitment::new(
                fixture.state().last_l1_height(),
                *fixture.state().last_l1_blkid(),
            );
            let state = fixture.state().to_container();
            storage
                .ol_block()
                .put_block_data_blocking(to_ol_block(block))
                .unwrap();
            storage
                .ol_state()
                .put_toplevel_ol_state_blocking(terminal, state)
                .unwrap();
            storage
                .ol_checkpoint()
                .insert_epoch_summary_blocking(EpochSummary::new(
                    header.epoch(),
                    terminal,
                    previous_terminal,
                    last_l1,
                    *header.state_root(),
                ))
                .unwrap();
            set_canonical_tip(&storage, terminal);
            previous_terminal = terminal;
        }

        let mut asm = CheckpointState::init(CheckpointInitConfig {
            sequencer_key: Buf32::zero(),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1_height: 0,
            genesis_ol_blkid: *genesis_commitment.blkid(),
        });
        asm.queue_predicate_transition(PendingPredicateTransition::new(
            new_predicate.clone(),
            boundary,
        ));

        // The worker builds the checkpoints of the epoch that ends at B and of
        // the V2 epoch after it, then refuses to replay the epoch after the V2
        // enactment and stores nothing for it.
        let ctx = CheckpointWorkerContextImpl::new(
            Arc::clone(&storage),
            OLRuntimeParams::test_default(),
            genesis_l1,
        );
        let ends_at_b = ctx.get_canonical_epoch_commitment_at(1).unwrap().unwrap();
        let after_b = ctx.get_canonical_epoch_commitment_at(2).unwrap().unwrap();
        let after_v2 = ctx.get_canonical_epoch_commitment_at(3).unwrap().unwrap();
        let mut state = OLCheckpointServiceState::new(ctx);
        state.initialize().expect("initialize checkpoint cursor");
        let err = state
            .handle_chain_update()
            .expect_err("no checkpoint follows the V2 enactment");
        assert!(format!("{err:#}").contains("upgrade required"), "{err:#}");
        assert_eq!(state.last_processed_epoch(), Some(2));
        assert!(
            storage
                .ol_checkpoint()
                .get_checkpoint_payload_entry_blocking(after_v2)
                .unwrap()
                .is_none()
        );
        let v2_payload = storage
            .ol_checkpoint()
            .get_checkpoint_payload_entry_blocking(after_b)
            .unwrap()
            .expect("the checkpoint of the V2 epoch is stored");
        assert_eq!(v2_payload.new_tip().l1_height(), v2_boundary);

        let payload = storage
            .ol_checkpoint()
            .get_checkpoint_payload_entry_blocking(ends_at_b)
            .unwrap()
            .expect("the checkpoint ending at B is stored");
        let coverage = verify_progression(
            asm.verified_tip(),
            payload.new_tip(),
            boundary + 2,
            asm.next_transition(),
        )
        .expect("OL-produced checkpoint stays within its predicate territory");
        assert_eq!(payload.new_tip().l1_height(), boundary);
        assert_eq!(
            coverage,
            CheckpointL1Range::Range {
                start_height: 1,
                end_height: boundary,
            }
        );
        // Extending this same checkpoint by one L1 block must fail.
        let crossing_tip = CheckpointTip::new(1, boundary + 1, *payload.new_tip().l2_commitment());
        assert!(
            verify_progression(
                asm.verified_tip(),
                &crossing_tip,
                boundary + 2,
                asm.next_transition(),
            )
            .is_err()
        );
        // The old AlwaysAccept key admits the checkpoint ending at B, then ASM
        // promotes the new key for the following checkpoint.
        asm.advance(&payload, AsmManifestRangeHash::ZERO).unwrap();
        assert_eq!(asm.checkpoint_predicate(), &new_predicate);
        assert!(asm.next_transition().is_none());
    }

    /// The checkpoint worker replays the first V1 epoch after the last V0
    /// terminal, which processed the enactment that ends V0, under V1, as
    /// block assembly built it. Replaying it under V0 would fail: this binary
    /// does not execute V0 blocks.
    #[test]
    fn builds_the_first_v1_checkpoint_after_the_v0_enactment() {
        let mut sim_state = make_genesis_state();
        seed_accounts(&mut sim_state);
        let genesis_anchor =
            L1BlockCommitment::new(sim_state.last_l1_height(), *sim_state.last_l1_blkid());
        let enactment =
            make_checkpoint_predicate_enactment_manifest(genesis_anchor.height() + 1, 1);
        let genesis = execute_block(
            &mut sim_state,
            &BlockInfo::new_genesis(EPOCH_RUNNER_GENESIS_TIMESTAMP),
            None,
            BlockComponents::new_manifests(vec![enactment.clone()]).as_terminal(),
        )
        .expect("genesis executes");

        // Relabel genesis as the last V0 terminal, under its bare root.
        sim_state.set_spec_versions(OLSpecVersions::uniform(OLSpecId::V0));
        let v0_root = sim_state.compute_state_root().expect("V0 root");
        let genesis_header = tamper_state_root(genesis.header(), v0_root);
        let genesis_commitment = genesis_header.compute_block_commitment();
        let genesis_l1 =
            L1BlockCommitment::new(sim_state.last_l1_height(), *sim_state.last_l1_blkid());
        let pre_epoch_state = sim_state.to_container();

        let mut blocks = Vec::new();
        let terminal = run_terminal(
            &mut sim_state,
            &mut blocks,
            &genesis_header,
            make_empty_manifest(genesis_l1.height() + 1, 1),
        );
        let terminal_header = terminal.header().clone();
        let terminal_commitment = terminal_header.compute_block_commitment();
        let post_epoch_l1 =
            L1BlockCommitment::new(sim_state.last_l1_height(), *sim_state.last_l1_blkid());

        let storage = create_cursor_test_storage();
        storage.l1().put_block_data(enactment).unwrap();
        storage
            .ol_block()
            .put_block_data_blocking(OLBlockV1::new(
                SignedOLBlockHeaderV1::new(genesis_header.clone(), Buf64::zero()),
                genesis.body().clone(),
            ))
            .unwrap();
        for block in &blocks {
            storage
                .ol_block()
                .put_block_data_blocking(block.clone())
                .unwrap();
        }
        storage
            .ol_state()
            .put_toplevel_ol_state_blocking(genesis_commitment, pre_epoch_state)
            .unwrap();
        let genesis_summary = EpochSummary::new(
            0,
            genesis_commitment,
            OLBlockCommitment::null(),
            genesis_l1,
            v0_root,
        );
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(genesis_summary)
            .unwrap();
        let summary = genesis_summary.create_next_epoch_summary(
            terminal_commitment,
            post_epoch_l1,
            *terminal_header.state_root(),
        );
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(summary)
            .unwrap();

        set_canonical_tip(&storage, terminal_commitment);
        let ctx = CheckpointWorkerContextImpl::new(
            Arc::clone(&storage),
            OLRuntimeParams::test_default(),
            genesis_anchor,
        );
        let payload = process_epoch_and_load_payload(&storage, ctx, summary.get_epoch_commitment());
        assert_eq!(payload.new_tip().l2_commitment(), &terminal_commitment);
        assert_eq!(payload.new_tip().l1_height(), post_epoch_l1.height());
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
        let genesis_anchor =
            L1BlockCommitment::new(sim_state.last_l1_height(), *sim_state.last_l1_blkid());
        let genesis = run_genesis(&mut sim_state);
        let pre_epoch_state = sim_state.to_container();
        let genesis_l1 =
            L1BlockCommitment::new(sim_state.last_l1_height(), *sim_state.last_l1_blkid());

        let mut blocks = Vec::new();
        let inbox_msg = snark_inbox_msg();
        let gam = OLTransactionV1::new(
            OLTransactionDataV1::from_gam_bytes(snark_id, inbox_msg.payload().data().to_vec())
                .expect("gam payload"),
            TxProofsV1::new_empty(),
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

        let post_epoch_l1 =
            L1BlockCommitment::new(sim_state.last_l1_height(), *sim_state.last_l1_blkid());
        let post_epoch_state = sim_state.into_container();
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
        // Spec selection reads the manifest the genesis epoch processed last.
        for manifest in genesis
            .body()
            .manifests()
            .map(|container| container.manifests())
            .unwrap_or_default()
        {
            storage
                .l1()
                .put_block_data(manifest.clone())
                .expect("insert genesis manifest");
        }
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

        set_canonical_tip(&storage, *summary.terminal());
        let ctx = CheckpointWorkerContextImpl::new(
            Arc::clone(&storage),
            OLRuntimeParams::test_default(),
            genesis_anchor,
        );
        let stored = process_epoch_and_load_payload(&storage, ctx, commitment);

        // The sidecar must carry the withdrawal-intent log, and it must decode
        // to the amount and destination the update emitted.
        let withdrawals = decode_withdrawal_logs(&stored);
        assert_eq!(withdrawals.len(), 1, "must have only one withdrawal log");
        let withdrawal = withdrawals
            .first()
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
            MemoryStateBaseLayer::from_container(post_epoch_state.clone())
                .get_account_state(snark_id)
                .unwrap()
                .unwrap()
                .balance(),
            BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
            "withdrawal must debit the full seeded balance"
        );
    }
}
