//! Concrete [`SequencerContext`] implementation for the Strata node.

use std::sync::Arc;

use async_trait::async_trait;
use strata_btcio::writer::EnvelopeHandle;
use strata_consensus_logic::{FcmServiceHandle, message::ForkChoiceMessage};
use strata_csm_types::PayloadIntent;
use strata_db_types::types::OLCheckpointEntry;
use strata_identifiers::Epoch;
use strata_ol_block_assembly::{BlockAssemblyError, BlockasmHandle};
use strata_ol_chain_types_new::OLBlock;
use strata_ol_sequencer::{
    BlockCompletionData, BlockGenerationConfig, Duty, SequencerContext, SequencerContextError,
    extract_duties,
};
use strata_primitives::{OLBlockCommitment, OLBlockId};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tracing::{debug, warn};

/// Node-level context providing concrete infrastructure for the sequencer service.
pub(crate) struct NodeSequencerContext {
    blockasm_handle: Arc<BlockasmHandle>,
    envelope_handle: Arc<EnvelopeHandle>,
    storage: Arc<NodeStorage>,
    fcm_handle: Arc<FcmServiceHandle>,
    status_channel: Arc<StatusChannel>,
    ol_block_time_ms: u64,
}

impl NodeSequencerContext {
    pub(crate) fn new(
        blockasm_handle: Arc<BlockasmHandle>,
        envelope_handle: Arc<EnvelopeHandle>,
        storage: Arc<NodeStorage>,
        fcm_handle: Arc<FcmServiceHandle>,
        status_channel: Arc<StatusChannel>,
        ol_block_time_ms: u64,
    ) -> Self {
        Self {
            blockasm_handle,
            envelope_handle,
            storage,
            fcm_handle,
            status_channel,
            ol_block_time_ms,
        }
    }

    /// Resolve the current chain tip block ID.
    async fn resolve_tip(&self) -> Result<OLBlockId, SequencerContextError> {
        if let Some(tip) = self
            .status_channel
            .get_ol_sync_status()
            .map(|s| *s.tip_blkid())
        {
            return Ok(tip);
        }

        match self.storage.ol_block().get_canonical_tip_async().await {
            Ok(Some(commitment)) => Ok(*commitment.blkid()),
            Ok(None) => {
                warn!("canonical tip not found yet");
                Ok(OLBlockId::default())
            }
            Err(e) => Err(SequencerContextError::Db(e)),
        }
    }
}

#[async_trait]
impl SequencerContext for NodeSequencerContext {
    async fn poll_duties(&self) -> Result<Vec<Duty>, SequencerContextError> {
        let tip_blkid = self.resolve_tip().await?;
        if tip_blkid == OLBlockId::default() {
            return Ok(vec![]);
        }

        extract_duties(
            self.blockasm_handle.as_ref(),
            tip_blkid,
            self.storage.as_ref(),
        )
        .await
        .map_err(|source| SequencerContextError::DutyExtraction { tip_blkid, source })
    }

    async fn generate_template_for_tip(&self) -> Result<Option<OLBlockId>, SequencerContextError> {
        let tip_blkid = self.resolve_tip().await?;
        if tip_blkid == OLBlockId::default() {
            debug!("template generation skipped: canonical tip unavailable");
            return Ok(None);
        }

        debug!(tip_blkid = ?tip_blkid, "template generation attempt");

        let parent_block = self
            .storage
            .ol_block()
            .get_block_data_async(tip_blkid)
            .await
            .map_err(SequencerContextError::Db)?
            .ok_or_else(|| SequencerContextError::TemplateGeneration {
                tip_blkid,
                source: BlockAssemblyError::Other(format!("parent block {tip_blkid} not found")),
            })?;
        let parent_header = parent_block.header();

        let parent_commitment = OLBlockCommitment::new(parent_header.slot(), tip_blkid);
        let target_ts = parent_header
            .timestamp()
            .saturating_add(self.ol_block_time_ms);
        let config = BlockGenerationConfig::new(parent_commitment).with_ts(target_ts);

        debug!(
            tip_blkid = ?tip_blkid,
            parent_slot = parent_header.slot(),
            parent_ts = parent_header.timestamp(),
            target_ts,
            "submitting template generation request"
        );

        self.blockasm_handle
            .generate_block_template(config)
            .await
            .map_err(|source| SequencerContextError::TemplateGeneration { tip_blkid, source })?;

        // Block assembly decides whether this was a cache hit or a new template generation.
        debug!(tip_blkid = ?tip_blkid, "template generation request completed");

        Ok(Some(tip_blkid))
    }

    async fn complete_block_template(
        &self,
        template_id: OLBlockId,
        completion: BlockCompletionData,
    ) -> Result<OLBlock, SequencerContextError> {
        self.blockasm_handle
            .complete_block_template(template_id, completion)
            .await
            .map_err(|source| SequencerContextError::TemplateCompletion {
                template_id,
                source,
            })
    }

    async fn store_block(&self, block: OLBlock) -> Result<(), SequencerContextError> {
        self.storage
            .ol_block()
            .put_block_data_async(block)
            .await
            .map_err(SequencerContextError::Db)
    }

    async fn submit_chain_tip(&self, blkid: OLBlockId) -> Result<(), SequencerContextError> {
        let submitted = self
            .fcm_handle
            .submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(blkid))
            .await;
        if !submitted {
            return Err(SequencerContextError::FcmChannelClosed { blkid });
        }
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        epoch: Epoch,
    ) -> Result<Option<OLCheckpointEntry>, SequencerContextError> {
        self.storage
            .ol_checkpoint()
            .get_checkpoint_async(epoch)
            .await
            .map_err(SequencerContextError::Db)
    }

    async fn submit_checkpoint_intent(
        &self,
        intent: PayloadIntent,
    ) -> Result<Option<u64>, SequencerContextError> {
        self.envelope_handle
            .submit_intent_async_with_idx(intent)
            .await
            .map_err(|source| SequencerContextError::CheckpointIntentSubmission { source })
    }

    async fn persist_checkpoint(
        &self,
        epoch: Epoch,
        entry: OLCheckpointEntry,
    ) -> Result<(), SequencerContextError> {
        self.storage
            .ol_checkpoint()
            .put_checkpoint_async(epoch, entry)
            .await
            .map_err(SequencerContextError::Db)
    }
}
