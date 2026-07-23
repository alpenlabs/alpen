//! Block transactional processing.

use ssz::Decode;
use strata_acct_types::*;
use strata_ledger_types::*;
use strata_msg_fmt::{Msg, MsgRef};
use strata_ol_chain_types::*;
use strata_ol_msg_types::PREDICATE_UPDATE_MSG_TYPE_ID;
use strata_predicate::PredicateKey;
use tracing::{info, trace};

use crate::{
    OutputCtx, account_processing,
    constants::SEQUENCER_ACCT_ID,
    context::{BasicExecContext, TxExecContext},
    errors::{ExecError, ExecResult},
    msg_payload_coin::MsgPayloadCoin,
    proof_verification::{TxProofVerificationContext, TxProofVerifierImpl, TxProofsTracker},
    sau_processing,
};

/// Process a block's transaction segment.
///
/// This is called for every block.
#[tracing::instrument(skip_all, fields(tx_count = tx_seg.txs().len()))]
pub fn process_block_tx_segment<S: IStateAccessorMut>(
    state: &mut S,
    tx_seg: &OLTxSegment,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    for (i, tx) in tx_seg.txs().iter().enumerate() {
        let txid = tx.compute_txid();
        trace!(index = i, %txid, kind = %tx.payload().type_id(), "processing tx");
        process_single_tx(state, tx, context).map_err(|e| e.with_tx(txid, i))?
    }

    Ok(())
}

/// Processes a single tx, typically as part of a block.
///
/// This can also be used in mempool logic for trying to figure out if we can
/// apply a tx into a block.
pub fn process_single_tx<S: IStateAccessorMut>(
    state: &mut S,
    tx: &OLTransaction,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    // 1. Check the transaction's constraints.
    check_tx_constraints(tx.constraints(), state)?;

    // 2. Depending on its payload type, we handle it different ways.
    let sender_acct = match tx.payload() {
        TransactionPayload::GenericAccountMessage(gam_payload) => {
            verify_gam_tx(gam_payload, tx.data().effects())?;
            SEQUENCER_ACCT_ID
        }

        TransactionPayload::SnarkAccountUpdate(sau_payload) => {
            let target = *sau_payload.target();
            let effects = tx.data().effects();
            let tx_proofs = tx.proofs();

            // Call out to verify the update.
            process_update_tx(state, sau_payload, effects, tx_proofs, context)?;

            target
        }
    };

    // 3. Apply effects.
    apply_tx_effects(
        state,
        sender_acct,
        tx.data().effects(),
        context.basic_context(),
    )?;

    Ok(())
}

fn verify_gam_tx(gam: &GamTxPayload, fx: &TxEffects) -> ExecResult<()> {
    // 1. Check that we're not sending any value via transfers.
    if fx.transfers_iter().count() != 0 {
        return Err(ExecError::TxStructureCheckFailed("nonzero transfers"));
    }

    // 2. Extract the message we want to send.
    let mut msgs_iter = fx.messages_iter();
    let msg = match (msgs_iter.next(), msgs_iter.next()) {
        (Some(m), None) if m.payload().value().is_zero() => m,
        _ => {
            return Err(ExecError::TxStructureCheckFailed(
                "multiple messages or nonzero value",
            ));
        }
    };

    // This is weird, it should make more sense when accounts have senders.
    if msg.dest() != *gam.target() {
        return Err(ExecError::TxStructureCheckFailed("mismatched target"));
    }

    Ok(())
}

fn process_update_tx<S: IStateAccessorMut>(
    state: &mut S,
    sau_payload: &SauTxPayload,
    effects: &TxEffects,
    tx_proofs: &TxProofs,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    // 1. Read account state and verify effects are safe to apply.
    let target = *sau_payload.target();
    let account_state = state
        .get_account_state(target)?
        .ok_or(ExecError::UnknownAccount(target))?;

    verify_effects_safe(effects, state, account_state)?;

    // 2. Verify the update by calling out to the snark account library.
    let state_ctx = TxProofVerificationContext::from_account_and_state(state, account_state);
    let proof_tracker = TxProofsTracker::from_txproofs(tx_proofs);
    let mut verifier = TxProofVerifierImpl::new(state_ctx, proof_tracker);
    sau_processing::verify_snark_acct_update_proofs(
        target,
        account_state,
        sau_payload.operation(),
        effects,
        &mut verifier,
    )?;
    if !verifier.is_exhausted() {
        return Err(ExecError::Acct(AcctError::InvalidUpdateProof {
            account_id: target,
        }));
    }

    // 3. Actually take balance and write new account inner state.
    let serial = account_state.serial();
    let op = sau_payload.operation();
    let upd = op.update();
    // Predicate rotations activate on consumption: if this update consumed
    // admin predicate-update messages, the account's update VK rotates now.
    // The update consuming the message is therefore the last one verified
    // under the old key, per the Alpen upgrade design. Later rotations in
    // the same update win, preserving admin ordering.
    let consumed_rotations: Vec<PredicateKey> = op
        .messages_iter()
        .filter_map(parse_predicate_update_message)
        .collect();
    state.update_account(target, |astate| -> ExecResult<_> {
        // SAFETY: These panics are checked ahead of time so can never get hit.

        // Extract the snark account state so we can modify it.
        let acct_tstate = astate
            .as_snark_account_mut()
            .expect("ol/stf: account changed type");

        let new_seqno = upd
            .seq_no()
            .checked_add(1)
            .ok_or(ExecError::MaxSeqNumberReached { account_id: target })?;
        acct_tstate.set_proof_state(
            upd.proof_state().inner_state_root(),
            upd.proof_state().new_next_msg_idx(),
            new_seqno.into(),
        );

        for new_vk in consumed_rotations {
            info!(account_id = %target, "activating consumed predicate key rotation");
            acct_tstate.set_update_vk(new_vk);
        }

        Ok(())
    })??;

    // Emit log after successful update.
    let log = upd
        .get_log_data()
        .expect("extra_data bounded by SSZ MAX_EXTRA_DATA_BYTES(1024) exceeds VarVec bound");
    context.emit_typed_log(serial, &log)?;
    Ok(())
}

