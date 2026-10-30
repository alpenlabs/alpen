//! Context trait for checkpoint worker dependencies.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use strata_asm_checkpoint_types::CheckpointPayload;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Epoch, EpochCommitment, L1BlockCommitment, OLBlockCommitment, Slot};
use strata_ol_block_ancestry_provider::{BlockAncestryProvider, EpochBlockAncestry};
use strata_ol_canonical_chain_provider::{OLCanonicalChainProvider, OLCanonicalChainProviderImpl};
use strata_ol_chain_types_v1::{AsmManifest, OLBlockHeaderV1, OLBlockId, OLBlockV1, OLLog};
use strata_ol_params::OLRuntimeParams;
use strata_ol_spec_selection::select_spec_after_epoch;
use strata_ol_state_container::OLStateContainer;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_stf::OLSpecId;
use strata_primitives::nonempty_vec::NonEmptyVec;
use strata_storage::NodeStorage;
use tracing::{debug, warn};

use crate::{CanonicalEpochReconciler, compute_epoch_da};

pub(crate) type StateDiffRaw = Vec<u8>;

/// Result of waiting for a checkpoint proof.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProofWaitOutcome {
    /// The proof bytes are ready; empty when no prover is configured.
    Ready(Vec<u8>),

    /// The epoch commitment is no longer canonical.
    Superseded,
}

/// Context providing dependencies for the checkpoint worker.
///
/// This trait abstracts storage and data providers, enabling testing
/// with mock implementations and future production providers.
pub(crate) trait CheckpointWorkerContext: Send + Sync + 'static {
    /// Finds the canonical point after which checkpoint processing can resume.
    fn find_canonical_resume_point(
        &self,
        last_processed: EpochCommitment,
        history_base: Option<EpochCommitment>,
    ) -> anyhow::Result<Option<EpochCommitment>>;

    /// Gets the canonical tip, or `None` for an empty index.
    fn get_canonical_tip(&self) -> anyhow::Result<Option<OLBlockCommitment>>;

    /// Gets the canonical block at `slot`, or `None` when there is no canonical block at that slot.
    fn get_canonical_block_at(&self, slot: Slot) -> anyhow::Result<Option<OLBlockCommitment>>;

    /// Gets the canonical commitment for `epoch`.
    fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> anyhow::Result<Option<EpochCommitment>>;

    /// Get the epoch summary for a commitment.
    fn get_epoch_summary(
        &self,
        commitment: EpochCommitment,
    ) -> anyhow::Result<Option<EpochSummary>>;

    /// Get a checkpoint payload entry for the given epoch commitment.
    fn get_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
    ) -> anyhow::Result<Option<CheckpointPayload>>;

    /// Get the last checkpointed payload commitment, if any.
    fn get_last_checkpoint_payload_epoch(&self) -> anyhow::Result<Option<EpochCommitment>>;

    /// Gets the base of retained OL history.
    fn get_history_base(&self) -> anyhow::Result<Option<EpochCommitment>>;

    /// Store a checkpoint payload entry for the given epoch commitment.
    fn put_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
    ) -> anyhow::Result<()>;

    /// Waits for proof bytes while the selected commitment remains canonical.
    fn get_proof(&self, epoch: &EpochCommitment) -> anyhow::Result<ProofWaitOutcome>;

    /// Gets the OL block header for the given block id.
    fn get_block_header(
        &self,
        blkid: &OLBlockCommitment,
    ) -> anyhow::Result<Option<OLBlockHeaderV1>>;

    /// Collects the starting block's epoch in execution order and its previous terminal.
    fn collect_epoch_block_commitments(
        &self,
        start: &OLBlockHeaderV1,
    ) -> anyhow::Result<EpochBlockAncestry>;

    /// Gets an OL block by its block ID.
    fn get_block(&self, id: &OLBlockId) -> anyhow::Result<Option<OLBlockV1>>;

    /// Gets the OL state snapshot at a given block commitment.
    fn get_ol_state(
        &self,
        commitment: &OLBlockCommitment,
    ) -> anyhow::Result<Option<OLStateContainer>>;

    /// Returns the L1 block OL genesis anchors to, where the genesis epoch's
    /// L1 range starts.
    fn genesis_l1_block(&self) -> L1BlockCommitment;

    /// Gets the stored ASM manifest of the L1 block `block`, if present.
    fn get_l1_manifest(&self, block: &L1BlockCommitment) -> anyhow::Result<Option<AsmManifest>>;

    /// Fetches da data for epoch. Returns state diff and OL logs.
    fn fetch_da_for_epoch(
        &self,
        summary: &EpochSummary,
    ) -> anyhow::Result<(StateDiffRaw, Vec<OLLog>)>;
}

