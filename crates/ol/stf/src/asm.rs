use strata_acct_types::{AccountSerial, MsgPayload, SystemAccount};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::{CheckpointUpdate, DepositLog, constants::LogTypeId};
use strata_ledger_types::{IL1ViewState, StateAccessor};
use strata_ol_chain_types_new::{LogEmitter, OLLog};
use strata_primitives::l1::BitcoinAmount;
use thiserror::Error;

use crate::{
    context::BlockExecContext,
    error::{StfError, StfResult},
    update::send_message,
};

pub(crate) fn process_asm_log(
    ctx: &BlockExecContext,
    state_accessor: &mut impl StateAccessor,
    log: &AsmLogEntry,
) -> StfResult<()> {
    let parsed_log = log.clone().try_into();
    match parsed_log.map_err(|_| StfError::InvalidAsmLog)? {
        ParsedAsmLog::Checkpoint(ckpt) => process_checkpoint(ctx, state_accessor, &ckpt),
        ParsedAsmLog::Deposit(dep) => process_deposit(ctx, state_accessor, &dep),
    }
}

/// Processes a deposit from L1 bridge during epoch sealing.
///
/// Sends deposited funds to the account associated with the EE ID in the deposit log.
///
/// # Warning
/// If no account exists for the serial, funds are currently dropped silently.
/// This needs to be handled - either error, send to treasury, or prominently log.
fn process_deposit(
    ctx: &BlockExecContext,
    state_accessor: &mut impl StateAccessor,
    dep: &DepositLog,
) -> StfResult<()> {
    let serial = AccountSerial::new(dep.ee_id);
    let acct_id = state_accessor.get_account_id_from_serial(serial)?;

    let Some(acct_id) = acct_id else {
        // FIXME: Funds are being dropped! Should either error, send to treasury, or log prominently
        return Ok(());
    };

    // Construct message payload.
    let amt = BitcoinAmount::from_sat(dep.amount);
    let msg_payload = MsgPayload::new(amt, dep.to_raw_bytes());

    // Send deposit message from bridge
    send_message(
        ctx,
        state_accessor,
        SystemAccount::Bridge.id(),
        acct_id,
        &msg_payload,
    )?;

    let log = OLLog::deposit_ack(acct_id, dep.addr.clone(), dep.amount.into());
    LogEmitter::emit_log(ctx, log);

    // Increment bridged btc.
    let l1_view = state_accessor.l1_view_mut();
    // TODO: coin abstraction.
    let _ = l1_view.increment_total_ledger_balance(amt);

    Ok(())
}

fn process_checkpoint(
    ctx: &BlockExecContext,
    state_accessor: &mut impl StateAccessor,
    ckpt: &CheckpointUpdate,
) -> StfResult<()> {
    // TODO: what else? Maybe store bitcoin txid for bookkeeping?
    let l1_view = state_accessor.l1_view_mut();
    l1_view.set_asm_recorded_epoch(ckpt.epoch_commitment);

    // Using system account zero address here since checkpoint is not associated with any account
    // and we have account id in OLLog. I don't want to make it optional.
    let log = OLLog::checkpoint_ack(SystemAccount::Zero.id(), ckpt.epoch_commitment);
    ctx.emit_log(log);
    Ok(())
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
        // Get the type ID and try to convert it to LogTypeId enum
        let type_id = log.ty().ok_or(AsmParseError::LogTypeNotPresent)?;
        let log_type =
            LogTypeId::from_type_id_raw(type_id).ok_or(AsmParseError::UnknownLogType(type_id))?;

        match log_type {
            LogTypeId::CheckpointUpdate => log
                .try_into_log::<CheckpointUpdate>()
                .map(Self::Checkpoint)
                .map_err(|_| AsmParseError::InvalidLogData),

            LogTypeId::Deposit => log
                .try_into_log::<DepositLog>()
                .map(Self::Deposit)
                .map_err(|_| AsmParseError::InvalidLogData),

            _ => Err(AsmParseError::UnsupportedLogType(log_type)),
        }
    }
}

/// Error type for parsing ASM log entries.
#[derive(Clone, Debug, Error)]
enum AsmParseError {
    /// The log type identifier is not recognized.
    #[error("unknown log type {0}")]
    UnknownLogType(u16),

    /// The log data could not be parsed into the expected format.
    #[error("invalid log data")]
    InvalidLogData,

    #[error("unsupported log type: {0:?}")]
    UnsupportedLogType(LogTypeId),

    #[error("log type not present in log")]
    LogTypeNotPresent,
}
