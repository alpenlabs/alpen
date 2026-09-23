//! Block links: verified by executing the block against the pre-state.

use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::WriteTrackingState;
use strata_ol_stf_v1::verify_block;

use super::artifact::OLExecOutput;
use crate::graph_types::OLBlockLink;
use crate::providers::OLStateStore;
use crate::step::{PreState, StepError, exec_step_error};

pub(super) fn process_block_link<S: OLStateStore>(
    runtime_params: &OLRuntimeParams,
    pre_state: &PreState<'_, '_, S>,
    link: &OLBlockLink,
) -> Result<OLExecOutput, StepError> {
    let mut state = WriteTrackingState::new_empty(pre_state);
    let logs = verify_block(
        &mut state,
        link.header(),
        link.parent_header(),
        link.block().body(),
        runtime_params,
    )
    .map_err(exec_step_error)?;

    Ok(OLExecOutput::new(
        link.header().clone(),
        state.into_batch(),
        logs,
    ))
}
