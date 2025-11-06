use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SystemAccount};
use strata_ledger_types::StateAccessor;
use strata_ol_chain_types_new::OLLog;

use crate::error::{StfError, StfResult};

pub(crate) fn handle_bridge_gateway_msg<S: StateAccessor>(
    _state_accessor: &mut S,
    sender: AccountId,
    payload: &MsgPayload,
) -> StfResult<Vec<OLLog>> {
    // Since the sender account's balance will be deduced later and there's no point in adding
    // balance to a bridge gateway system account, we can just emit OLLog from here.

    // Create WithdrawalIntent log
    let log = OLLog::withdrawal_intent(sender, payload.value(), payload.data().to_vec());

    Ok(vec![log])
}

pub(crate) fn handle_bridge_gateway_transfer<S: StateAccessor>(
    _state_accessor: &mut S,
    _from: AccountId,
    _amt: BitcoinAmount,
) -> StfResult<Vec<OLLog>> {
    Err(StfError::UnsupportedTransferTo(SystemAccount::Bridge.id()))
}
