//! OL block assembly service implementation.

use std::{fmt::Display, marker::PhantomData};

use ssz::Encode;
use strata_crypto::hash::raw;
use strata_identifiers::OLBlockId;
use strata_ol_chain_types::verify_sequencer_signature;
use strata_ol_chain_types_new::{OLBlock, OLBlockHeader};
use strata_ol_state_types::StateProvider;
use strata_params::RollupParams;
use strata_service::{AsyncService, Response, Service};

use crate::{
    BlockAssemblyStateAccess, EpochSealingPolicy, FullBlockTemplate, MempoolProvider,
    block_assembly::generate_block_template_inner,
    command::BlockasmCommand,
    error::BlockAssemblyError,
    state::BlockasmServiceState,
    types::{BlockCompletionData, BlockGenerationConfig, BlockTemplate},
};

/// OL block assembly service that processes commands.
#[derive(Debug)]
pub(crate) struct BlockasmService<M: MempoolProvider, E: EpochSealingPolicy, S> {
    _phantom: PhantomData<(M, E, S)>,
}

impl<M, E, S> Service for BlockasmService<M, E, S>
where
    M: MempoolProvider,
    E: EpochSealingPolicy,
    S: StateProvider + Send + Sync + 'static,
    S::Error: Display,
    S::State: BlockAssemblyStateAccess,
{
    type State = BlockasmServiceState<M, E, S>;
    type Msg = BlockasmCommand;
    type Status = BlockasmServiceStatus;

    fn get_status(_state: &Self::State) -> Self::Status {
        BlockasmServiceStatus
    }
}

impl<M, E, S> AsyncService for BlockasmService<M, E, S>
where
    M: MempoolProvider,
    E: EpochSealingPolicy,
    S: StateProvider + Send + Sync + 'static,
    S::Error: Display,
    S::State: BlockAssemblyStateAccess,
{
    async fn on_launch(_state: &mut Self::State) -> anyhow::Result<()> {
        Ok(())
    }

    async fn process_input(state: &mut Self::State, input: &Self::Msg) -> anyhow::Result<Response> {
        match input {
            BlockasmCommand::GenerateBlockTemplate { config, completion } => {
                let result = generate_block_template(state, config.clone()).await;
                _ = completion.send(result).await;
            }

            BlockasmCommand::CompleteBlockTemplate {
                template_id,
                data,
                completion,
            } => {
                let result = complete_block_template(state, *template_id, data.clone());
                _ = completion.send(result).await;
            }
        }

        Ok(Response::Continue)
    }
}

/// Generate a new block template.
async fn generate_block_template<
    M: MempoolProvider,
    E: EpochSealingPolicy,
    S: StateProvider + Send + Sync + 'static,
>(
    state: &mut BlockasmServiceState<M, E, S>,
    config: BlockGenerationConfig,
) -> Result<FullBlockTemplate, BlockAssemblyError>
where
    S::Error: Display,
    S::State: BlockAssemblyStateAccess,
{
    // Check if we already have a pending template for this parent block ID
    if let Ok(template) = state
        .state_mut()
        .get_pending_block_template_by_parent(config.parent_block_id())
    {
        return Ok(template);
    }

    // Generate new template (stub for now - will be implemented in block_assembly.rs)
    let result = generate_block_template_inner(
        state.context(),
        state.epoch_sealing_policy(),
        state.sequencer_config(),
        config,
    )
    .await?;

    let (full_template, failed_txs) = result.into_parts();

    // Report failed transactions back to mempool
    if !failed_txs.is_empty() {
        MempoolProvider::report_invalid_transactions(state.context(), &failed_txs).await?;
    }

    let template_id = full_template.get_blockid();

    state
        .state_mut()
        .insert_template(template_id, full_template.clone());

    Ok(full_template)
}

/// Complete a block template with signature.
///
/// The signature is provided by the caller (sequencer) via `BlockCompletionData`. The flow is:
/// 1. Sequencer calls `GenerateBlockTemplate` to get a template with header hash
/// 2. Sequencer signs the header hash externally (e.g., via signing service)
/// 3. Sequencer calls `CompleteBlockTemplate` with the signature
/// 4. This function validates the signature before completing the block
///
/// The completed block is returned to the caller, who is responsible for submitting it
/// to the Fork Choice Manager (FCM) and storage.
fn complete_block_template<M: MempoolProvider, E: EpochSealingPolicy, S>(
    state: &mut BlockasmServiceState<M, E, S>,
    template_id: OLBlockId,
    completion_data: BlockCompletionData,
) -> Result<OLBlock, BlockAssemblyError> {
    // Get template to verify signature before removing it
    let template_ref = state.state_mut().get_pending_block_template(template_id)?;

    // Verify signature first (before removing from cache)
    if !check_completion_data(
        state.rollup_params(),
        template_ref.header(),
        &completion_data,
    ) {
        return Err(BlockAssemblyError::InvalidSignature(template_id));
    }

    // Signature valid - now remove template from cache
    let template = state.state_mut().remove_template(template_id)?;

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
pub(crate) struct BlockasmServiceStatus;
