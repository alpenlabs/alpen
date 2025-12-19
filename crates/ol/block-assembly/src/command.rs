//! Command types for OL block assembly service.

use strata_identifiers::OLBlockId;
use strata_ol_chain_types_new::OLBlock;
use strata_service::CommandCompletionSender;

use crate::{
    error::BlockAssemblyError,
    types::{BlockCompletionData, BlockGenerationConfig, BlockTemplate},
};

/// Type alias for block template generation result.
type GenerateBlockTemplateResult = Result<BlockTemplate, BlockAssemblyError>;

/// Type alias for block template completion result.
type CompleteBlockTemplateResult = Result<OLBlock, BlockAssemblyError>;

/// Commands that can be sent to the OL block assembly service.
#[derive(Debug)]
pub(crate) enum BlockAssemblyCommand {
    /// Generate a new block template based on the provided configuration.
    GenerateBlockTemplate {
        /// Configuration for block generation (parent block ID, timestamp).
        config: BlockGenerationConfig,
        /// Completion sender for the generated template.
        completion: CommandCompletionSender<GenerateBlockTemplateResult>,
    },

    /// Complete a block template with signature to create a final OL block.
    CompleteBlockTemplate {
        /// Template ID (block ID) to complete.
        template_id: OLBlockId,
        /// Completion data (signature).
        completion_data: BlockCompletionData,
        /// Completion sender for the completed block.
        completion: CommandCompletionSender<CompleteBlockTemplateResult>,
    },
}

/// Helper function to create a completion channel pair.
///
/// Returns (CommandCompletionSender, Receiver) for command-response pattern.
pub(crate) fn create_completion<T>() -> (
    CommandCompletionSender<T>,
    tokio::sync::oneshot::Receiver<T>,
) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    (CommandCompletionSender::new(tx), rx)
}