/// Safety-net interval for the proof wait loop.
///
/// The checkpoint worker sleeps on a [`Condvar`] that the proof storer
/// signals after writing a proof. The timeout also lets the worker detect
/// reorgs when no proof notification arrives.
const PROOF_WAIT_INTERVAL: Duration = Duration::from_secs(2);

/// Shared signal between the proof storer and the checkpoint worker.
///
/// The proof storer calls `notify` after persisting a proof to the DB.
/// The checkpoint worker blocks on `wait`, which returns as soon as
/// the signal arrives (or after a safety-net timeout as a fallback).
#[derive(Debug)]
pub struct ProofNotify {
    mu: Mutex<()>,
    cv: Condvar,
}

impl ProofNotify {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self {
            mu: Mutex::new(()),
            cv: Condvar::new(),
        }
    }

    /// Signals that a proof has been stored. Wakes the checkpoint worker.
    pub fn notify(&self) {
        let _guard = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        self.cv.notify_all();
    }

    /// Blocks until signaled or `PROOF_WAIT_INTERVAL` elapses.
    fn wait(&self) {
        let guard = self.mu.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self
            .cv
            .wait_timeout(guard, PROOF_WAIT_INTERVAL)
            .unwrap_or_else(|e| e.into_inner());
    }
}

impl Default for ProofNotify {
    fn default() -> Self {
        Self::new()
    }
}

/// Prover configuration for the checkpoint worker.
///
/// When provided, the worker reads proofs from the proof DB and waits
/// while the selected commitment remains canonical. The checkpoint predicate
/// in `AsmParams` determines whether proofs are required; if the
/// predicate rejects empty proofs (e.g. `Sp1Groth16`), there is no
/// safe fallback, so the worker must wait for a real proof.
#[derive(Debug)]
pub struct ProverConfig {
    /// Notifier shared with the proof storer wakes the worker on new proofs.
    pub notify: Arc<ProofNotify>,
}

/// Production context implementation.
///
/// When a [`ProverConfig`] is set, reads proofs from the proof DB and waits
/// for the prover to deliver them. Without it, returns empty proofs.
pub(crate) struct CheckpointWorkerContextImpl {
    canonical_chain_provider: OLCanonicalChainProviderImpl,
    epoch_reconciler: CanonicalEpochReconciler,
    block_ancestry_provider: BlockAncestryProvider,
    storage: Arc<NodeStorage>,
    runtime_params: OLRuntimeParams,
    genesis_l1_block: L1BlockCommitment,
    /// When present, a prover is running and `get_proof` waits for proofs.
    /// When absent, canonical commitments use empty proofs.
    prover: Option<ProverConfig>,
}

impl CheckpointWorkerContextImpl {
    /// Creates a new context without a prover.
    ///
    /// Returns empty proof bytes for canonical commitments.
    pub(crate) fn new(
        storage: Arc<NodeStorage>,
        runtime_params: OLRuntimeParams,
        genesis_l1_block: L1BlockCommitment,
    ) -> Self {
        Self {
            canonical_chain_provider: OLCanonicalChainProviderImpl::new(&storage),
            epoch_reconciler: CanonicalEpochReconciler::new(&storage),
            block_ancestry_provider: BlockAncestryProvider::new(storage.ol_block().clone()),
            storage,
            runtime_params,
            genesis_l1_block,
            prover: None,
        }
    }

