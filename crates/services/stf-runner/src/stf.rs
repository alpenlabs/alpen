// TODO: move this module out of this crate, possibly in chaintsn
use strata_asm_common::{AsmError, Mismatched, TypeId as AsmTypeId};
use strata_asm_logs::{
    CheckpointUpdateLog, DepositLog,
    constants::{CHECKPOINT_UPDATE_LOG_TYPE, DEPOSIT_LOG_TYPE_ID},
};
use strata_chainexec::BlockExecutionOutput;
use strata_chaintsn::context::StateAccessor;
use strata_primitives::{buf::Buf32, params::RollupParams};
use thiserror::Error;

use crate::{
    account::{AccountId, AccountSerial, MessageData, SnarkAccountMessageEntry},
    block::{L1Update, OLBlock, OLBlockHeader, OLLog},
    ledger::{LedgerError, LedgerProvider},
    state::{L1View, OLState},
    tx_exec::execute_transaction,
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

    /// Mismatched sequence
    #[error("mismatched sequence: {0}")]
    MismatchedSequence(Mismatched<u64>),

    #[error("mismatched message index: {0}")]
    MismatchedMsgIdx(Mismatched<u64>),

    #[error("invalid outputs")]
    InvalidOutputs,

    #[error("invalid witness")]
    InvalidWitness,

    /// Generic error for now.
    // TODO: make errors more specific as things get clearer
    #[error("something else: {0}")]
    Other(String),

    #[error("insufficient balance: available {available}, spent {spent}")]
    InsufficientBalance { available: u64, spent: u64 },

    #[error("state root mismatch: {0}")]
    MismatchedStateRoot(Mismatched<Buf32>),

    #[error("logs root mismatch: {0}")]
    MismatchedLogsRoot(Mismatched<Buf32>),

    #[error("body root mismatch: {0}")]
    MismatchedBodyRoot(Mismatched<Buf32>),
}

impl StfError {
    pub fn mismatched_state_root(expected: Buf32, actual: Buf32) -> Self {
        Self::MismatchedStateRoot(Mismatched::new(expected, actual))
    }
    pub fn mismatched_logs_root(expected: Buf32, actual: Buf32) -> Self {
        Self::MismatchedLogsRoot(Mismatched::new(expected, actual))
    }

    pub fn mismatched_sequence(expected: u64, actual: u64) -> Self {
        Self::MismatchedSequence(Mismatched::new(expected, actual))
    }

    pub fn mismatched_msg_idx(expected: u64, actual: u64) -> Self {
        Self::MismatchedMsgIdx(Mismatched::new(expected, actual))
    }

    pub fn mismatched_body_root(expected: Buf32, actual: Buf32) -> Self {
        Self::MismatchedBodyRoot(Mismatched::new(expected, actual))
    }
}

pub type StfResult<T> = Result<T, StfError>;

pub fn execute_block(
    prev_header: &OLBlockHeader,
    block: &OLBlock,
    params: &RollupParams,
    // state accessor is expected to allow CRUD operations on state
    state_accessor: &mut impl StateAccessor<OLState, L1View>,
    // ledger_provider is expected to allow CRUD operations on accounts ledger
    ledger_provider: &mut impl LedgerProvider,
) -> StfResult<BlockExecutionOutput<OLState, OLLog>> {
    // Validate continuity of block header
    block
        .pre_exec_validate(params, prev_header)
        .map_err(StfError::InvalidBlockHeader)?;

    // Execute block without checking header
    let out = execute_block_raw(block, params, state_accessor, ledger_provider)?;

    // Validate state and log roots
    block.post_exec_validate(&out)?; // todo: be consistent with the errors

    Ok(out)
}

/// Processes block and returns result without validating the state root with block header.
pub fn execute_block_raw(
    block: &OLBlock,
    params: &RollupParams,
    // state accessor is expected to allow CRUD operations on state
    state_accessor: &mut impl StateAccessor<OLState, L1View>,
    // ledger_provider is expected to allow CRUD operations on accounts ledger
    ledger_provider: &mut impl LedgerProvider,
) -> StfResult<BlockExecutionOutput<OLState, OLLog>> {
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
        // Check pre-sealing state root
        let exp_preseal_state_root = state_accessor.get_toplevel_state().compute_root();
        assert_eq!(exp_preseal_state_root, *l1update.preseal_state_root()); // maybe return err?

        let seal_logs = seal_epoch(params, state_accessor, ledger_provider, l1update)?;
        logs.extend(seal_logs);

        // Increment the cur epoch now that we have sealed this epoch
        state_accessor.set_cur_epoch(state_accessor.cur_epoch() + 1);
    }

    // Update cur slot after processing block
    state_accessor.set_slot(block.signed_header().header().slot());

    // Set accounts root
    state_accessor.set_accounts_root(ledger_provider.accounts_root()?);

    // Check state root
    let new_state = state_accessor.get_toplevel_state().clone();
    let state_root = new_state.compute_root();

    let out = BlockExecutionOutput::new(state_root, logs, new_state);
    Ok(out)
}

fn seal_epoch(
    _params: &RollupParams,
    state_accessor: &mut impl StateAccessor<OLState, L1View>,
    ledger_provider: &mut impl LedgerProvider,
    l1update: &L1Update,
) -> StfResult<Vec<OLLog>> {
    let mut logs = Vec::new();
    for manifest in l1update.manifests() {
        for asmlog in manifest.logs() {
            // TODO: may need to abstract with system message
            if asmlog.ty() == Some(DEPOSIT_LOG_TYPE_ID) {
                let dep = asmlog.try_into_log::<DepositLog>()?;
                process_deposit(&dep, state_accessor, ledger_provider)?;
            } else if asmlog.ty() == Some(CHECKPOINT_UPDATE_LOG_TYPE) {
                let ckpt = asmlog.try_into_log::<CheckpointUpdateLog>()?;
                process_checkpoint(&ckpt, state_accessor)?;
            } else if let Some(t) = asmlog.ty() {
                return Err(StfError::UnsupportedLogType(t));
            }
        }
    }
    state_accessor.set_l1_view(L1View::new(
        l1update.new_l1_blk_hash,
        l1update.new_l1_blk_height,
    ));
    Ok(logs)
}

fn bridge_account_id() -> Buf32 {
    let acc = [1; 32]; // TODO: change this
    acc.into()
}

fn process_deposit(
    dep: &DepositLog,
    state_accessor: &mut impl StateAccessor<OLState, L1View>,
    ledger_provider: &mut impl LedgerProvider,
) -> StfResult<()> {
    let serial = dep.ee_id as u32;
    let acct_id = ledger_provider
        .get_account_id(serial)?
        .ok_or(StfError::NonExistentAccountSerial(serial))?;
    let mut acct_state = ledger_provider
        .get_account_state(&acct_id)?
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
    ledger_provider.insert_message(&acct_id, message)?;
    ledger_provider.set_account_state(acct_id, acct_state)?;

    Ok(())
}

// TODO: this should be concretely serialized, perhaps SSZ?
fn deposit_log_to_msg_payload(dep: &DepositLog) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&dep.amount.to_be_bytes());
    payload.extend_from_slice(&dep.addr);
    payload
}

fn process_checkpoint(
    ckpt: &CheckpointUpdateLog,
    state_accessor: &mut impl StateAccessor<OLState, L1View>,
) -> StfResult<()> {
    state_accessor.set_recorded_epoch(ckpt.epoch_commitment);
    Ok(())
}
