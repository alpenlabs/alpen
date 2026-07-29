//! Account-specific interaction handling, such as messages.

use bitcoin_bosd::Descriptor;
use strata_acct_types::{
    AccountId, BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL, MsgPayload,
};
use strata_ledger_types::*;
use strata_msg_fmt::MsgRef;
use strata_ol_chain_types::SimpleWithdrawalIntentLogData;
use strata_ol_msg_types::OLMessageExt;
use strata_snark_acct_sys as snark_sys;
use tracing::*;

use crate::{
    context::BasicExecContext, errors::ExecResult, msg_payload_coin::MsgPayloadCoin,
    output::OutputCtx,
};

/// Credits `coin` to `target`'s balance and runs `post` for any additional
/// account-specific processing, within a single [`IStateAccessorMut::update_account`]
/// transaction.
///
/// The coin is moved into the update closure only when it actually runs.  If
/// `update_account` returns an error before invoking the closure (e.g. the
/// account is missing), the coin is recovered from the slot and defused so the
/// error propagates cleanly instead of tripping [`Coin`]'s drop panic; the whole
/// STF is discarded on that error, so no value is lost.
pub(crate) fn credit_account<S: IStateAccessorMut>(
    state: &mut S,
    target: AccountId,
    coin: Coin,
    post: impl FnOnce(&mut S::AccountStateMut) -> ExecResult<()>,
) -> ExecResult<()> {
    let mut slot = Some(coin);
    let res = state.update_account(target, |acct_state| -> ExecResult<()> {
        acct_state.add_balance(slot.take().expect("ol/stf: coin present in credit closure"));
        post(acct_state)
    });

    match res {
        // The closure ran, so `add_balance` consumed the coin already.
        Ok(inner) => inner,
        Err(e) => {
            // The closure never ran; recover and defuse the still-live coin.
            if let Some(coin) = slot.take() {
                coin.safely_consume_unchecked();
            }
            Err(e.into())
        }
    }
}

/// Credits `coin` to `target`'s balance with no additional postprocessing.
///
/// This is the plain-transfer counterpart to [`credit_account`], sharing its
/// coin-recovery behavior on a failed [`IStateAccessorMut::update_account`].
pub(crate) fn credit_account_noop<S: IStateAccessorMut>(
    state: &mut S,
    target: AccountId,
    coin: Coin,
) -> ExecResult<()> {
    credit_account(state, target, coin, |_| Ok(()))
}

/// Processes a message by delivering it to its destination, which might involve
/// touching the ledger state.
pub(crate) fn process_message<S: IStateAccessorMut>(
    state: &mut S,
    sender: AccountId,
    target: AccountId,
    msg: MsgPayloadCoin,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    match target {
        // Bridge gateway messages.
        BRIDGE_GATEWAY_ACCT_ID => {
            handle_bridge_gateway_message(state, sender, msg, context)?;
        }

        // Any other address we assume is a ledger account, so we have to look it up.
        _ => {
            // Check if the account exists first.  Defuse the payload on error so
            // it doesn't drop while still live.
            match state.check_account_exists(target) {
                Ok(true) => {}
                Ok(false) => {
                    warn!(
                        %target,
                        %sender,
                        value = %msg.coin_amt(),
                        "limboing message to nonexistent target account",
                    );
                    handle_misplaced_funds(state, msg.into_coin())?;
                    return Ok(());
                }
                Err(e) => {
                    msg.into_coin().safely_consume_unchecked();
                    return Err(e.into());
                }
            }

            // Reconstitute the coin and a record payload for any snark inbox.
            let (coin, record) = msg.into_coin_and_record();

            // Credit the coin, then run snark postprocessing for snark accounts.
            credit_account(state, target, coin, |acct_state| {
                if let Ok(sastate) = acct_state.as_snark_account_mut() {
                    handle_snark_account_message(sastate, sender, &record, context)?;
                }
                // Empty accounts don't need any additional processing.
                Ok(())
            })?;
        }
    }

    Ok(())
}

