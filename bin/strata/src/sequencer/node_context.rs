//! Concrete [`SequencerContext`] implementation for the Strata node.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use strata_db_types::ol_block::BlockStatus;
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

        let parent_header = self
            .storage
            .ol_block()
            .get_ol_header_async(tip_blkid)
            .await
            .map_err(SequencerContextError::Db)?
            .ok_or_else(|| SequencerContextError::TemplateGeneration {
                tip_blkid,
                source: BlockAssemblyError::Other(format!("parent block {tip_blkid} not found")),
            })?;

        let parent_ts = parent_header.timestamp();
        let target_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_millis() as u64;

        let time_since_parent = target_ts.saturating_sub(parent_ts);
        if is_too_soon(parent_ts, target_ts, self.ol_block_time_ms) {
            debug!(
                time_since_parent,
                block_time_ms = self.ol_block_time_ms,
                parent_ts,
                target_ts,
                "template generation skipped: too soon after parent"
            );
            return Ok(None);
        }

        let threshold_ms = late_block_threshold_ms(self.ol_block_time_ms);
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

fn is_too_soon(parent_ts: u64, target_ts: u64, block_time_ms: u64) -> bool {
    target_ts.saturating_sub(parent_ts) < early_block_threshold_ms(block_time_ms)
}

fn early_block_threshold_ms(block_time_ms: u64) -> u64 {
    block_time_ms.saturating_mul(100 - BLOCK_TS_DRIFT_TOLERANCE_PCT) / 100
}

fn late_block_threshold_ms(block_time_ms: u64) -> u64 {
    block_time_ms.saturating_mul(100 + BLOCK_TS_DRIFT_TOLERANCE_PCT) / 100
}

