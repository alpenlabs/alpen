//! Input fetcher for checkpoint proofs.
//!
//! Reads OL state, blocks, and DA data directly from local [`NodeStorage`]
//! to construct [`CheckpointProverInput`] without RPC calls.

use std::sync::Arc;

use async_trait::async_trait;
use strata_identifiers::Epoch;
use strata_ol_state_support_types::DaAccumulatingState;
use strata_ol_stf::execute_block_batch;
use strata_paas::InputFetcher;
use strata_proofimpl_checkpoint_new::program::CheckpointProverInput;
use strata_storage::NodeStorage;
use tokio::task::spawn_blocking;
use tracing::debug;

use super::{errors::ProverError, task::CheckpointTask};

/// Fetches checkpoint proof inputs from local node storage.
///
/// Reads the OL state snapshot, epoch blocks, and DA state diff for a given
/// epoch directly from the node's storage managers.
#[derive(Clone)]
pub(crate) struct CheckpointInputFetcher {
    storage: Arc<NodeStorage>,
}

impl CheckpointInputFetcher {
    pub(crate) fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl InputFetcher<CheckpointTask> for CheckpointInputFetcher {
    type Input = CheckpointProverInput;
    type Error = ProverError;

    async fn fetch_input(&self, program: &CheckpointTask) -> Result<Self::Input, Self::Error> {
        let epoch = program.epoch;
        let epoch_index = u64::from(epoch);
        debug!(%epoch_index, "fetching checkpoint proof input");

        let storage = Arc::clone(&self.storage);
        spawn_blocking(move || fetch_input_blocking(storage, epoch))
            .await
            .map_err(|e| ProverError::DaReplay(format!("input fetch task join error: {e}")))?
    }
}

fn fetch_input_blocking(
    storage: Arc<NodeStorage>,
    epoch: Epoch,
) -> Result<CheckpointProverInput, ProverError> {
    let epoch_index = u64::from(epoch);
    debug!(%epoch_index, "fetching checkpoint proof input (blocking)");

    // Get epoch summary.
    let commitment = storage
        .ol_checkpoint()
        .get_canonical_epoch_commitment_at_blocking(epoch)?
        .ok_or(ProverError::EpochCommitmentNotFound(epoch_index))?;

    let summary = storage
        .ol_checkpoint()
        .get_epoch_summary_blocking(commitment)?
        .ok_or(ProverError::EpochSummaryNotFound(epoch_index))?;

    let terminal = summary.terminal();
    let prev_terminal = summary.prev_terminal();

    // Get the parent block header (last block of the previous epoch).
    let parent_block = storage
        .ol_block()
        .get_block_data_blocking(*prev_terminal.blkid())?
        .ok_or(ProverError::BlockNotFound(prev_terminal.slot()))?;
    let parent = parent_block.header().clone();

    // Get the OL state snapshot at the previous terminal block.
    let start_state = storage
        .ol_state()
        .get_toplevel_ol_state_blocking(*prev_terminal)?
        .ok_or_else(|| ProverError::StateNotFound(format!("{prev_terminal:?}")))?;

    // Collect epoch blocks by walking the parent chain backwards from the
    // terminal block to the previous terminal. This is the canonical,
    // fork-safe approach: it follows actual parent pointers rather than
    // iterating by slot, so it always produces the correct canonical
    // sequence even during reorgs.
    let mut blocks = Vec::new();
    let mut cur_id = *terminal.blkid();

    loop {
        let block = storage
            .ol_block()
            .get_block_data_blocking(cur_id)?
            .ok_or_else(|| {
                ProverError::StateNotFound(format!(
                    "block {cur_id:?} missing during epoch {epoch_index} chain traversal"
                ))
            })?;

        let parent_id = *block.header().parent_blkid();
        blocks.push(block);

        if parent_id == *prev_terminal.blkid() {
            break;
        }

        cur_id = parent_id;
    }

    blocks.reverse();

    // Compute DA state diff bytes by replaying the epoch blocks through a
    // [`DaAccumulatingState`] wrapper. This intercepts state mutations to
    // build the same DA diff that the guest program will verify. Computing
    // it here (rather than reading a checkpoint entry) ensures the diff
    // is available before the checkpoint entry is written.
    let da_state_diff_bytes = {
        let mut da_state = DaAccumulatingState::new((*start_state).clone());
        execute_block_batch(&mut da_state, &blocks, &parent)
            .map_err(|e| ProverError::DaReplay(e.to_string()))?;
        da_state
            .take_completed_epoch_da_blob()
            .map_err(|e| ProverError::DaReplay(e.to_string()))?
            .ok_or_else(|| {
                ProverError::DaReplay("no DA blob produced after epoch replay".to_string())
            })?
    };

    debug!(
        %epoch_index,
        num_blocks = blocks.len(),
        da_bytes_len = da_state_diff_bytes.len(),
        "assembled checkpoint proof input"
    );

    Ok(CheckpointProverInput {
        start_state: (*start_state).clone(),
        blocks,
        parent,
        da_state_diff_bytes,
    })
}
