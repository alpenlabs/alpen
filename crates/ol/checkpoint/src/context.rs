//! Context trait for checkpoint worker dependencies.

use std::{
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Epoch, EpochCommitment, OLBlockCommitment};
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader, OLBlockId, OLLog};
use strata_ol_state_support_types::{DaAccumulatingState, MemoryStateBaseLayer};
use strata_ol_state_types::OLState;
use strata_ol_stf::execute_block_batch;
use strata_primitives::nonempty_vec::NonEmptyVec;
use strata_storage::NodeStorage;
use tracing::{debug, warn};
use zkaleido::{ProofReceiptWithMetadata, ZkVm};
use zkaleido_sp1_groth16_verifier::{
    GROTH16_PROOF_COMPRESSED_SIZE, GROTH16_PROOF_UNCOMPRESSED_SIZE, VK_HASH_PREFIX_LENGTH,
};

pub(crate) type StateDiffRaw = Vec<u8>;

/// Context providing dependencies for the checkpoint worker.
///
/// This trait abstracts storage and data providers, enabling testing
/// with mock implementations and future production providers.
pub(crate) trait CheckpointWorkerContext: Send + Sync + 'static {
    /// Get the last summarized epoch index, if any.
    fn get_last_summarized_epoch(&self) -> anyhow::Result<Option<Epoch>>;

    /// Get the canonical epoch commitment for a given epoch index.
    fn get_canonical_epoch_commitment_at(
        &self,
        index: Epoch,
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

    /// Store a checkpoint payload entry for the given epoch commitment.
    fn put_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
    ) -> anyhow::Result<()>;

    /// Gets proof bytes for the checkpoint.
    fn get_proof(&self, epoch: &EpochCommitment) -> anyhow::Result<Vec<u8>>;

    /// Gets the OL block header for the given block id.
    fn get_block_header(&self, blkid: &OLBlockCommitment) -> anyhow::Result<Option<OLBlockHeader>>;

    /// Gets an OL block by its block ID.
    fn get_block(&self, id: &OLBlockId) -> anyhow::Result<Option<OLBlock>>;

    /// Gets the OL state snapshot at a given block commitment.
    fn get_ol_state(&self, commitment: &OLBlockCommitment) -> anyhow::Result<Option<OLState>>;

    /// Fetches da data for epoch. Returns state diff and OL logs.
    fn fetch_da_for_epoch(
        &self,
        summary: &EpochSummary,
    ) -> anyhow::Result<(StateDiffRaw, Vec<OLLog>)>;
}

/// Safety-net interval for the proof wait loop.
///
/// The checkpoint worker sleeps on a [`Condvar`] that the proof storer
/// signals immediately after writing a proof, so in practice the worker
/// wakes up right away. This timeout is only a fallback in case a
/// signal is missed (e.g. spurious wakeup).
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
/// indefinitely for the prover to store them. The checkpoint predicate
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
    storage: Arc<NodeStorage>,
    /// When present, a prover is running and `get_proof` waits for proofs.
    /// When absent, `get_proof` returns empty immediately.
    prover: Option<ProverConfig>,
}

impl CheckpointWorkerContextImpl {
    /// Creates a new context without a prover.
    ///
    /// `get_proof` always returns empty bytes.
    pub(crate) fn new(storage: Arc<NodeStorage>) -> Self {
        Self {
            storage,
            prover: None,
        }
    }

    /// Creates a new context with an integrated prover.
    pub(crate) fn with_prover(storage: Arc<NodeStorage>, prover: ProverConfig) -> Self {
        Self {
            storage,
            prover: Some(prover),
        }
    }

