use strata_acct_types::{AcctResult, AccountId, BitcoinAmount, MsgPayload};
use strata_ledger_types::ISnarkAccountState;
use strata_ol_chain_types_new::OLLog;
use strata_primitives::Epoch;
use strata_snark_acct_types::MessageEntry;

pub fn handle_snark_msg(
    cur_epoch: Epoch,
    snark_state: &mut impl ISnarkAccountState,
    from: AccountId,
    payload: &MsgPayload,
) -> AcctResult<Vec<OLLog>> {
    let msg = MessageEntry::new(from, cur_epoch, payload.clone());
    snark_state.insert_inbox_message(msg)?;
    Ok(Vec::new())
}

pub fn handle_snark_transfer(
    _cur_epoch: Epoch,
    _snark_state: &mut impl ISnarkAccountState,
    _from: AccountId,
    _amt: BitcoinAmount,
) -> AcctResult<Vec<OLLog>> {
    // Nothing to do yet, the balance is already updated.
    Ok(Vec::new())
}
