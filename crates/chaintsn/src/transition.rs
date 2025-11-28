//! Top-level CL state transition logic.  This is largely stubbed off now, but
//! we'll replace components with real implementations as we go along.

use rand_core::{RngCore, SeedableRng};
use strata_asm_types::{
    DepositInfo, DepositSpendInfo, L1BlockManifest, ProtocolOperation, WithdrawalFulfillmentInfo,
};
use strata_bridge_types::{
    DepositIntent, DepositState, DispatchCommand, WithdrawOutput, WithdrawalIntent,
};
use strata_checkpoint_types::{verify_signed_checkpoint_sig, Checkpoint, SignedCheckpoint};
use strata_ol_chain_types::{L2BlockBody, L2BlockHeader, L2Header};
use strata_ol_chainstate_types::StateCache;
use strata_params::RollupParams;
use strata_predicate::PredicateResult;
use strata_primitives::{epoch::EpochCommitment, l1::L1BlockId, l2::L2BlockCommitment};
use strata_state::exec_update::{self, Op};
use tracing::warn;

use crate::{
    checkin::{process_l1_view_update, SegmentAuxData},
    context::StateAccessor,
    errors::{OpError, TsnError},
    legacy::FauxStateCache,
    macros::*,
    slot_rng::SlotRng,
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
    header: &L2BlockHeader,
    body: &L2BlockBody,
    params: &RollupParams,
) -> Result<(), TsnError> {
    let mut rng = compute_init_slot_rng(state);

    // Update basic bookkeeping.
    let prev_tip_slot = state.state_untracked().chain_tip_slot();
    let prev_tip_blkid = header.parent();
    state.set_slot(header.slot());
    state.set_prev_block(L2BlockCommitment::new(prev_tip_slot, *prev_tip_blkid));
    advance_epoch_tracking(state)?;
    // TODO: Fixme
    // if state.state_untracked().cur_epoch() != header.parent_header().epoch() {
    //     return Err(TsnError::MismatchEpoch(
    //         header.parent_header().epoch(),
    //         state.state_untracked().cur_epoch(),
    //     ));
    // }

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

#[allow(dead_code, clippy::allow_attributes, reason = "used for chaintsn")]
fn process_l1_block(
    state: &mut StateCache,
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

fn process_proto_op(
    state: &mut StateCache,
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

fn process_l1_checkpoint(
    state: &mut StateCache,
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

    let _receipt = ckpt.construct_receipt();
    let fin_epoch = state.state().finalized_epoch();

    // Note: This is error because this is done by the sequencer
    if !is_checkpoint_null(ckpt, fin_epoch) && ckpt_epoch != fin_epoch.epoch() + 1 {
        error!(%ckpt_epoch, "Invalid checkpoint: proof for invalid epoch");
        return Err(OpError::EpochNotExtend);
    }

    // Also just check if the epoch numbers in batch transition and batch info match
    if ckpt_epoch != ckpt.batch_info().epoch() {
        return Err(OpError::MalformedCheckpoint);
    }
    verify_checkpoint_proof(ckpt, params).map_err(|_| OpError::InvalidProof)?;

    // Copy the epoch commitment and make it finalized.
    let _old_fin_epoch = state.state().finalized_epoch();
    let new_fin_epoch = ckpt.batch_info().get_epoch_commitment();

    // TODO go through and do whatever stuff we need to do now that's finalized

    state.set_finalized_epoch(new_fin_epoch);
    trace!(?new_fin_epoch, "observed finalized checkpoint");

    Ok(())
}

/// Verify that the provided checkpoint proof is valid for the given params.
///
/// # Caution
///
/// If the checkpoint proof is empty, this function returns an `Ok(())`.
// FIXME this does not belong here, it should be in a more general module probably
pub fn verify_checkpoint_proof(
    checkpoint: &Checkpoint,
    rollup_params: &RollupParams,
) -> PredicateResult<()> {
    let checkpoint_idx = checkpoint.batch_info().epoch();
    let proof_receipt = checkpoint.construct_receipt();

    // FIXME: we are accepting empty proofs for now (devnet) to reduce dependency on the prover
    // infra.
    let is_empty_proof = proof_receipt.proof().is_empty();
    let allow_empty = rollup_params.proof_publish_mode.allow_empty();

    if is_empty_proof && allow_empty {
        warn!(%checkpoint_idx, "Verifying empty proof as correct");
        return Ok(());
    }

    rollup_params.checkpoint_predicate().verify_claim_witness(
        proof_receipt.public_values().as_bytes(),
        proof_receipt.proof().as_bytes(),
    )
}

/// Checks if the given checkpoint is null based on previous finalized epoch. A checkpoint is
/// considered null epoch if and only if it's epoch is 0 and the state's finalized epoch is 0.
/// Note that for null epoch we don't do continuity check.
fn is_checkpoint_null(ckpt: &Checkpoint, finalized_epoch: &EpochCommitment) -> bool {
    let ckpt_epoch = ckpt.batch_transition().epoch;
    // Checkpoint is null only if its epoch is 0 and the state's finalized epoch is 0.
    ckpt_epoch == 0 && finalized_epoch.epoch() == 0
}

fn process_l1_deposit(
    state: &mut StateCache,
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
/// Mark the withdrawal as being executed and prevent reassignment to another operator.
fn process_withdrawal_fulfillment(
    state: &mut StateCache,
    info: &WithdrawalFulfillmentInfo,
) -> Result<(), OpError> {
    if !state.is_valid_withdrawal_fulfillment(info.deposit_idx) {
        return Err(OpError::InvalidDeposit {
            deposit_idx: info.deposit_idx,
            operator_idx: 0, // Placeholder, actual operator is derived from state
        });
    }

    state.mark_deposit_fulfilled(info);
    Ok(())
}

/// Locked deposit on L1 has been spent.
fn process_deposit_spent(state: &mut StateCache, info: &DepositSpendInfo) -> Result<(), OpError> {
    // Currently, we are not tracking how this was spent, only that it was.

    state.mark_deposit_reimbursed(info.deposit_idx);
    Ok(())
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
#[expect(dead_code, reason = "used for chaintsn")]
fn check_chain_integrity(
    cur_safe_height: u64,
    _cur_safe_blkid: &L1BlockId,
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
            #[expect(unreachable_patterns, reason = "unreachable")]
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
                if cur_block_height >= dstate.fulfillment_deadline() {
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
    use strata_asm_types::{L1BlockManifest, L1Tx, ProtocolOperation, WithdrawalFulfillmentInfo};
    use strata_bridge_types::{DepositState, DepositsTable, DispatchCommand, DispatchedState};
    use strata_ol_chainstate_types::{Chainstate, StateCache};
    use strata_primitives::{buf::Buf32, l1::BitcoinAmount};
    use strata_test_utils::ArbitraryGenerator;
    use strata_test_utils_l2::gen_params;

    use super::next_rand_op_pos;
    use crate::{slot_rng::SlotRng, transition::process_l1_block};

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

    // #[test]
    // fn test_process_l1_view_update_with_empty_payload() {
    //     let chs: Chainstate = ArbitraryGenerator::new().generate();
    //     let params = gen_params();

    //     let mut state_cache = MemStateAccessor::new(chs);

    //     // Empty L1Segment payloads
    //     let l1_segment = L1Segment::new_empty(chs.l1_view().safe_height());

    //     // let previous_maturation_queue =
    //     // Process the empty payload
    //     let result = process_l1_view_update(&mut state_cache, &l1_segment, params.rollup());
    //     assert_eq!(state_cache.state(), &chs);
    //     assert!(result.is_ok());
    // }

    #[test]
    fn test_process_l1_block_with_duplicate_withdrawal_fulfillment() {
        let mut arb = ArbitraryGenerator::new();

        // Setup chainstate with a deposit in dispatched state
        let mut chs: Chainstate = arb.generate();
        let mut deposit_table = DepositsTable::new_empty();
        deposit_table.create_next_deposit(
            arb.generate(),
            vec![0, 1, 2],
            BitcoinAmount::from_int_btc(10),
        );
        let deposit = deposit_table.get_deposit_mut(0).expect("should exist");
        deposit.set_state(DepositState::Dispatched(DispatchedState::new(
            DispatchCommand::new(vec![]),
            0,
            100,
        )));
        *chs.deposits_table_mut() = deposit_table;

        let params = gen_params();

        // withdrawal fulfillment op that will be seen twice
        let withdrawal_fulfillment_protocol_op =
            ProtocolOperation::WithdrawalFulfillment(WithdrawalFulfillmentInfo {
                deposit_idx: 0,
                amt: BitcoinAmount::from_int_btc(10),
                txid: Buf32::zero(),
            });

        // send protocol op for first time. Should set deposit to fulfilled state
        let block_manifest = L1BlockManifest::new(
            arb.generate(),
            vec![L1Tx::new(
                arb.generate(),
                arb.generate(),
                vec![withdrawal_fulfillment_protocol_op.clone()],
            )],
            chs.cur_epoch(),
            chs.l1_view().safe_height() + 1,
        );

        let mut state_cache = StateCache::new(chs.clone());
        let result = process_l1_block(&mut state_cache, &block_manifest, params.rollup());
        assert!(result.is_ok(), "test: invoke process_l1_block");
        let new_deposit_state = state_cache
            .state()
            .deposits_table()
            .get_deposit(0)
            .expect("should exist")
            .deposit_state();
        assert!(
            matches!(new_deposit_state, DepositState::Fulfilled(_)),
            "test: deposit state not set to fulfilled"
        );

        // Duplicate withdrawal fulfillment seen. Should be ignored and chainstate not updated
        let block_manifest = L1BlockManifest::new(
            arb.generate(),
            vec![L1Tx::new(
                arb.generate(),
                arb.generate(),
                vec![withdrawal_fulfillment_protocol_op],
            )],
            chs.cur_epoch(),
            chs.l1_view().safe_height() + 1,
        );

        let chs = state_cache.state();
        let mut state_cache = StateCache::new(chs.clone());
        let result = process_l1_block(&mut state_cache, &block_manifest, params.rollup());
        assert!(result.is_ok(), "test: invoke process_l1_block");
        assert_eq!(
            state_cache.state(),
            chs,
            "test: chainstate changed unexpectedly"
        );
    }
}