    /// Normalizes proof bytes for checkpoint payload encoding.
    ///
    /// SP1 Groth16 proofs persisted by the SP1 host include a 4-byte verifying-key hash prefix.
    /// ASM checkpoint predicate verification expects the raw Groth16 witness bytes (128/256 bytes),
    /// so strip that SP1 prefix when present. The producing backend is read off the receipt's
    /// metadata rather than carried in the proof key.
    fn payload_proof_bytes(receipt: &ProofReceiptWithMetadata) -> Vec<u8> {
        let proof_bytes = receipt.receipt().proof().as_bytes();
        let prefixed_compressed_len = GROTH16_PROOF_COMPRESSED_SIZE + VK_HASH_PREFIX_LENGTH;
        let prefixed_uncompressed_len = GROTH16_PROOF_UNCOMPRESSED_SIZE + VK_HASH_PREFIX_LENGTH;

        if matches!(receipt.metadata().zkvm(), ZkVm::SP1)
            && (proof_bytes.len() == prefixed_compressed_len
                || proof_bytes.len() == prefixed_uncompressed_len)
        {
            return proof_bytes[VK_HASH_PREFIX_LENGTH..].to_vec();
        }

        proof_bytes.to_vec()
    }

    /// Attempts to read a non-empty proof from the proof DB for the given epoch commitment.
    ///
    /// Returns `Ok(Some(bytes))` if a valid proof is found, `Ok(None)` if no
    /// proof is available yet.
    fn try_read_proof(&self, commitment: EpochCommitment) -> anyhow::Result<Option<Vec<u8>>> {
        if let Some(receipt) = self.storage.checkpoint_proof().get_proof(&commitment)? {
            let proof_bytes = Self::payload_proof_bytes(&receipt);
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
    fn get_last_summarized_epoch(&self) -> anyhow::Result<Option<Epoch>> {
        self.storage
            .ol_checkpoint()
            .get_last_summarized_epoch_blocking()
            .map_err(Into::into)
    }

    fn get_canonical_epoch_commitment_at(
        &self,
        index: Epoch,
    ) -> anyhow::Result<Option<EpochCommitment>> {
        self.storage
            .ol_checkpoint()
            .get_canonical_epoch_commitment_at_blocking(index)
            .map_err(Into::into)
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

    /// Returns proof bytes for the given epoch.
    // TODO(STR-3064): move proof waiting to the service/runner layer using
    // tokio::select! over a watch channel instead of blocking on Condvar here.
    /// The flow depends on whether a prover is configured:
    ///
    /// 1. No prover (`self.prover` is `None`): returns empty bytes.
    /// 2. Prover configured: checks the proof DB first (handles restarts where the proof was
    ///    already stored). If not found, enters a wait loop where `ProofNotify::wait` blocks until
    ///    the proof storer signals, then re-checks the DB. The loop waits indefinitely until a
    ///    proof appears.
    fn get_proof(&self, epoch: &EpochCommitment) -> anyhow::Result<Vec<u8>> {
        let Some(prover) = &self.prover else {
            return Ok(Vec::new());
        };

        let epoch_idx = u64::from(epoch.epoch());

        // Check DB first — proof may already exist (e.g. after restart).
        if let Some(bytes) = self.try_read_proof(*epoch)? {
            return Ok(bytes);
        }

        // Wait loop: sleep on the condvar until the proof storer wakes us,
        // then check DB. Repeat until proof is found.
        loop {
            debug!(epoch = epoch_idx, "waiting for checkpoint proof...");
            prover.notify.wait();

            if let Some(bytes) = self.try_read_proof(*epoch)? {
                return Ok(bytes);
            }
        }
    }

    fn get_block_header(
        &self,
        terminal: &OLBlockCommitment,
    ) -> anyhow::Result<Option<OLBlockHeader>> {
        let maybe_block = self
            .storage
            .ol_block()
            .get_block_data_blocking(*terminal.blkid())?;
        Ok(maybe_block.map(|block| block.header().clone()))
    }

    fn get_block(&self, id: &OLBlockId) -> anyhow::Result<Option<OLBlock>> {
        self.storage
            .ol_block()
            .get_block_data_blocking(*id)
            .map_err(Into::into)
    }

    fn get_ol_state(&self, commitment: &OLBlockCommitment) -> anyhow::Result<Option<OLState>> {
        let state = self
            .storage
            .ol_state()
            .get_toplevel_ol_state_blocking(*commitment)?;
        Ok(state.map(|arc| (*arc).clone()))
    }

    fn fetch_da_for_epoch(
        &self,
        summary: &EpochSummary,
    ) -> anyhow::Result<(StateDiffRaw, Vec<OLLog>)> {
        let (statediff, logs, terminal_header) = replay_epoch_and_compute_da(self, summary)?;
        assert_terminal_commitment_matches(&terminal_header, summary.terminal())?;
        Ok((statediff, logs))
    }
}

fn assert_terminal_commitment_matches(
    terminal_header: &OLBlockHeader,
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

/// Replays epoch blocks to produce DA state diff bytes, accumulated logs, and
/// the terminal header.
///
/// Loads the OL state at the previous terminal block, wraps it in
/// `DaAccumulatingState` to intercept mutations, then re-executes every block
/// in the epoch. The DA blob is extracted from the accumulating layer and the
/// logs are collected from each block's execution output.
fn replay_epoch_and_compute_da<C: CheckpointWorkerContext>(
    ctx: &C,
    summary: &EpochSummary,
) -> anyhow::Result<(Vec<u8>, Vec<OLLog>, OLBlockHeader)> {
    let epoch_blocks = collect_epoch_blocks(summary, ctx)?;

    let prev_terminal = summary.prev_terminal();
    let prev_terminal_header = ctx.get_block_header(prev_terminal)?.ok_or_else(|| {
        anyhow::anyhow!("missing prev terminal block header for {:?}", prev_terminal)
    })?;

    let ol_state_raw = ctx
        .get_ol_state(prev_terminal)?
        .ok_or_else(|| anyhow::anyhow!("missing OL state at prev terminal {:?}", prev_terminal))?;
    let ol_state = MemoryStateBaseLayer::new(ol_state_raw);

    let mut da_state = DaAccumulatingState::new(ol_state);

    let logs = execute_block_batch(&mut da_state, &epoch_blocks, &prev_terminal_header)
        .map_err(|e| anyhow::anyhow!("epoch block replay failed: {e}"))?;

    let terminal_header = epoch_blocks.ensured_last().header().clone();

    // Extract the DA blob from the accumulating layer.
    let da_bytes = da_state
        .take_completed_epoch_da_blob()
        .map_err(|e| anyhow::anyhow!("DA accumulation failed: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("no DA blob produced after epoch replay"))?;

    Ok((da_bytes, logs, terminal_header))
}

/// Collects all blocks in an epoch by walking backwards from the terminal block.
///
/// Returns blocks in forward order (first block of epoch first, terminal last).
fn collect_epoch_blocks<C: CheckpointWorkerContext>(
    summary: &EpochSummary,
    ctx: &C,
) -> anyhow::Result<NonEmptyVec<OLBlock>> {
    let terminal_blkid = summary.terminal().blkid();
    let prev_terminal_blkid = summary.prev_terminal().blkid();
    let prev_terminal_slot = summary.prev_terminal().slot();

    let mut blocks = Vec::new();
    let mut cur_id = *terminal_blkid;

    loop {
        let block = ctx
            .get_block(&cur_id)?
            .ok_or_else(|| anyhow::anyhow!("missing block {cur_id:?} while collecting epoch"))?;

        anyhow::ensure!(
            block.header().slot() > prev_terminal_slot,
            "block at slot {} is at or below prev terminal slot {}; \
             epoch chain is broken",
            block.header().slot(),
            prev_terminal_slot,
        );

        // Check if the same epoch is being traversed.
        anyhow::ensure!(
            block.header().epoch() == summary.epoch(),
            "Obtained a block with different epoch, expected {}, obtained {}",
            summary.epoch(),
            block.header().epoch(),
        );

        let parent_id = *block.header().parent_blkid();
        blocks.push(block);

        if parent_id == *prev_terminal_blkid {
            break;
        }

        cur_id = parent_id;
    }

    blocks.reverse();
    let blocks =
        NonEmptyVec::try_from_vec(blocks).map_err(|_| anyhow::anyhow!("Non-empty epoch blocks"))?;
    Ok(blocks)
}
