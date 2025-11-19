//! Block transactional processing.

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
use strata_ledger_types::{AccountTypeState, IAccountState, StateAccessor};
use strata_ol_chain_types_new::{
    OLTransaction, OLTxSegment, TransactionAttachment, TransactionPayload,
};
use strata_snark_acct_types::SnarkAccountUpdateContainer;

use crate::{
    account_processing,
    constants::SEQUENCER_ACCT_ID,
    context::{BlockContext, TxExecContext},
    errors::{ExecError, ExecResult},
};

/// Process a block's transaction segment.
///
/// This is called for every block.
pub fn process_block_tx_segment<S: StateAccessor>(
    state: &mut S,
    tx_seg: &OLTxSegment,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    for tx in tx_seg.txs() {
        process_single_tx(state, tx, context)?;
    }

    // TODO
    Ok(())
}

/// Processes a single tx, typically as part of a block.
///
/// This can also be used in mempool logic for trying to figure out if we can
/// apply a tx into a block.
pub fn process_single_tx<S: StateAccessor>(
    state: &mut S,
    tx: &OLTransaction,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    // 1. Check the transaction's attachments.
    if !check_tx_attachments(tx.attachments(), &context.to_block_context()) {
        return Err(ExecError::TxConditionCheckFailed);
    }

    // 2. Depending on its payload type, we handle it different ways.
    match tx.payload() {
        TransactionPayload::GenericAccountMessage(gam) => {
            // Construct the message we want to send and then hand it off.
            let mp = MsgPayload::new(BitcoinAmount::from(0), gam.payload().to_vec());
            account_processing::process_message(
                state,
                SEQUENCER_ACCT_ID,
                *gam.target(),
                mp,
                context.basic_context(),
            )?;
        }

        TransactionPayload::SnarkAccountUpdate(update) => {
            // 1. Fetch the account and make sure it's a snark account we can use.
            let astate = state
                .get_account_state_mut(*update.target())?
                .ok_or(ExecError::UnknownAccount(*update.target()))?;

            let AccountTypeState::Snark(sas) = astate.get_type_state()? else {
                return Err(ExecError::IncorrectTxTargetType);
            };

            process_update_tx(
                state,
                update.target(),
                sas,
                update.update_container(),
                context,
            )?;
        }
    }

    Ok(())
}

fn process_update_tx<S: StateAccessor>(
    state: &mut S,
    target: &AccountId,
    mut sastate: <S::AccountState as IAccountState>::SnarkAccountState,
    update: &SnarkAccountUpdateContainer,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    // TODO snark account processing

    Ok(())
}

/// Checks that a tx is valid based on conditions in its attachments.  Returns
/// false if any condition is not satisfied.
///
/// This DOES NOT perform any other validation on the tx.
fn check_tx_attachments(atch: &TransactionAttachment, context: &BlockContext<'_>) -> bool {
    // Check slot ranges.
    if let Some(min_slot) = atch.min_slot() {
        if context.slot() < min_slot {
            return false;
        }
    }

    if let Some(max_slot) = atch.max_slot() {
        if context.slot() > max_slot {
            return false;
        }
    }

    true
}