    /// Creates a new context with an integrated prover.
    pub(crate) fn with_prover(
        storage: Arc<NodeStorage>,
        runtime_params: OLRuntimeParams,
        genesis_l1_block: L1BlockCommitment,
        prover: ProverConfig,
    ) -> Self {
        Self {
            canonical_chain_provider: OLCanonicalChainProviderImpl::new(&storage),
            epoch_reconciler: CanonicalEpochReconciler::new(&storage),
            block_ancestry_provider: BlockAncestryProvider::new(storage.ol_block().clone()),
            storage,
            runtime_params,
            genesis_l1_block,
            prover: Some(prover),
        }
    }

    /// Attempts to read a non-empty proof from the proof DB for the given epoch commitment.
    ///
    /// Returns `Ok(Some(bytes))` if a valid proof is found, `Ok(None)` if no
    /// proof is available yet.
    fn try_read_proof(&self, commitment: EpochCommitment) -> anyhow::Result<Option<Vec<u8>>> {
        if let Some(receipt) = self.storage.checkpoint_proof().get_proof(&commitment)? {
            let proof_bytes = receipt.receipt().proof().as_bytes().to_vec();
            if proof_bytes.is_empty() {
                warn!(%commitment, "empty proof receipt found");
                return Ok(None);
            }
            debug!(%commitment, "proof found for checkpoint");
            return Ok(Some(proof_bytes));
        }
        Ok(None)
    }
}

impl CheckpointWorkerContext for CheckpointWorkerContextImpl {
    fn find_canonical_resume_point(
        &self,
        last_processed: EpochCommitment,
        history_base: Option<EpochCommitment>,
    ) -> anyhow::Result<Option<EpochCommitment>> {
        Ok(self
            .epoch_reconciler
            .find_canonical_resume_point(last_processed, history_base)?)
    }

    fn get_canonical_tip(&self) -> anyhow::Result<Option<OLBlockCommitment>> {
        Ok(self.canonical_chain_provider.get_canonical_tip_blocking()?)
    }

    fn get_canonical_block_at(&self, slot: Slot) -> anyhow::Result<Option<OLBlockCommitment>> {
        Ok(self
            .canonical_chain_provider
            .get_canonical_block_at_blocking(slot)?)
    }

    fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> anyhow::Result<Option<EpochCommitment>> {
        Ok(self
            .canonical_chain_provider
            .get_canonical_epoch_commitment_at_blocking(epoch)?)
    }

    fn get_epoch_summary(
        &self,
        commitment: EpochCommitment,
    ) -> anyhow::Result<Option<EpochSummary>> {
        self.storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(commitment)
            .map_err(Into::into)
    }

