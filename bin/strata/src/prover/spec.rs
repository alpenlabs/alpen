//! Proof specification for checkpoint proofs.
//!
//! Implements [`ProofSpec`] from `strata_prover_core`: identifies the task
//! as an [`EpochCommitment`] and fetches the proof input from local
//! [`NodeStorage`] without any RPC round-trip.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize, io::Error as BorshIoError};
use strata_identifiers::{Epoch, EpochCommitment};
use strata_ol_state_support_types::{DaAccumulatingState, MemoryStateBaseLayer};
use strata_ol_stf::execute_block_batch;
use strata_paas::{ProofSpec, ProverError as PaasError, ProverResult};
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use strata_storage::NodeStorage;
use tokio::task::spawn_blocking;
use tracing::debug;

use super::errors::ProverError;

/// Task identifier for checkpoint proofs.
///
/// Newtype over [`EpochCommitment`] so we can attach the byte-encoding and
/// display bounds that [`strata_paas::TaskKey`] requires without polluting
/// the domain type.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize)]
pub(crate) struct CheckpointTask(pub EpochCommitment);

impl fmt::Display for CheckpointTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<CheckpointTask> for Vec<u8> {
    fn from(task: CheckpointTask) -> Self {
        borsh::to_vec(&task).expect("CheckpointTask borsh-serializable")
    }
}

impl TryFrom<Vec<u8>> for CheckpointTask {
    type Error = BorshIoError;

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        borsh::from_slice(&bytes)
    }
}

/// Proof specification for integrated checkpoint proving.
#[derive(Clone)]
pub(crate) struct CheckpointSpec {
    storage: Arc<NodeStorage>,
}

impl CheckpointSpec {
    pub(crate) fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl ProofSpec for CheckpointSpec {
    type Task = CheckpointTask;
    type Program = CheckpointProgram;

    async fn fetch_input(&self, task: &Self::Task) -> ProverResult<CheckpointProverInput> {
        let commitment = task.0;
        debug!(epoch = %commitment.epoch, "fetching checkpoint proof input");
        let storage = Arc::clone(&self.storage);
        // All storage access is blocking; hop to a blocking thread so we
        // don't stall the async runtime while reading blocks and state.
        spawn_blocking(move || fetch_input_blocking(storage, commitment))
            .await
            .map_err(|e| PaasError::TransientFailure(format!("input fetch join: {e}")))?
            .map_err(PaasError::from)
    }
}

fn fetch_input_blocking(
    storage: Arc<NodeStorage>,
    task_commitment: EpochCommitment,
) -> Result<CheckpointProverInput, ProverError> {
    let epoch: Epoch = task_commitment.epoch;
    let epoch_index = u64::from(epoch);
    debug!(%epoch_index, "fetching checkpoint proof input (blocking)");

    // Ensure this task still matches the canonical commitment for the epoch.
    let canonical_commitment = storage
        .ol_checkpoint()
        .get_canonical_epoch_commitment_at_blocking(epoch)?
        .ok_or(ProverError::EpochCommitmentNotFound(epoch_index))?;
    if canonical_commitment != task_commitment {
        return Err(ProverError::StaleTaskCommitment {
            epoch: epoch_index,
            task: task_commitment,
            canonical: canonical_commitment,
        });
    }

    let summary = storage
        .ol_checkpoint()
        .get_epoch_summary_blocking(task_commitment)?
        .ok_or(ProverError::EpochSummaryNotFound(epoch_index))?;

    let terminal = summary.terminal();
    let prev_terminal = summary.prev_terminal();
    let prev_terminal_slot = prev_terminal.slot();
    let target_epoch = summary.epoch();

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

        let block_header = block.header();
        let block_slot = block_header.slot();
        let block_epoch = block_header.epoch();
        if block_slot <= prev_terminal_slot {
            return Err(ProverError::StateNotFound(format!(
                "block at slot {block_slot} is at or below prev terminal slot \
                 {prev_terminal_slot} while collecting epoch {epoch_index}"
            )));
        }
        if block_epoch != target_epoch {
            return Err(ProverError::StateNotFound(format!(
                "obtained block from different epoch while collecting epoch {epoch_index}: \
                 expected {target_epoch}, got {block_epoch}"
            )));
        }

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
        let mut da_state =
            DaAccumulatingState::new(MemoryStateBaseLayer::new((*start_state).clone()));
        execute_block_batch(&mut da_state, &blocks, &parent)
            .map_err(|e| ProverError::DaComputation(e.to_string()))?;
        da_state
            .take_completed_epoch_da_blob()
            .map_err(|e| ProverError::DaComputation(e.to_string()))?
            .ok_or_else(|| {
                ProverError::DaComputation("no DA blob produced after epoch replay".to_string())
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
