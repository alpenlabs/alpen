//! Cross-mode consistency tests for checkpoint-sync epoch reconstruction.
//!
//! Each test runs one epoch two ways — block sync (block-by-block STF) and
//! checkpoint sync ([`apply_checkpoint_epoch`]) — and asserts the generated
//! values match. The comparison has three tiers:
//!
//! - Tier 1, byte-identical (consensus): toplevel state, state root, summary.
//! - Tier 2, equal modulo a documented mode difference at the [`IndexerWrites`] layer: snark update
//!   records are rebuilt from the checkpoint sidecar's `ol_logs` so the per-account sequence
//!   matches block sync.
//! - Tier 3, not compared: per-block write batches (checkpoint sync has none).
//!
//! Note: the `Some` vs `None` block-attribution difference between sync modes
//! lives one layer down, when [`IndexerWrites`] is converted into the
//! attributed `IndexingWrites` written to the indexing store. That conversion
//! is not covered here.

use std::collections::HashMap;

use strata_acct_types::AccountId;
use strata_asm_common::AsmManifest;
use strata_asm_proto_checkpoint_types::{
    CheckpointPayload, CheckpointSidecar, OLLog, SimpleWithdrawalIntentLogData,
    TerminalHeaderComplement,
};
use strata_checkpoint_types::EpochSummary;
use strata_codec::encode_to_vec;
use strata_identifiers::{Buf32, Epoch, EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_support_types::{IndexerWrites, MemoryStateBaseLayer};
use strata_ol_state_types::OLState;

use super::fixture::{BuiltEpoch, EpochShape, build_epoch};
use crate::{
    WorkerError, WorkerResult,
    output::OLBlockExecutionOutput,
    state::{AppliedEpochArtifacts, apply_checkpoint_epoch},
    traits::ChainWorkerContext,
};

/// A [`ChainWorkerContext`] backed by in-memory maps for the four reads
/// [`apply_checkpoint_epoch`] performs. All other methods are unreachable.
struct MockChainWorkerContext {
    /// Checkpoint payloads keyed by epoch commitment.
    checkpoint_payloads: HashMap<EpochCommitment, CheckpointPayload>,
    /// Epoch summaries keyed by epoch index.
    epoch_summaries: HashMap<Epoch, Vec<EpochSummary>>,
    /// OL states keyed by block commitment.
    ol_states: HashMap<OLBlockCommitment, OLState>,
    /// ASM manifests keyed by L1 height.
    manifests: HashMap<u32, AsmManifest>,
}

impl MockChainWorkerContext {
    fn new() -> Self {
        Self {
            checkpoint_payloads: HashMap::new(),
            epoch_summaries: HashMap::new(),
            ol_states: HashMap::new(),
            manifests: HashMap::new(),
        }
    }
}

impl ChainWorkerContext for MockChainWorkerContext {
    fn fetch_checkpoint_payload(
        &self,
        epoch: &EpochCommitment,
    ) -> WorkerResult<Option<CheckpointPayload>> {
        Ok(self.checkpoint_payloads.get(epoch).cloned())
    }

    fn fetch_canonical_epoch_summary_at(&self, epoch: Epoch) -> WorkerResult<Option<EpochSummary>> {
        Ok(self
            .epoch_summaries
            .get(&epoch)
            .and_then(|x| x.first())
            .cloned())
    }

    fn fetch_ol_state(&self, commitment: OLBlockCommitment) -> WorkerResult<Option<OLState>> {
        Ok(self.ol_states.get(&commitment).cloned())
    }

    fn fetch_l1_manifests(&self, from: u32, to: u32) -> WorkerResult<Vec<AsmManifest>> {
        let mut out = Vec::new();
        for height in from..=to {
            let manifest = self
                .manifests
                .get(&height)
                .cloned()
                .ok_or(WorkerError::MissingDependency("mock l1 manifest"))?;
            out.push(manifest);
        }
        Ok(out)
    }

    // Methods below are not exercised by checkpoint-sync reconstruction.

    fn fetch_block(&self, _blkid: &OLBlockId) -> WorkerResult<Option<OLBlock>> {
        unimplemented!("not used by apply_checkpoint_epoch")
    }

    fn fetch_blocks_at_slot(&self, _slot: u64) -> WorkerResult<Vec<OLBlockId>> {
        unimplemented!("not used by apply_checkpoint_epoch")
    }

    fn fetch_header(&self, _blkid: &OLBlockId) -> WorkerResult<Option<OLBlockHeader>> {
        unimplemented!("not used by apply_checkpoint_epoch")
    }

    fn fetch_chain_tip(&self) -> WorkerResult<Option<OLBlockCommitment>> {
        unimplemented!("not used by apply_checkpoint_epoch")
    }

    fn fetch_write_batch(
        &self,
        _commitment: OLBlockCommitment,
    ) -> WorkerResult<
        Option<strata_ol_state_types::WriteBatch<strata_ol_state_types::OLAccountState>>,
    > {
        unimplemented!("not used by apply_checkpoint_epoch")
    }

    fn store_block_output(
        &self,
        _block: &OLBlock,
        _commitment: OLBlockCommitment,
        _output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        unimplemented!("not used by apply_checkpoint_epoch")
    }

    fn store_toplevel_state(
        &self,
        _commitment: OLBlockCommitment,
        _state: OLState,
    ) -> WorkerResult<()> {
        unimplemented!("not used by apply_checkpoint_epoch")
    }

    fn store_summary(&self, _summary: EpochSummary) -> WorkerResult<()> {
        unimplemented!("not used by apply_checkpoint_epoch")
    }

    fn fetch_summary(&self, _epoch: &EpochCommitment) -> WorkerResult<EpochSummary> {
        unimplemented!("not used by apply_checkpoint_epoch")
    }

    fn merge_epoch_data(&self, _epoch: &EpochCommitment) -> WorkerResult<()> {
        unimplemented!("not used by apply_checkpoint_epoch")
    }

    fn prefill_l1_block_refs_mmr(&self) -> WorkerResult<()> {
        unimplemented!("not used by apply_checkpoint_epoch")
    }

    fn apply_epoch_indexing(
        &self,
        _epoch: &EpochCommitment,
        _output: &OLBlockExecutionOutput,
    ) -> WorkerResult<()> {
        unimplemented!("not used by apply_checkpoint_epoch")
    }
}

/// Populates a mock context with everything `apply_checkpoint_epoch` needs to
/// reconstruct `built`'s epoch.
fn mock_for(built: &BuiltEpoch) -> (MockChainWorkerContext, EpochCommitment) {
    let mut ctx = MockChainWorkerContext::new();

    // Prev-epoch summary so get_prev_terminal resolves the base state.
    ctx.epoch_summaries
        .insert(built.prev_epoch_idx, vec![built.prev_summary]);
    ctx.ol_states
        .insert(built.prev_terminal, built.pre_epoch_state.clone());

    // Manifests over the epoch's L1 height range.
    for (height, manifest) in &built.manifests_by_height {
        ctx.manifests.insert(*height, manifest.clone());
    }

    let epoch = built.epoch_commitment;
    ctx.checkpoint_payloads
        .insert(epoch, built.checkpoint_payload.clone());

    (ctx, epoch)
}

#[test]
fn test_apply_checkpoint_deposit_manifest_only() {
    let built = build_epoch(EpochShape::DepositManifestOnly);
    let (ctx, epoch) = mock_for(&built);

    let artifacts = apply_checkpoint_epoch(&ctx, epoch).expect("apply_checkpoint_epoch");
    assert_consistent(&built, &artifacts);
}

#[test]
fn test_apply_checkpoint_snark_multi_update_and_deposit() {
    let built = build_epoch(EpochShape::SnarkMultiUpdateAndDeposit);
    let (ctx, epoch) = mock_for(&built);

    let artifacts = apply_checkpoint_epoch(&ctx, epoch).expect("apply_checkpoint_epoch");
    assert_consistent(&built, &artifacts);

    // Sanity: both modes genuinely produced 2 snark records.
    assert_eq!(
        artifacts
            .output
            .indexer_writes()
            .snark_state_updates()
            .len(),
        2,
        "checkpoint sync should produce one snark record per ol_logs entry"
    );
    assert_eq!(
        built
            .block_sync_indexer_writes()
            .snark_state_updates()
            .len(),
        2,
        "block sync should produce one snark record per update tx"
    );
}

#[test]
fn test_apply_checkpoint_skips_non_snark_log_in_sidecar() {
    let built = build_epoch(EpochShape::SnarkMultiUpdateAndDeposit);

    let payload = built.checkpoint_payload.clone();
    let sidecar = payload.sidecar();
    let snark_serial = sidecar
        .ol_logs()
        .first()
        .expect("snark-multi-update has at least one log")
        .account_serial();
    let withdrawal_log = OLLog::new(
        snark_serial,
        encode_to_vec(
            &SimpleWithdrawalIntentLogData::new(100_000_000, b"bc1qbogusdest".to_vec(), 0)
                .expect("withdrawal log data"),
        )
        .expect("encode withdrawal log"),
    );

    let mut spliced_logs: Vec<OLLog> = sidecar.ol_logs().to_vec();
    spliced_logs.push(withdrawal_log);

    let complement = sidecar.terminal_header_complement();
    let spliced_sidecar = CheckpointSidecar::new(
        sidecar.ol_state_diff().to_vec(),
        spliced_logs,
        TerminalHeaderComplement::new(
            complement.timestamp(),
            *complement.parent_blkid(),
            *complement.body_root(),
            *complement.logs_root(),
        ),
    )
    .expect("rebuild sidecar with spliced log");
    let spliced_payload = CheckpointPayload::new(
        *payload.new_tip(),
        spliced_sidecar,
        payload.proof().to_vec(),
    )
    .expect("rebuild payload");

    let mut built_with_extra = built;
    built_with_extra.checkpoint_payload = spliced_payload;
    let (ctx, epoch) = mock_for(&built_with_extra);

    let artifacts = apply_checkpoint_epoch(&ctx, epoch).expect("apply_checkpoint_epoch");
    assert_consistent(&built_with_extra, &artifacts);

    // The extra non-snark log must not produce a snark record.
    assert_eq!(
        artifacts
            .output
            .indexer_writes()
            .snark_state_updates()
            .len(),
        built_with_extra
            .block_sync_indexer_writes()
            .snark_state_updates()
            .len(),
        "non-snark log must be skipped during snark-record reconstruction"
    );
}

#[test]
fn test_apply_checkpoint_epoch_is_deterministic() {
    // MMR retry semantics rely on artifact-level determinism (stable index
    // values across two reconstructions of the same payload). Lock that down.
    let built = build_epoch(EpochShape::SnarkMultiUpdateAndDeposit);
    let (ctx, epoch) = mock_for(&built);

    let a = apply_checkpoint_epoch(&ctx, epoch).expect("first apply");
    let b = apply_checkpoint_epoch(&ctx, epoch).expect("second apply");

    assert_eq!(a.terminal, b.terminal, "terminal commitment must be stable");
    assert_eq!(a.summary, b.summary, "epoch summary must be stable");
    assert_eq!(
        a.output.computed_state_root(),
        b.output.computed_state_root(),
        "state root must be stable"
    );
    assert_indexer_writes_consistent(a.output.indexer_writes(), b.output.indexer_writes());
}

#[test]
fn test_apply_checkpoint_rejects_genesis_epoch() {
    let ctx = MockChainWorkerContext::new();
    let epoch0 = EpochCommitment::new(0, 0, OLBlockId::from(Buf32::zero()));
    let err = apply_checkpoint_epoch(&ctx, epoch0).unwrap_err();
    assert!(matches!(err, WorkerError::CannotApplyGenesisEpoch));
}

#[test]
fn test_apply_checkpoint_missing_payload() {
    let ctx = MockChainWorkerContext::new();
    let epoch = EpochCommitment::new(1, 5, OLBlockId::from(Buf32::from([1u8; 32])));
    let err = apply_checkpoint_epoch(&ctx, epoch).unwrap_err();
    assert!(matches!(err, WorkerError::MissingCheckpointPayload(_)));
}

/// Asserts the checkpoint-reconstructed artifacts match the block-sync run.
fn assert_consistent(built: &BuiltEpoch, artifacts: &AppliedEpochArtifacts) {
    // Tier 1 — byte-identical: state root, toplevel state, summary.
    assert_eq!(
        artifacts.output.computed_state_root(),
        &built.block_sync_state_root,
        "reconstructed state root must equal block-sync root"
    );
    assert_eq!(
        MemoryStateBaseLayer::new(artifacts.new_state.clone())
            .compute_state_root()
            .expect("reconstructed root"),
        built.block_sync_state_root,
        "reconstructed state must hash to the block-sync root"
    );
    assert_eq!(
        &artifacts.summary, &built.block_sync_summary,
        "reconstructed epoch summary must equal the block-sync summary"
    );

    // Tier 2 — equal modulo documented mode differences.
    assert_indexer_writes_consistent(
        built.block_sync_indexer_writes(),
        artifacts.output.indexer_writes(),
    );
}

/// Compares block-sync vs checkpoint-sync [`IndexerWrites`] under the tier-2
/// rules.
fn assert_indexer_writes_consistent(block_sync: &IndexerWrites, checkpoint: &IndexerWrites) {
    // Created accounts: same set.
    let mut bs_created: Vec<_> = block_sync
        .created_accounts()
        .iter()
        .map(|c| c.account_id())
        .collect();
    let mut cp_created: Vec<_> = checkpoint
        .created_accounts()
        .iter()
        .map(|c| c.account_id())
        .collect();
    bs_created.sort();
    cp_created.sort();
    assert_eq!(
        bs_created, cp_created,
        "created accounts must match across sync modes"
    );

    // Inbox messages: same set of (account, MMR index, entry). MMR index is
    // included because DA reconstruction must preserve message positions.
    let mut fs_inbox: Vec<_> = block_sync
        .inbox_messages()
        .iter()
        .map(|w| (w.account_id(), w.index(), w.entry().clone()))
        .collect();
    let mut cp_inbox: Vec<_> = checkpoint
        .inbox_messages()
        .iter()
        .map(|w| (w.account_id(), w.index(), w.entry().clone()))
        .collect();
    fs_inbox.sort_by_key(|(id, idx, _)| (*id, *idx));
    cp_inbox.sort_by_key(|(id, idx, _)| (*id, *idx));
    assert_eq!(
        fs_inbox, cp_inbox,
        "inbox messages (incl. MMR index) must match across sync modes"
    );

    // L1 block records: same set of (height, record). Both paths emit writes
    // for the same L1 heights; this catches a checkpoint-sync bug that drops
    // or reorders them.
    let mut fs_l1: Vec<_> = block_sync
        .l1_block_records()
        .iter()
        .map(|m| (m.height, m.record.clone()))
        .collect();
    let mut cp_l1: Vec<_> = checkpoint
        .l1_block_records()
        .iter()
        .map(|m| (m.height, m.record.clone()))
        .collect();
    fs_l1.sort_by_key(|(h, _)| *h);
    cp_l1.sort_by_key(|(h, _)| *h);
    assert_eq!(
        fs_l1, cp_l1,
        "L1 block record writes must match across sync modes"
    );

    // Snark updates: per-account ordered sequence equality. Checkpoint sync now
    // rebuilds per-update records from the sidecar's `ol_logs`, so the record
    // count and order must match block sync.
    //
    // `state` (inner state root) is deliberately not compared: checkpoint sync
    // has no per-update intermediate roots on chain, so the records carry a
    // sentinel `Hash::default()`.
    let fs_snark = group_snark_by_account(block_sync);
    let cp_snark = group_snark_by_account(checkpoint);
    assert_eq!(
        fs_snark, cp_snark,
        "per-account snark update sequence (seqno, next_inbox_idx, extra_data) must match"
    );

    // Predicate-key updates: same sequence. No current fixture shape exercises
    // a non-empty case (a future `EePredicateUpdate` shape will); this guards
    // against a regression that spuriously emits one under either mode.
    let fs_pred: Vec<_> = block_sync
        .predicate_key_updates()
        .iter()
        .map(|u| (u.account_id(), u.new_vk().clone()))
        .collect();
    let cp_pred: Vec<_> = checkpoint
        .predicate_key_updates()
        .iter()
        .map(|u| (u.account_id(), u.new_vk().clone()))
        .collect();
    assert_eq!(
        fs_pred, cp_pred,
        "predicate-key updates must match across sync modes"
    );
}

/// `(seqno, next_inbox_idx, extra_data)` — the comparable snark-update fields.
type SnarkUpdateProj = (u64, u64, Option<Vec<u8>>);

/// Groups snark updates by account, preserving emission order within each
/// account. Returns the comparable projection per record — see the call site
/// for why `state` is excluded.
fn group_snark_by_account(writes: &IndexerWrites) -> HashMap<AccountId, Vec<SnarkUpdateProj>> {
    let mut out: HashMap<AccountId, Vec<SnarkUpdateProj>> = HashMap::new();
    for upd in writes.snark_state_updates() {
        out.entry(upd.account_id()).or_default().push((
            *upd.seqno().inner(),
            upd.next_read_idx(),
            upd.extra_data().map(<[u8]>::to_vec),
        ));
    }
    out
}

// =============================================================================
// Three-write composite idempotency test
// =============================================================================
//
// `ChainWorkerServiceState::apply_checkpoint` performs three writes in sequence
// (store_toplevel_state, apply_epoch_indexing, store_summary) which need to be idempotent because
// right now we don't have atomic writes throughout multiple db managers.

mod db_idempotency {
    use std::{collections::BTreeSet, sync::Arc};

    use strata_db_store_sled::{
        MmrIndexDb, SledDbConfig, ol_checkpoint::db::OLCheckpointDBSled,
        ol_state::db::OLStateDBSled, ol_state_index::db::OLStateIndexingDBSled,
    };
    use strata_db_types::{
        errors::DbError,
        ol_state_index::{AccountUpdateRecord, InboxMessageRecord},
    };
    use strata_identifiers::AccountId;
    use strata_ledger_types::IStateAccessor;
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_storage::{
        MmrId, MmrIndexManager, OLCheckpointManager, OLStateIndexingManager, OLStateManager,
    };
    use threadpool::ThreadPool;
    use typed_sled::SledDb;

    use super::{EpochCommitment, build_epoch, mock_for};
    use crate::{
        context::{build_checkpoint_indexing_writes, index_inbox_mmr_writes},
        state::{AppliedEpochArtifacts, apply_checkpoint_epoch},
        tests::fixture::EpochShape,
    };

    /// Sled-backed wiring for the four managers `apply_checkpoint`'s writes
    /// touch. Each is opened on its own temporary sled, matching how
    /// production constructs them per storage tree.
    struct WriteHarness {
        ol_state: Arc<OLStateManager>,
        ol_indexing: Arc<OLStateIndexingManager>,
        ol_checkpoint: Arc<OLCheckpointManager>,
        mmr_index: Arc<MmrIndexManager>,
    }

    impl WriteHarness {
        fn new() -> Self {
            let pool = ThreadPool::new(1);
            Self {
                ol_state: Arc::new(OLStateManager::new(pool.clone(), make_ol_state_db())),
                ol_indexing: Arc::new(OLStateIndexingManager::new(
                    pool.clone(),
                    make_ol_state_indexing_db(),
                )),
                ol_checkpoint: Arc::new(OLCheckpointManager::new(
                    pool.clone(),
                    make_ol_checkpoint_db(),
                )),
                mmr_index: Arc::new(MmrIndexManager::new(pool, make_mmr_index_db())),
            }
        }

        /// Runs the same three writes `ChainWorkerServiceState::apply_checkpoint`
        /// runs, in the same order.
        fn run_three_writes(
            &self,
            epoch: EpochCommitment,
            artifacts: &AppliedEpochArtifacts,
        ) -> anyhow::Result<()> {
            self.ol_state
                .put_toplevel_ol_state_blocking(artifacts.terminal, artifacts.new_state.clone())?;
            let indexing_writes = build_checkpoint_indexing_writes(&artifacts.output);
            self.ol_indexing
                .apply_epoch_indexing_blocking(epoch, indexing_writes)?;
            index_inbox_mmr_writes(&self.mmr_index, &artifacts.output)?;

            let commitment = artifacts.summary.get_epoch_commitment();
            self.ol_indexing
                .set_epoch_commitment_blocking(commitment.epoch(), commitment)?;
            match self
                .ol_checkpoint
                .insert_epoch_summary_blocking(artifacts.summary)
            {
                Ok(()) => {}
                Err(DbError::OverwriteEpoch(c)) if c == commitment => {
                    let existing = self
                        .ol_checkpoint
                        .get_epoch_summary_blocking(commitment)?
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "OverwriteEpoch reported but get_epoch_summary returned None for {commitment}"
                            )
                        })?;
                    anyhow::ensure!(
                        existing == artifacts.summary,
                        "epoch summary content mismatch on retry"
                    );
                }
                Err(e) => return Err(e.into()),
            }
            Ok(())
        }

        /// Reads back everything the three writes touched, projected to a
        /// shape we can compare with `assert_eq!`.
        fn snapshot(
            &self,
            epoch: EpochCommitment,
            terminal: super::OLBlockCommitment,
            accounts: &[AccountId],
        ) -> DbSnapshot {
            let toplevel_state_root = self
                .ol_state
                .get_toplevel_ol_state_blocking(terminal)
                .expect("get_toplevel_ol_state")
                .map(|arc| {
                    MemoryStateBaseLayer::new((*arc).clone())
                        .compute_state_root()
                        .expect("compute_state_root")
                });
            let summary = self
                .ol_checkpoint
                .get_epoch_summary_blocking(epoch)
                .expect("get_epoch_summary");
            let canonical = self
                .ol_checkpoint
                .get_canonical_epoch_commitment_at_blocking(epoch.epoch())
                .expect("get_canonical_epoch_commitment");
            let mut per_account = Vec::new();
            for &acct in accounts {
                let updates = self
                    .ol_indexing
                    .get_account_update_records_blocking(epoch.epoch(), acct)
                    .expect("get_account_update_records")
                    .unwrap_or_default();
                let inbox = self
                    .ol_indexing
                    .get_account_inbox_records_blocking(epoch.epoch(), acct)
                    .expect("get_account_inbox_records")
                    .unwrap_or_default();
                let mmr_handle = self.mmr_index.get_handle(MmrId::SnarkMsgInbox(acct));
                let mmr_leaves = mmr_handle
                    .get_num_leaves_blocking()
                    .expect("get_num_leaves");
                per_account.push((acct, updates, inbox, mmr_leaves));
            }
            DbSnapshot {
                toplevel_state_root,
                summary,
                canonical_epoch_commitment: canonical,
                per_account,
            }
        }
    }

    #[derive(Debug, PartialEq)]
    struct DbSnapshot {
        toplevel_state_root: Option<strata_identifiers::Buf32>,
        summary: Option<strata_checkpoint_types::EpochSummary>,
        canonical_epoch_commitment: Option<EpochCommitment>,
        per_account: Vec<(
            AccountId,
            Vec<AccountUpdateRecord>,
            Vec<InboxMessageRecord>,
            u64,
        )>,
    }

    fn make_temp_sled() -> Arc<SledDb> {
        let db = sled::Config::new()
            .temporary(true)
            .open()
            .expect("temporary sled");
        Arc::new(SledDb::new(db).expect("typed_sled::SledDb"))
    }

    fn make_ol_state_db() -> Arc<OLStateDBSled> {
        Arc::new(OLStateDBSled::new(make_temp_sled(), SledDbConfig::test()).expect("OLStateDBSled"))
    }

    fn make_ol_state_indexing_db() -> Arc<OLStateIndexingDBSled> {
        Arc::new(
            OLStateIndexingDBSled::new(make_temp_sled(), SledDbConfig::test())
                .expect("OLStateIndexingDBSled"),
        )
    }

    fn make_ol_checkpoint_db() -> Arc<OLCheckpointDBSled> {
        Arc::new(
            OLCheckpointDBSled::new(make_temp_sled(), SledDbConfig::test())
                .expect("OLCheckpointDBSled"),
        )
    }

    fn make_mmr_index_db() -> Arc<MmrIndexDb> {
        Arc::new(MmrIndexDb::new(make_temp_sled(), SledDbConfig::test()).expect("MmrIndexDb"))
    }

    /// Running the three writes twice must yield identical db state.
    #[test]
    fn test_apply_checkpoint_writes_are_idempotent() {
        let built = build_epoch(EpochShape::SnarkMultiUpdateAndDeposit);
        let (mock_ctx, epoch) = mock_for(&built);
        let artifacts = apply_checkpoint_epoch(&mock_ctx, epoch).expect("apply_checkpoint_epoch");

        // Accounts touched by the fixture's writes — used to scope snapshot reads.
        let touched_accounts: Vec<AccountId> = artifacts
            .output
            .indexer_writes()
            .snark_state_updates()
            .iter()
            .map(|u| u.account_id())
            .chain(
                artifacts
                    .output
                    .indexer_writes()
                    .inbox_messages()
                    .iter()
                    .map(|w| w.account_id()),
            )
            .chain(
                artifacts
                    .output
                    .indexer_writes()
                    .created_accounts()
                    .iter()
                    .map(|c| c.account_id()),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        assert!(
            !touched_accounts.is_empty(),
            "fixture must touch at least one account for the snapshot to be meaningful"
        );

        let harness = WriteHarness::new();
        harness
            .run_three_writes(epoch, &artifacts)
            .expect("first apply");
        let first = harness.snapshot(epoch, artifacts.terminal, &touched_accounts);

        harness
            .run_three_writes(epoch, &artifacts)
            .expect("second apply (idempotency check)");
        let second = harness.snapshot(epoch, artifacts.terminal, &touched_accounts);

        assert_eq!(
            first, second,
            "second apply must leave db state byte-identical to first"
        );
    }
}
