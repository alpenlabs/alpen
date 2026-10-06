//! Proof specification for checkpoint proofs.
//!
//! Implements [`ProofSpec`] from `strata_prover_core`: identifies the task
//! as an [`EpochCommitment`] and fetches the proof input from local
//! [`NodeStorage`] without any RPC round-trip.

use std::sync::Arc;

use async_trait::async_trait;
pub(crate) use strata_checkpoint_types::CheckpointProofTask as CheckpointTask;
use strata_checkpoint_types::EpochSummary;
use strata_identifiers::{Epoch, EpochCommitment};
use strata_ol_checkpoint::compute_epoch_da;
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_container::OLStateContainer;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{OLSpecId, UnknownOLSpecId};
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

/// Returns the service version for a task from its exact committed epoch start.
///
/// This performs blocking storage reads. The caller must use a blocking task when
/// routing from an async worker. Missing state remains retryable; ASM's current
/// predicate and the latest local OL state never select the proving program.
pub(crate) fn checkpoint_task_spec(
    storage: &NodeStorage,
    task: CheckpointTask,
) -> Result<OLSpecId, ProverError> {
    Ok(read_checkpoint_start(storage, task.0)?.spec)
}

/// Selects the program for the epoch following this committed start state.
///
/// Matches the spec check in [`strata_proofimpl_checkpoint::process_ol_stf_core`].
/// V0 has no committed staged spec, so the first V1 epoch uses V0's successor.
fn checkpoint_epoch_spec(start_state: &OLStateContainer) -> Result<OLSpecId, UnknownOLSpecId> {
    // TODO(STR-4082): When adding another proving spec, keep this node rule in sync with
    // the guest's spec check in `process_ol_stf_core`.
    if start_state.cur_spec() == OLSpecId::V0 {
        Ok(OLSpecId::V0.successor().expect("V0 has a successor spec"))
    } else {
        start_state.root().staged_spec()
    }
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

struct CheckpointStart {
    summary: EpochSummary,
    state: Arc<OLStateContainer>,
    spec: OLSpecId,
}

/// Resolves the same canonical task and committed state for routing and input assembly.
fn read_checkpoint_start(
    storage: &NodeStorage,
    task_commitment: EpochCommitment,
) -> Result<CheckpointStart, ProverError> {
    let epoch: Epoch = task_commitment.epoch;
    let epoch_index = u64::from(epoch);
    validate_checkpoint_task_commitment(storage, task_commitment)?;

    let summary = storage
        .ol_checkpoint()
        .get_epoch_summary_blocking(task_commitment)?
        .ok_or(ProverError::EpochSummaryNotFound(epoch_index))?;

    let previous_terminal = *summary.prev_terminal();
    let state = storage
        .ol_state()
        .get_toplevel_ol_state_blocking(previous_terminal)?
        .ok_or(ProverError::EpochStartStateNotFound {
            commitment: previous_terminal,
        })?;
    let spec = checkpoint_epoch_spec(&state)?;
    Ok(CheckpointStart {
        summary,
        state,
        spec,
    })
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

    let CheckpointStart {
        summary,
        state: start_state,
        spec,
    } = read_checkpoint_start(&storage, task_commitment)?;
    if spec != assigned_spec {
        return Err(ProverError::MisroutedTask {
            task: task_commitment,
            assigned: assigned_spec,
            required: spec,
        });
    }

    let terminal = summary.terminal();
    let prev_terminal = summary.prev_terminal();
    let prev_terminal_slot = prev_terminal.slot();
    let target_epoch = summary.epoch();

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

#[cfg(test)]
mod tests {
    use strata_ol_state_container::test_utils::{
        create_test_container_with_staged, create_test_genesis_container,
    };
    use strata_ol_state_types::OLSpecVersions;

    use super::*;

    #[test]
    fn selects_staged_spec_instead_of_current_spec() {
        let state = create_test_container_with_staged(0);
        assert_eq!(state.cur_spec(), OLSpecId::V1);
        assert_eq!(checkpoint_epoch_spec(&state), Ok(OLSpecId::V0));
    }

    #[test]
    fn selects_v1_for_first_checkpoint_after_v0_state() {
        let (_, chainstate) = create_test_genesis_container().into_parts();
        let state = OLStateContainer::new(OLSpecVersions::uniform(OLSpecId::V0), chainstate);
        assert_eq!(checkpoint_epoch_spec(&state), Ok(OLSpecId::V1));
    }

    #[test]
    fn unknown_staged_spec_never_falls_back_to_current_spec() {
        let state = create_test_container_with_staged(2);
        assert_eq!(checkpoint_epoch_spec(&state).unwrap_err().raw(), 2);
    }
}
