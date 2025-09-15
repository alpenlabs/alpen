// TODO: move this module out of this crate, possibly in chaintsn
use strata_asm_common::{AsmError, TypeId as AsmTypeId};
use strata_asm_logs::{
    CheckpointUpdate, DepositLog,
    constants::{CHECKPOINT_UPDATE_LOG_TYPE, DEPOSIT_LOG_TYPE_ID},
};
use strata_chainexec::BlockExecutionOutput;
use strata_chaintsn::context::StateAccessor;
use strata_primitives::{buf::Buf32, params::RollupParams};
use thiserror::Error;

use crate::{
    account::{AccountId, AccountSerial, MessageData, SnarkAccountMessageEntry},
    block::{L1Update, OLBlock, OLBlockHeader, OLLog, Transaction},
    ledger::{LedgerError, LedgerProvider},
    state::OLState,
};

/// Any error that happens during executing the State Transition Function (STF)
#[derive(Debug, Error)]
pub enum StfError {
    #[error("invalid block header: {0}")]
    InvalidBlockHeader(String),

    #[error("unsupported asm log type: {0}")]
    UnsupportedLogType(AsmTypeId),

    #[error(transparent)]
    AsmError(#[from] AsmError),

    #[error(transparent)]
    LedgerError(#[from] LedgerError),

    #[error("non-existent account serial: {0}")]
    NonExistentAccountSerial(AccountSerial),

    // TODO: possibly can be merged with above
    #[error("non-existent account : {0}")]
    NonExistentAccount(AccountId),
}

// FIXME: There's a lot of func arguments :o
pub fn process_block(
    _prev_state: &OLState, // Possibly redudant with state_accessor
    prev_header: &OLBlockHeader,
    block: &OLBlock,
    params: &RollupParams,
    state_accessor: &mut impl StateAccessor,
    ledger_provider: &mut impl LedgerProvider,
) -> Result<BlockExecutionOutput<OLState, OLLog>, StfError> {
    // Validate block header
    block
        .validate_block_header(params, prev_header)
        .map_err(StfError::InvalidBlockHeader)?;
    let mut logs = Vec::new();

    // process txs if any
    if let Some(txs) = block.body().txs() {
        for tx in txs {
            let ex_logs = execute_transaction(params, state_accessor, ledger_provider, tx)?;
            logs.extend(ex_logs);
        }
    }

    // process l1 update if any, which means terminal block and the epoch is being sealed
    if let Some(l1update) = block.body().l1update() {
        let seal_logs = seal_epoch(params, l1update, state_accessor, ledger_provider)?;
        logs.extend(seal_logs);
    }

    let wb = OLState::default();
    let out = BlockExecutionOutput::new(Buf32::zero(), logs, wb);
    Ok(out)
}

fn execute_transaction(
    params: &RollupParams,
    state_accessor: &mut impl StateAccessor,
    ledger_provider: &mut impl LedgerProvider,
    tx: &Transaction,
) -> Result<Vec<OLLog>, StfError> {
    todo!()
}

fn seal_epoch(
    _params: &RollupParams,
    l1update: &L1Update,
    state_accessor: &mut impl StateAccessor,
    ledger_provider: &mut impl LedgerProvider,
) -> Result<Vec<OLLog>, StfError> {
    let mut logs = Vec::new();
    for manifest in l1update.manifests() {
        for asmlog in manifest.logs() {
            // TODO: may need to abstract with system message
            if asmlog.ty() == DEPOSIT_LOG_TYPE_ID {
                let dep = asmlog.try_into_log::<DepositLog>()?;
                process_deposit(&dep, state_accessor, ledger_provider)?;
            } else if asmlog.ty() == CHECKPOINT_UPDATE_LOG_TYPE {
                let ckpt = asmlog.try_into_log::<CheckpointUpdate>()?;
                process_checkpoint(&ckpt, state_accessor)?;
            } else {
                return Err(StfError::UnsupportedLogType(asmlog.ty()));
            }
        }
    }
    Ok(logs)
}

fn bridge_account_id() -> Buf32 {
    let acc = [1; 32]; // TODO: change this
    acc.into()
}

fn process_deposit(
    dep: &DepositLog,
    state_accessor: &mut impl StateAccessor,
    ledger_provider: &mut impl LedgerProvider,
) -> Result<(), StfError> {
    let serial = dep.ee_id as u32;
    let acct_id = ledger_provider
        .account_id(serial)?
        .ok_or(StfError::NonExistentAccountSerial(serial))?;
    let mut acct_state = ledger_provider
        .account_state(acct_id)?
        .ok_or(StfError::NonExistentAccount(acct_id))?;

    acct_state.balance += dep.amount;
    let message = SnarkAccountMessageEntry {
        source: bridge_account_id(),
        included_epoch: state_accessor.cur_epoch(),
        data: MessageData {
            transferred_value: dep.amount,
            payload: deposit_log_to_msg_payload(dep),
        },
    };
    ledger_provider.insert_message(acct_id, message)?;
    ledger_provider.set_account_state(acct_id, acct_state)?;

    Ok(())
}

fn deposit_log_to_msg_payload(dep: &DepositLog) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&dep.amount.to_be_bytes());
    payload.extend_from_slice(&dep.addr);
    payload
}

fn process_checkpoint(
    _ckpt: &CheckpointUpdate,
    _state_accessor: &mut impl StateAccessor,
) -> Result<(), StfError> {
    todo!()
}
