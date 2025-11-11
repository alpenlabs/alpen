use strata_acct_types::{AccountId, AcctError, BitcoinAmount, MsgPayload};
use strata_ledger_types::{AccountTypeState, Coin, IAccountState, IL1ViewState, StateAccessor};
use strata_snark_acct_sys::{handle_snark_msg, handle_snark_transfer};

use crate::{
    context::BlockExecContext,
    error::StfResult,
    system_handlers::{get_system_msg_handler, get_system_transfer_handler},
};

/// Sends a message with attached value from one account to another.
///
/// Creates a [`Coin`] for the message value and adds it to the recipient's balance.
/// Routes to system handlers for system accounts, or to account-type-specific handlers.
///
/// # Safety
/// Creates a coin with `new_unchecked` - caller must ensure sender has sufficient balance.
/// The recipient's `add_balance` implementation must call `safely_consume_unchecked` on the coin.
pub(crate) fn send_message<S: StateAccessor>(
    ctx: &BlockExecContext,
    state_accessor: &mut S,
    from: AccountId,
    to: AccountId,
    msg_payload: &MsgPayload,
) -> StfResult<()> {
    let cur_epoch = state_accessor.l1_view().cur_epoch();
    let Some(target_acct) = state_accessor.get_account_state_mut(to)? else {
        return Err(AcctError::NonExistentAccount(to).into());
    };

    // First update the balance
    let coin = Coin::new_unchecked(msg_payload.value());
    target_acct.add_balance(coin); // NOTE: the add_balance method should consume the coin

    if let Some(sys_handler) = get_system_msg_handler::<S>(to) {
        return sys_handler(ctx, state_accessor, from, msg_payload);
    };

    match target_acct.get_type_state_mut()? {
        AccountTypeState::Empty => {
            // todo: what exactly to do?
            Ok(())
        }
        AccountTypeState::Snark(snark_state) => {
            handle_snark_msg(ctx, cur_epoch, snark_state, from, msg_payload)?;
            Ok(())
        }
    }
}

/// Sends a value transfer from one account to another (no message payload).
///
/// Similar to [`send_message`] but without message data. Creates a [`Coin`] for the amount
/// and routes through system/account-type handlers.
///
/// # Safety
/// Creates a coin with `new_unchecked` - caller must ensure sender has sufficient balance.
/// The recipient's `add_balance` implementation must call `safely_consume_unchecked` on the coin.
pub(crate) fn send_transfer<S: StateAccessor>(
    ctx: &BlockExecContext,
    state_accessor: &mut S,
    from: AccountId,
    to: AccountId,
    amt: BitcoinAmount,
) -> StfResult<()> {
    let cur_epoch = state_accessor.l1_view().cur_epoch();
    let Some(target_acct) = state_accessor.get_account_state_mut(to)? else {
        return Err(AcctError::NonExistentAccount(to).into());
    };

    // First update the balance
    let coin = Coin::new_unchecked(amt);
    target_acct.add_balance(coin); // NOTE: the add_balance method should consume the coin

    if let Some(sys_handler) = get_system_transfer_handler::<S>(to) {
        return sys_handler(ctx, state_accessor, from, amt);
    };

    match target_acct.get_type_state_mut()? {
        AccountTypeState::Empty => {
            // todo: what exactly to do?
            Ok(())
        }
        AccountTypeState::Snark(snark_state) => {
            handle_snark_transfer(ctx, cur_epoch, snark_state, from, amt)?;
            Ok(())
        }
    }
}
