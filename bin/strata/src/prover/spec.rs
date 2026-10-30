//! Proof specification for checkpoint proofs.
//!
//! Implements [`ProofSpec`] from `strata_prover_core`: identifies the task
//! as an [`EpochCommitment`] and fetches the proof input from local
//! [`NodeStorage`] without any RPC round-trip.

use std::sync::Arc;

use async_trait::async_trait;
pub(crate) use strata_checkpoint_types::CheckpointProofTask as CheckpointTask;
use strata_identifiers::{Epoch, EpochCommitment};
use strata_ol_block_ancestry_provider::BlockAncestryProvider;
use strata_ol_canonical_chain_provider::{OLCanonicalChainProvider, OLCanonicalChainProviderImpl};
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
    canonical_chain_provider: OLCanonicalChainProviderImpl,
    block_ancestry_provider: BlockAncestryProvider,
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
            canonical_chain_provider: OLCanonicalChainProviderImpl::new(&storage),
            block_ancestry_provider: BlockAncestryProvider::new(storage.ol_block().clone()),
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
        let block_ancestry_provider = self.block_ancestry_provider.clone();
        let runtime_params = self.runtime_params;
        let assigned_spec = self.assigned_spec;
        // Recovery takes this path before resuming a saved backend request too.
        let assembled = async {
            ensure_canonical_task(&self.canonical_chain_provider, commitment)
                .await
                .map_err(PaasError::from)?;
            let input = spawn_blocking(move || {
                fetch_input_blocking(
                    storage,
                    &block_ancestry_provider,
                    commitment,
                    runtime_params,
                    assigned_spec,
                )
            })
            .await
            .map_err(|error| PaasError::Storage(format!("input fetch join: {error}")))?
            .map_err(PaasError::from)?;
            ensure_canonical_task(&self.canonical_chain_provider, commitment)
                .await
                .map_err(PaasError::from)?;
            Ok(input)
        }
        .await;
        InputResolution::from_result(assembled)
    }
}

/// Checks eligibility before routing, assembling input, or resuming a saved task.
pub(super) async fn ensure_canonical_task(
    provider: &impl OLCanonicalChainProvider,
    commitment: EpochCommitment,
) -> Result<(), ProverError> {
    let epoch = commitment.epoch();
    let canonical = provider
        .get_canonical_epoch_commitment_at_async(epoch)
        .await?
        .ok_or(ProverError::EpochCommitmentNotFound(u64::from(epoch)))?;
    if canonical != commitment {
        return Err(ProverError::StaleTaskCommitment {
            epoch: u64::from(epoch),
            task: commitment,
            canonical,
        });
    }
    Ok(())
}

/// Reads the spec used by the task's epoch from its terminal OL state.
///
/// The chain worker stores this state before publishing the epoch summary. Its
/// current spec identifies the completed epoch, so no transition rule needs to
/// be repeated here.
///
/// Performs blocking storage reads. Missing terminal state remains retryable;
/// neither ASM state nor a different OL state supplies a fallback.
pub(crate) fn checkpoint_task_spec(
    storage: &NodeStorage,
    task: CheckpointTask,
) -> Result<OLSpecId, ProverError> {
    let commitment = task.0.to_block_commitment();
    let state = storage
        .ol_state()
        .get_toplevel_ol_state_blocking(commitment)?
        .ok_or(ProverError::EpochTerminalStateNotFound { commitment })?;
    Ok(state.cur_spec())
}

fn fetch_input_blocking(
    storage: Arc<NodeStorage>,
    block_ancestry_provider: &BlockAncestryProvider,
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
    let summary_commitment = summary.get_epoch_commitment();
    if summary_commitment != task_commitment {
        return Err(ProverError::EpochCommitmentMismatch {
            expected: task_commitment,
            actual: summary_commitment,
        });
    }
    let terminal = summary.terminal();
    let prev_terminal = summary.prev_terminal();
    let terminal_header = storage
        .ol_block()
        .get_ol_header_blocking(*terminal.blkid())?
        .ok_or(ProverError::BlockNotFound(terminal.slot()))?;
    let terminal_commitment = terminal_header.compute_block_commitment();
    let header_epoch_commitment =
        EpochCommitment::from_terminal(terminal_header.epoch(), terminal_commitment);
    if header_epoch_commitment != task_commitment {
        return Err(ProverError::EpochCommitmentMismatch {
            expected: task_commitment,
            actual: header_epoch_commitment,
        });
    }
    if !terminal_header.is_terminal() {
        return Err(ProverError::MissingTerminalFlag {
            block: terminal_commitment,
        });
    }
    let ancestry =
        block_ancestry_provider.collect_epoch_block_commitments_blocking(&terminal_header)?;
    if ancestry.previous_terminal() != prev_terminal {
        return Err(ProverError::EpochPredecessorMismatch {
            expected: *prev_terminal,
            actual: *ancestry.previous_terminal(),
        });
    }

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

    let blocks = ancestry
        .blocks()
        .iter()
        .map(|commitment| {
            let block = storage
                .ol_block()
                .get_block_data_blocking(*commitment.blkid())?
                .ok_or_else(|| {
                    ProverError::StateNotFound(format!(
                        "block {commitment} missing during epoch {epoch_index} input assembly"
                    ))
                })?;
            let actual = block.header().compute_block_commitment();
            if actual != *commitment {
                return Err(ProverError::BlockCommitmentMismatch {
                    expected: *commitment,
                    actual,
                });
            }
            Ok(block)
        })
        .collect::<Result<Vec<_>, ProverError>>()?;

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
