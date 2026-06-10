//! Concrete [`SequencerContext`] implementation for the Strata node.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use strata_db_types::traits::BlockStatus;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
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
        let target_slot = parent_commitment.slot().saturating_add(1);

        let high_watermark = self
            .storage
            .ol_block()
            .get_block_high_watermark_async()
            .await
            .map_err(SequencerContextError::Db)?;
        if target_slot_at_or_below_high_watermark(target_slot, high_watermark.as_ref()) {
            let high_watermark = high_watermark
                .expect("target slot can only be below high-watermark when high-watermark exists");
            debug!(
                tip_blkid = ?tip_blkid,
                target_slot,
                high_watermark = %high_watermark,
                "template generation skipped: target slot is at or below block high-watermark"
            );
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

        match self
            .blockasm_handle
            .generate_block_template(config.clone())
            .await
        {
            Ok(_) => {}
            Err(BlockAssemblyError::TemplateAlreadyCompletedForParent { parent, block })
                if parent == tip_blkid =>
            {
                let status = self
                    .storage
                    .ol_block()
                    .get_block_status_async(*block.blkid())
                    .await
                    .map_err(SequencerContextError::Db)?;

                if should_release_completed_tombstone(status) {
                    let released = self
                        .blockasm_handle
                        .release_completed_template_status(parent, block)
                        .await
                        .map_err(|source| SequencerContextError::TemplateGeneration {
                            tip_blkid,
                            source,
                        })?;

                    if released {
                        debug!(
                            tip_blkid = ?tip_blkid,
                            completed_block = %block,
                            "released invalid completed-template tombstone; retrying generation"
                        );
                        self.blockasm_handle
                            .generate_block_template(config)
                            .await
                            .map_err(|source| SequencerContextError::TemplateGeneration {
                                tip_blkid,
                                source,
                            })?;
                    } else {
                        debug!(
                            tip_blkid = ?tip_blkid,
                            completed_block = %block,
                            "template generation skipped: completed-template tombstone was already changed"
                        );
                        return Ok(None);
                    }
                } else {
                    debug!(
                        tip_blkid = ?tip_blkid,
                        completed_block = %block,
                        ?status,
                        "template generation skipped: parent already completed"
                    );
                    return Ok(None);
                }
            }
            Err(BlockAssemblyError::TemplateAlreadyCompletedForParent { parent, block }) => {
                debug!(tip_blkid = ?tip_blkid, completed_parent = ?parent, completed_block = %block, "template generation skipped: parent already completed");
                return Ok(None);
            }
            Err(source) => {
                return Err(SequencerContextError::TemplateGeneration { tip_blkid, source });
            }
        }

        debug!(tip_blkid = ?tip_blkid, "template generation request completed");

        Ok(Some(tip_blkid))
    }
}

fn target_slot_at_or_below_high_watermark(
    target_slot: u64,
    high_watermark: Option<&OLBlockCommitment>,
) -> bool {
    high_watermark.is_some_and(|high_watermark| target_slot <= high_watermark.slot())
}

