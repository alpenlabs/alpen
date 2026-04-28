//! Concrete [`SequencerContext`] implementation for the Strata node.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use strata_identifiers::OLBlockId;
use strata_ol_block_assembly::{BlockAssemblyError, BlockasmHandle};
use strata_ol_sequencer::{BlockGenerationConfig, SequencerContext, SequencerContextError};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;
use tracing::{debug, warn};

use crate::sequencer::tip::resolve_canonical_tip;

/// Percentage drift of block wall-clock spacing above the configured
/// `ol_block_time_ms` that triggers a cadence warning.
const BLOCK_TS_DRIFT_TOLERANCE_PCT: u64 = 20;

/// Node-level context providing concrete infrastructure for the sequencer service.
pub(crate) struct NodeSequencerContext {
    blockasm_handle: Arc<BlockasmHandle>,
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
    ol_block_time_ms: u64,
}

impl NodeSequencerContext {
    pub(crate) fn new(
        blockasm_handle: Arc<BlockasmHandle>,
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
        ol_block_time_ms: u64,
    ) -> Self {
        Self {
            blockasm_handle,
            storage,
            status_channel,
            ol_block_time_ms,
        }
    }
}

#[async_trait]
impl SequencerContext for NodeSequencerContext {
    async fn generate_template_for_tip(&self) -> Result<Option<OLBlockId>, SequencerContextError> {
        let Some(parent_commitment) = resolve_canonical_tip(&self.status_channel, &self.storage)
            .await
            .map_err(SequencerContextError::Db)?
        else {
            debug!("template generation skipped: canonical tip unavailable");
            return Ok(None);
        };
        let tip_blkid = *parent_commitment.blkid();

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

        let parent_ts = parent_header.timestamp();
        let target_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_millis() as u64;

        let time_since_parent = target_ts.saturating_sub(parent_ts);
        let threshold_ms = self.ol_block_time_ms * (100 + BLOCK_TS_DRIFT_TOLERANCE_PCT) / 100;
        if time_since_parent > threshold_ms {
            warn!(
                time_since_parent,
                block_time_ms = self.ol_block_time_ms,
                threshold_ms,
                parent_ts,
                target_ts,
                "block wall-clock interval exceeds block_time by more than {BLOCK_TS_DRIFT_TOLERANCE_PCT}%",
            );
        }

        let config = BlockGenerationConfig::new(parent_commitment).with_ts(target_ts);

        debug!(
            tip_blkid = ?tip_blkid,
            parent_slot = parent_header.slot(),
            parent_ts,
            target_ts,
            "submitting template generation request"
        );

        self.blockasm_handle
            .generate_block_template(config)
            .await
            .map_err(|source| SequencerContextError::TemplateGeneration { tip_blkid, source })?;

        debug!(tip_blkid = ?tip_blkid, "template generation request completed");

        Ok(Some(tip_blkid))
    }
}
