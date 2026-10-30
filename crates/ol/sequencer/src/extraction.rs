use std::future::Future;

use strata_asm_checkpoint_types::CheckpointPayload;
use strata_db_types::l1_writer::L1BundleStatus;
use strata_db_types::DbResult;
use strata_identifiers::{Epoch, EpochCommitment};
use strata_ol_block_assembly::{BlockAssemblyError, BlockasmHandle};
use strata_primitives::OLBlockId;
use strata_storage::NodeStorage;
use tracing::debug;

use crate::{BlockSigningDuty, CheckpointSigningDuty, Duty, Error, RevealTxSigningDuty};

/// Reads checkpoint candidates and their exact signing artifacts.
pub trait CheckpointDutyContext: Send + Sync {
    /// Returns the earliest epoch in the unsigned-checkpoint queue, or `None` if empty.
    ///
    /// Queue membership does not establish canonicality or payload availability.
    fn get_next_unsigned_checkpoint_epoch(
        &self,
    ) -> impl Future<Output = DbResult<Option<Epoch>>> + Send;

    /// Returns the canonical commitment for `epoch`.
    ///
    /// Returns `None` when there is no canonical commitment for `epoch`.
    fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> impl Future<Output = DbResult<Option<EpochCommitment>>> + Send;

    /// Returns the payload for the exact commitment, or `None` if absent.
    fn get_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
    ) -> impl Future<Output = DbResult<Option<CheckpointPayload>>> + Send;

    /// Returns whether submission has been recorded for the exact commitment.
    ///
    /// A signing entry does not imply L1 inclusion or finalization.
    fn has_checkpoint_signing_entry(
        &self,
        commitment: EpochCommitment,
    ) -> impl Future<Output = DbResult<bool>> + Send;
}

/// Extract sequencer duties.
pub async fn extract_duties(
    blockasm: &BlockasmHandle,
    tip_blkid: OLBlockId,
    node_storage: &NodeStorage,
    checkpoint_ctx: &impl CheckpointDutyContext,
) -> Result<Vec<Duty>, Error> {
    let mut duties = vec![];
    let block_high_watermark = node_storage
        .ol_block()
        .get_block_high_watermark_async()
        .await?;

    // Block duties. Read-only lookup; generation is handled by GenerationTick.
    match blockasm.get_block_template(tip_blkid).await {
        Ok(template) => {
            let template_slot = template.header().slot();
            let should_offer = block_high_watermark
                .as_ref()
                .is_none_or(|high_watermark| template_slot > high_watermark.slot());
            if should_offer {
                let blkduty = BlockSigningDuty::new(template);
                duties.push(Duty::SignBlock(blkduty));
            } else if let Some(high_watermark) = block_high_watermark.as_ref() {
                debug!(
                    tip_blkid = ?tip_blkid,
                    template_slot,
                    high_watermark = %high_watermark,
                    "cached block template is at or below block high-watermark; skipping block duty"
                );
            } else {
                unreachable!("block duty should only be skipped when a high-watermark exists");
            }
        }
        Err(BlockAssemblyError::NoPendingTemplateForParent(_)) => {
            debug!(
                tip_blkid = ?tip_blkid,
                "no cached template for tip parent; skipping block duty"
            );
        }
        Err(err) => return Err(err.into()),
    }

    // Checkpoint duties
    let unsigned_checkpoint = get_earliest_unsigned_checkpoint(checkpoint_ctx).await?;
    duties.extend(
        unsigned_checkpoint
            .into_iter()
            .map(CheckpointSigningDuty::new)
            .map(Duty::SignCheckpoint),
    );

    // Payload signing duties
    let pending_payloads = get_pending_payload_duties(node_storage).await?;
    duties.extend(pending_payloads.into_iter().map(Duty::SignRevealTx));

    Ok(duties)
}

/// Gets the earliest unsigned checkpoint
pub(crate) async fn get_earliest_unsigned_checkpoint(
    ctx: &impl CheckpointDutyContext,
) -> Result<Option<CheckpointPayload>, Error> {
    let Some(epoch) = ctx.get_next_unsigned_checkpoint_epoch().await? else {
        return Ok(None);
    };
    let Some(commitment) = ctx.get_canonical_epoch_commitment_at(epoch).await? else {
        return Ok(None);
    };
    if ctx.has_checkpoint_signing_entry(commitment).await? {
        return Ok(None);
    }
    let Some(payload) = ctx.get_checkpoint_payload(commitment).await? else {
        return Ok(None);
    };
    if ctx.get_canonical_epoch_commitment_at(epoch).await? != Some(commitment) {
        return Ok(None);
    }
    Ok(Some(payload))
}

