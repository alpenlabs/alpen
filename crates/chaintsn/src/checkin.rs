//! L1 check-in logic.

use strata_asm_common::AsmManifest;
use strata_ol_chain_types::L1Segment;
use strata_params::RollupParams;

use crate::{
    context::{AuxProvider, ProviderError, ProviderResult, StateAccessor},
    errors::{OpError, TsnError},
    legacy::FauxStateCache,
    macros::*,
};

/// Provider for aux data taking from a block's L1 segment.
///
/// This is intended as a transitional data structure while we refactor these
/// pieces of the state transition logic.
#[derive(Debug, Clone)]
pub struct SegmentAuxData<'b> {
    first_height: u64,
    segment: &'b L1Segment,
}

impl<'b> SegmentAuxData<'b> {
    pub fn new(first_height: u64, segment: &'b L1Segment) -> Self {
        Self {
            first_height,
            segment,
        }
    }
}

impl<'b> AuxProvider for SegmentAuxData<'b> {
    fn get_l1_tip_height(&self) -> u64 {
        self.segment.new_height()
    }

    fn get_l1_block_manifest(&self, height: u64) -> ProviderResult<AsmManifest> {
        if height < self.first_height {
            return Err(ProviderError::OutOfBounds);
        }

        let idx = height - self.first_height;

        let mf = self
            .segment
            .new_manifests()
            .get(idx as usize)
            .ok_or(ProviderError::OutOfBounds)?;

        Ok(mf.clone())
    }
}

/// Update our view of the L1 state, playing out downstream changes from that.
///
/// Returns true if there epoch needs to be updated.
pub fn process_l1_view_update<'s, S: StateAccessor>(
    state: &mut FauxStateCache<'s, S>,
    prov: &impl AuxProvider,
    params: &RollupParams,
) -> Result<bool, TsnError> {
    let l1v = state.state().l1_view();

    // If there's no new blocks we can abort.
    let new_tip_height = prov.get_l1_tip_height();
    if prov.get_l1_tip_height() == l1v.safe_height() {
        return Ok(false);
    }

    let cur_safe_height = l1v.safe_height();

    // Validate the new blocks actually extend the tip.  This is what we have to tweak to make
    // more complicated to check the PoW.
    // FIXME: This check is just redundant.
    if new_tip_height <= l1v.safe_height() {
        return Err(TsnError::L1SegNotExtend);
    }

    let prev_finalized_epoch = *state.state().finalized_epoch();

    // Go through each manifest and process it.
    for height in (cur_safe_height + 1)..=new_tip_height {
        let mf = prov.get_l1_block_manifest(height)?;

        // Note: PoW checks are done in ASM STF when the manifest is created
        // We don't need to validate headers here anymore

        process_asm_logs(state, &mf, params)?;
    }

    // If prev_finalized_epoch is null, i.e. this is the genesis batch, it is
    // always safe to update the epoch.
    if prev_finalized_epoch.is_null() {
        return Ok(true);
    }

    // For all other non-genesis batch, we need to check that the new finalized epoch has been
    // updated when processing L1Checkpoint
    let new_finalized_epoch = state.state().finalized_epoch();

    // This checks to make sure that the L1 segment actually advances the
    // observed final epoch.  We don't want to allow segments that don't
    // advance the finalized epoch.
    //
    // QUESTION: why again exactly?
    if new_finalized_epoch.epoch() <= prev_finalized_epoch.epoch() {
        return Err(TsnError::EpochNotExtend);
    }

    Ok(true)
}

fn process_asm_logs<'s, S: StateAccessor>(
    state: &mut FauxStateCache<'s, S>,
    manifest: &AsmManifest,
    params: &RollupParams,
) -> Result<(), TsnError> {
    use strata_asm_manifest_types::{CheckpointAckLogData, CHECKPOINT_ACK_ASM_LOG_TYPE_ID};
    use strata_msg_fmt::Msg;

    // Iterate through ASM logs looking for checkpoint acknowledgments
    let in_blkid = manifest.blkid();
    for log in manifest.logs() {
        // Try to parse as SPS-52 message
        let Some(msg) = log.try_as_msg() else {
            continue;
        };

        // Check if this is a checkpoint ack log
        if msg.ty() != CHECKPOINT_ACK_ASM_LOG_TYPE_ID {
            continue;
        }

        // Try to decode checkpoint ack data
        let Ok(ack_data) = log.try_into_log::<CheckpointAckLogData>() else {
            warn!(%in_blkid, "failed to decode checkpoint ack log");
            continue;
        };

        // Process the checkpoint acknowledgment
        if let Err(e) = process_checkpoint_ack(state, &ack_data, params) {
            warn!(?ack_data, %in_blkid, %e, "invalid checkpoint ack");
        }
    }

    Ok(())
}

fn process_checkpoint_ack<'s, S: StateAccessor>(
    _state: &mut FauxStateCache<'s, S>,
    ack_data: &strata_asm_manifest_types::CheckpointAckLogData,
    _params: &RollupParams,
) -> Result<(), OpError> {
    // The checkpoint ack tells us that a checkpoint for a specific epoch was observed on L1
    let ack_epoch = ack_data.epoch();

    // TODO: This needs proper implementation to verify the checkpoint
    // For now, we just accept checkpoint acks from ASM logs as they've been validated by ASM STF

    // This is a placeholder - the actual logic needs to verify against stored checkpoints
    trace!(%ack_epoch, "observed checkpoint acknowledgment");

    Ok(())
}
