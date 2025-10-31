// TODO: some of the methods should probably exist in account crates.

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SystemAccount};
use strata_ledger_types::{IAccountState, ISnarkAccountState, StateAccessor};
use strata_ol_chain_types_new::{Epoch, OLLog};
use strata_snark_acct_types::MessageEntry;

use crate::error::{StfError, StfResult};

type MsgHandler<S> = fn(&mut S, AccountId, &MsgPayload) -> StfResult<Vec<OLLog>>;
type TransferHandler<S> = fn(&mut S, AccountId, BitcoinAmount) -> StfResult<Vec<OLLog>>;

// todo: use Lazy more sophisticated registry for handlers
pub(crate) fn get_system_msg_handler<S: StateAccessor>(
    acct_id: AccountId,
) -> Option<MsgHandler<S>> {
    if acct_id == SystemAccount::Bridge.id() {
        Some(handle_bridge_gateway_msg)
    } else {
        None
    }
}

// todo: use Lazy or more sophisticated registry for handlers
pub(crate) fn get_system_transfer_handler<S: StateAccessor>(
    acct_id: AccountId,
) -> Option<TransferHandler<S>> {
    if acct_id == SystemAccount::Bridge.id() {
        Some(handle_bridge_gateway_transfer)
    } else {
        None
    }
}

pub(crate) fn handle_bridge_gateway_msg<S: StateAccessor>(
    _state_accessor: &mut S,
    sender: AccountId,
    payload: &MsgPayload,
) -> StfResult<Vec<OLLog>> {
    // Since the sender account's balance will be deduced later and there's no point in adding
    // balance to a bridge gateway system account, we can just emit OLLog from here.

    // TODO: Will this log indicate withdrawal? Do we have to explicitly create WithdrawalIntent and
    // serialize it? maybe not.
    let log = OLLog::new(sender, payload.as_ssz_bytes());
    Ok(vec![log])
}

pub(crate) fn handle_bridge_gateway_transfer<S: StateAccessor>(
    _state_accessor: &mut S,
    _from: AccountId,
    _amt: BitcoinAmount,
) -> StfResult<Vec<OLLog>> {
    Err(StfError::Other(
        "transfer not supported for system accounts".to_string(),
    ))
}

pub(crate) fn handle_snark_msg<S: StateAccessor>(
    cur_epoch: Epoch,
    snark_state: &mut <S::AccountState as IAccountState>::SnarkAccountState,
    from: AccountId,
    payload: &MsgPayload,
) -> StfResult<Vec<OLLog>> {
    let msg = MessageEntry::new(from, cur_epoch, payload.clone());
    snark_state.insert_inbox_message(msg)?;
    Ok(Vec::new())
}

pub(crate) fn handle_snark_transfer<S: StateAccessor>(
    _cur_epoch: Epoch,
    _snark_state: &mut <S::AccountState as IAccountState>::SnarkAccountState,
    _from: AccountId,
    _amt: BitcoinAmount,
) -> StfResult<Vec<OLLog>> {
    // Nothing to do yet, the balance is already updated.
    Ok(Vec::new())
}
