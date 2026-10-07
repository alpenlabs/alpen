//! Proof specification for checkpoint proofs.
//!
//! Implements [`ProofSpec`] from `strata_prover_core`: identifies the task
//! as an [`EpochCommitment`] and fetches the proof input from local
//! [`NodeStorage`] without any RPC round-trip.

use std::sync::Arc;

use async_trait::async_trait;
pub(crate) use strata_checkpoint_types::CheckpointProofTask as CheckpointTask;
use strata_identifiers::{Epoch, EpochCommitment};
use strata_ol_checkpoint::compute_epoch_da;
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::OLSpecId;
use strata_paas::{InputResolution, ProofSpec, ProverError as PaasError, ProverResult};
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use strata_storage::NodeStorage;
use tokio::task::spawn_blocking;
use tracing::debug;

use super::errors::ProverError;

/// Proof specification for integrated checkpoint proving.
#[derive(Clone)]
pub(crate) struct CheckpointSpec {
    storage: Arc<NodeStorage>,
    runtime_params: OLRuntimeParams,
    assigned_spec: OLSpecId,
}

impl CheckpointSpec {
    pub(crate) fn new(
        storage: Arc<NodeStorage>,
        runtime_params: OLRuntimeParams,
        assigned_spec: OLSpecId,
    ) -> Self {
        Self {
            storage,
            runtime_params,
            assigned_spec,
        }
    }
}

#[async_trait]
impl ProofSpec for CheckpointSpec {
    type Task = CheckpointTask;
    type Program = CheckpointProgram;

    async fn resolve_input(
        &self,
        task: &Self::Task,
    ) -> ProverResult<InputResolution<CheckpointProverInput>> {
        let commitment = task.0;
        debug!(epoch = %commitment.epoch, "fetching checkpoint proof input");
        let storage = Arc::clone(&self.storage);
        let runtime_params = self.runtime_params;
        let assigned_spec = self.assigned_spec;
        // All storage access is blocking; hop to a blocking thread so we
        // don't stall the async runtime while reading blocks and state. A join
        // error is an infra fault (Err → retried); the inner classification
        // (epoch-not-ready → Blocked, DB → Err, else → Rejected) is bridged by
        // `InputResolution::from_result`.
        let assembled = spawn_blocking(move || {
            fetch_input_blocking(storage, commitment, runtime_params, assigned_spec)
        })
        .await
        .map_err(|error| PaasError::Storage(format!("input fetch join: {error}")))?;
        InputResolution::from_result(assembled.map_err(PaasError::from))
    }
}

/// Reads the spec used by the task's epoch from its terminal OL state.
///
/// The chain worker stores this state before publishing the epoch summary. Its
/// current spec identifies the completed epoch; its staged spec is for the next
/// epoch. No transition rule needs to be repeated here.
///
/// Performs blocking storage reads. Missing terminal state remains retryable;
/// neither ASM state nor a different OL state supplies a fallback.
pub(crate) fn checkpoint_task_spec(
    storage: &NodeStorage,
    task: CheckpointTask,
) -> Result<OLSpecId, ProverError> {
    validate_checkpoint_task_commitment(storage, task.0)?;
    let commitment = task.0.to_block_commitment();
    let state = storage
        .ol_state()
        .get_toplevel_ol_state_blocking(commitment)?
        .ok_or(ProverError::EpochTerminalStateNotFound { commitment })?;
    Ok(state.cur_spec())
}

/// Checks that the task still matches the canonical commitment for its epoch.
fn validate_checkpoint_task_commitment(
    storage: &NodeStorage,
    task_commitment: EpochCommitment,
) -> Result<(), ProverError> {
    let epoch = task_commitment.epoch;
    let epoch_index = u64::from(epoch);
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

    Ok(())
}

fn fetch_input_blocking(
    storage: Arc<NodeStorage>,
    task_commitment: EpochCommitment,
    runtime_params: OLRuntimeParams,
    assigned_spec: OLSpecId,
) -> Result<CheckpointProverInput, ProverError> {
    let epoch: Epoch = task_commitment.epoch;
    let epoch_index = u64::from(epoch);
    debug!(%epoch_index, "fetching checkpoint proof input (blocking)");

    let spec = checkpoint_task_spec(&storage, CheckpointTask(task_commitment))?;
    if spec != assigned_spec {
        return Err(ProverError::MisroutedTask {
            task: task_commitment,
            assigned: assigned_spec,
            required: spec,
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

    let start_state = storage
        .ol_state()
        .get_toplevel_ol_state_blocking(*prev_terminal)?
        .ok_or(ProverError::EpochStartStateNotFound {
            commitment: *prev_terminal,
        })?;

    // The previous terminal header authenticates the witness's starting state.
    let parent = storage
        .ol_block()
        .get_ol_header_blocking(*prev_terminal.blkid())?
        .ok_or(ProverError::BlockNotFound(prev_terminal.slot()))?;

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

    let da_output = compute_epoch_da(
        spec,
        MemoryStateBaseLayer::from_container((*start_state).clone()),
        &blocks,
        &parent,
        &runtime_params,
    )
    .map_err(|err| ProverError::DaComputation(err.to_string()))?;
    let (da_state_diff_bytes, _) = da_output.into_parts();

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
