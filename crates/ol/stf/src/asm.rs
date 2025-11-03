use strata_acct_types::{AccountSerial, MsgPayload, SystemAccount};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::{
    CheckpointUpdate, DepositLog,
    constants::{CHECKPOINT_UPDATE_LOG_TYPE, DEPOSIT_LOG_TYPE_ID},
};
use strata_ledger_types::{IL1ViewState, StateAccessor};
use strata_ol_chain_types_new::OLLog;
use strata_primitives::l1::BitcoinAmount;
use thiserror::Error;

use crate::{
    error::{StfError, StfResult},
    update::send_message,
};

pub(crate) fn process_asm_log(
    state_accessor: &mut impl StateAccessor,
    log: &AsmLogEntry,
) -> StfResult<Vec<OLLog>> {
    let parsed_log = log.clone().try_into();
    match parsed_log.map_err(|_| StfError::InvalidAsmLog)? {
        ParsedAsmLog::Checkpoint(ckpt) => process_checkpoint(state_accessor, &ckpt),
        ParsedAsmLog::Deposit(dep) => process_deposit(state_accessor, &dep),
    }
}

fn process_deposit(
    state_accessor: &mut impl StateAccessor,
    dep: &DepositLog,
) -> StfResult<Vec<OLLog>> {
    let serial = AccountSerial::new(dep.ee_id);
    let acct_id = state_accessor.get_account_id_from_serial(serial)?;

    let Some(acct_id) = acct_id else {
        return Ok(Vec::new());
    };

    // Add balance to account.
    let amt = BitcoinAmount::from_sat(dep.amount);
    let msg_payload = MsgPayload::new(amt, dep.as_raw_msg_bytes());

    // Send deposit message from bridge
    send_message(
        state_accessor,
        SystemAccount::Bridge.id(),
        acct_id,
        &msg_payload,
    )?;

    // Increment bridged btc.
    let l1_view = state_accessor.l1_view_mut();
    let _ = l1_view.increment_total_ledger_balance(amt);
    // No logs
    Ok(Vec::new())
}

fn process_checkpoint(
    state_accessor: &mut impl StateAccessor,
    ckpt: &CheckpointUpdate,
) -> StfResult<Vec<OLLog>> {
    // TODO: what else? Maybe store bitcoin txid for bookkeeping?
    let l1_view = state_accessor.l1_view_mut();
    l1_view.set_asm_recorded_epoch(ckpt.epoch_commitment);

    // No logs for now
    Ok(Vec::new())
}

#[derive(Clone, Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "exists ephemerally, so should not be an issue"
)]
enum ParsedAsmLog {
    Checkpoint(CheckpointUpdate),
    Deposit(DepositLog),
}

impl TryFrom<AsmLogEntry> for ParsedAsmLog {
    type Error = AsmParseError;

    fn try_from(log: AsmLogEntry) -> Result<Self, Self::Error> {
        match log.ty() {
            Some(CHECKPOINT_UPDATE_LOG_TYPE) => log
                .try_into_log::<CheckpointUpdate>()
                .map(Self::Checkpoint)
                .map_err(|_| AsmParseError::InvalidLogData),

            Some(DEPOSIT_LOG_TYPE_ID) => log
                .try_into_log::<DepositLog>()
                .map(Self::Deposit)
                .map_err(|_| AsmParseError::InvalidLogData),
            Some(_) | None => Err(AsmParseError::UnknownLogType),
        }
    }
}

/// Error type for parsing ASM log entries.
#[derive(Clone, Debug, Error)]
enum AsmParseError {
    /// The log type identifier is not recognized.
    #[error("unknown log type")]
    UnknownLogType,

    /// The log data could not be parsed into the expected format.
    #[error("invalid log data")]
    InvalidLogData,
}
