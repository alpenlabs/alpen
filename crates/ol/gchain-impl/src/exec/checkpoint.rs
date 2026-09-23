//! Checkpoint links: reconstructed from the DA payload observed on L1.

use strata_checkpoint_types::reconstruct_terminal_header;
use strata_ol_da_types_v1::OLDaSchemeV1;
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::WriteTrackingState;
use strata_ol_state_types::IStateAccessor;
use strata_ol_stf_v1::apply_da_epoch;

use super::artifact::OLExecOutput;
use crate::errors::InvalidLinkError;
use crate::graph_types::OLCheckpointLink;
use crate::providers::{L1ManifestProvider, OLStateStore};
use crate::step::{PreState, StepError, assemble_checkpoint_inputs, exec_step_error};

pub(super) fn process_checkpoint_link<S: OLStateStore>(
    runtime_params: &OLRuntimeParams,
    manifest_provider: &impl L1ManifestProvider,
    pre_state: &PreState<'_, '_, S>,
    link: &OLCheckpointLink,
) -> Result<OLExecOutput, StepError> {
    let inputs = assemble_checkpoint_inputs(manifest_provider, pre_state, link)?;

    let mut state = WriteTrackingState::new_empty(pre_state);
    apply_da_epoch::<_, OLDaSchemeV1>(
        &mut state,
        &inputs.epoch_info,
        inputs.da_payload,
        &inputs.manifests,
        runtime_params,
    )
    .map_err(exec_step_error)?;

    // The reconstructed header is what binds the reconstructed state to the
    // checkpoint's terminal block: if the state root is wrong, the header
    // hashes to a different block ID than the tip names.
    let state_root = state.compute_state_root()?;
    let sidecar = link.payload().sidecar();
    let header = reconstruct_terminal_header(
        link.payload().new_tip(),
        sidecar.terminal_header_complement(),
        state_root,
    )
    .map_err(InvalidLinkError::TerminalHeader)?;

    Ok(OLExecOutput::new(
        header,
        state.into_batch(),
        sidecar.ol_logs().to_vec(),
    ))
}
