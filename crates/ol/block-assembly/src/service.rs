//! OL block assembly service implementation.

use ssz::Encode;
use strata_identifiers::{OLBlockId, hash::raw};
use strata_ol_chain_types::verify_sequencer_signature;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_params::RollupParams;
use strata_service::{AsyncService, Response, Service};

use crate::{
    block_assembly::generate_block_template_inner,
    command::BlockAssemblyCommand,
    error::BlockAssemblyError,
    state::BlockAssemblyServiceState,
    types::{BlockCompletionData, BlockGenerationConfig, BlockTemplate},
};

/// OL block assembly service that processes commands.
#[derive(Debug)]
pub(crate) struct BlockAssemblyService;

impl Service for BlockAssemblyService {
    type State = BlockAssemblyServiceState;
    type Msg = BlockAssemblyCommand;
    type Status = BlockAssemblyServiceStatus;

    fn get_status(_state: &Self::State) -> Self::Status {
        BlockAssemblyServiceStatus
    }
}

impl AsyncService for BlockAssemblyService {
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        match input {
            BlockAssemblyCommand::GenerateBlockTemplate { config, completion } => {
                let result = generate_block_template(state, config.clone());
                completion.send(result).await;
            }

            BlockAssemblyCommand::CompleteBlockTemplate {
                template_id,
                completion_data,
                completion,
            } => {
                let result = complete_block_template(state, *template_id, completion_data.clone());
                completion.send(result).await;
            }
        }

        Ok(Response::Continue)
    }
}

/// Generate a new block template.
fn generate_block_template(
    state: &mut BlockAssemblyServiceState,
    config: BlockGenerationConfig,
) -> Result<BlockTemplate, BlockAssemblyError> {
    // Check if we already have a pending template for this parent block ID
    if let Ok(template) = state
        .state_mut()
        .get_pending_block_template_by_parent(config.parent_block_id())
    {
        return Ok(template);
    }

    // Generate new template (stub for now - will be implemented in block_assembly.rs)
    let full_template = generate_block_template_inner(config, state.context())?;

    let template = BlockTemplate::from_full_ref(&full_template);
    let template_id = full_template.get_blockid();

    state
        .state_mut()
        .insert_template(template_id, full_template);

    Ok(template)
}

/// Complete a block template with signature.
fn complete_block_template(
    state: &mut BlockAssemblyServiceState,
    template_id: OLBlockId,
    completion_data: BlockCompletionData,
) -> Result<OLBlock, BlockAssemblyError> {
    let template = state.state_mut().remove_template(template_id)?;

    // Verify signature
    if !check_completion_data(
        state.context().params().rollup(),
        template.header(),
        &completion_data,
    ) {
        return Err(BlockAssemblyError::InvalidSignature(template_id));
    }

    // Complete the template
    Ok(template.complete_block_template(completion_data))
}

/// Check if completion data (signature) is valid.
fn check_completion_data(
    rollup_params: &RollupParams,
    header: &OLBlockHeader,
    completion: &BlockCompletionData,
) -> bool {
    // Compute sighash from header (SSZ encoding)
    let encoded = header.as_ssz_bytes();
    let sighash = raw(&encoded);

    // Verify sequencer signature
    verify_sequencer_signature(rollup_params, &sighash, completion.signature())
}

/// Service status for OL block assembly.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct BlockAssemblyServiceStatus;
