//! Command types for OL block assembly service.

use strata_identifiers::OLBlockId;
use strata_ol_chain_types_new::OLBlock;
use strata_service::CommandCompletionSender;
use tokio::sync::oneshot;

use crate::{
    error::BlockAssemblyError,
    types::{BlockCompletionData, BlockGenerationConfig, BlockTemplate},
};

/// Type alias for block template generation result.
type GenerateBlockTemplateResult = Result<BlockTemplate, BlockAssemblyError>;

/// Type alias for block template completion result.
type CompleteBlockTemplateResult = Result<OLBlock, BlockAssemblyError>;

#[derive(Debug)]
pub(crate) enum BlockasmCommand {
    GenerateBlockTemplate {
        config: BlockGenerationConfig,
        completion: CommandCompletionSender<GenerateBlockTemplateResult>,
    },
    CompleteBlockTemplate {
        template_id: OLBlockId,
        data: BlockCompletionData,
        completion: CommandCompletionSender<CompleteBlockTemplateResult>,
    },
}

pub(crate) fn create_completion<T>() -> (CommandCompletionSender<T>, oneshot::Receiver<T>) {
    let (tx, rx) = oneshot::channel();
    (CommandCompletionSender::new(tx), rx)
}