fn should_release_completed_tombstone(status: Option<BlockStatus>) -> bool {
    matches!(status, Some(BlockStatus::Invalid))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use strata_config::{BlockAssemblyConfig, SequencerConfig};
    use strata_csm_types::{ClientState, L1Status};
    use strata_identifiers::{Buf32, Buf64, L1BlockCommitment, L1BlockId};
    use strata_ol_block_assembly::{
        BlockCompletionData, BlockasmBuilder, FixedSlotSealing, LimitAwareSealing,
        test_utils::{MockMempoolProvider, TestStorageFixtureBuilder},
    };
    use strata_ol_params::OLParams;
    use strata_ol_state_provider::OLStateManagerProviderImpl;
    use strata_predicate::PredicateKey;
    use strata_storage::NodeStorage;
    use strata_tasks::TaskManager;
    use tokio::runtime::Handle;

    use super::*;

    fn high_watermark(slot: u64) -> OLBlockCommitment {
        OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from([slot as u8; 32])))
    }

    async fn start_test_blockasm(storage: Arc<NodeStorage>) -> (TaskManager, Arc<BlockasmHandle>) {
        let task_manager = TaskManager::new(Handle::current());
        let executor = task_manager.create_executor();
        let state_provider = OLStateManagerProviderImpl::new(storage.ol_state().clone());
        let blockasm = BlockasmBuilder::new(
            Arc::new(OLParams::new_empty(L1BlockCommitment::default())),
            Arc::new(BlockAssemblyConfig::new(Duration::from_millis(1_000))),
            storage,
            Arc::new(MockMempoolProvider::new()),
            LimitAwareSealing::new(FixedSlotSealing::new(10)),
            state_provider,
            SequencerConfig::default(),
            PredicateKey::always_accept(),
            0,
        )
        .launch(&executor)
        .await
        .expect("test: launch block assembly service");

        (task_manager, Arc::new(blockasm))
    }

    fn test_status_channel() -> Arc<StatusChannel> {
        let l1_block = L1BlockCommitment::new(0, L1BlockId::from(Buf32::zero()));
        Arc::new(StatusChannel::new(
            ClientState::default(),
            l1_block,
            L1Status::default(),
            None,
            None,
        ))
    }

    #[test]
    fn target_slot_filter_respects_high_watermark() {
        assert!(!target_slot_at_or_below_high_watermark(5, None));

        let high_watermark = high_watermark(5);
        assert!(target_slot_at_or_below_high_watermark(
            4,
            Some(&high_watermark)
        ));
        assert!(target_slot_at_or_below_high_watermark(
            5,
            Some(&high_watermark)
        ));
        assert!(!target_slot_at_or_below_high_watermark(
            6,
            Some(&high_watermark)
        ));
    }

    #[test]
    fn completed_tombstone_release_requires_invalid_status() {
        assert!(!should_release_completed_tombstone(None));
        assert!(!should_release_completed_tombstone(Some(
            BlockStatus::Unchecked
        )));
        assert!(!should_release_completed_tombstone(Some(
            BlockStatus::Valid
        )));
        assert!(should_release_completed_tombstone(Some(
            BlockStatus::Invalid
        )));
    }

    #[tokio::test]
    async fn generation_releases_invalid_completed_tombstone_and_retries() {
        let (fixture, parent_commitment) = TestStorageFixtureBuilder::new()
            .with_genesis_parent_and_l1_manifest_count(0)
            .build_fixture()
            .await;
        let storage = fixture.storage().clone();
        storage
            .ol_block()
            .set_block_status_async(*parent_commitment.blkid(), BlockStatus::Valid)
            .await
            .expect("test: mark parent block valid");
        let (_task_manager, blockasm_handle) = start_test_blockasm(storage.clone()).await;

        let stale_template = blockasm_handle
            .generate_block_template(
                BlockGenerationConfig::new(parent_commitment).with_ts(1_000_000),
            )
            .await
            .expect("test: generate stale template");
        let stale_template_id = stale_template.get_blockid();

        let stale_block = blockasm_handle
            .complete_block_template(
                stale_template_id,
                BlockCompletionData::from_signature(Buf64::zero()),
            )
            .await
            .expect("test: complete stale template");
        let stale_block_commitment = storage
            .ol_block()
            .put_block_data_with_high_watermark_async(stale_block)
            .await
            .expect("test: persist stale block");
        blockasm_handle
            .record_persisted_block(stale_template_id)
            .await
            .expect("test: record stale block");
        storage
            .ol_block()
            .set_block_status_async(stale_template_id, BlockStatus::Invalid)
            .await
            .expect("test: mark stale block invalid");
        storage
            .ol_block()
            .clear_block_high_watermark_async(stale_block_commitment)
            .await
            .expect("test: clear stale block high-watermark");

        let sequencer_context = NodeSequencerContext::new(
            blockasm_handle.clone(),
            storage,
            test_status_channel(),
            SequencerConfig::default().ol_block_time_ms,
        );

        let generated_parent = sequencer_context
            .generate_template_for_tip()
            .await
            .expect("test: retry generation after invalid completed tombstone");
        assert_eq!(generated_parent, Some(*parent_commitment.blkid()));

        let replacement_template = blockasm_handle
            .get_block_template(*parent_commitment.blkid())
            .await
            .expect("test: replacement template is pending");
        assert_eq!(
            replacement_template.header().slot(),
            parent_commitment.slot() + 1
        );
        assert_ne!(replacement_template.get_blockid(), stale_template_id);
    }
}