fn should_release_completed_tombstone(status: Option<BlockStatus>) -> bool {
    matches!(status, Some(BlockStatus::Invalid))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use strata_config::{BlockAssemblyConfig, SequencerConfig};
    use strata_csm_types::{ClientState, L1Status};
    use strata_identifiers::{Buf32, Buf64, EpochCommitment, L1BlockCommitment, L1BlockId};
    use strata_ol_block_assembly::{
        BlockCompletionData, BlockasmBuilder, FixedSlotSealing, LimitAwareSealing,
        test_utils::{MockMempoolProvider, TestStorageFixtureBuilder},
    };
    use strata_ol_chain_types::{OLBlock, OLBlockHeader, SignedOLBlockHeader};
    use strata_ol_params::OLParams;
    use strata_ol_sequencer::SequencerBuilder;
    use strata_ol_state_provider::OLStateManagerProviderImpl;
    use strata_predicate::PredicateKey;
    use strata_service::AsyncServiceInput;
    use strata_status::{OLSyncStatus, OLSyncStatusUpdate};
    use strata_storage::NodeStorage;
    use strata_tasks::TaskManager;
    use tokio::{runtime::Handle, time::timeout};

    use super::*;

    const TEST_BLOCK_TIME_MS: u64 = 60_000;
    const TEST_MIN_SPACING_PCT: u64 = 100 - BLOCK_TS_DRIFT_TOLERANCE_PCT;
    const TEST_MIN_SPACING_MS: u64 = TEST_BLOCK_TIME_MS * TEST_MIN_SPACING_PCT / 100;
    const TEST_TICK_INTERVAL_MS: u64 = 10;

    fn high_watermark(slot: u64) -> OLBlockCommitment {
        OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from([slot as u8; 32])))
    }

    async fn start_test_blockasm(storage: Arc<NodeStorage>) -> (TaskManager, Arc<BlockasmHandle>) {
        let task_manager = TaskManager::new(Handle::current());
        let executor = task_manager.create_executor();
        let state_provider = OLStateManagerProviderImpl::new(storage.ol_state().clone());
        let blockasm = BlockasmBuilder::new(
            Arc::new(OLParams::default()),
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

    fn test_status_channel(l1_block: L1BlockCommitment) -> Arc<StatusChannel> {
        Arc::new(StatusChannel::new(
            ClientState::default(),
            l1_block,
            L1Status::default(),
            None,
            None,
        ))
    }

    fn test_l1_commitment(height: u32, block_id: L1BlockId) -> L1BlockCommitment {
        L1BlockCommitment::new(height, block_id)
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test: system time before unix epoch")
            .as_millis() as u64
    }

    async fn prepare_recent_parent_fixture() -> (
        TaskManager,
        Arc<BlockasmHandle>,
        Arc<NodeStorage>,
        Arc<StatusChannel>,
        OLBlockCommitment,
    ) {
        let (fixture, parent_commitment) = TestStorageFixtureBuilder::new()
            .with_genesis_parent_and_l1_manifest_count(0)
            .build_fixture()
            .await;
        let storage = fixture.storage().clone();

        let parent_block = storage
            .ol_block()
            .get_block_data_async(*parent_commitment.blkid())
            .await
            .expect("test: fetch parent block")
            .expect("test: parent block exists");
        let parent_state = storage
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("test: fetch parent state")
            .expect("test: parent state exists");

        let parent_header = parent_block.header();
        let recent_header = OLBlockHeader::new(
            now_ms(),
            parent_header.flags(),
            parent_header.slot(),
            parent_header.epoch(),
            *parent_header.parent_blkid(),
            *parent_header.body_root(),
            *parent_header.state_root(),
            *parent_header.logs_root(),
        );
        let recent_commitment =
            OLBlockCommitment::new(recent_header.slot(), recent_header.compute_blkid());
        let recent_parent_block = OLBlock::new(
            SignedOLBlockHeader::new(recent_header.clone(), Buf64::zero()),
            parent_block.body().clone(),
        );

        storage
            .ol_state()
            .put_toplevel_ol_state_async(recent_commitment, parent_state.as_ref().clone())
            .await
            .expect("test: store recent parent state");
        storage
            .ol_block()
            .put_block_data_async(recent_parent_block)
            .await
            .expect("test: store recent parent block");

        let safe_l1 = test_l1_commitment(1, L1BlockId::from(Buf32::from([1; 32])));
        let status_channel = test_status_channel(safe_l1);
        let safe_l1 = status_channel.get_cur_checkpoint_state().block;
        status_channel.update_ol_sync_status(OLSyncStatusUpdate::new(OLSyncStatus::new(
            recent_commitment,
            recent_header.epoch(),
            recent_header.is_terminal(),
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
            safe_l1,
        )));

        let (task_manager, blockasm_handle) = start_test_blockasm(storage.clone()).await;
        (
            task_manager,
            blockasm_handle,
            storage,
            status_channel,
            recent_commitment,
        )
    }

    async fn assert_no_pending_template(
        blockasm_handle: &BlockasmHandle,
        parent_commitment: OLBlockCommitment,
    ) {
        let err = blockasm_handle
            .get_block_template(*parent_commitment.blkid())
            .await
            .expect_err("test: no template should be pending for too-recent parent");
        assert!(
            matches!(err, BlockAssemblyError::NoPendingTemplateForParent(parent) if parent == *parent_commitment.blkid()),
            "unexpected block template lookup error: {err}"
        );
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
    fn too_soon_check_uses_lower_drift_tolerance() {
        let parent_ts = 1_000_000;
        let block_time_ms = 3_600_000;
        let min_spacing_ms = early_block_threshold_ms(block_time_ms);
        assert_eq!(
            early_block_threshold_ms(TEST_BLOCK_TIME_MS),
            TEST_MIN_SPACING_MS
        );

        assert!(is_too_soon(
            parent_ts,
            parent_ts + Duration::from_secs(5 * 60).as_millis() as u64,
            block_time_ms
        ));
        assert!(is_too_soon(
            parent_ts,
            parent_ts + min_spacing_ms - 1,
            block_time_ms
        ));
        assert!(!is_too_soon(
            parent_ts,
            parent_ts + min_spacing_ms,
            block_time_ms
        ));
        assert!(!is_too_soon(
            parent_ts,
            parent_ts + block_time_ms - 1,
            block_time_ms
        ));
        assert!(!is_too_soon(
            parent_ts,
            parent_ts + block_time_ms + 1,
            block_time_ms
        ));
    }

    #[test]
    fn too_soon_check_treats_backwards_clock_as_too_soon() {
        assert!(is_too_soon(1_000_000, 999_999, 3_600_000));
    }

    #[test]
    fn drift_thresholds_saturate_before_scaling() {
        assert_eq!(early_block_threshold_ms(1_000), 800);
        assert_eq!(early_block_threshold_ms(u64::MAX), u64::MAX / 100);
        assert_eq!(late_block_threshold_ms(1_000), 1_200);
        assert_eq!(late_block_threshold_ms(u64::MAX), u64::MAX / 100);
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
            test_status_channel(test_l1_commitment(1, L1BlockId::from(Buf32::from([1; 32])))),
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

    #[tokio::test]
    async fn generation_uses_header_only_terminal_tip() {
        let (fixture, parent_commitment) = TestStorageFixtureBuilder::new()
            .with_genesis_parent_and_l1_manifest_count(0)
            .build_fixture()
            .await;
        let storage = fixture.storage().clone();
        let parent_header = storage
            .ol_block()
            .get_ol_header_async(*parent_commitment.blkid())
            .await
            .expect("test: fetch parent header")
            .expect("test: parent header exists");
        let parent_epoch = parent_header.epoch();
        let parent_is_terminal = parent_header.is_terminal();
        storage
            .ol_block()
            .put_terminal_header_async(*parent_commitment.blkid(), parent_header)
            .await
            .expect("test: store terminal parent header");
        assert!(
            storage
                .ol_block()
                .del_block_data_async(*parent_commitment.blkid())
                .await
                .expect("test: delete parent block")
        );
        let (_task_manager, blockasm_handle) = start_test_blockasm(storage.clone()).await;
        let status_channel =
            test_status_channel(test_l1_commitment(1, L1BlockId::from(Buf32::from([1; 32]))));
        let safe_l1 = status_channel.get_cur_checkpoint_state().block;
        status_channel.update_ol_sync_status(OLSyncStatusUpdate::new(OLSyncStatus::new(
            parent_commitment,
            parent_epoch,
            parent_is_terminal,
            EpochCommitment::null(),
            EpochCommitment::null(),
            EpochCommitment::null(),
            safe_l1,
        )));
        let sequencer_context =
            NodeSequencerContext::new(blockasm_handle.clone(), storage, status_channel, 1);

        assert_eq!(
            sequencer_context
                .generate_template_for_tip()
                .await
                .expect("test: generate from header-only tip"),
            Some(*parent_commitment.blkid())
        );
        let template = blockasm_handle
            .get_block_template(*parent_commitment.blkid())
            .await
            .expect("test: pending template");
        assert_eq!(template.header().parent_blkid(), parent_commitment.blkid());
    }

    #[tokio::test]
    async fn block_generation_is_skipped_when_interval_not_reached() {
        let (_task_manager, blockasm_handle, storage, status_channel, parent_commitment) =
            prepare_recent_parent_fixture().await;
        let sequencer_context = NodeSequencerContext::new(
            blockasm_handle.clone(),
            storage,
            status_channel,
            TEST_BLOCK_TIME_MS,
        );

        let generated_parent = sequencer_context
            .generate_template_for_tip()
            .await
            .expect("test: too-recent generation should not error");

        assert_eq!(generated_parent, None);
        assert_no_pending_template(&blockasm_handle, parent_commitment).await;
    }

    #[tokio::test]
    async fn generation_tick_leaves_block_generation_skipped_when_interval_not_reached() {
        let (task_manager, blockasm_handle, storage, status_channel, parent_commitment) =
            prepare_recent_parent_fixture().await;
        let executor = task_manager.create_executor();
        let context = Arc::new(NodeSequencerContext::new(
            blockasm_handle.clone(),
            storage,
            status_channel,
            TEST_BLOCK_TIME_MS,
        ));
        let monitor = SequencerBuilder::new(context, Duration::from_millis(TEST_TICK_INTERVAL_MS))
            .launch(&executor)
            .await
            .expect("test: launch sequencer service");
        let mut status_listener = monitor.create_listener_input(&executor);

        timeout(Duration::from_secs(1), status_listener.recv_next())
            .await
            .expect("test: generation tick should publish a service status")
            .expect("test: status listener should not error")
            .expect("test: status listener should receive a status");

        assert_no_pending_template(&blockasm_handle, parent_commitment).await;
    }
}
