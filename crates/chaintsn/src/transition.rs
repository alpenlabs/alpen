//! Top-level CL state transition logic.  This is largely stubbed off now, but
//! we'll replace components with real implementations as we go along.
#![allow(unused)]

use std::{cmp::max, collections::HashMap};

use bitcoin::{block::Header, consensus, params::Params, OutPoint, Transaction};
use rand_core::{RngCore, SeedableRng};
use strata_crypto::groth16_verifier::verify_rollup_groth16_proof_receipt;
use strata_primitives::{
    batch::SignedCheckpoint,
    epoch::EpochCommitment,
    l1::{
        BitcoinAmount, DepositInfo, DepositSpendInfo, L1BlockManifest, L1HeaderRecord, L1TxRef,
        OutputRef, ProtocolOperation, WithdrawalFulfillmentInfo,
    },
    l2::L2BlockCommitment,
    params::RollupParams,
};
use strata_state::{
    batch::verify_signed_checkpoint_sig,
    block::L1Segment,
    bridge_ops::{DepositIntent, WithdrawalIntent},
    bridge_state::{DepositState, DispatchCommand, WithdrawOutput},
    exec_env::ExecEnvState,
    exec_update::{self, construct_ops_from_deposit_intents, ELDepositData, Op},
    prelude::*,
    state_op::StateCache,
    state_queue,
};
use tracing::warn;

use crate::{
    errors::{OpError, TsnError},
    macros::*,
    slot_rng::{self, SlotRng},
};

/// Processes a block, making writes into the provided state cache.
///
/// The cache will eventually be written to disk.  This does not check the
/// block's credentials, it plays out all the updates a block makes to the
/// chain, but it will abort if there are any semantic issues that
/// don't make sense.
///
/// This operates on a state cache that's expected to be empty, may panic if
/// changes have been made, although this is not guaranteed.  Does not check the
/// `state_root` in the header for correctness, so that can be unset so it can
/// be use during block assembly.
pub fn process_block(
    state: &mut StateCache,
    header: &impl L2Header,
    body: &L2BlockBody,
    params: &RollupParams,
) -> Result<(), TsnError> {
    // We want to fail quickly here because otherwise we don't know what's
    // happening.
    if !state.is_empty() {
        panic!("transition: state cache not fresh");
    }

    let mut rng = compute_init_slot_rng(state);

    // Update basic bookkeeping.
    let prev_tip_slot = state.state().chain_tip_slot();
    let prev_tip_blkid = *header.parent();
    state.set_slot(header.slot());
    state.set_prev_block(L2BlockCommitment::new(prev_tip_slot, prev_tip_blkid));
    advance_epoch_tracking(state)?;
    if state.state().cur_epoch() != header.epoch() {
        return Err(TsnError::MismatchEpoch(
            header.epoch(),
            state.state().cur_epoch(),
        ));
    }

    // Go through each stage and play out the operations it has.
    let has_new_epoch = process_l1_view_update(state, body.l1_segment(), params)?;
    let ready_withdrawals = process_execution_update(state, body.exec_segment().update())?;
    process_deposit_updates(state, ready_withdrawals, &mut rng, params)?;

    // If we checked in with L1, then advance the epoch.
    if has_new_epoch {
        state.set_epoch_finishing_flag(true);
    }

    Ok(())
}

/// Constructs the slot RNG used for processing the block.
///
/// This is meant to be independent of the block's body so that it's less
/// manipulatable.  Eventually we want to switch to a randao-ish scheme, but
/// let's not get ahead of ourselves.
fn compute_init_slot_rng(state: &StateCache) -> SlotRng {
    // Just take the last block's slot.
    let blkid_buf = *state.state().prev_block().blkid().as_ref();
    SlotRng::from_seed(blkid_buf)
}

