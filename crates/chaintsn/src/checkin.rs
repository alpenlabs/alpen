//! L1 check-in logic.

use strata_asm_common::AsmManifest;
use strata_asm_logs::{
    constants::{CHECKPOINT_UPDATE_LOG_TYPE, DEPOSIT_LOG_TYPE_ID},
    CheckpointUpdate, DepositLog,
};
use strata_bridge_types::DepositIntent;
use strata_identifiers::DepositDescriptor;
use strata_ol_chain_types::L1Segment;
use strata_params::RollupParams;
use strata_primitives::l1::{BitcoinAmount, L1BlockCommitment};

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
    _params: &RollupParams,
) -> Result<bool, TsnError> {
    let l1v = state.state().l1_view();

    // If there's no new blocks we can abort.
    if prov.get_l1_tip_height() == l1v.safe_height() {
        return Ok(false);
    }

    let new_tip_height = prov.get_l1_tip_height();
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

        process_asm_logs(state, &mf)?;

        // Advance the verified L1 tip to the latest manifest we've processed.
        let verified_blk = L1BlockCommitment::from_height_u64(mf.height(), *mf.blkid())
            .expect("height should be valid");
        state.update_verified_blk(verified_blk);
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
) -> Result<(), TsnError> {
    for log in manifest.logs() {
        match log.ty() {
            Some(CHECKPOINT_UPDATE_LOG_TYPE) => {
                if let Ok(ckpt_update) = log.try_into_log::<CheckpointUpdate>() {
                    if let Err(e) = process_l1_checkpoint(state, &ckpt_update) {
                        warn!(%e, "failed to process L1 checkpoint");
                    }
                }
            }
            Some(DEPOSIT_LOG_TYPE_ID) => {
                if let Ok(deposit) = log.try_into_log::<DepositLog>() {
                    if let Err(e) = process_l1_deposit(state, deposit) {
                        warn!(%e, "failed to process L1 deposit");
                    }
                }
            }
            _ => {
                warn!("invalid log type");
            }
        }
    }

    Ok(())
}

fn process_l1_checkpoint<'s, S: StateAccessor>(
    state: &mut FauxStateCache<'s, S>,
    ckpt_update: &CheckpointUpdate,
) -> Result<(), OpError> {
    debug!(?ckpt_update, "observed l1 checkpoint");
    let new_fin_epoch = ckpt_update.epoch_commitment();
    state.inner_mut().set_finalized_epoch(new_fin_epoch);
    Ok(())
}

fn process_l1_deposit<'s, S: StateAccessor>(
    state: &mut FauxStateCache<'s, S>,
    deposit: DepositLog,
) -> Result<(), OpError> {
    let amt = BitcoinAmount::from_sat(deposit.amount);
    let descriptor = DepositDescriptor::decode_from_slice(&deposit.destination)?;
    let ee_id = descriptor.dest_acct_serial();
    let dest_ident = descriptor.dest_subject().to_subject_id();
    let intent = DepositIntent::new(amt, dest_ident);
    state.insert_deposit_intent(*ee_id, intent);
    Ok(())
}
