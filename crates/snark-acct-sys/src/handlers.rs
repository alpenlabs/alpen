use strata_acct_types::{AccountId, AcctResult, BitcoinAmount, MsgPayload};
use strata_ledger_types::ISnarkAccountStateMut;
use strata_snark_acct_types::MessageEntry;

/// Does any extra steps after a snark message is received. Note that this function should be called
/// after updating the balance.
pub fn handle_snark_msg(
    cur_epoch: u32,
    snark_state: &mut impl ISnarkAccountStateMut,
    from: AccountId,
    payload: &MsgPayload,
) -> AcctResult<()> {
    let msg = MessageEntry::new(from, cur_epoch, payload.clone());
    snark_state.insert_inbox_message(msg)?;
    Ok(())
}

/// Does any extra steps after a snark transfer is received. Note that this function should be
/// called after updating the balance.
pub fn handle_snark_transfer(
    _cur_epoch: u32,
    _snark_state: &mut impl ISnarkAccountStateMut,
    _from: AccountId,
    _amt: BitcoinAmount,
) -> AcctResult<()> {
    // Nothing to do yet, the balance should be already updated.
    // TODO: anything more to be done?
    Ok(())
}
