use strata_asm_common::Mismatched;
use strata_chaintsn::context::StateAccessor;
use strata_primitives::{buf::Buf32, params::RollupParams};

use crate::{
    account::{
        AccountId, AccountInnerState, AccountUpdateOutputs, SnarkAccountMessageEntry,
        SnarkAccountState, SnarkAccountUpdate,
    },
    block::{OLLog, Transaction, TransactionPayload},
    ledger::LedgerProvider,
    state::OLState,
    stf::{StfError, StfResult},
};

pub(crate) fn execute_transaction(
    params: &RollupParams,
    state_accessor: &mut impl StateAccessor<OLState>,
    ledger_provider: &mut impl LedgerProvider,
    tx: &Transaction,
) -> StfResult<Vec<OLLog>> {
    let acct_id = tx.account_id();
    let txtype = tx.type_id();
    match tx.payload() {
        TransactionPayload::SnarkAccountUpdate { update, .. } => {
            let logs =
                execute_snark_update(params, state_accessor, ledger_provider, &acct_id, update)?;
            Ok(logs)
        }
        _ => {
            // unsupported
            let msg = format!("Unsupported transaction: {txtype:?}");
            Err(StfError::Other(msg))
        }
    }
}

fn execute_snark_update(
    _params: &RollupParams,
    state_accessor: &mut impl StateAccessor<OLState>,
    ledger_provider: &mut impl LedgerProvider,
    acct_id: &Buf32,
    update: &SnarkAccountUpdate,
) -> StfResult<Vec<OLLog>> {
    // verify update
    let mut acct_state = ledger_provider
        .get_account_state(acct_id)?
        .ok_or(StfError::NonExistentAccount(*acct_id))?;

    #[allow(irrefutable_let_patterns)]
    if let AccountInnerState::Snark(mut snark_state) = acct_state.inner_state {
        let (total_sent, out_msgs) = verify_update_correctness(
            state_accessor,
            ledger_provider,
            &snark_state,
            acct_id,
            update,
        )?;
        let logs = Vec::new();

        // Now apply updates
        snark_state.proof_state = update.data.new_state.clone();
        snark_state.seq_no = update.data.seq_no;
        acct_state.balance -= total_sent;
        acct_state.inner_state = AccountInnerState::Snark(snark_state);

        // Save the updated account state back to the ledger
        ledger_provider.set_account_state(*acct_id, acct_state)?;

        // Insert message to respective account
        for (acct_id, msg) in out_msgs {
            ledger_provider.insert_message(&acct_id, msg)?;
        }

        Ok(logs)
    } else {
        Err(StfError::Other(
            "Received snark update for non-snark account".to_string(),
        ))
    }
}

fn verify_update_correctness(
    state_accessor: &mut impl StateAccessor<OLState>,
    ledger_provider: &mut impl LedgerProvider,
    snark_state: &SnarkAccountState,
    acct_id: &AccountId,
    update: &SnarkAccountUpdate,
) -> StfResult<(u64, Vec<(AccountId, SnarkAccountMessageEntry)>)> {
    // Check if update matches the current account state
    if snark_state.seq_no != update.data.seq_no {
        return Err(StfError::MismatchedSequence(Mismatched::new(
            snark_state.seq_no,
            update.data.seq_no,
        )));
    }

    // output msgs
    let mut msgs = Vec::new();

    // Check message index progression
    let cur_idx = snark_state.proof_state.next_input_idx;
    let new_idx = update.data.new_state.next_input_idx;
    let exp_msg_idx = cur_idx + update.data.processed_msgs.len() as u64;
    if exp_msg_idx != new_idx {
        return Err(StfError::MismatchedMsgIdx(Mismatched::new(
            exp_msg_idx,
            new_idx,
        )));
    }

    // Verify ledger references
    // TODO: implement this later, not needed for now
    // if !verify_ledger_refs(&update.ledger_refs, ledger_provider)? {
    //     return Err(StfError::InvalidLedgerRefs);
    // }

    // Verify outputs can be applied safely
    let (total_sent, out_msgs) =
        verify_update_outputs_safe(snark_state, acct_id, &update.data.outputs, ledger_provider)?;
    msgs.extend(out_msgs);

    // Verify witness correctness
    verify_update_witness(snark_state, update, &update.witness)?;

    Ok((total_sent, msgs))
}

fn verify_update_outputs_safe(
    _snark_state: &SnarkAccountState,
    acct_id: &AccountId,
    outputs: &AccountUpdateOutputs,
    ledger_provider: &mut impl LedgerProvider,
) -> StfResult<(u64, Vec<(AccountId, SnarkAccountMessageEntry)>)> {
    let mut total_sent = 0u64; // use a wider type like u128 ??
    let acct_state = ledger_provider
        .get_account_state(acct_id)?
        .ok_or(StfError::NonExistentAccount(*acct_id))?;
    let cur_balance = acct_state.balance;

    // 1. Check transfers
    for t in &outputs.output_transfers {
        // Account existence check
        ledger_provider
            .get_account_state(&t.destination)?
            .ok_or(StfError::NonExistentAccount(t.destination))?;

        total_sent += t.transferred_value; // TODO: checked addition??
    }

    let mut out_msgs = Vec::new();
    // 2. Check messages
    for m in &outputs.output_messages {
        // Account existence check
        ledger_provider
            .get_account_state(&m.destination)?
            .ok_or(StfError::NonExistentAccount(m.destination))?;

        // TODO: send messages later. We don't have inter-account messages for mainnet.
    }

    // 3. Ensure we don’t overspend
    if total_sent > cur_balance {
        return Err(StfError::InsufficientBalance {
            available: cur_balance,
            spent: total_sent,
        });
    }

    Ok((total_sent, out_msgs))
}

fn verify_update_witness(
    snark_state: &SnarkAccountState,
    update: &SnarkAccountUpdate,
    witness: &[u8],
) -> StfResult<()> {
    // TODO: implement correctly, for now just ok
    Ok(())
}