/// Applies the effects of a transaction (transfers and messages) to account state.
pub(crate) fn apply_tx_effects<S: IStateAccessorMut>(
    state: &mut S,
    source: AccountId,
    effects: &TxEffects,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    let total_sent = effects
        .get_total_value_sent()
        .ok_or(ExecError::AmountOverflow)?;

    // 1. Debit the source once, obtaining a single coin to split across the
    // effects.
    let mut remaining = debit_source(state, source, total_sent)?;

    // 2. Send funds out, splitting the debited coin per effect.  Each per-effect
    // coin is owned by the callee, which consumes it on every path; if a call
    // errors we still hold `remaining`, so we defuse it before propagating rather
    // than dropping a live coin (the whole STF is discarded on that error, so no
    // value is lost).
    if let Err(e) = distribute_effects(state, source, effects, context, &mut remaining) {
        remaining.safely_consume_unchecked();
        return Err(e);
    }

    // Everything debited has been distributed; the remainder must be zero.
    remaining.consume_zero();

    Ok(())
}

/// Distributes `remaining` across the transaction's effects, splitting a coin
/// off for each transfer and message so Rust enforces that the pieces sum to
/// exactly what was debited.
///
/// On error, `remaining` is left holding the undistributed value for the caller
/// to dispose of.
fn distribute_effects<S: IStateAccessorMut>(
    state: &mut S,
    source: AccountId,
    effects: &TxEffects,
    context: &BasicExecContext<'_>,
    remaining: &mut Coin,
) -> ExecResult<()> {
    for t in effects.transfers_iter() {
        let coin = split_effect_value(remaining, t.value());
        account_processing::process_transfer(state, source, t.dest(), coin, context)?;
    }

    for m in effects.messages_iter() {
        let coin = split_effect_value(remaining, m.payload().value());
        let msg = MsgPayloadCoin::new(coin, m.payload().payload_data().clone());
        account_processing::process_message(state, source, m.dest(), msg, context)?;
    }

    Ok(())
}

/// Debits `total_sent` from the source account, returning it as a single
/// [`Coin`] to be distributed across the transaction's effects.
///
/// A zero total needs no debit and yields an empty coin.  This also covers the
/// magic sequencer account, which is never a real ledger account: it only
/// appears for GAM txs, which we separately require to send zero value, so we
/// never attempt to debit it.
fn debit_source<S: IStateAccessorMut>(
    state: &mut S,
    source: AccountId,
    total_sent: BitcoinAmount,
) -> ExecResult<Coin> {
    if total_sent.is_zero() {
        return Ok(Coin::zero());
    }

    state.update_account(source, |astate| -> ExecResult<Coin> {
        Ok(astate.take_balance(total_sent)?)
    })?
}

/// Splits an effect's value off of the debited coin.
///
/// The value is guaranteed to fit because [`verify_effects_safe`] checks the
/// per-effect values sum to `total_sent` before the debit, so this failing
/// signals an accounting bug rather than bad input.
fn split_effect_value(remaining: &mut Coin, value: BitcoinAmount) -> Coin {
    remaining
        .split_out(value)
        .expect("ol/stf: effect value exceeds debited total")
}

/// Checks that a tx's constraints are valid for the current slot in state.
pub fn check_tx_constraints<S: IStateAccessorMut>(
    constraints: &TxConstraints,
    state: &S,
) -> ExecResult<()> {
    let current_slot = state.cur_slot();

    if let Some(min_slot) = constraints.min_slot()
        && current_slot < min_slot
    {
        return Err(ExecError::TransactionNotMature(min_slot, current_slot));
    }

    if let Some(max_slot) = constraints.max_slot()
        && current_slot > max_slot
    {
        return Err(ExecError::TransactionExpired(max_slot, current_slot));
    }

    Ok(())
}

