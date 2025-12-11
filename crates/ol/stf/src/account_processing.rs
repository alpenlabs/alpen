//! Account-specific interaction handling, such as messages.

use strata_acct_types::{AccountId, MsgPayload};
use strata_codec::encode_to_vec;
use strata_ledger_types::{Coin, IAccountStateMut, ISnarkAccountStateMut, IStateAccessor};
use strata_msg_fmt::MsgRef;
use strata_ol_chain_types_new::{OLLog, SimpleWithdrawalIntentLogData};
use strata_ol_msg_types::OLMessageExt;
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
pub(crate) fn process_message<S: IStateAccessor>(
    state: &mut S,
    sender: AccountId,
    target: AccountId,
    msg: MsgPayload,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    match target {
        // Bridge gateway messages.
        BRIDGE_GATEWAY_ACCT_ID => {
            handle_bridge_gateway_message(sender, msg, context)?;
        }

        // Any other address we assume is a ledger account, so we have to look it up.
        _ => {
            // Check if the account exists first.
            if !state.check_account_exists(target)? {
                // If we don't find it then we can just ignore it.
                // TODO do something with the funds we're throwing away by doing this
                return Ok(());
            }

            // Update the account within a closure.
            state.update_account(target, |acct_state| -> ExecResult<()> {
                // First, just increase the balance right now.
                let coin = Coin::new_unchecked(msg.value());
                acct_state.add_balance(coin);

                // Then depending on the type we call a different handler function
                // for postprocessing.
                if let Ok(sastate) = acct_state.as_snark_account_mut() {
                    // Call the handler fn now that we've increased the balance.
                    handle_snark_account_message(sastate, sender, &msg, context)?;
                }
                // Empty accounts don't need any additional processing.

                Ok(())
            })??;
        }
    }

    Ok(())
}

fn handle_bridge_gateway_message(
    _sender: AccountId,
    payload: MsgPayload,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // Parse the message from the payload data.
    let Ok(msg) = MsgRef::try_from(payload.data()) else {
        // Invalid message format, just ignore.
        return Ok(());
    };

    let Some(withdrawal_data) = msg.try_as_withdrawal() else {
        // Not a withdrawal message, or malformed, just ignore.
        //
        // TODO maybe reroute this to a different thing?
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

fn handle_snark_account_message<S: ISnarkAccountStateMut>(
    sastate: &mut S,
    sender: AccountId,
    payload: &MsgPayload,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // Construct the message entry to insert.
    let msg_ent = MessageEntry::new(sender, context.epoch(), payload.clone());

    // And then just insert it.
    sastate.insert_inbox_message(msg_ent)?;

    Ok(())
}
