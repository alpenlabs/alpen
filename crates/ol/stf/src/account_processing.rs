//! Account-specific interaction handling, such as messages.

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
use strata_ledger_types::{Coin, IAccountStateMut, ISnarkAccountStateMut, IStateAccessor};
use strata_msg_fmt::{Msg, MsgRef};
use strata_ol_chain_types_new::SimpleWithdrawalIntentLogData;
use strata_ol_msg_types::OLMessageExt;
use strata_snark_acct_sys as snark_sys;
use tracing::{debug, warn};

use crate::{
    constants::{BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL},
    context::BasicExecContext,
    errors::ExecResult,
    output::OutputCtx,
};

/// Processes a message by delivering it to its destination, which might involve
/// touching the ledger state.
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
            handle_bridge_gateway_message(state, sender, msg, context)?;
        }

        // Any other address we assume is a ledger account, so we have to look it up.
        _ => {
            // Check if the account exists first.
            if !state.check_account_exists(target)? {
                // If we don't find it then we can just ignore it.
                // TODO do something with the funds we're throwing away by doing this
                debug!(
                    %sender,
                    %target,
                    value = %msg.value(),
                    "dropping message: target account does not exist",
                );
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

pub(crate) fn process_transfer<S: IStateAccessor>(
    state: &mut S,
    sender: AccountId,
    target: AccountId,
    value: BitcoinAmount,
    _context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    match target {
        // Bridge gateway transfer.
        BRIDGE_GATEWAY_ACCT_ID => {
            // TODO: what do we do with direct transfers to bridge accounts? Emit log?
            debug!(
                %sender,
                %value,
                "dropping transfer to bridge gateway account"
            );
        }

        // Any other address we assume is a ledger account, so we have to look it up.
        _ => {
            // Check if the account exists first.
            if !state.check_account_exists(target)? {
                // If we don't find it then we can just ignore it.
                // TODO: do something with the funds we're throwing away by doing this
                debug!(
                    %sender,
                    %target,
                    %value,
                    "dropping transfer: target account does not exist",
                );
                return Ok(());
            }

            // Update the account within a closure.
            state.update_account(target, |acct_state| -> ExecResult<()> {
                // Just increase the balance right now.
                let coin = Coin::new_unchecked(value);
                acct_state.add_balance(coin);
                Ok(())
            })??;
        }
    }

    Ok(())
}

fn handle_bridge_gateway_message<S: IStateAccessor>(
    _state: &mut S,
    _sender: AccountId,
    payload: MsgPayload,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // 1. Parse the message from the payload data.
    let Ok(msg) = MsgRef::try_from(payload.data()) else {
        // Invalid message format, just ignore.
        debug!(
            value = %payload.value(),
            "dropping bridge gateway message: invalid message format",
        );
        return Ok(());
    };

    let Some(withdrawal_data) = msg.try_as_withdrawal() else {
        // Not a withdrawal message, or malformed, just ignore.
        //
        // TODO maybe reroute this to a different thing?
        debug!(
            msg_ty = msg.ty(),
            value = %payload.value(),
            "dropping bridge gateway message: not a withdrawal",
        );
        return Ok(());
    };

    // 2. Check if the withdrawal amount is a positive exact multiple of the denomination
    let withdrawal_amt = payload.value();

    // TODO(STR-2974) move to params struct
    let withdrawal_denom: u64 = 100_000_000;

    // 3. Verify the amount is a positive exact multiple of the denomination.
    let amt_raw: u64 = withdrawal_amt.into();
    if amt_raw == 0 || !amt_raw.is_multiple_of(withdrawal_denom) {
        warn!(%amt_raw, "ignoring withdrawal with invalid amount");
        return Ok(());
    }

    // 4. If it is, then we can emit a OL log with the amount and destination.
    let log_data = SimpleWithdrawalIntentLogData {
        amt: withdrawal_amt.into(),
        selected_operator: withdrawal_data.selected_operator(),
        dest: withdrawal_data.into_dest_desc(),
    };
    context.emit_typed_log(BRIDGE_GATEWAY_ACCT_SERIAL, &log_data)?;

    Ok(())
}

fn handle_snark_account_message<S: ISnarkAccountStateMut>(
    sastate: &mut S,
    sender: AccountId,
    payload: &MsgPayload,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    let cur_epoch = context.epoch();
    snark_sys::handle_snark_msg(cur_epoch, sastate, sender, payload)?;
    Ok(())
}