pub(crate) fn process_transfer<S: IStateAccessorMut>(
    state: &mut S,
    _sender: AccountId,
    target: AccountId,
    coin: Coin,
    _context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    match target {
        // Bridge gateway transfer, not permitted.
        BRIDGE_GATEWAY_ACCT_ID => {
            // Just sweep to limbo unconditionally.
            warn!("limboing transfer to bridge gateway acct");
            handle_misplaced_funds(state, coin)?;
        }

        // Any other address we assume is a ledger account, so we have to look it up.
        _ => {
            // Check if the account exists first.  Defuse the coin on error so it
            // doesn't drop while still live.
            match state.check_account_exists(target) {
                Ok(true) => {}
                Ok(false) => {
                    warn!(
                        %target,
                        value = %coin.amt(),
                        "limboing transfer to nonexistent target account",
                    );
                    handle_misplaced_funds(state, coin)?;
                    return Ok(());
                }
                Err(e) => {
                    coin.safely_consume_unchecked();
                    return Err(e.into());
                }
            }

            // Credit the coin to the account.
            credit_account_noop(state, target, coin)?;
        }
    }

    Ok(())
}

fn handle_bridge_gateway_message<S: IStateAccessorMut>(
    state: &mut S,
    sender: AccountId,
    payload: MsgPayloadCoin,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    let amt_raw: u64 = payload.coin_amt().into();

    // 1. Parse the message from the payload data.
    let Ok(msg) = MsgRef::try_from(payload.data()) else {
        // Invalid message format, sweep to limbo.
        warn!(%sender, "limboing malformed message sent to bridge gateway acct");
        handle_misplaced_funds(state, payload.into_coin())?;
        return Ok(());
    };

    let Some(withdrawal_data) = msg.try_as_withdrawal() else {
        // Not a withdrawal message, or malformed, sweep to limbo.
        warn!(%sender, "limboing non-withdrawal message sent to bridge gateway acct");
        handle_misplaced_funds(state, payload.into_coin())?;
        return Ok(());
    };

    // 2. Validate the withdrawal amount against params.
    let Some(bridge_params) = context.bridge_params() else {
        warn!(%sender, %amt_raw, "limboing withdrawal without bridge params sent to bridge gateway acct");
        handle_misplaced_funds(state, payload.into_coin())?;
        return Ok(());
    };

    if !bridge_params.validate_withdrawal_amount(amt_raw) {
        warn!(%sender, %amt_raw, "limboing bad amount sent to bridge gateway acct");
        handle_misplaced_funds(state, payload.into_coin())?;
        return Ok(());
    }

    // 3. Validate the withdrawal descriptor against the configured BOSD policy.
    let dest_desc = withdrawal_data.dest_desc();
    let dest_desc_len = dest_desc.len();
    if !bridge_params.validate_withdrawal_descriptor_len(dest_desc_len)
        || Descriptor::from_bytes(dest_desc).is_err()
    {
        warn!(
            %sender,
            %amt_raw,
            dest_desc_len,
            "limboing bad destination descriptor sent to bridge gateway acct",
        );
        handle_misplaced_funds(state, payload.into_coin())?;
        return Ok(());
    }

    // 4. If it is, then we can emit a OL log with the amount and destination.
    let selected_operator = withdrawal_data.selected_operator();
    let dest = withdrawal_data.into_dest_desc();
    let log_data = SimpleWithdrawalIntentLogData {
        amt: amt_raw,
        selected_operator,
        dest,
    };
    // Defuse the payload on error so it doesn't drop while still live (the log
    // block cap can be exceeded on an otherwise-valid withdrawal).
    if let Err(e) = context.emit_typed_log(BRIDGE_GATEWAY_ACCT_SERIAL, &log_data) {
        payload.into_coin().safely_consume_unchecked();
        return Err(e);
    }
    info!(
        %sender,
        amount_sat = amt_raw,
        selected_operator,
        dest_desc_len,
        "emitted bridge withdrawal intent log",
    );
    // The value has left the OL as a withdrawal, so we consume it here.
    payload.into_coin().safely_consume_unchecked();

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

/// Handles misplaced funds.
///
/// This currently just sends funds to limbo.  It's broken out so that we can
/// maintain the same code path when we change this behavior.
pub(crate) fn handle_misplaced_funds(
    state: &mut impl IStateAccessorMut,
    coin: Coin,
) -> ExecResult<()> {
    state.add_limbo_funds_coin(coin)?;
    Ok(())
}
