//! Command types for OL block assembly service.

use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_ol_chain_types::OLBlock;
use strata_service::CommandCompletionSender;
use tokio::sync::oneshot;

use crate::{
    error::BlockAssemblyError,
    types::{BlockCompletionData, BlockGenerationConfig, FullBlockTemplate},
};

/// Type alias for block template generation result.
type GenerateBlockTemplateResult = Result<FullBlockTemplate, BlockAssemblyError>;

/// Type alias for block template lookup result.
type GetBlockTemplateResult = Result<FullBlockTemplate, BlockAssemblyError>;

/// Type alias for block template completion result.
type CompleteBlockTemplateResult = Result<OLBlock, BlockAssemblyError>;

/// Type alias for completed-template status release result.
type ReleaseCompletedTemplateStatusResult = bool;

/// Type alias for recording a persisted block result.
type RecordPersistedBlockResult = Result<(), BlockAssemblyError>;

#[derive(Debug)]
pub(crate) enum BlockasmCommand {
    GenerateBlockTemplate {
        config: BlockGenerationConfig,
        completion: CommandCompletionSender<GenerateBlockTemplateResult>,
    },
    GetBlockTemplate {
        parent_block_id: OLBlockId,
        completion: CommandCompletionSender<GetBlockTemplateResult>,
    },
    CompleteBlockTemplate {
        /// The ID of the cached template to complete into a block.
        template_id: OLBlockId,
        data: BlockCompletionData,
        completion: CommandCompletionSender<CompleteBlockTemplateResult>,
    },
    ReleaseCompletedTemplateStatus {
        /// Parent for the completed-template status.
        parent_block_id: OLBlockId,
        /// Block commitment stored in the completed-template status.
        block: OLBlockCommitment,
        completion: CommandCompletionSender<ReleaseCompletedTemplateStatusResult>,
    },
    RecordPersistedBlock {
        /// The ID of the template that produced the persisted block.
        template_id: OLBlockId,
        completion: CommandCompletionSender<RecordPersistedBlockResult>,
    },
}

pub(crate) fn create_completion<T>() -> (CommandCompletionSender<T>, oneshot::Receiver<T>) {
    let (tx, rx) = oneshot::channel();
    (CommandCompletionSender::new(tx), rx)
}
