//! Block template generation service launcher.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use strata_consensus_logic::{FcmServiceHandle, message::ForkChoiceMessage};
use strata_db_types::ol_block::BlockStatus;
use strata_identifiers::{EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_ol_chain_types::OLBlock;
use strata_ol_sequencer::{SequencerBuilder, SequencerServiceStatus};
use strata_service::ServiceMonitor;
use strata_storage::NodeStorage;
use tracing::{error, info};

use super::node_context::NodeSequencerContext;
use crate::run_context::RunContext;

/// Starts the block production service (template generation only).
pub(crate) fn start_block_producer(
    runctx: &RunContext,
) -> Result<ServiceMonitor<SequencerServiceStatus>> {
    let handles = runctx
        .sequencer_handles()
        .ok_or_else(|| anyhow!("sequencer handles not available (is_sequencer=true required)"))?;

    let ol_block_interval_ms = runctx
        .config()
        .sequencer
        .as_ref()
        .ok_or_else(|| anyhow!("sequencer config required when block producer is enabled"))?
        .ol_block_time_ms;

    let fcm_handle = runctx
        .fcm_handle()
        .ok_or_else(|| anyhow!("fcm handle not available (is_sequencer=true required)"))?;
    runctx
        .task_manager()
        .handle()
        .block_on(process_startup_high_watermark_block(
            runctx.storage().as_ref(),
            fcm_handle.as_ref(),
        ))?;

    let context = Arc::new(NodeSequencerContext::new(
        handles.blockasm_handle().clone(),
        runctx.storage().clone(),
        runctx.status_channel().clone(),
        ol_block_interval_ms,
    ));

    let service_monitor = runctx.task_manager().handle().block_on(async {
        SequencerBuilder::new(context, Duration::from_millis(ol_block_interval_ms))
            .launch(runctx.executor())
            .await
    })?;

    info!(%ol_block_interval_ms, "block producer service started");

    Ok(service_monitor)
}

async fn process_startup_high_watermark_block(
    storage: &NodeStorage,
    fcm_handle: &FcmServiceHandle,
) -> Result<()> {
    let high_watermark = storage
        .ol_block()
        .get_block_high_watermark_async()
        .await
        .context("failed to get OL block high-watermark")?;

    let Some(high_watermark) = high_watermark else {
        return Ok(());
    };

    let block_id = *high_watermark.blkid();
    let block = storage
        .ol_block()
        .get_block_data_async(block_id)
        .await
        .context("failed to get high-watermark OL block")?;

    let status = storage
        .ol_block()
        .get_block_status_async(block_id)
        .await
        .context("failed to get high-watermark OL block status")?;

    match decide_startup_high_watermark_block_action(high_watermark, block.as_ref(), status)? {
        HighWatermarkBlockAction::Skip => {
            info!(
                block_id = %block_id,
                slot = high_watermark.slot(),
                "high-watermark OL block already processed by FCM"
            );
            Ok(())
        }
        HighWatermarkBlockAction::Clear => {
            // Drop the invalid block's state-indexing writes before clearing
            // the high-watermark, so the replacement block built for this
            // slot doesn't conflict against stale indexing rows. Idempotent;
            // also covers a crash between the FCM-side rollback and clear.
            let block = block.expect("decide_startup_high_watermark_block_action checked presence");
            let cutoff = OLBlockCommitment::new(
                high_watermark.slot().saturating_sub(1),
                *block.header().parent_blkid(),
            );
            storage
                .ol_state_indexing()
                .rollback_to_block_async(block.header().epoch(), cutoff)
                .await
                .inspect_err(|err| {
                    error!(
                        block_id = %block_id,
                        slot = high_watermark.slot(),
                        %err,
                        "failed to roll back state indexing for invalid high-watermark OL block on startup; replacement generation for this slot remains blocked"
                    );
                })
                .context("failed to roll back state indexing for invalid high-watermark OL block on startup")?;

            // An invalid terminal block may have stored its epoch summary
            // before being rejected. Drop it so it cannot shadow the
            // replacement terminal's summary in canonical lookups.
            if block.header().is_terminal() {
                let summary_commitment =
                    EpochCommitment::new(block.header().epoch(), high_watermark.slot(), block_id);
                storage
                    .ol_checkpoint()
                    .del_epoch_summary_async(summary_commitment)
                    .await
                    .inspect_err(|err| {
                        error!(
                            block_id = %block_id,
                            slot = high_watermark.slot(),
                            %err,
                            "failed to delete epoch summary of invalid high-watermark OL terminal block on startup; replacement generation for this slot remains blocked"
                        );
                    })
                    .context("failed to delete epoch summary of invalid high-watermark OL terminal block on startup")?;
            }

            let cleared = storage
                .ol_block()
                .clear_block_high_watermark_async(high_watermark)
                .await
                .inspect_err(|err| {
                    error!(
                        block_id = %block_id,
                        slot = high_watermark.slot(),
                        %err,
                        "failed to clear invalid high-watermark OL block on startup; replacement generation for this slot remains blocked"
                    );
                })
                .context("failed to clear invalid high-watermark OL block on startup")?;

            if cleared {
                info!(
                    block_id = %block_id,
                    slot = high_watermark.slot(),
                    "cleared invalid high-watermark OL block on startup"
                );
            }

            Ok(())
        }
        HighWatermarkBlockAction::Resubmit(block_id) => {
            let submitted = fcm_handle
                .submit_chain_tip_msg_async(ForkChoiceMessage::NewBlock(block_id))
                .await;
            if !submitted {
                bail!("failed to resubmit high-watermark OL block {block_id} to FCM");
            }

            info!(
                block_id = %block_id,
                slot = high_watermark.slot(),
                "resubmitted high-watermark OL block to FCM"
            );
            Ok(())
        }
    }
}

/// Startup action for the OL block recorded by the local high-watermark.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum HighWatermarkBlockAction {
    Skip,
    Clear,
    Resubmit(OLBlockId),
}