/// Verifies if the [`TxEffects`] from a tx are "safe" to apply, given the current account
/// state and some ledger context.
///
/// Specifically, it checks that all the destinations can receive the outputs
/// and that we don't overdraw the account.
pub fn verify_effects_safe<S: IStateAccessorMut>(
    fx: &TxEffects,
    state: &S,
    acct: &S::AccountState,
) -> ExecResult<()> {
    let mut total_sent = BitcoinAmount::zero();

    // We're actually making the same checks in both places, so we can chain the
    // iterators like this.
    let outp_iter = fx
        .transfers_iter()
        .map(|t| (t.dest(), t.value()))
        .chain(fx.messages_iter().map(|m| (m.dest(), m.payload().value())));

    for (dest, amt) in outp_iter {
        if !dest.is_special() && !state.check_account_exists(dest)? {
            return Err(ExecError::UnknownAccount(dest));
        }

        total_sent = total_sent
            .checked_add(amt)
            .ok_or(ExecError::AmountOverflow)?;
    }

    if total_sent > acct.balance() {
        return Err(ExecError::BalanceUnderflow);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::make_account_id;

    fn make_gam_payload(target_acct_id: AccountId) -> GamTxPayload {
        GamTxPayload::new(target_acct_id).expect("test target_acct_id should be valid")
    }

    fn push_test_message(
        effects: &mut TxEffects,
        dest: AccountId,
        value_sat: u64,
        data: Vec<u8>,
    ) -> bool {
        effects
            .push_message(dest, value_sat, data)
            .expect("test message payload should fit within SSZ max length")
    }

    fn assert_gam_structure_error(
        target_acct_id: AccountId,
        effects: &TxEffects,
        expected_reason: &'static str,
    ) {
        let err = verify_gam_tx(&make_gam_payload(target_acct_id), effects)
            .expect_err("invalid GAM structure should fail");
        assert!(matches!(
            err,
            ExecError::TxStructureCheckFailed(reason) if reason == expected_reason
        ));
    }

    #[test]
    fn test_verify_gam_tx_accepts_single_zero_message_to_target() {
        let target_acct_id = make_account_id(1);
        let mut effects = TxEffects::default();
        assert!(push_test_message(
            &mut effects,
            target_acct_id,
            0,
            vec![1, 2, 3]
        ));

        verify_gam_tx(&make_gam_payload(target_acct_id), &effects)
            .expect("valid GAM effects should pass");
    }

    #[test]
    fn test_verify_gam_tx_rejects_transfer_effects() {
        let target_acct_id = make_account_id(1);
        let mut effects = TxEffects::default();
        assert!(effects.push_transfer(target_acct_id, 1));
        assert!(push_test_message(&mut effects, target_acct_id, 0, vec![]));

        assert_gam_structure_error(target_acct_id, &effects, "nonzero transfers");
    }

    #[test]
    fn test_verify_gam_tx_rejects_missing_message() {
        let target_acct_id = make_account_id(1);
        let effects = TxEffects::default();

        assert_gam_structure_error(
            target_acct_id,
            &effects,
            "multiple messages or nonzero value",
        );
    }

    #[test]
    fn test_verify_gam_tx_rejects_multiple_messages() {
        let target_acct_id = make_account_id(1);
        let mut effects = TxEffects::default();
        assert!(push_test_message(&mut effects, target_acct_id, 0, vec![1]));
        assert!(push_test_message(&mut effects, target_acct_id, 0, vec![2]));

        assert_gam_structure_error(
            target_acct_id,
            &effects,
            "multiple messages or nonzero value",
        );
    }

    #[test]
    fn test_verify_gam_tx_rejects_nonzero_message_value() {
        let target_acct_id = make_account_id(1);
        let mut effects = TxEffects::default();
        assert!(push_test_message(&mut effects, target_acct_id, 1, vec![]));

        assert_gam_structure_error(
            target_acct_id,
            &effects,
            "multiple messages or nonzero value",
        );
    }

    #[test]
    fn test_verify_gam_tx_rejects_mismatched_message_target() {
        let target_acct_id = make_account_id(1);
        let message_dest_acct_id = make_account_id(2);
        let mut effects = TxEffects::default();
        assert!(push_test_message(
            &mut effects,
            message_dest_acct_id,
            0,
            vec![]
        ));

        assert_gam_structure_error(target_acct_id, &effects, "mismatched target");
    }
}

/// Parses an admin predicate-update message consumed by an account update.
///
/// Only messages sourced from [`ADMIN_MSG_ACCT_ID`] are honored — the source
/// of an inbox message is the authenticated sender account, and the admin id
/// is reserved, so ordinary accounts cannot forge a rotation.
fn parse_predicate_update_message(entry: &MessageEntry) -> Option<PredicateKey> {
    if entry.source() != ADMIN_MSG_ACCT_ID {
        return None;
    }
    let msg = MsgRef::try_from(entry.payload_buf()).ok()?;
    if msg.ty() != PREDICATE_UPDATE_MSG_TYPE_ID {
        return None;
    }
    PredicateKey::from_ssz_bytes(msg.body()).ok()
}
