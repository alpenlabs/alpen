use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
use strata_ledger_types::{IAccountState, ISnarkAccountState, StateAccessor};
use strata_ol_chain_types_new::Epoch;
use strata_ol_state_types::OLState;
use strata_snark_acct_types::MessageEntry;

use crate::error::StfResult;

pub(crate) fn handle_bridge_gateway_msg<S: StateAccessor<GlobalState = OLState>>(
    state_accessor: &mut S,
    from: AccountId,
    payload: &MsgPayload,
) -> StfResult<()> {
    todo!()
}

pub(crate) fn handle_snark_msg<S: StateAccessor<GlobalState = OLState>>(
    cur_epoch: Epoch,
    snark_state: &mut <S::AccountState as IAccountState>::SnarkAccountState,
    from: AccountId,
    payload: &MsgPayload,
) -> StfResult<()> {
    let msg = MessageEntry::new(from, cur_epoch, payload.clone());
    Ok(snark_state.insert_inbox_message(msg)?)
}

pub(crate) fn handle_snark_transfer<S: StateAccessor<GlobalState = OLState>>(
    _cur_epoch: Epoch,
    _snark_state: &mut <S::AccountState as IAccountState>::SnarkAccountState,
    _from: AccountId,
    _amt: BitcoinAmount,
) -> StfResult<()> {
    // Nothing to do yet, the balance is already updated.
    Ok(())
}