    fn get_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
    ) -> anyhow::Result<Option<CheckpointPayload>> {
        self.storage
            .ol_checkpoint()
            .get_checkpoint_payload_entry_blocking(commitment)
            .map_err(Into::into)
    }

    fn get_last_checkpoint_payload_epoch(&self) -> anyhow::Result<Option<EpochCommitment>> {
        self.storage
            .ol_checkpoint()
            .get_last_checkpoint_payload_epoch_blocking()
            .map_err(Into::into)
    }

    fn get_history_base(&self) -> anyhow::Result<Option<EpochCommitment>> {
        self.storage
            .ol_block()
            .get_history_base_blocking()
            .map_err(Into::into)
    }

    fn put_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
    ) -> anyhow::Result<()> {
        self.storage
            .ol_checkpoint()
            .put_checkpoint_payload_entry_blocking(commitment, payload)
            .map_err(Into::into)
    }

    // TODO(STR-3064): move proof waiting to the service/runner layer using
    // tokio::select! over a watch channel instead of blocking on Condvar here.
    fn get_proof(&self, epoch: &EpochCommitment) -> anyhow::Result<ProofWaitOutcome> {
        loop {
            if self.get_canonical_epoch_commitment_at(epoch.epoch())? != Some(*epoch) {
                return Ok(ProofWaitOutcome::Superseded);
            }
            let Some(prover) = &self.prover else {
                return Ok(ProofWaitOutcome::Ready(Vec::new()));
            };
            if let Some(bytes) = self.try_read_proof(*epoch)? {
                return Ok(ProofWaitOutcome::Ready(bytes));
            }

            // The timeout also detects reorgs when no proof notification arrives.
            debug!(%epoch, "waiting for checkpoint proof");
            prover.notify.wait();
        }
    }

    fn get_block_header(
        &self,
        terminal: &OLBlockCommitment,
    ) -> anyhow::Result<Option<OLBlockHeaderV1>> {
        self.storage
            .ol_block()
            .get_ol_header_blocking(*terminal.blkid())
            .map_err(Into::into)
    }

    fn collect_epoch_block_commitments(
        &self,
        start: &OLBlockHeaderV1,
    ) -> anyhow::Result<EpochBlockAncestry> {
        Ok(self
            .block_ancestry_provider
            .collect_epoch_block_commitments_blocking(start)?)
    }

    fn get_block(&self, id: &OLBlockId) -> anyhow::Result<Option<OLBlockV1>> {
        self.storage
            .ol_block()
            .get_block_data_blocking(*id)
            .map_err(Into::into)
    }

    fn get_ol_state(
        &self,
        commitment: &OLBlockCommitment,
    ) -> anyhow::Result<Option<OLStateContainer>> {
        let state = self
            .storage
            .ol_state()
            .get_toplevel_ol_state_blocking(*commitment)?;
        Ok(state.map(|arc| (*arc).clone()))
    }

    fn genesis_l1_block(&self) -> L1BlockCommitment {
        self.genesis_l1_block
    }

    fn get_l1_manifest(&self, block: &L1BlockCommitment) -> anyhow::Result<Option<AsmManifest>> {
        // Manifests are stored by block, so another block's manifest is never
        // returned for this one.
        Ok(self.storage.l1().get_block_manifest(block.blkid())?)
    }

    fn fetch_da_for_epoch(
        &self,
        summary: &EpochSummary,
    ) -> anyhow::Result<(StateDiffRaw, Vec<OLLog>)> {
        let (statediff, logs, terminal_header) =
            replay_epoch_and_compute_da(self, summary, &self.runtime_params)?;
        assert_terminal_commitment_matches(&terminal_header, summary.terminal())?;
        Ok((statediff, logs))
    }
}

fn assert_terminal_commitment_matches(
    terminal_header: &OLBlockHeaderV1,
    expected_terminal: &OLBlockCommitment,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        terminal_header.slot() == expected_terminal.slot(),
        "terminal header slot mismatch: expected {}, got {}",
        expected_terminal.slot(),
        terminal_header.slot()
    );
    anyhow::ensure!(
        terminal_header.compute_blkid() == *expected_terminal.blkid(),
        "terminal header block id mismatch: expected {:?}, got {:?}",
        expected_terminal.blkid(),
        terminal_header.compute_blkid()
    );
    Ok(())
}

