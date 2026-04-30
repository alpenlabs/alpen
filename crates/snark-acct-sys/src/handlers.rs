use strata_acct_types::{AccountId, BitcoinAmount, MessageEntry, MsgPayload};
use strata_ledger_types::{ExecResult, ISnarkAccountStateMut};

/// Does any extra steps after a snark message is received. Note that this function should be called
/// after updating the balance.
pub fn handle_snark_msg(
    cur_epoch: u32,
    snark_state: &mut impl ISnarkAccountStateMut,
    from: AccountId,
    payload: &MsgPayload,
) -> ExecResult<()> {
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
) -> ExecResult<()> {
    // Nothing to do yet, the balance should be already updated.
    Ok(())
}
