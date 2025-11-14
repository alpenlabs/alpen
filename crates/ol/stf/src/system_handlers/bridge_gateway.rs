use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SystemAccount};
use strata_ledger_types::StateAccessor;
use strata_ol_chain_types_new::{LogEmitter, OLLog};

use crate::{
    context::BlockExecContext,
    error::{StfError, StfResult},
};

pub(crate) fn handle_bridge_gateway_msg<S: StateAccessor>(
    ctx: &impl LogEmitter,
    _state_accessor: &mut S,
    sender: AccountId,
    payload: &MsgPayload,
) -> StfResult<()> {
    // Since the sender account's balance will be deduced later and there's no point in adding
    // balance to a bridge gateway system account, we can just emit OLLog from here.

    // Create WithdrawalIntent log.
    let log = OLLog::withdrawal_intent(sender, payload.value(), payload.data().to_vec());
    ctx.emit_log(log);

    Ok(())
}

pub(crate) fn handle_bridge_gateway_transfer<S: StateAccessor>(
    _ctx: &BlockExecContext,
    _state_accessor: &mut S,
    _from: AccountId,
    _amt: BitcoinAmount,
) -> StfResult<()> {
    Err(StfError::UnsupportedTransferTo(SystemAccount::Bridge.id()))
}
