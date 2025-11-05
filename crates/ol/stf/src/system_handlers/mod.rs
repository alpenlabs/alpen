mod bridge_gateway;

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SystemAccount};
use strata_ledger_types::StateAccessor;
use strata_ol_chain_types_new::OLLog;

use crate::{
    error::StfResult,
    system_handlers::bridge_gateway::{handle_bridge_gateway_msg, handle_bridge_gateway_transfer},
};

type MsgHandler<S> = fn(&mut S, AccountId, &MsgPayload) -> StfResult<Vec<OLLog>>;
type TransferHandler<S> = fn(&mut S, AccountId, BitcoinAmount) -> StfResult<Vec<OLLog>>;

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
