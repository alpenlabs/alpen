//! Account-specific interaction handling, such as messages.

use strata_acct_types::{AccountId, AcctError, MsgPayload};
use strata_ledger_types::{
    AccountTypeState, Coin, IAccountState, ISnarkAccountState, StateAccessor,
};
use strata_snark_acct_types::MessageEntry;

use crate::{
    constants::BRIDGE_GATEWAY_ACCT_ID,
    context::BasicExecContext,
    errors::{ExecError, ExecResult},
};

/// Processes a message by delivering it to its destination, which might involve
/// touching the ledger state.
///
/// This takes a [`EpochContext`] because messages can be issued both in regular
/// block processing and at epoch sealing.
pub(crate) fn process_message<S: StateAccessor>(
    state: &mut S,
    sender: AccountId,
    target: AccountId,
    msg: MsgPayload,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    match target {
        // Bridge gateway messages.
        BRIDGE_GATEWAY_ACCT_ID => {
            handle_bridge_gateway_message(state, sender, msg, context)?;
        }

        // Any other address we assume is a ledger account, so we look it up.
        _ => {
            // TODO adapt the account state traits to make this all more
            // amendable to avoiding copies/clones

            // Make a copy of the account state.  I don't love this.
            let mut acct_state = state
                .get_account_state(target)?
                .cloned()
                .ok_or(AcctError::MissingExpectedAccount(target))?;

            // First, just increase the balance right now.
            let coin = Coin::new_unchecked(msg.value());
            acct_state.add_balance(coin);

            // Then depending on the type we call a different handler function
            // for postprocessing.
            let mut tystate = acct_state.get_type_state().expect("stf/acct: type state");
            match &mut tystate {
                AccountTypeState::Empty => {
                    // Do nothing.
                }

                AccountTypeState::Snark(sastate) => {
                    // Call the handler fn now that we've increased the balance.
                    handle_snark_account_message(state, sastate, sender, msg, context)?;
                }
            }

            // Update with the now-modified type state.
            acct_state.set_type_state(tystate)?;

            // Then write the whole account back with the changes.
            state.update_account_state(target, acct_state)?;
        }
    }

    //

    // TODO
    Ok(())
}

fn handle_bridge_gateway_message<S: StateAccessor>(
    state: &mut S,
    sender: AccountId,
    payload: MsgPayload,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // TODO log the withdrawal intent
    Ok(())
}

fn handle_snark_account_message<S: StateAccessor>(
    state: &mut S,
    mut sastate: &mut <S::AccountState as IAccountState>::SnarkAccountState,
    sender: AccountId,
    payload: MsgPayload,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // Construct the message entry to insert.
    let msg_ent = MessageEntry::new(sender, context.epoch(), payload);

    // And then just insert it.
    sastate.insert_inbox_message(msg_ent)?;

    Ok(())
}
