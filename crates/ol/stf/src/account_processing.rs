//! Account-specific interaction handling, such as messages.

use strata_acct_types::{AccountId, AcctError, MsgPayload};
use strata_codec::encode_to_vec;
use strata_ledger_types::{
    AccountTypeState, Coin, IAccountState, ISnarkAccountState, StateAccessor,
};
use strata_msg_fmt::{Msg, MsgRef};
use strata_ol_chain_types_new::{OLLog, SimpleWithdrawalIntentLogData};
use strata_ol_msg_types::{OLMessageExt, WITHDRAWAL_MSG_TYPE_ID};
use strata_snark_acct_types::MessageEntry;

use crate::{
    constants::{BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL},
    context::BasicExecContext,
    errors::{ExecError, ExecResult},
    output::OutputCtx,
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

        // Any other address we assume is a ledger account, so we have to look it up.
        _ => {
            // TODO adapt the account state traits to make this all more
            // amendable to avoiding copies/clones

            // Make a copy of the account state.  I don't love this.
            let Some(mut acct_state) = state.get_account_state(target)?.cloned() else {
                // If we don't find it then we can just ignore it.
                // TODO do something with the funds we're throwing away by doing this
                return Ok(());
            };

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

    Ok(())
}

fn handle_bridge_gateway_message<S: StateAccessor>(
    state: &mut S,
    sender: AccountId,
    payload: MsgPayload,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // Parse the message from the payload data.
    let Ok(msg) = MsgRef::try_from(payload.data()) else {
        // Invalid message format, just ignore.
        return Ok(());
    };

    if msg.ty() != WITHDRAWAL_MSG_TYPE_ID {
        // Some other message type, just ignore.
        return Ok(());
    }

    let Some(withdrawal_data) = msg.try_as_withdrawal() else {
        // Invalid withdrawal message, just ignore.
        return Ok(());
    };

    // Check if the withdrawal amount is in allowed denominations
    let withdrawal_amt = payload.value();

    // TODO move to params struct
    let withdrawal_denoms = &[100_000_000];

    // Make that the amount is an appropriate denomination.
    if !withdrawal_denoms.contains(&withdrawal_amt.into()) {
        return Ok(());
    }

    // If it is, then we can emit a OL log with the amount and destination.
    let log_data = SimpleWithdrawalIntentLogData {
        amt: withdrawal_amt.into(),
        dest: withdrawal_data.into_dest_desc(),
    };

    // Encode the log data and then just emit it.
    let encoded_log = encode_to_vec(&log_data)?;
    let log = OLLog::new(BRIDGE_GATEWAY_ACCT_SERIAL, encoded_log);
    context.emit_log(log);

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
