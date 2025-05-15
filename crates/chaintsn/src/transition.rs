//! Top-level CL state transition logic.  This is largely stubbed off now, but
//! we'll replace components with real implementations as we go along.
#![allow(unused)]

use std::{cmp::max, collections::HashMap};

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
    checkin::{process_l1_view_update, SegmentAuxData},
    context::{BlockHeaderContext, StateAccessor},
    errors::{OpError, TsnError},
    legacy::FauxStateCache,
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
    state: &mut impl StateAccessor,
    header: &impl BlockHeaderContext,
    body: &L2BlockBody,
    params: &RollupParams,
) -> Result<(), TsnError> {
    let mut rng = compute_init_slot_rng(state);

    // Update basic bookkeeping.
    let prev_tip_slot = state.state_untracked().chain_tip_slot();
    let prev_tip_blkid = header.parent_blkid();
    state.set_slot(header.slot());
    state.set_prev_block(L2BlockCommitment::new(prev_tip_slot, *prev_tip_blkid));
    advance_epoch_tracking(state)?;
    if state.state_untracked().cur_epoch() != header.parent_header().epoch() {
        return Err(TsnError::MismatchEpoch(
            header.parent_header().epoch(),
            state.state_untracked().cur_epoch(),
        ));
    }

    // Go through each stage and play out the operations it has.
    //
    // For now, we have to wrap these calls in some annoying bookkeeping while/
    // we transition to the new context traits.
    let cur_l1_height = state.state_untracked().l1_view().safe_height();
    let l1_prov = SegmentAuxData::new(cur_l1_height + 1, body.l1_segment());
    let mut faux_sc = FauxStateCache::new(state);
    let has_new_epoch = process_l1_view_update(&mut faux_sc, &l1_prov, params)?;
    let ready_withdrawals = process_execution_update(&mut faux_sc, body.exec_segment().update())?;
    process_deposit_updates(&mut faux_sc, ready_withdrawals, &mut rng, params)?;

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
fn compute_init_slot_rng(state: &impl StateAccessor) -> SlotRng {
    // Just take the last block's slot.
    let blkid_buf = *state.prev_block().blkid().as_ref();
    SlotRng::from_seed(blkid_buf)
}

/// Advances the epoch bookkeeping, if this is first slot of new epoch.
fn advance_epoch_tracking(state: &mut impl StateAccessor) -> Result<(), TsnError> {
    if !state.epoch_finishing_flag() {
        return Ok(());
    }

    let prev_block = state.state_untracked().prev_block();
    let cur_epoch = state.state_untracked().cur_epoch();
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
fn process_execution_update<'s, 'u, S: StateAccessor>(
    state: &mut FauxStateCache<'s, S>,
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
fn process_deposit_updates<'s, S: StateAccessor>(
    state: &mut FauxStateCache<'s, S>,
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
    use crate::{checkin::process_l1_view_update, slot_rng::SlotRng};

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