fn decide_startup_high_watermark_block_action(
    high_watermark: OLBlockCommitment,
    block: Option<&OLBlock>,
    status: Option<BlockStatus>,
) -> Result<HighWatermarkBlockAction> {
    let block_id = *high_watermark.blkid();
    let block = block.ok_or_else(|| anyhow!("high-watermark OL block {block_id} is missing"))?;

    if block.header().slot() != high_watermark.slot() {
        bail!(
            "high-watermark OL block {block_id} has slot {}, expected {}",
            block.header().slot(),
            high_watermark.slot()
        );
    }

    Ok(match status {
        Some(BlockStatus::Valid) => HighWatermarkBlockAction::Skip,
        Some(BlockStatus::Invalid) => {
            // TODO(STR-2141): `BlockStatus::Invalid` also represents local execution failures.
            // Revisit high-watermark clearing once FCM distinguishes consensus-invalid blocks
            // from transient execution failures.
            HighWatermarkBlockAction::Clear
        }
        Some(BlockStatus::Unchecked) | None => HighWatermarkBlockAction::Resubmit(block_id),
    })
}

#[cfg(test)]
mod tests {
    use strata_identifiers::{Buf32, Buf64};
    use strata_ol_chain_types::{
        BlockFlags, OLBlockBody, OLBlockHeader, OLTxSegment, SignedOLBlockHeader,
    };

    use super::*;

    fn block_at_slot(slot: u64) -> OLBlock {
        let header = OLBlockHeader::new(
            0,
            BlockFlags::from(0),
            slot,
            0,
            OLBlockId::from(Buf32::from([0x11; 32])),
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        let signed_header = SignedOLBlockHeader::new(header, Buf64::zero());
        let body = OLBlockBody::new_common(OLTxSegment::new(vec![]).expect("empty tx segment"));
        OLBlock::new(signed_header, body)
    }

    fn commitment_for(block: &OLBlock) -> OLBlockCommitment {
        OLBlockCommitment::new(block.header().slot(), block.header().compute_blkid())
    }

    #[test]
    fn startup_action_resubmits_unprocessed_statuses() {
        let block = block_at_slot(7);
        let high_watermark = commitment_for(&block);
        let block_id = *high_watermark.blkid();

        let unchecked = decide_startup_high_watermark_block_action(
            high_watermark,
            Some(&block),
            Some(BlockStatus::Unchecked),
        )
        .expect("unchecked high-watermark block should resubmit");
        assert_eq!(unchecked, HighWatermarkBlockAction::Resubmit(block_id));

        let missing_status =
            decide_startup_high_watermark_block_action(high_watermark, Some(&block), None)
                .expect("missing status high-watermark block should resubmit");
        assert_eq!(missing_status, HighWatermarkBlockAction::Resubmit(block_id));
    }

    #[test]
    fn startup_action_clears_invalid_status() {
        let block = block_at_slot(7);
        let high_watermark = commitment_for(&block);
        let invalid = decide_startup_high_watermark_block_action(
            high_watermark,
            Some(&block),
            Some(BlockStatus::Invalid),
        )
        .expect("invalid high-watermark block should clear");
        assert_eq!(invalid, HighWatermarkBlockAction::Clear);
    }

    #[test]
    fn startup_action_skips_valid_status() {
        let block = block_at_slot(8);
        let high_watermark = commitment_for(&block);

        let valid = decide_startup_high_watermark_block_action(
            high_watermark,
            Some(&block),
            Some(BlockStatus::Valid),
        )
        .expect("valid high-watermark block should skip");
        assert_eq!(valid, HighWatermarkBlockAction::Skip);
    }

    #[test]
    fn startup_action_errors_when_recorded_block_is_missing() {
        let block = block_at_slot(9);
        let high_watermark = commitment_for(&block);
        let err = decide_startup_high_watermark_block_action(
            high_watermark,
            None,
            Some(BlockStatus::Unchecked),
        )
        .expect_err("missing high-watermark block should error");

        assert!(
            err.to_string().contains("high-watermark OL block"),
            "unexpected error: {err:#}"
        );
        assert!(
            err.to_string().contains("is missing"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn startup_action_errors_when_recorded_block_slot_mismatches() {
        let block = block_at_slot(10);
        let high_watermark = OLBlockCommitment::new(11, block.header().compute_blkid());
        let err = decide_startup_high_watermark_block_action(
            high_watermark,
            Some(&block),
            Some(BlockStatus::Unchecked),
        )
        .expect_err("slot-mismatched high-watermark block should error");

        assert!(
            err.to_string().contains("has slot 10, expected 11"),
            "unexpected error: {err:#}"
        );
    }
}
