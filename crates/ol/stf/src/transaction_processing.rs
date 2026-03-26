//! Block transactional processing.

use strata_acct_types::*;
use strata_ledger_types::*;
use strata_ol_chain_types_new::*;
use strata_snark_acct_sys as snark_sys;
use strata_snark_acct_types::{LedgerRefs, ProofState, Seqno};

use crate::{
    account_processing,
    constants::SEQUENCER_ACCT_ID,
    context::{BasicExecContext, TxExecContext},
    errors::{ExecError, ExecResult},
    proof_verification::{TxProofVerificationContext, TxProofVerifierImpl, TxProofsTracker},
};

/// Process a block's transaction segment.
///
/// This is called for every block.
pub fn process_block_tx_segment<S: IStateAccessor>(
    state: &mut S,
    tx_seg: &OLTxSegment,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    for (i, tx) in tx_seg.txs().iter().enumerate() {
        process_single_tx(state, tx, context).map_err(|e| e.with_tx(tx.compute_txid(), i))?
    }

    Ok(())
}

/// Processes a single tx, typically as part of a block.
///
/// This can also be used in mempool logic for trying to figure out if we can
/// apply a tx into a block.
pub fn process_single_tx<S: IStateAccessor>(
    state: &mut S,
    tx: &OLTransaction,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    // 1. Check the transaction's constraints.
    check_tx_constraints(tx.constraints(), state)?;

    // 2. Depending on its payload type, we handle it different ways.
    match tx.payload() {
        TransactionPayload::GenericAccountMessage(gam_payload) => {
            process_gam_tx(state, gam_payload, tx.data().effects(), context)?;
        }

        TransactionPayload::SnarkAccountUpdate(sau_payload) => {
            let target = *sau_payload.target();
            let effects = tx.data().effects();
            let tx_proofs = tx.proofs();

            process_update_tx(state, target, sau_payload, effects, tx_proofs, context)?;
        }
    }

    Ok(())
}

fn process_gam_tx<S: IStateAccessor>(
    state: &mut S,
    gam: &GamTxPayload,
    fx: &TxEffects,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    // Check that we're not sending any value via transfers.
    if fx.transfers_iter().count() != 0 {
        return Err(ExecError::TxStructureCheckFailed("nonzero transfers"));
    }

    // Extract the message we want to send.
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

    // Hand off the message we want to send.
    account_processing::process_message(
        state,
        SEQUENCER_ACCT_ID,
        *gam.target(),
        msg.payload().clone(),
        context.basic_context(),
    )?;

    Ok(())
}

fn process_update_tx<S: IStateAccessor>(
    state: &mut S,
    target: AccountId,
    sau_payload: &SauTxPayload,
    effects: &TxEffects,
    tx_proofs: &TxProofs,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    // 1. Read account state and verify effects are safe to apply.
    let account_state = state
        .get_account_state(target)?
        .ok_or(ExecError::UnknownAccount(target))?;
    let snark_acct_state = account_state
        .as_snark_account()
        .map_err(|_| ExecError::IncorrectTxTargetType)?;

    verify_effects_safe(effects, state, account_state)?;

    // 2. Build SnarkAccountUpdateData from the new tx types.
    let op = sau_payload.operation();
    let upd = op.update();
    let proof_state = ProofState::new(
        upd.proof_state().inner_state_root(),
        upd.proof_state().new_next_msg_idx(),
    );
    let ledger_refs = convert_sau_ledger_refs(op.ledger_refs());
    let processed_messages: Vec<_> = op.messages_iter().cloned().collect();

    let update_data = snark_sys::SnarkAccountUpdateData::new(
        Seqno::from(upd.seq_no()),
        proof_state,
        processed_messages,
        ledger_refs,
        effects.clone(),
        upd.extra_data().to_vec(),
    );

    // 3. Verify the update by calling out to the snark account library.
    let state_ctx = TxProofVerificationContext::from_account_and_state(state, account_state);
    let proof_tracker = TxProofsTracker::from_txproofs(tx_proofs);
    let mut verifier = TxProofVerifierImpl::new(state_ctx, proof_tracker);
    snark_sys::verify_update_correctness(target, snark_acct_state, &update_data, &mut verifier)?;

    // 4. Actually take balance and write new account inner state.
    state.update_account(target, |astate| -> ExecResult<_> {
        // SAFETY: These panics are checked ahead of time so can never get hit.

        // Deduct balance for all effects first.
        let total_sent = effects.get_total_value_sent().unwrap();
        let coin = astate
            .take_balance(total_sent)
            .expect("ol/stf: account changed balance");
        coin.safely_consume_unchecked(); // maybe we'll use this in the future

        // Extract the snark account state so we can modify it.
        let acct_tstate = astate
            .as_snark_account_mut()
            .expect("ol/stf: account changed type");

        let new_seqno = upd
            .seq_no()
            .checked_add(1)
            .ok_or(ExecError::MaxSeqNumberReached { account_id: target })?;
        acct_tstate.update_inner_state(
            upd.proof_state().inner_state_root(),
            upd.proof_state().new_next_msg_idx(),
            new_seqno.into(),
            upd.extra_data(),
        )?;

        Ok(())
    })??;

    // Step 5: Apply effects.
    apply_tx_effects(state, target, effects, context.basic_context())?;

    Ok(())
}

/// Converts `SauTxLedgerRefs` (new chain type) to `LedgerRefs` (snark-acct-types).
fn convert_sau_ledger_refs(sau_refs: &SauTxLedgerRefs) -> LedgerRefs {
    match sau_refs.asm_history_proofs() {
        Some(claim_list) => {
            let claims: Vec<AccumulatorClaim> = claim_list
                .claims()
                .iter()
                .map(|c| {
                    let hash: [u8; 32] = c.entry_hash().into();
                    AccumulatorClaim::new(c.idx(), hash)
                })
                .collect();
            LedgerRefs::new(claims)
        }
        None => LedgerRefs::new_empty(),
    }
}

/// Applies the effects of a transaction (transfers and messages) to account state.
fn apply_tx_effects<S: IStateAccessor>(
    state: &mut S,
    source: AccountId,
    effects: &TxEffects,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    for t in effects.transfers_iter() {
        account_processing::process_transfer(state, source, t.dest(), t.value(), context)?;
    }

    for m in effects.messages_iter() {
        account_processing::process_message(state, source, m.dest(), m.payload().clone(), context)?;
    }

    Ok(())
}

/// Checks that a tx's constraints are valid for the current slot in state.
pub fn check_tx_constraints<S: IStateAccessor>(
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
pub fn verify_effects_safe<S: IStateAccessor>(
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
