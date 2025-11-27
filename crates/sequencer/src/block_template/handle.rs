use std::sync::Arc;

use strata_consensus_logic::{message::ForkChoiceMessage, sync_manager::SyncManager};
use strata_ol_chain_types::{L2BlockBundle, L2BlockId, L2Header};
use strata_storage::{L2BlockManager, SequencerPayloadManager};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use super::{BlockCompletionData, BlockGenerationConfig, BlockTemplate, Error, SharedState};

/// Request to be sent from [`TemplateManagerHandle`] to the worker task.
///
/// Each passes a [`oneshot::Sender`] to return the value.
#[derive(Debug)]
pub enum TemplateManagerRequest {
    /// Build and return a new block template signable by sequencer.
    GenerateBlockTemplate(
        BlockGenerationConfig,
        oneshot::Sender<Result<BlockTemplate, Error>>,
    ),

    /// Provide [`BlockCompletionData`] for an existing template to create
    /// a complete [`L2BlockBundle`]
    CompleteBlockTemplate(
        L2BlockId,
        BlockCompletionData,
        oneshot::Sender<Result<L2BlockBundle, Error>>,
    ),
}

/// Handle for communication with the template manager worker.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct TemplateManagerHandle {
    tx: mpsc::Sender<TemplateManagerRequest>,
    shared: SharedState,
    l2_block_manager: Arc<L2BlockManager>,
    sync_manager: Arc<SyncManager>,

    /// Sequencer payload manager for persisting exec payloads.
    /// Used for EE consistency recovery on startup.
    sequencer_payload_manager: Arc<SequencerPayloadManager>,
}

impl TemplateManagerHandle {
    /// Create new instance.
    // TODO make this not pub
    pub fn new(
        tx: mpsc::Sender<TemplateManagerRequest>,
        shared: SharedState,
        l2_block_manager: Arc<L2BlockManager>,
        sync_manager: Arc<SyncManager>,
        sequencer_payload_manager: Arc<SequencerPayloadManager>,
    ) -> Self {
        Self {
            tx,
            shared,
            l2_block_manager,
            sync_manager,
            sequencer_payload_manager,
        }
    }

    async fn request<R>(
        &self,
        build_request: impl FnOnce(oneshot::Sender<Result<R, Error>>) -> TemplateManagerRequest,
    ) -> Result<R, Error> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(build_request(tx))
            .await
            .map_err(|_| Error::RequestChannelClosed)?;

        match rx.await {
            Ok(res) => res,
            // oneshot tx is dropped
            Err(_) => Err(Error::ResponseChannelClosed),
        }
    }

    /// Generate a new block template based on provided [`BlockGenerationConfig`].
    /// Will return cached template for request if it exists.
    pub async fn generate_block_template(
        &self,
        config: BlockGenerationConfig,
    ) -> Result<BlockTemplate, Error> {
        // check if pending template exists
        if let Ok(template) = self
            .shared
            .read()
            .await
            .get_pending_block_template_by_parent(config.parent_block_id())
        {
            return Ok(template);
        }

        self.request(|tx| TemplateManagerRequest::GenerateBlockTemplate(config.clone(), tx))
            .await
    }

    /// Complete specified template with [`BlockCompletionData`] and submit to FCM.
    ///
    /// This also persists the exec payload to the sequencer database for EE consistency
    /// recovery in case reth loses blocks on restart.
    pub async fn complete_block_template(
        &self,
        template_id: L2BlockId,
        completion: BlockCompletionData,
    ) -> Result<L2BlockId, Error> {
        let block_bundle = self
            .request(|tx| {
                TemplateManagerRequest::CompleteBlockTemplate(template_id, completion, tx)
            })
            .await?;

        // Persist exec payload to sequencer db for EE consistency recovery.
        // This is done before saving to L2BlockManager to ensure the payload is persisted
        // even if subsequent operations fail.
        let slot = block_bundle.header().slot();
        let exec_payload = block_bundle.accessory().exec_payload().to_vec();
        if let Err(e) = self
            .sequencer_payload_manager
            .put_exec_payload_async(slot, template_id, exec_payload)
            .await
        {
            // Log the error but don't fail - the block can still be processed
            // and the payload can be recovered from the L2BlockBundle if needed
            warn!(
                %slot,
                %template_id,
                error = %e,
                "failed to persist exec payload to sequencer db"
            );
        }

        // save block to db
        self.l2_block_manager
            .put_block_data_async(block_bundle)
            .await?;

        // send blockid to fcm
        if !self
            .sync_manager
            .submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(template_id))
            .await
        {
            return Err(Error::FcmChannelClosed);
        }

        Ok(template_id)
    }

    /// Get a pending block template from cache if it exists.
    pub async fn get_block_template(&self, template_id: L2BlockId) -> Result<BlockTemplate, Error> {
        self.shared
            .read()
            .await
            .get_pending_block_template(template_id)
    }
}
