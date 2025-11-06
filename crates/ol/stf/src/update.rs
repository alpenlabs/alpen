use strata_acct_types::{AccountId, AcctError, BitcoinAmount, MsgPayload};
use strata_ledger_types::{AccountTypeState, Coin, IAccountState, IL1ViewState, StateAccessor};
use strata_snark_acct_sys::{VerifiedUpdate, handle_snark_msg, handle_snark_transfer};

use crate::{
    context::BlockExecContext,
    error::StfResult,
    system_handlers::{get_system_msg_handler, get_system_transfer_handler},
};

pub(crate) fn apply_update_outputs<'a, S: StateAccessor>(
    ctx: &BlockExecContext,
    state_accessor: &mut S,
    sender: AccountId,
    verified_update: VerifiedUpdate<'a>,
) -> StfResult<()> {
    let outputs = verified_update.operation().outputs();
    let transfers = outputs.transfers();
    let messages = outputs.messages();

    // Process transfers
    for transfer in transfers {
        send_transfer(
            ctx,
            state_accessor,
            sender,
            transfer.dest(),
            transfer.value(),
        )?;
    }

    // Process messages
    for msg in messages {
        let payload = msg.payload();
        send_message(ctx, state_accessor, sender, msg.dest(), payload)?;
    }

    Ok(())
}

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
            let logs = handle_snark_msg(cur_epoch, snark_state, from, msg_payload)?;
            ctx.emit_logs(logs);
            Ok(())
        }
    }
}

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
            let logs = handle_snark_transfer(cur_epoch, snark_state, from, amt)?;
            ctx.emit_logs(logs);
            Ok(())
        }
    }
}
