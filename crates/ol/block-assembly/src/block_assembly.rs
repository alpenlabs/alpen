//! Block assembly logic (stub implementation).
use strata_config::SequencerConfig;
use strata_db_types::errors::DbError;

use crate::{
    AccumulatorProofGenerator, BlockAssemblyResult, EpochSealingPolicy, MempoolProvider,
    context::BlockAssemblyAnchorContext,
    error::BlockAssemblyError,
    types::{BlockGenerationConfig, BlockTemplateResult},
};

/// Generate a block template (stub implementation).
///
/// This is a placeholder that will be fully implemented in the next commit.
/// For now, it returns an error to indicate that the implementation is pending.
///
/// Returns a [`BlockTemplateResult`] containing both the generated template and
/// any transactions that failed validation during assembly.
pub(crate) async fn generate_block_template_inner<C, E>(
    _ctx: &C,
    _epoch_sealing_policy: &E,
    _sequencer_config: &SequencerConfig,
    _block_generation_config: BlockGenerationConfig,
) -> BlockAssemblyResult<BlockTemplateResult>
where
    C: BlockAssemblyAnchorContext + AccumulatorProofGenerator + MempoolProvider,
    E: EpochSealingPolicy,
{
    Err(BlockAssemblyError::Database(DbError::Other(
        "Block assembly implementation pending".to_string(),
    )))
}