/// Replays epoch blocks to produce the checkpoint's OL-originated DA diff,
/// accumulated logs, and terminal header.
///
/// Loads the OL state at the previous terminal block, wraps it in
/// `DaAccumulatingState`, then re-executes every block only through its
/// pre-drain phase. The resulting diff covers OL transaction effects but
/// deliberately excludes terminal ASM effects, which reconstruction derives
/// independently by replaying the epoch's L1 manifests.
fn replay_epoch_and_compute_da<C: CheckpointWorkerContext>(
    ctx: &C,
    summary: &EpochSummary,
    runtime_params: &OLRuntimeParams,
) -> anyhow::Result<(Vec<u8>, Vec<OLLog>, OLBlockHeaderV1)> {
    let epoch_blocks = collect_epoch_blocks(summary, ctx)?;

    let prev_terminal = summary.prev_terminal();
    let prev_terminal_header = ctx.get_block_header(prev_terminal)?.ok_or_else(|| {
        anyhow::anyhow!("missing prev terminal block header for {:?}", prev_terminal)
    })?;

    let ol_state_raw = ctx
        .get_ol_state(prev_terminal)?
        .ok_or_else(|| anyhow::anyhow!("missing OL state at prev terminal {:?}", prev_terminal))?;
    let parent = EpochCommitment::from_terminal(summary.epoch() - 1, *prev_terminal);
    let spec = select_spec_after_epoch_terminal_block(ctx, parent, &ol_state_raw)?;
    let da_output = compute_epoch_da(
        spec,
        MemoryStateBaseLayer::from_container(ol_state_raw),
        &epoch_blocks,
        &prev_terminal_header,
        runtime_params,
    )?;
    let (da_bytes, logs) = da_output.into_parts();

    let terminal_header = epoch_blocks.ensured_last().header().clone();

    Ok((da_bytes, logs, terminal_header))
}

/// Selects the spec of the epoch after `parent`, whose terminal state is
/// `parent_state`, with [`select_spec_after_epoch`], as block assembly built
/// the epoch.
///
/// The parent epoch's L1 range starts at the last L1 block of the epoch its
/// own summary links to; the genesis epoch starts at the L1 anchor.
///
/// # Errors
///
/// Returns an error, carrying [`UpgradeRequired`](strata_ol_stf::UpgradeRequired)
/// if the epoch runs a spec this binary does not implement, or if the parent
/// epoch's summaries or last manifest are missing or inconsistent.
fn select_spec_after_epoch_terminal_block<C: CheckpointWorkerContext>(
    ctx: &C,
    parent: EpochCommitment,
    parent_state: &OLStateContainer,
) -> anyhow::Result<OLSpecId> {
    select_spec_after_epoch(
        parent,
        parent_state.spec_versions(),
        parent_state.chainstate().last_l1_block(),
        ctx.genesis_l1_block(),
        |epoch| ctx.get_epoch_summary(epoch),
        |block| ctx.get_l1_manifest(&block),
    )
    .map_err(|err| match err.into_lookup_error() {
        Ok(lookup) => lookup,
        Err(err) => err.into(),
    })
}

