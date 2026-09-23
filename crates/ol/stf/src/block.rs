//! Whole-block and block-batch operations.

use strata_ol_chain_types_v1::{OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLLog};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_types::{IStateAccessorMut, OLSpecId};
use strata_ol_stf_v1::{
    BlockComponents, BlockContext, CompletedBlock, ConstructBlockOutput, ExecResult,
};

use crate::spec::{OLStfSpec, dispatch_spec};
use crate::v1::StfV1;

/// Verifies a block end to end under `spec`, returning the block's logs.
///
/// See [`strata_ol_stf_v1::verify_block`] for the V1 checks.
pub fn verify_block<S: IStateAccessorMut>(
    spec: OLSpecId,
    state: &mut S,
    header: &OLBlockHeaderV1,
    parent_header: Option<&OLBlockHeaderV1>,
    body: &OLBlockBodyV1,
    runtime_params: &OLRuntimeParams,
) -> ExecResult<Vec<OLLog>> {
    dispatch_spec!(spec => verify_block(state, header, parent_header, body, runtime_params))
}

/// Executes block components under `spec` and completes the block header.
///
/// See [`strata_ol_stf_v1::construct_block`].
pub fn construct_block<S: IStateAccessorMut>(
    spec: OLSpecId,
    state: &mut S,
    block_context: BlockContext<'_>,
    block_components: BlockComponents,
    runtime_params: &OLRuntimeParams,
) -> ExecResult<ConstructBlockOutput> {
    dispatch_spec!(spec => construct_block(state, block_context, block_components, runtime_params))
}

/// Executes block components under `spec` and returns only the completed block.
///
/// See [`strata_ol_stf_v1::execute_and_complete_block`].
pub fn execute_and_complete_block<S: IStateAccessorMut>(
    spec: OLSpecId,
    state: &mut S,
    block_context: BlockContext<'_>,
    block_components: BlockComponents,
    runtime_params: &OLRuntimeParams,
) -> ExecResult<CompletedBlock> {
    dispatch_spec!(spec => execute_and_complete_block(
        state,
        block_context,
        block_components,
        runtime_params,
    ))
}

/// Verifies a batch of blocks under `spec` without the epoch-terminal drain,
/// returning the concatenated pre-drain logs.
///
/// DA rebuild paths use this to accumulate an epoch diff that excludes the
/// terminal ASM effects. See [`strata_ol_stf_v1::execute_block_batch_predrain`].
pub fn execute_block_batch_predrain<S: IStateAccessorMut>(
    spec: OLSpecId,
    state: &mut S,
    blocks: &[OLBlockV1],
    initial_parent: &OLBlockHeaderV1,
    runtime_params: &OLRuntimeParams,
) -> ExecResult<Vec<OLLog>> {
    dispatch_spec!(spec => execute_block_batch_predrain(
        state,
        blocks,
        initial_parent,
        runtime_params,
    ))
}
