//! Block transactional processing.

use strata_acct_types::{
    AccountId, AcctError, BitcoinAmount, MsgPayload, SentMessage, SentTransfer, TxEffects,
};
use strata_ledger_types::{
    IAccountState, IAccountStateMut, ISnarkAccountState, ISnarkAccountStateMut, IStateAccessor,
};
use strata_ol_chain_types_new::{
    OLTransaction, OLTxSegment, SauTxPayload, TxConstraints, TxProofs, TransactionPayload,
};
use strata_snark_acct_sys as snark_sys;
use strata_snark_acct_types::{LedgerRefs, ProofState, Seqno};

use crate::{
    account_processing,
    constants::SEQUENCER_ACCT_ID,
    context::{BasicExecContext, BlockContext, TxExecContext},
    errors::{ExecError, ExecResult},
    output::OutputCtx,
    proof_verification::TxProofVerifierImpl,
};

/// Process a block's transaction segment.
///
/// This is called for every block.
pub fn process_block_tx_segment<S: IStateAccessor>(
    state: &mut S,
    tx_seg: &OLTxSegment,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    for tx in tx_seg.txs() {
        process_single_tx(state, tx, context)?;
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
        TransactionPayload::GenericAccountMessage(gam) => {
            // Construct the message we want to send and then hand it off.
            let mp = MsgPayload::new(BitcoinAmount::from(0), vec![]);
            account_processing::process_message(
                state,
                SEQUENCER_ACCT_ID,
                *gam.target(),
                mp,
                context.basic_context(),
            )?;
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

fn process_update_tx<S: IStateAccessor>(
    state: &mut S,
    target: AccountId,
    sau_payload: &SauTxPayload,
    effects: &TxEffects,
    tx_proofs: &TxProofs,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    // Step 1: Read account state for verification.
    let account_state = state
        .get_account_state(target)?
        .ok_or(ExecError::UnknownAccount(target))?;
    let snark_acct_state = account_state
        .as_snark_account()
        .map_err(|_| ExecError::IncorrectTxTargetType)?;
    let cur_balance = account_state.balance();

    // Step 2: Build SnarkAccountUpdateData from the new tx types.
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

    // Step 3: Verify the update (all checks delegated to snark-acct-sys).
    let mut verifier = TxProofVerifierImpl::new(tx_proofs);
    snark_sys::verify_update_correctness(
        state,
        target,
        snark_acct_state,
        &update_data,
        cur_balance,
        &mut verifier,
    )?;

    // Step 4: Mutate account state and collect effects.
    let fx_buf = state.update_account(target, |astate| -> ExecResult<_> {
        // Deduct balance for all effects first.
        let total_sent = compute_effects_total_value(effects)
            .ok_or(ExecError::Acct(AcctError::BitcoinAmountOverflow))?;
        let coin = astate
            .take_balance(total_sent)
            .map_err(|_| ExecError::InsufficientAccountBalance(target, total_sent))?;
        coin.safely_consume_unchecked();

        // Update proof state.
        let snrk_acct_state = astate
            .as_snark_account_mut()
            .map_err(|_| ExecError::IncorrectTxTargetType)?;

        let new_seqno = upd
            .seq_no()
            .checked_add(1)
            .ok_or(ExecError::MaxSeqNumberReached { account_id: target })?;
        snrk_acct_state.update_inner_state(
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
fn convert_sau_ledger_refs(
    sau_refs: &strata_ol_chain_types_new::SauTxLedgerRefs,
) -> LedgerRefs {
    match sau_refs.asm_history_proofs() {
        Some(claim_list) => {
            let claims: Vec<strata_acct_types::AccumulatorClaim> = claim_list
                .claims()
                .iter()
                .map(|c| {
                    let hash: [u8; 32] = c.entry_hash().into();
                    strata_acct_types::AccumulatorClaim::new(c.idx(), hash)
                })
                .collect();
            LedgerRefs::new(claims)
        }
        None => LedgerRefs::new_empty(),
    }
}

/// Computes the total value of all transfers and messages in effects.
fn compute_effects_total_value(effects: &TxEffects) -> Option<BitcoinAmount> {
    let mut total: u64 = 0;

    for t in effects.transfers_iter() {
        total = total.checked_add(t.value().into())?;
    }

    for m in effects.messages_iter() {
        total = total.checked_add(m.payload().value().into())?;
    }

    Some(BitcoinAmount::from_sat(total))
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
        account_processing::process_message(
            state,
            source,
            m.dest(),
            m.payload().clone(),
            context,
        )?;
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

/// Validates transaction sequence number using next-expected semantics.
pub fn check_snark_account_seq_no(
    account: AccountId,
    tx_seq_no: u64,
    expected_seq_no: u64,
) -> ExecResult<()> {
    if tx_seq_no != expected_seq_no {
        return Err(ExecError::InvalidSequenceNumber(
            account,
            expected_seq_no,
            tx_seq_no,
        ));
    }
    Ok(())
}

/// Gets an account state, returning an error if it doesn't exist.
pub fn get_account_state<S: IStateAccessor>(
    state: &S,
    account: AccountId,
) -> ExecResult<&S::AccountState> {
    state
        .get_account_state(account)?
        .ok_or(ExecError::UnknownAccount(account))
}

/// Gets the current sequence number for a Snark account.
pub fn get_snark_account_seq_no<S: IStateAccessor>(
    state: &S,
    account: AccountId,
) -> ExecResult<u64> {
    let account_state = get_account_state(state, account)?;

    if account_state.ty() != strata_acct_types::AccountTypeId::Snark {
        return Err(ExecError::IncorrectTxTargetType);
    }

    let snark_state = account_state.as_snark_account()?;

    Ok(*snark_state.seqno().inner())
}
