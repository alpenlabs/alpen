//! L1 check-in logic.

use bitcoin::{block::Header, consensus, params::Params};
use strata_crypto::groth16_verifier::verify_rollup_groth16_proof_receipt;
use strata_primitives::{
    batch::SignedCheckpoint,
    l1::{
        DepositInfo, DepositSpendInfo, L1BlockManifest, ProtocolOperation,
        WithdrawalFulfillmentInfo,
    },
    params::RollupParams,
};
use strata_state::{
    batch::verify_signed_checkpoint_sig, block::L1Segment, bridge_ops::DepositIntent,
};

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

    fn get_l1_block_manifest(&self, height: u64) -> ProviderResult<L1BlockManifest> {
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

        // PoW checks are done when we try to update the HeaderVerificationState
        let header: Header = consensus::deserialize(mf.header()).expect("invalid bitcoin header");
        state.update_header_vs(&header, &Params::new(params.network))?;

        process_protocol_ops(state, &mf, params)?;
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

fn process_protocol_ops<'s, S: StateAccessor>(
    state: &mut FauxStateCache<'s, S>,
    block_mf: &L1BlockManifest,
    params: &RollupParams,
) -> Result<(), TsnError> {
    // Just iterate through every tx's operation and call out to the handlers for that.
    for tx in block_mf.txs() {
        let in_blkid = block_mf.blkid();
        for op in tx.protocol_ops() {
            // Try to process it, log a warning if there's an error.
            if let Err(e) = process_proto_op(state, block_mf, op, params) {
                warn!(?op, %in_blkid, %e, "invalid protocol operation");
            }
        }
    }

    Ok(())
}

fn process_proto_op<'s, S: StateAccessor>(
    state: &mut FauxStateCache<'s, S>,
    block_mf: &L1BlockManifest,
    op: &ProtocolOperation,
    params: &RollupParams,
) -> Result<(), OpError> {
    match &op {
        ProtocolOperation::Checkpoint(ckpt) => {
            process_l1_checkpoint(state, block_mf, ckpt, params)?;
        }

        ProtocolOperation::Deposit(info) => {
            process_l1_deposit(state, block_mf, info)?;
        }

        ProtocolOperation::WithdrawalFulfillment(info) => {
            process_withdrawal_fulfillment(state, info)?;
        }

        ProtocolOperation::DepositSpent(info) => {
            process_deposit_spent(state, info)?;
        }

        // Other operations we don't do anything with for now.
        _ => {}
    }

    Ok(())
}

fn process_l1_checkpoint<'s, S: StateAccessor>(
    state: &mut FauxStateCache<'s, S>,
    _src_block_mf: &L1BlockManifest,
    signed_ckpt: &SignedCheckpoint,
    params: &RollupParams,
) -> Result<(), OpError> {
    // If signature verification failed, return early and do **NOT** finalize epoch
    // Note: This is not an error because anyone is able to post data to L1
    if !verify_signed_checkpoint_sig(signed_ckpt, &params.cred_rule) {
        warn!("Invalid checkpoint: signature");
        return Err(OpError::InvalidSignature);
    }

    let ckpt = signed_ckpt.checkpoint(); // inner data
    let ckpt_epoch = ckpt.batch_transition().epoch;

    let receipt = ckpt.construct_receipt();

    // Note: This is error because this is done by the sequencer
    if ckpt_epoch != 0 && ckpt_epoch != state.state().finalized_epoch().epoch() + 1 {
        error!(%ckpt_epoch, "Invalid checkpoint: proof for invalid epoch");
        return Err(OpError::EpochNotExtend);
    }

    // TODO refactor this to encapsulate the conditional verification into
    // another fn so we don't have to think about it here
    if receipt.proof().is_empty() {
        warn!(%ckpt_epoch, "Empty proof posted");
        // If the proof is empty but empty proofs are not allowed, this will fail.
        if !params.proof_publish_mode.allow_empty() {
            error!(%ckpt_epoch, "Invalid checkpoint: Received empty proof while in strict proof mode. Check `proof_publish_mode` in rollup parameters; set it to a non-strict mode (e.g., `timeout`) to accept empty proofs.");
            return Err(OpError::InvalidProof);
        }
    } else {
        // Otherwise, verify the non-empty proof.
        verify_rollup_groth16_proof_receipt(&receipt, &params.rollup_vk).map_err(|error| {
            error!(%ckpt_epoch, %error, "Failed to verify non-empty proof for epoch");
            OpError::InvalidProof
        })?;
    }

    // Copy the epoch commitment and make it finalized.
    let _old_fin_epoch = state.state().finalized_epoch();
    let new_fin_epoch = ckpt.batch_info().get_epoch_commitment();

    // TODO go through and do whatever stuff we need to do now that's finalized

    state.inner_mut().set_finalized_epoch(new_fin_epoch);
    trace!(?new_fin_epoch, "observed finalized checkpoint");

    Ok(())
}

fn process_l1_deposit<'s, S: StateAccessor>(
    state: &mut FauxStateCache<'s, S>,
    _src_block_mf: &L1BlockManifest,
    info: &DepositInfo,
) -> Result<(), OpError> {
    let requested_idx = info.deposit_idx;
    let outpoint = info.outpoint;

    // Create the deposit entry to track it on the bridge side.
    //
    // Right now all operators sign all deposits, take them all.
    let all_operators = state.state().operator_table().indices().collect::<_>();
    let ok = state.insert_deposit_entry(requested_idx, outpoint, info.amt, all_operators);

    // If we inserted it successfully, create the intent.
    if ok {
        // Insert an intent to credit the destination with it.
        let deposit_intent = DepositIntent::new(info.amt, info.address.clone());
        state.insert_deposit_intent(0, deposit_intent);

        // Logging so we know if it got there.
        trace!(?outpoint, "handled deposit");
    } else {
        warn!(?outpoint, %requested_idx, "ignoring deposit that would have overwritten entry");
    }

    Ok(())
}

/// Withdrawal Fulfillment with correct metadata is seen.
/// Mark the withthdrawal as being executed and prevent reassignment to another operator.
fn process_withdrawal_fulfillment<'s, S: StateAccessor>(
    state: &mut FauxStateCache<'s, S>,
    info: &WithdrawalFulfillmentInfo,
) -> Result<(), OpError> {
    state.mark_deposit_fulfilled(info);
    Ok(())
}

/// Locked deposit on L1 has been spent.
fn process_deposit_spent<'s, S: StateAccessor>(
    state: &mut FauxStateCache<'s, S>,
    info: &DepositSpendInfo,
) -> Result<(), OpError> {
    // Currently, we are not tracking how this was spent, only that it was.
    state.mark_deposit_reimbursed(info.deposit_idx);
    Ok(())
}