/// Gets payload entries pending an external signature.
async fn get_pending_payload_duties(
    node_storage: &NodeStorage,
) -> Result<Vec<RevealTxSigningDuty>, Error> {
    let l1_writer = node_storage.l1_writer();

    let mut idx = l1_writer.get_next_payload_idx_async().await?;
    let mut duties = vec![];

    while idx > 0 {
        idx -= 1;
        let Some(entry) = l1_writer.get_payload_entry_by_idx_async(idx).await? else {
            break;
        };
        if entry.status == L1BundleStatus::Finalized {
            break;
        }
        if let L1BundleStatus::PendingRevealTxSign(sighash) = entry.status {
            if entry.payload_signature.is_none() {
                duties.push(RevealTxSigningDuty::new(idx, sighash));
            }
        }
    }

    Ok(duties)
}

#[cfg(test)]
mod tests {
    use strata_db_types::DbError;

    use super::*;
    use crate::test_utils::CheckpointFixture;

    #[test]
    fn test_checkpoint_duties_follow_unsigned_epoch_progression() {
        let fixture = CheckpointFixture::new();
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            None
        );

        // Epochs complete and become canonical before their payloads are ready.
        let first = fixture.extend(fixture.genesis(), 1);
        fixture.publish(first);
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            None
        );
        let first_payload = fixture.store_payload(first);
        let second = fixture.extend(first, 2);
        fixture.publish(second);
        let second_payload = fixture.store_payload(second);

        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            Some(first_payload)
        );
        fixture.record_submission(first, 10);
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            Some(second_payload)
        );
        fixture.record_submission(second, 11);
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            None
        );
    }

    #[test]
    fn test_checkpoint_duty_waits_for_canonical_payload_instead_of_using_fork() {
        let fixture = CheckpointFixture::new();
        let first = fixture.extend(fixture.genesis(), 1);
        fixture.publish(first);
        fixture.store_payload(first);
        fixture.record_submission(first, 10);

        // Both branches have summaries, but only the orphan has a payload initially.
        let left = fixture.extend(first, 2);
        let right = fixture.extend(first, 3);
        let (orphan, canonical) = if left < right {
            (left, right)
        } else {
            (right, left)
        };
        fixture.store_payload(orphan);
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            None
        );
        fixture.publish(canonical);
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            None
        );

        let payload = fixture.store_payload(canonical);
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            Some(payload)
        );
        fixture.record_submission(canonical, 11);
        let next = fixture.extend(canonical, 4);
        fixture.publish(next);
        let next_payload = fixture.store_payload(next);
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            Some(next_payload)
        );

        // An abandoned payload requeues the already-signed epoch at the head.
        // Selection stops there, blocking the later canonical epoch's duty too.
        fixture.store_payload(orphan);
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            None
        );
    }

    #[test]
    fn test_checkpoint_duty_rechecks_after_reorg() {
        let fixture = CheckpointFixture::new();
        let first = fixture.extend(fixture.genesis(), 1);
        fixture.publish(first);
        fixture.store_payload(first);
        let replacement = fixture.extend(fixture.genesis(), 2);
        let replacement_payload = fixture.store_payload(replacement);

        // Fork choice changes after loading the old payload. Only the next poll offers the new one.
        fixture.reorg_after_payload(replacement);
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            None
        );
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            Some(replacement_payload)
        );

        // Rolling back to genesis removes the completed epoch entirely.
        fixture.reorg_after_payload(fixture.genesis());
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            None
        );
        assert_eq!(
            fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
            None
        );
    }

    #[test]
    fn test_checkpoint_duty_propagates_canonical_read_failures() {
        let fixture = CheckpointFixture::new();
        let first = fixture.extend(fixture.genesis(), 1);
        fixture.publish(first);
        let payload = fixture.store_payload(first);
        for successful_reads in [0, 1] {
            fixture.fail_canonical_read_after(successful_reads);
            assert!(matches!(
                fixture.get_earliest_unsigned_checkpoint_blocking(),
                Err(Error::Database(DbError::Busy))
            ));
            assert_eq!(
                fixture.get_earliest_unsigned_checkpoint_blocking().unwrap(),
                Some(payload.clone())
            );
        }
    }
}
