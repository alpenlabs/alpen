use strata_acct_types::{AccountSerial, MsgPayload, SystemAccount};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::{CheckpointUpdate, DepositLog, ParsedAsmLog};
use strata_ledger_types::{
    AccountTypeState, Coin, IAccountState, ISnarkAccountState, StateAccessor,
};
use strata_ol_chain_types_new::OLLog;
use strata_ol_state_types::OLState;
use strata_primitives::l1::BitcoinAmount;
use strata_snark_acct_types::MessageEntry;

use crate::error::{StfError, StfResult};

pub(crate) fn process_asm_log(
    state_accessor: &mut impl StateAccessor<GlobalState = OLState>,
    log: &AsmLogEntry,
) -> StfResult<Vec<OLLog>> {
    let log = log.clone();
    match log.try_into().map_err(|_| StfError::InvalidAsmLog)? {
        ParsedAsmLog::Checkpoint(ckpt) => process_checkpoint(state_accessor, &ckpt),
        ParsedAsmLog::Deposit(dep) => process_deposit(state_accessor, &dep),
    }
}

fn process_deposit(
    state_accessor: &mut impl StateAccessor<GlobalState = OLState>,
    dep: &DepositLog,
) -> StfResult<Vec<OLLog>> {
    let serial = dep.ee_id as u32;
    let acct_id = state_accessor.get_account_id_from_serial(AccountSerial::new(serial))?;
    let cur_epoch = state_accessor.global().cur_epoch();

    let Some(acct_id) = acct_id else {
        return Ok(Vec::new());
    };

    let Some(acct_state) = state_accessor.get_account_state_mut(acct_id)? else {
        return Err(StfError::NonExistentAccount(acct_id));
    };

    // Add balance to account.
    let amt = BitcoinAmount::from_sat(dep.amount);
    let coin = Coin::new_unchecked(amt);
    acct_state.add_balance(coin);

    // Insert message to snark account inbox.
    match acct_state.get_type_state_mut()? {
        AccountTypeState::Snark(snark_state) => {
            // Insert to msg box
            let data = dep.as_raw_msg_bytes();
            let payload = MsgPayload::new(amt, data);
            let msg = MessageEntry::new(SystemAccount::Bridge.id(), cur_epoch as u32, payload);
            snark_state.insert_inbox_message(msg)?;
        }
        _ => {
            // TODO: what to do? leave it as is?
        }
    }

    // Increment bridged btc.
    let state = state_accessor.global_mut();
    state.increment_total_deposited_balance(amt);
    // No logs
    Ok(Vec::new())
}

fn process_checkpoint(
    state_accessor: &mut impl StateAccessor<GlobalState = OLState>,
    ckpt: &CheckpointUpdate,
) -> StfResult<Vec<OLLog>> {
    // TODO: what else? Maybe store bitcoin txid for bookkeeping?
    let l1_view = state_accessor.global_mut().l1_view_mut();
    l1_view.set_recorded_epoch(ckpt.epoch_commitment);

    // No logs for now
    Ok(Vec::new())
}
