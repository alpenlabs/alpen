mod bridge_gateway;

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SystemAccount};
use strata_ledger_types::StateAccessor;

use crate::{
    context::BlockExecContext,
    error::StfResult,
    system_handlers::bridge_gateway::{handle_bridge_gateway_msg, handle_bridge_gateway_transfer},
};

type MsgHandler<S> = fn(&BlockExecContext, &mut S, AccountId, &MsgPayload) -> StfResult<()>;
type TransferHandler<S> = fn(&BlockExecContext, &mut S, AccountId, BitcoinAmount) -> StfResult<()>;

pub(crate) fn get_system_msg_handler<S: StateAccessor>(
    acct_id: AccountId,
) -> Option<MsgHandler<S>> {
    if acct_id == SystemAccount::Bridge.id() {
        Some(handle_bridge_gateway_msg)
    } else {
        None
    }
}

pub(crate) fn get_system_transfer_handler<S: StateAccessor>(
    acct_id: AccountId,
) -> Option<TransferHandler<S>> {
    if acct_id == SystemAccount::Bridge.id() {
        Some(handle_bridge_gateway_transfer)
    } else {
        None
    }
}