/// Update our view of the L1 state, playing out downstream changes from that.
///
/// Returns true if there epoch needs to be updated.
fn process_l1_view_update(
    state: &mut StateCache,
    l1seg: &L1Segment,
    params: &RollupParams,
) -> Result<bool, TsnError> {
    let l1v = state.state().l1_view();

    if l1seg.new_manifests().is_empty() {
        return Ok(false);
    }

    let cur_safe_height = l1v.safe_height();

    // Validate the new blocks actually extend the tip.  This is what we have to tweak to make
    // more complicated to check the PoW.
    let new_tip_height = cur_safe_height + l1seg.new_manifests().len() as u64;
    // FIXME: This check is just redundant.
    if new_tip_height <= l1v.safe_height() {
        return Err(TsnError::L1SegNotExtend);
    }

    let prev_finalized_epoch = *state.state().finalized_epoch();

    // Go through each manifest and process it.
    for (off, b) in l1seg.new_manifests().iter().enumerate() {
        // PoW checks are done when we try to update the HeaderVerificationState
        let header: Header = consensus::deserialize(b.header()).expect("invalid bitcoin header");
        state.update_header_vs(&header, &Params::new(params.network))?;

        let height = cur_safe_height + off as u64 + 1;
        process_l1_block(state, b, params)?;
        state.update_safe_block(height, b.record().clone());
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

fn process_l1_block(
    state: &mut StateCache,
    block_mf: &L1BlockManifest,
    params: &RollupParams,
) -> Result<(), TsnError> {
    let blkid = block_mf.blkid();

    // Just iterate through every tx's operation and call out to the handlers for that.
    for tx in block_mf.txs() {
        for op in tx.protocol_ops() {
            // Try to process it, log a warning if there's an error.
            if let Err(e) = process_proto_op(state, block_mf, op, params) {
                warn!(?op, in_blkid = %blkid, %e, "invalid protocol operation");
            }
        }
    }

    debug!(%blkid, "processed block manifest");

    Ok(())
}

fn process_proto_op(
    state: &mut StateCache,
    block_mf: &L1BlockManifest,
    op: &ProtocolOperation,
    params: &RollupParams,
) -> Result<(), OpError> {
    match &op {
        ProtocolOperation::Checkpoint(ckpt) => {
            let epoch = ckpt.checkpoint().batch_info().epoch();
            debug!(%epoch, "processing checkpoint proto-op");
            process_l1_checkpoint(state, block_mf, ckpt, params)?;
        }

        ProtocolOperation::Deposit(info) => {
            let deposit_idx = info.deposit_idx;
            debug!(%deposit_idx, "processing deposit proto-op");
            process_l1_deposit(state, block_mf, info)?;
        }

        ProtocolOperation::WithdrawalFulfillment(info) => {
            let deposit_idx = info.deposit_idx;
            let txid = &info.txid;
            debug!(%deposit_idx, %txid, "processing withdrawal fulfillment proto-op");
            process_withdrawal_fulfillment(state, info)?;
        }

        ProtocolOperation::DepositSpent(info) => {
            let deposit_idx = info.deposit_idx;
            let txid = &info.deposit_idx;
            debug!(%deposit_idx, %txid, "processing reimbursement proto-op");
            process_deposit_spent(state, info)?;
        }

        // Other operations we don't do anything with for now.
        _ => {}
    }

    Ok(())
}

fn process_l1_checkpoint(
    state: &mut StateCache,
    src_block_mf: &L1BlockManifest,
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
    let old_fin_epoch = state.state().finalized_epoch();
    let new_fin_epoch = ckpt.batch_info().get_epoch_commitment();

    // TODO go through and do whatever stuff we need to do now that's finalized

    state.set_finalized_epoch(new_fin_epoch);
    trace!(?new_fin_epoch, "observed finalized checkpoint");

    Ok(())
}

fn process_l1_deposit(
    state: &mut StateCache,
    src_block_mf: &L1BlockManifest,
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
fn process_withdrawal_fulfillment(
    state: &mut StateCache,
    info: &WithdrawalFulfillmentInfo,
) -> Result<(), OpError> {
    if !state.check_deposit_exists(info.deposit_idx) {
        return Err(OpError::UnknownDeposit(info.deposit_idx));
    }

    state.mark_deposit_fulfilled(info);
    Ok(())
}

/// Locked deposit on L1 has been spent.
fn process_deposit_spent(state: &mut StateCache, info: &DepositSpendInfo) -> Result<(), OpError> {
    // Currently, we are not tracking how this was spent, only that it was.
    if !state.check_deposit_exists(info.deposit_idx) {
        return Err(OpError::UnknownDeposit(info.deposit_idx));
    }

    state.mark_deposit_reimbursed(info.deposit_idx);
    Ok(())
}

/// Advances the epoch bookkeeping, if this is first slot of new epoch.
fn advance_epoch_tracking(state: &mut StateCache) -> Result<(), TsnError> {
    if !state.should_finish_epoch() {
        return Ok(());
    }

    let prev_block = state.state().prev_block();
    let cur_epoch = state.state().cur_epoch();
    let ended_epoch = EpochCommitment::new(cur_epoch, prev_block.slot(), *prev_block.blkid());
    state.set_prev_epoch(ended_epoch);
    state.set_cur_epoch(cur_epoch + 1);
    state.set_epoch_finishing_flag(false);
    Ok(())
}

/// Checks the attested block IDs and parent blkid connections in new blocks.
// TODO unit tests
fn check_chain_integrity(
    cur_safe_height: u64,
    cur_safe_blkid: &L1BlockId,
    new_height: u64,
    new_blocks: &[L1BlockManifest],
) -> Result<(), TsnError> {
    // Check that the heights match.
    if new_height != cur_safe_height + new_blocks.len() as u64 {
        // This is basically right for both cases.
        return Err(TsnError::SkippedBlock);
    }

    // Iterate over all the blocks in the new list and make sure they match.
    for (i, e) in new_blocks.iter().enumerate() {
        let height = cur_safe_height + i as u64;

        // Make sure the hash matches.
        let computed_id = L1BlockId::compute_from_header_buf(e.header());
        let attested_id = e.record().blkid();
        if computed_id != *attested_id {
            return Err(TsnError::L1BlockIdMismatch(
                height,
                *attested_id,
                computed_id,
            ));
        }

        // Make sure matches parent.
        // TODO FIXME I think my impl for parent_blkid is incorrect, fix this later
        /*let blk_parent = e.record().parent_blkid();
        if i == 0 {
            if blk_parent != *pivot_blkid {
                return Err(TsnError::L1BlockParentMismatch(h, blk_parent, *pivot_blkid));
            }
        } else {
            let parent_payload = &new_blocks[i - 1];
            let parent_id = parent_payload.record().blkid();
            if blk_parent != *parent_id {
                return Err(TsnError::L1BlockParentMismatch(h, blk_parent, *parent_id));
            }
        }*/
    }

    Ok(())
}

/// Process an execution update, to change an exec env state.
///
/// This is meant to be kinda generic so we can reuse it across multiple exec
/// envs if we decide to go in that direction.
///
/// Note: As this is currently written, it assumes that the withdrawal state is
/// correct, which means that the sequencer kinda just gets to decide what the
/// withdrawals are.  Fortunately this is fine for now, since we're relying on
/// operators to also check all the parts of the state transition themselves,
/// including the EL payload itself.
///
/// Note: Currently this returns a ref to the withdrawal intents passed in the
/// exec update, but really it might need to be a ref into the state cache.
/// This will probably be substantially refactored in the future though.
fn process_execution_update<'u>(
    state: &mut StateCache,
    update: &'u exec_update::ExecUpdate,
) -> Result<&'u [WithdrawalIntent], TsnError> {
    // for all the ops, corresponding to DepositIntent, remove those DepositIntent the ExecEnvState
    let applied_ops = update.input().applied_ops();

    let applied_deposit_intent_idx = applied_ops
        .iter()
        .filter_map(|op| match op {
            Op::Deposit(deposit) => Some(deposit.intent_idx()),
            _ => None,
        })
        .max();

    if let Some(intent_idx) = applied_deposit_intent_idx {
        state.consume_deposit_intent(intent_idx);
    }

    Ok(update.output().withdrawals())
}

/// Iterates over the deposits table, making updates where needed.
///
/// Includes:
/// * Processes L1 withdrawals that are safe to dispatch to specific deposits.
/// * Reassigns deposits that have passed their deadling to new operators.
/// * Cleans up deposits that have been handled and can be removed.
fn process_deposit_updates(
    state: &mut StateCache,
    ready_withdrawals: &[WithdrawalIntent],
    rng: &mut SlotRng,
    params: &RollupParams,
) -> Result<(), TsnError> {
    // TODO make this capable of handling multiple denominations, have to decide
    // how those get represented first though

    let num_deposit_ents = state.state().deposits_table().len();

    // This determines how long we'll keep trying to service a withdrawal before
    // updating it or doing something else with it.  This is also what we use
    // when we decide to reset an assignment.
    let cur_block_height = state.state().l1_view().safe_height();
    let new_exec_height = cur_block_height as u32 + params.dispatch_assignment_dur;

    // Sequence in which we assign the operators to the deposits.  This is kinda
    // shitty because it might not account for available funds but it works for
    // devnet.
    //
    // TODO make this actually pick operators and not always use the first one,
    // this will be easier when we have operators able to reason about the funds
    // they have available on L1 on the rollup chain, perhaps a nomination queue
    //
    // TODO the way we pick assignees right now is a bit weird, we compute a
    // possible list for all possible new assignees, but then if we encounter a
    // deposit that needs reassignment we pick it directly at the time we need
    // it instead of taking it out of the precomputed table, this seems fine and
    // minimizes total calls to the RNG but feels odd since the order we pick the
    // numbers isn't the same as the order we've assigned
    let num_operators = state.state().operator_table().len();

    // A bit of a sanity check, but also idk it's weird to not have this.
    if num_operators == 0 {
        return Err(TsnError::NoOperators);
    }

    let ops_seq = (0..ready_withdrawals.len())
        .map(|_| next_rand_op_pos(rng, num_operators))
        .collect::<Vec<_>>();

    let mut next_intent_to_assign = 0;
    let mut deposit_idxs_to_remove = Vec::new();

    for deposit_entry_idx in 0..num_deposit_ents {
        let ent = state
            .state()
            .deposits_table()
            .get_entry_at_pos(deposit_entry_idx)
            .expect("chaintsn: inconsistent state");
        let deposit_idx = ent.idx();

        let have_ready_intent = next_intent_to_assign < ready_withdrawals.len();

        match ent.deposit_state() {
            DepositState::Created(_) => {
                // TODO I think we can remove this state
            }

            DepositState::Accepted => {
                // If we have an intent to assign, we can dispatch it to this deposit.
                if have_ready_intent {
                    let intent = &ready_withdrawals[next_intent_to_assign];
                    let op_idx = ops_seq[next_intent_to_assign % ops_seq.len()];

                    let outp = WithdrawOutput::new(intent.destination().clone(), *intent.amt());
                    let withdrawal_txid = *intent.withdrawal_txid();
                    let cmd = DispatchCommand::new(vec![outp]);
                    state.assign_withdrawal_command(
                        deposit_idx,
                        op_idx,
                        cmd,
                        new_exec_height as u64,
                        withdrawal_txid,
                    );

                    next_intent_to_assign += 1;
                }
            }

            DepositState::Dispatched(dstate) => {
                // Check if the deposit is past the threshold.
                if cur_block_height >= dstate.exec_deadline() {
                    // Pick the next assignee, if there are any.
                    let new_op_pos = if num_operators > 1 {
                        // Compute a random offset from 1 to (num_operators - 1),
                        // ensuring we pick a different operator than the current one.
                        let offset = 1 + (rng.next_u32() % (num_operators - 1));
                        (dstate.assignee() + offset) % num_operators
                    } else {
                        // If there is only a single operator, we remain with the current assignee.
                        dstate.assignee()
                    };

                    // Convert their position in the table to their global index.
                    let op_idx = state
                        .state()
                        .operator_table()
                        .get_entry_at_pos(new_op_pos)
                        .expect("chaintsn: inconsistent state")
                        .idx();

                    state.reset_deposit_assignee(deposit_idx, op_idx, new_exec_height as u64);
                }
            }

            DepositState::Fulfilled(_) => {
                // dont reassign executing withdrawals as front payment has been done.
                // nothing else to do here for now
            }

            DepositState::Reimbursed => {
                deposit_idxs_to_remove.push(deposit_idx);
            }
        }
    }

    // Sanity check.  For devnet this should never fail since we should never be
    // able to withdraw more than was deposited, so we should never run out of
    // deposits to assign withdrawals to.
    if next_intent_to_assign != ready_withdrawals.len() {
        return Err(TsnError::InsufficientDepositsForIntents);
    }

    // TODO remove stale deposit idxs

    Ok(())
}

/// Wrapper to safely select a random operator index using wide reduction
/// This will return a deterministically-random index in the range `[0, num)`
fn next_rand_op_pos(rng: &mut SlotRng, num: u32) -> u32 {
    // This won't meaningfully truncate since `num` is `u32`
    (rng.next_u64() % (num as u64)) as u32
}

#[cfg(test)]
mod tests {
    use rand_core::SeedableRng;
    use strata_primitives::{
        buf::Buf32,
        l1::{
            BitcoinAmount, DepositInfo, DepositUpdateTx, L1BlockManifest, L1HeaderRecord, L1Tx,
            ProtocolOperation,
        },
        l2::L2BlockId,
        params::OperatorConfig,
    };
    use strata_state::{
        block::{ExecSegment, L1Segment, L2BlockBody},
        bridge_state::OperatorTable,
        chain_state::Chainstate,
        exec_env::ExecEnvState,
        exec_update::{ExecUpdate, UpdateInput, UpdateOutput},
        genesis::GenesisStateData,
        header::{L2BlockHeader, L2Header},
        l1::L1ViewState,
        state_op::StateCache,
    };
    use strata_test_utils::{l2::gen_params, ArbitraryGenerator};

    use super::{next_rand_op_pos, process_block};
    use crate::{slot_rng::SlotRng, transition::process_l1_view_update};

    #[test]
    // Confirm that operator index sampling is deterministic and in bounds
    fn deterministic_index_sampling() {
        let num = 123;
        let mut rng = SlotRng::from_seed([1u8; 32]);
        let mut same_rng = SlotRng::from_seed([1u8; 32]);

        let index = next_rand_op_pos(&mut rng, num);
        let same_index = next_rand_op_pos(&mut same_rng, num);

        assert_eq!(index, same_index);
        assert!(index < num);
    }

    #[test]
    fn test_process_l1_view_update_with_empty_payload() {
        let chs: Chainstate = ArbitraryGenerator::new().generate();
        let params = gen_params();

        let mut state_cache = StateCache::new(chs.clone());

        // Empty L1Segment payloads
        let l1_segment = L1Segment::new_empty(chs.l1_view().safe_height());

        // let previous_maturation_queue =
        // Process the empty payload
        let result = process_l1_view_update(&mut state_cache, &l1_segment, params.rollup());
        assert_eq!(state_cache.state(), &chs);
        assert!(result.is_ok());
    }
}
