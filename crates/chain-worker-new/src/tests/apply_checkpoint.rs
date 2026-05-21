//! Cross-mode consistency tests for checkpoint-sync epoch reconstruction.
//!
//! Each test runs one epoch two ways — full sync (block-by-block STF) and
//! checkpoint sync ([`apply_checkpoint_epoch`]) — and asserts the generated
//! values match. The comparison has three tiers:
//!
//! - Tier 1, byte-identical (consensus): toplevel state, state root, summary.
//! - Tier 2, equal modulo a documented mode difference: index records carry block attribution under
//!   full sync, `None` under checkpoint sync; snark update records collapse to one per account
//!   under checkpoint sync.
//! - Tier 3, not compared: per-block write batches (checkpoint sync has none).

use std::collections::HashMap;

use strata_asm_common::AsmManifest;
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_checkpoint_types::EpochSummary;
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

    fn fetch_epoch_summaries(&self, epoch: Epoch) -> WorkerResult<Vec<EpochSummary>> {
        Ok(self
            .epoch_summaries
            .get(&epoch)
            .cloned()
            .unwrap_or_default())
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
fn test_apply_checkpoint_snark_update() {
    let built = build_epoch(EpochShape::SnarkUpdate);
    let (ctx, epoch) = mock_for(&built);

    let artifacts = apply_checkpoint_epoch(&ctx, epoch).expect("apply_checkpoint_epoch");
    assert_consistent(&built, &artifacts);
}

#[test]
fn test_apply_checkpoint_snark_update_and_deposit() {
    let built = build_epoch(EpochShape::SnarkUpdateAndDeposit);
    let (ctx, epoch) = mock_for(&built);

    let artifacts = apply_checkpoint_epoch(&ctx, epoch).expect("apply_checkpoint_epoch");
    assert_consistent(&built, &artifacts);
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

/// Asserts the checkpoint-reconstructed artifacts match the full-sync run.
fn assert_consistent(built: &BuiltEpoch, artifacts: &AppliedEpochArtifacts) {
    // Tier 1 — byte-identical: state root, toplevel state, summary.
    assert_eq!(
        artifacts.output.computed_state_root(),
        &built.full_sync_state_root,
        "reconstructed state root must equal full-sync root"
    );
    assert_eq!(
        MemoryStateBaseLayer::new(artifacts.new_state.clone())
            .compute_state_root()
            .expect("reconstructed root"),
        built.full_sync_state_root,
        "reconstructed state must hash to the full-sync root"
    );
    assert_eq!(
        &artifacts.summary, &built.full_sync_summary,
        "reconstructed epoch summary must equal the full-sync summary"
    );

    // Tier 2 — equal modulo documented mode differences.
    assert_indexer_writes_consistent(
        built.full_sync_indexer_writes(),
        artifacts.output.indexer_writes(),
    );
}

/// Compares full-sync vs checkpoint-sync [`IndexerWrites`] under the tier-2
/// rules: same created-account set, same inbox messages, and snark updates
/// matching on final `seq_no` / `next_inbox_idx` per account.
fn assert_indexer_writes_consistent(full_sync: &IndexerWrites, checkpoint: &IndexerWrites) {
    // Created accounts: same set.
    let mut fs_created: Vec<_> = full_sync
        .created_accounts()
        .iter()
        .map(|c| c.account_id())
        .collect();
    let mut cp_created: Vec<_> = checkpoint
        .created_accounts()
        .iter()
        .map(|c| c.account_id())
        .collect();
    fs_created.sort();
    cp_created.sort();
    assert_eq!(
        fs_created, cp_created,
        "created accounts must match across sync modes"
    );

    // Inbox messages: same set of (account, entry) pairs.
    let mut fs_inbox: Vec<_> = full_sync
        .inbox_messages()
        .iter()
        .map(|w| (w.account_id(), w.entry().clone()))
        .collect();
    let mut cp_inbox: Vec<_> = checkpoint
        .inbox_messages()
        .iter()
        .map(|w| (w.account_id(), w.entry().clone()))
        .collect();
    fs_inbox.sort_by_key(|(id, _)| *id);
    cp_inbox.sort_by_key(|(id, _)| *id);
    assert_eq!(
        fs_inbox, cp_inbox,
        "inbox messages must match across sync modes"
    );

    // Snark updates: final-state equivalence per account. Full sync emits one
    // update per tx; checkpoint sync collapses to one per account. So the
    // checkpoint record for an account must match the *last* full-sync record
    // for that account on seq_no and next_inbox_idx.
    let mut fs_final: HashMap<_, (u64, u64)> = HashMap::new();
    for upd in full_sync.snark_state_updates() {
        fs_final.insert(
            upd.account_id(),
            (*upd.seqno().inner(), upd.next_read_idx()),
        );
    }
    let mut cp_final: HashMap<_, (u64, u64)> = HashMap::new();
    for upd in checkpoint.snark_state_updates() {
        cp_final.insert(
            upd.account_id(),
            (*upd.seqno().inner(), upd.next_read_idx()),
        );
    }
    assert_eq!(
        fs_final, cp_final,
        "final snark seq_no/next_inbox_idx must match across sync modes"
    );
}
