use strata_acct_types::{AccountId, AcctResult, BitcoinAmount, MsgPayload};
use strata_ledger_types::ISnarkAccountState;
use strata_ol_chain_types_new::LogEmitter;
use strata_primitives::Epoch;
use strata_snark_acct_types::MessageEntry;

pub fn handle_snark_msg(
    _logs_emitter: &impl LogEmitter,
    cur_epoch: Epoch,
    snark_state: &mut impl ISnarkAccountState,
    from: AccountId,
    payload: &MsgPayload,
) -> AcctResult<()> {
    let msg = MessageEntry::new(from, cur_epoch, payload.clone());
    snark_state.insert_inbox_message(msg)?;
    Ok(())
}

pub fn handle_snark_transfer(
    _logs_emitter: &impl LogEmitter,
    _cur_epoch: Epoch,
    _snark_state: &mut impl ISnarkAccountState,
    _from: AccountId,
    _amt: BitcoinAmount,
) -> AcctResult<()> {
    // Nothing to do yet, the balance should be already updated.
    Ok(())
}
