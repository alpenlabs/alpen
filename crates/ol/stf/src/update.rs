use strata_acct_types::{AccountId, AcctError, BitcoinAmount, MsgPayload};
use strata_ledger_types::{AccountTypeState, Coin, IAccountState, IL1ViewState, StateAccessor};
use strata_ol_chain_types_new::OLLog;
use strata_snark_acct_sys::{VerifiedUpdate, handle_snark_msg, handle_snark_transfer};

use crate::{
    error::StfResult,
    system_handlers::{get_system_msg_handler, get_system_transfer_handler},
};

pub(crate) fn apply_update_outputs<'a, S: StateAccessor>(
    state_accessor: &mut S,
    sender: AccountId,
    verified_update: VerifiedUpdate<'a>,
) -> StfResult<Vec<OLLog>> {
    let outputs = verified_update.operation().outputs();
    let transfers = outputs.transfers();
    let messages = outputs.messages();
    let mut logs = Vec::new();

    // Process transfers
    for transfer in transfers {
        let xfer_logs = send_transfer(state_accessor, sender, transfer.dest(), transfer.value())?;
        logs.extend(xfer_logs);
    }

    // Process messages
    for msg in messages {
        let payload = msg.payload();
        let msg_logs = send_message(state_accessor, sender, msg.dest(), payload)?;
        logs.extend(msg_logs);
    }

    Ok(logs)
}

pub(crate) fn send_message<S: StateAccessor>(
    state_accessor: &mut S,
    from: AccountId,
    to: AccountId,
    msg_payload: &MsgPayload,
) -> StfResult<Vec<OLLog>> {
    let cur_epoch = state_accessor.l1_view().cur_epoch();
    let Some(target_acct) = state_accessor.get_account_state_mut(to)? else {
        return Err(AcctError::NonExistentAccount(to).into());
    };

    // First update the balance
    let coin = Coin::new_unchecked(msg_payload.value());
    target_acct.add_balance(coin);

    if let Some(sys_handler) = get_system_msg_handler::<S>(to) {
        return sys_handler(state_accessor, from, msg_payload);
    };

    match target_acct.get_type_state_mut()? {
        AccountTypeState::Empty => {
            // todo: what exactly to do?
            Ok(Vec::new())
        }
        AccountTypeState::Snark(snark_state) => {
            Ok(handle_snark_msg(cur_epoch, snark_state, from, msg_payload)?)
        }
    }
}

pub(crate) fn send_transfer<S: StateAccessor>(
    state_accessor: &mut S,
    from: AccountId,
    to: AccountId,
    amt: BitcoinAmount,
) -> StfResult<Vec<OLLog>> {
    let cur_epoch = state_accessor.l1_view().cur_epoch();
    let Some(target_acct) = state_accessor.get_account_state_mut(to)? else {
        return Err(AcctError::NonExistentAccount(to).into());
    };

    // First update the balance
    let coin = Coin::new_unchecked(amt);
    target_acct.add_balance(coin);

    if let Some(sys_handler) = get_system_transfer_handler::<S>(to) {
        return sys_handler(state_accessor, from, amt);
    };

    match target_acct.get_type_state_mut()? {
        AccountTypeState::Empty => {
            // todo: what exactly to do?
            Ok(Vec::new())
        }
        AccountTypeState::Snark(snark_state) => {
            Ok(handle_snark_transfer(cur_epoch, snark_state, from, amt)?)
        }
    }
}
