use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
use strata_ledger_types::{IAccountState, ISnarkAccountState, StateAccessor};
use strata_ol_chain_types_new::Epoch;
use strata_snark_acct_types::MessageEntry;

use crate::error::{StfError, StfResult};

pub(crate) fn handle_bridge_gateway_msg<S: StateAccessor>(
    _state_accessor: &mut S,
    _from: AccountId,
    _payload: &MsgPayload,
) -> StfResult<()> {
    todo!()
}

pub(crate) fn handle_bridge_gateway_transfer<S: StateAccessor>(
    _state_accessor: &mut S,
    _from: AccountId,
    _amt: BitcoinAmount,
) -> StfResult<()> {
    Err(StfError::other(
        "transfer not supported for system accounts",
    ))
}

pub(crate) fn handle_snark_msg<S: StateAccessor>(
    cur_epoch: Epoch,
    snark_state: &mut <S::AccountState as IAccountState>::SnarkAccountState,
    from: AccountId,
    payload: &MsgPayload,
) -> StfResult<()> {
    let msg = MessageEntry::new(from, cur_epoch, payload.clone());
    Ok(snark_state.insert_inbox_message(msg)?)
}

pub(crate) fn handle_snark_transfer<S: StateAccessor>(
    _cur_epoch: Epoch,
    _snark_state: &mut <S::AccountState as IAccountState>::SnarkAccountState,
    _from: AccountId,
    _amt: BitcoinAmount,
) -> StfResult<()> {
    // Nothing to do yet, the balance is already updated.
    Ok(())
}
