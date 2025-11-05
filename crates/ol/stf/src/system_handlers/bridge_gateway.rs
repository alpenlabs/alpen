use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SystemAccount, strata_codec::Codec};
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

    // encode the log
    let mut buf = Vec::new();
    payload.encode(&mut buf).map_err(StfError::CodecError)?;
    let log = OLLog::new(sender, buf);
    Ok(vec![log])
}

pub(crate) fn handle_bridge_gateway_transfer<S: StateAccessor>(
    _state_accessor: &mut S,
    _from: AccountId,
    _amt: BitcoinAmount,
) -> StfResult<Vec<OLLog>> {
    Err(StfError::UnsupportedTransferTo(SystemAccount::Bridge.id()))
}
