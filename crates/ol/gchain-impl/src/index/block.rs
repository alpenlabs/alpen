//! Block links: executed again through the indexer layer.

use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::{IndexerState, IndexerWrites, WriteTrackingState};
use strata_ol_stf_v1::verify_block;

use crate::graph_types::OLBlockLink;
use crate::providers::OLStateStore;
use crate::step::{PreState, StepError, exec_step_error};

pub(super) fn index_block_link<S: OLStateStore>(
    runtime_params: &OLRuntimeParams,
    pre_state: &PreState<'_, '_, S>,
    link: &OLBlockLink,
) -> Result<IndexerWrites, StepError> {
    let mut state = IndexerState::new(WriteTrackingState::new_empty(pre_state));
    verify_block(
        &mut state,
        link.header(),
        link.parent_header(),
        link.block().body(),
        runtime_params,
    )
    .map_err(exec_step_error)?;

    let (_, writes) = state.into_parts();
    Ok(writes)
}