/// Loads the summary's epoch blocks in execution order, checking both terminal boundaries.
fn collect_epoch_blocks<C: CheckpointWorkerContext>(
    summary: &EpochSummary,
    ctx: &C,
) -> anyhow::Result<NonEmptyVec<OLBlockV1>> {
    let terminal = ctx
        .get_block_header(summary.terminal())?
        .ok_or_else(|| anyhow::anyhow!("missing terminal block header {}", summary.terminal()))?;
    assert_terminal_commitment_matches(&terminal, summary.terminal())?;
    anyhow::ensure!(
        terminal.is_terminal() && terminal.epoch() == summary.epoch(),
        "summary terminal is not a terminal block for epoch {}",
        summary.epoch()
    );
    let ancestry = ctx.collect_epoch_block_commitments(&terminal)?;
    anyhow::ensure!(
        ancestry.previous_terminal() == summary.prev_terminal(),
        "previous terminal block mismatch (expected {}, got {})",
        summary.prev_terminal(),
        ancestry.previous_terminal()
    );

    let blocks = ancestry
        .blocks()
        .iter()
        .map(|commitment| {
            let block = ctx.get_block(commitment.blkid())?.ok_or_else(|| {
                anyhow::anyhow!("missing block {} while collecting epoch", commitment)
            })?;
            anyhow::ensure!(
                block.header().compute_block_commitment() == *commitment,
                "block commitment mismatch while collecting epoch (expected {})",
                commitment
            );
            Ok(block)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(NonEmptyVec::try_from_vec(blocks).expect("epoch ancestry includes the starting block"))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;

    use strata_checkpoint_types::EpochSummary;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::DbError;
    use strata_db_types::backend::DatabaseBackend;
    use strata_db_types::checkpoint_proof::{CheckpointProofDatabase, ProofReceiptEntry};
    use strata_identifiers::{
        Buf32, Buf64, EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
    };
    use strata_ol_chain_types_v1::{
        BlockFlagsV1, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLTxSegmentV1,
        SignedOLBlockHeaderV1,
    };
    use strata_ol_params::OLRuntimeParams;
    use strata_storage::create_node_storage;

    use super::{
        CheckpointWorkerContext, CheckpointWorkerContextImpl, PROOF_WAIT_INTERVAL, ProofNotify,
        ProofWaitOutcome, ProverConfig, collect_epoch_blocks,
    };

    struct EpochBlocksFixture {
        context: CheckpointWorkerContextImpl,
        genesis: OLBlockHeaderV1,
    }

    impl EpochBlocksFixture {
        fn new() -> Self {
            let storage = Arc::new(
                create_node_storage(
                    get_test_sled_backend(),
                    strata_storage::test_runtime_handle(),
                )
                .expect("create epoch-block test storage"),
            );
            let mut flags = BlockFlagsV1::zero();
            flags.set_is_terminal(true);
            let genesis = OLBlockHeaderV1::new(
                0,
                flags,
                0,
                0,
                OLBlockId::from(Buf32::zero()),
                Buf32::zero(),
                Buf32::zero(),
                Buf32::zero(),
            );
            storage
                .ol_block()
                .put_terminal_header_blocking(genesis.compute_blkid(), genesis.clone())
                .expect("store genesis terminal header");
            Self {
                context: CheckpointWorkerContextImpl::new(
                    storage,
                    OLRuntimeParams::test_default(),
                    L1BlockCommitment::default(),
                ),
                genesis,
            }
        }

        fn append_block(
            &self,
            parent: &OLBlockHeaderV1,
            terminal: bool,
            branch: u8,
        ) -> OLBlockHeaderV1 {
            let body = OLBlockBodyV1::new_common(
                OLTxSegmentV1::new(vec![]).expect("construct empty transaction segment"),
            );
            let mut flags = BlockFlagsV1::zero();
            flags.set_is_terminal(terminal);
            let header = OLBlockHeaderV1::new(
                parent.timestamp() + 1,
                flags,
                parent.slot() + 1,
                parent.epoch() + u32::from(parent.is_terminal()),
                parent.compute_blkid(),
                body.compute_hash_commitment(),
                Buf32::from([branch; 32]),
                Buf32::zero(),
            );
            let block = OLBlockV1::new(
                SignedOLBlockHeaderV1::new(header.clone(), Buf64::zero()),
                body,
            );
            self.context
                .storage
                .ol_block()
                .put_block_data_blocking(block)
                .expect("store epoch block");
            header
        }

        fn make_epoch_summary(
            &self,
            terminal: &OLBlockHeaderV1,
            previous: &OLBlockHeaderV1,
        ) -> EpochSummary {
            EpochSummary::new(
                terminal.epoch(),
                terminal.compute_block_commitment(),
                previous.compute_block_commitment(),
                L1BlockCommitment::new(1, L1BlockId::default()),
                Buf32::zero(),
            )
        }
    }

    #[test]
    fn test_collect_epoch_blocks_follows_summary_branch_in_execution_order() {
        let fixture = EpochBlocksFixture::new();
        let pivot = fixture.append_block(&fixture.genesis, true, 1);
        let canonical = fixture.append_block(&pivot, true, 1);
        let fork_previous = fixture.append_block(&pivot, true, 2);
        let first = fixture.append_block(&fork_previous, false, 2);
        let terminal = fixture.append_block(&first, true, 2);
        fixture
            .context
            .storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(
                canonical.slot(),
                vec![canonical.compute_blkid()],
            )
            .expect("publish competing canonical terminal");

        let blocks = collect_epoch_blocks(
            &fixture.make_epoch_summary(&terminal, &fork_previous),
            &fixture.context,
        )
        .expect("collect summary branch");
        assert_eq!(
            blocks
                .iter()
                .map(|block| block.header().compute_block_commitment())
                .collect::<Vec<_>>(),
            vec![
                first.compute_block_commitment(),
                terminal.compute_block_commitment()
            ]
        );
    }

    #[test]
    fn test_collect_epoch_blocks_rejects_summary_predecessor_mismatch() {
        let fixture = EpochBlocksFixture::new();
        let previous = fixture.append_block(&fixture.genesis, true, 1);
        let terminal = fixture.append_block(&previous, true, 1);
        let fork_previous = fixture.append_block(&fixture.genesis, true, 2);
        for predecessor in [
            fork_previous.compute_block_commitment(),
            OLBlockCommitment::new(previous.slot() + 1, previous.compute_blkid()),
        ] {
            let summary = EpochSummary::new(
                terminal.epoch(),
                terminal.compute_block_commitment(),
                predecessor,
                L1BlockCommitment::new(1, L1BlockId::default()),
                Buf32::zero(),
            );
            let error = collect_epoch_blocks(&summary, &fixture.context)
                .expect_err("reject mismatched summary predecessor");
            assert!(
                error
                    .to_string()
                    .contains("previous terminal block mismatch"),
                "{error:?}"
            );
        }
    }

    #[test]
    fn test_collect_epoch_blocks_rejects_invalid_summary_terminal() {
        let fixture = EpochBlocksFixture::new();
        let nonterminal = fixture.append_block(&fixture.genesis, false, 1);
        let terminal = fixture.append_block(&nonterminal, true, 1);
        let cases = [
            fixture.make_epoch_summary(&nonterminal, &fixture.genesis),
            EpochSummary::new(
                2,
                terminal.compute_block_commitment(),
                fixture.genesis.compute_block_commitment(),
                L1BlockCommitment::new(1, L1BlockId::default()),
                Buf32::zero(),
            ),
        ];
        for summary in cases {
            let error = collect_epoch_blocks(&summary, &fixture.context)
                .expect_err("reject invalid summary terminal");
            assert!(
                error.to_string().contains("not a terminal block for epoch"),
                "{error:?}"
            );
        }
    }

    #[test]
    fn test_collect_epoch_blocks_requires_body_for_retained_terminal() {
        let fixture = EpochBlocksFixture::new();
        let mut flags = BlockFlagsV1::zero();
        flags.set_is_terminal(true);
        let terminal = OLBlockHeaderV1::new(
            1,
            flags,
            1,
            1,
            fixture.genesis.compute_blkid(),
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        fixture
            .context
            .storage
            .ol_block()
            .put_terminal_header_blocking(terminal.compute_blkid(), terminal.clone())
            .expect("store retained terminal header");
        let error = collect_epoch_blocks(
            &fixture.make_epoch_summary(&terminal, &fixture.genesis),
            &fixture.context,
        )
        .expect_err("terminal body is required for checkpoint replay");
        assert!(error.to_string().contains("missing block"), "{error:?}");
    }

    fn proof_test_context() -> (
        CheckpointWorkerContextImpl,
        Arc<ProofNotify>,
        EpochCommitment,
    ) {
        let storage = Arc::new(
            create_node_storage(
                get_test_sled_backend(),
                strata_storage::test_runtime_handle(),
            )
            .unwrap(),
        );
        let mut flags = BlockFlagsV1::zero();
        flags.set_is_terminal(true);
        let header = OLBlockHeaderV1::new(
            1_000,
            flags,
            1,
            1,
            OLBlockId::from(Buf32::zero()),
            Buf32::from([1; 32]),
            Buf32::from([2; 32]),
            Buf32::from([3; 32]),
        );
        let terminal = header.compute_block_commitment();
        storage
            .ol_block()
            .put_terminal_header_blocking(*terminal.blkid(), header)
            .unwrap();
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(EpochSummary::new(
                1,
                terminal,
                OLBlockCommitment::null(),
                L1BlockCommitment::new(1, L1BlockId::default()),
                Buf32::from([2; 32]),
            ))
            .unwrap();
        storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(1, vec![*terminal.blkid()])
            .unwrap();
        let notify = Arc::new(ProofNotify::new());
        let context = CheckpointWorkerContextImpl::with_prover(
            storage,
            OLRuntimeParams::test_default(),
            L1BlockCommitment::default(),
            ProverConfig {
                notify: Arc::clone(&notify),
            },
        );
        (context, notify, EpochCommitment::from_terminal(1, terminal))
    }

    #[test]
    fn test_empty_proof_requires_canonical_commitment() {
        let (mut context, _, epoch) = proof_test_context();
        context.prover = None;
        assert_eq!(
            context.get_proof(&epoch).unwrap(),
            ProofWaitOutcome::Ready(vec![])
        );
        context
            .storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(epoch.last_slot(), vec![])
            .unwrap();
        assert_eq!(
            context.get_proof(&epoch).unwrap(),
            ProofWaitOutcome::Superseded
        );
    }

    #[test]
    fn test_proof_wait_cancels_after_notification_or_timeout() {
        for notify_on_reorg in [true, false] {
            let (context, notify, epoch) = proof_test_context();
            let storage = Arc::clone(&context.storage);
            let (started_tx, started_rx) = mpsc::channel();
            let (result_tx, result_rx) = mpsc::channel();
            let worker = thread::spawn(move || {
                started_tx.send(()).unwrap();
                result_tx.send(context.get_proof(&epoch)).unwrap();
            });
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            // No proof exists, so the worker must not finish on the old branch.
            assert!(matches!(
                result_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            storage
                .ol_block()
                .replace_canonical_suffix_from_blocking(epoch.last_slot(), vec![])
                .unwrap();
            if notify_on_reorg {
                notify.notify();
            }
            // With no notification, the existing timeout still cancels the wait.
            let result = result_rx
                .recv_timeout(PROOF_WAIT_INTERVAL + Duration::from_secs(5))
                .unwrap()
                .unwrap();
            worker.join().unwrap();
            assert_eq!(result, ProofWaitOutcome::Superseded);
        }
    }

    #[test]
    fn test_proof_storage_failure_not_superseded() {
        let (context, _, epoch) = proof_test_context();
        // A malformed stored receipt produces a typed storage decoding failure.
        context
            .storage
            .db()
            .checkpoint_proof_db()
            .put_proof(epoch, ProofReceiptEntry::new(vec![0xff]))
            .unwrap();
        let err = context.get_proof(&epoch).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<DbError>(),
            Some(DbError::CodecError(_))
        ));
    }

    #[test]
    fn production_context_gets_header_from_terminal_header_record() {
        let storage = Arc::new(
            create_node_storage(
                get_test_sled_backend(),
                strata_storage::test_runtime_handle(),
            )
            .expect("create test storage"),
        );
        let mut flags = BlockFlagsV1::zero();
        flags.set_is_terminal(true);
        let header = OLBlockHeaderV1::new(
            1_000,
            flags,
            9,
            3,
            OLBlockId::from(Buf32::zero()),
            Buf32::from([1; 32]),
            Buf32::from([2; 32]),
            Buf32::from([3; 32]),
        );
        let commitment = header.compute_block_commitment();
        storage
            .ol_block()
            .put_terminal_header_blocking(*commitment.blkid(), header.clone())
            .expect("store terminal header");

        let context = CheckpointWorkerContextImpl::new(
            storage,
            OLRuntimeParams::test_default(),
            L1BlockCommitment::default(),
        );
        assert_eq!(
            context
                .get_block_header(&commitment)
                .expect("get block header"),
            Some(header)
        );
    }
}
