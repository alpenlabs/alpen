//! Block transactional processing.

use strata_acct_types::{AccountId, AcctError, BitcoinAmount, MsgPayload, SentMessage};
use strata_codec::encode_to_vec;
use strata_ledger_types::{IAccountState, IAccountStateMut, ISnarkAccountStateMut, IStateAccessor};
use strata_ol_chain_types_new::{
    OLLog, OLTransaction, OLTxSegment, SnarkAccountUpdateLogData, TransactionAttachment,
    TransactionPayload,
};
use strata_snark_acct_sys as snark_sys;
use strata_snark_acct_types::{LedgerInterface, SnarkAccountUpdateContainer};

use crate::{
    account_processing,
    constants::SEQUENCER_ACCT_ID,
    context::{BasicExecContext, BlockContext, TxExecContext},
    errors::{ExecError, ExecResult},
    output::OutputCtx,
};

/// Process a block's transaction segment.
///
/// This is called for every block.
pub fn process_block_tx_segment<S: IStateAccessor>(
    state: &mut S,
    tx_seg: &OLTxSegment,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    for tx in tx_seg.txs() {
        process_single_tx(state, tx, context)?;
    }

    Ok(())
}

/// Processes a single tx, typically as part of a block.
///
/// This can also be used in mempool logic for trying to figure out if we can
/// apply a tx into a block.
pub fn process_single_tx<S: IStateAccessor>(
    state: &mut S,
    tx: &OLTransaction,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    // 1. Check the transaction's attachments.
    if !check_tx_attachments(tx.attachment(), &context.to_block_context()) {
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
            let target = *update.target();

            process_update_tx(state, target, update.update_container(), context)?;
        }
    }

    Ok(())
}

/// Container to accumulate effects of an account interaction we'll play out
/// later.
#[derive(Clone, Debug)]
struct AcctInteractionBuffer {
    messages: Vec<SentMessage>,
    transfers: Vec<(AccountId, BitcoinAmount)>,
}

impl AcctInteractionBuffer {
    fn new_empty() -> Self {
        Self {
            messages: Vec::new(),
            transfers: Vec::new(),
        }
    }

    fn add_sent_message(&mut self, sent_msg: SentMessage) {
        self.messages.push(sent_msg);
    }

    fn send_message_to(&mut self, dest: AccountId, payload: MsgPayload) {
        self.add_sent_message(SentMessage::new(dest, payload));
    }

    fn send_transfer_to(&mut self, dest: AccountId, amount: BitcoinAmount) {
        self.transfers.push((dest, amount));
    }
}

impl LedgerInterface for AcctInteractionBuffer {
    type Error = std::convert::Infallible;

    fn send_transfer(&mut self, dest: AccountId, value: BitcoinAmount) -> Result<(), Self::Error> {
        self.send_transfer_to(dest, value);
        Ok(())
    }

    fn send_message(&mut self, dest: AccountId, payload: MsgPayload) -> Result<(), Self::Error> {
        self.send_message_to(dest, payload);
        Ok(())
    }
}

fn process_update_tx<S: IStateAccessor>(
    state: &mut S,
    target: AccountId,
    update: &SnarkAccountUpdateContainer,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    let operation = update.base_update().operation();

    // Step 1: Read account state outside closure for verification
    let account_state = state
        .get_account_state(target)?
        .ok_or(ExecError::UnknownAccount(target))?;
    let snark_state = account_state
        .as_snark_account()
        .map_err(|_| ExecError::IncorrectTxTargetType)?;
    let cur_balance = account_state.balance();

    // Step 2: Verify the update (needs state.asm_manifests_mmr())
    let verified_update =
        snark_sys::verify_update_correctness(state, target, snark_state, update, cur_balance)?;

    // Step 3: Mutate and collect effects (inside closure)
    let (fx_buf, account_serial) = state.update_account(target, |astate| -> ExecResult<_> {
        // Deduct balance for all outputs first
        let total_sent = operation
            .outputs()
            .compute_total_value()
            .ok_or(ExecError::Acct(AcctError::BitcoinAmountOverflow))?;
        let coin = astate
            .take_balance(total_sent)
            .map_err(|_| ExecError::InsufficientAccountBalance(target, total_sent))?;
        coin.safely_consume_unchecked(); // TODO: better usage?

        // Now get snark account state and update proof state
        let sastate = astate
            .as_snark_account_mut()
            .map_err(|_| ExecError::IncorrectTxTargetType)?;

        sastate.update_inner_state(
            operation.new_state().inner_state(),
            operation.new_state().next_inbox_msg_idx(),
            operation.seq_no().into(),
            operation.extra_data(),
        )?;

        // Collect effects using snark-acct-sys
        let mut fx_buf = AcctInteractionBuffer::new_empty();
        snark_sys::apply_update_outputs(&mut fx_buf, verified_update)
            .expect("AcctInteractionBuffer operations are infallible");

        Ok((fx_buf, astate.serial()))
    })??;

    // Step 4: Apply effects
    apply_interactions(state, target, fx_buf, context.basic_context())?;

    // Step 5: Emit log
    let log_data = SnarkAccountUpdateLogData::new(
        operation.new_state().next_inbox_msg_idx(),
        operation.extra_data().to_vec(),
    )
    .ok_or(ExecError::Codec(
        strata_codec::CodecError::OverflowContainer,
    ))?;

    let log_payload = encode_to_vec(&log_data)?;
    let log = OLLog::new(account_serial, log_payload);
    context.emit_log(log);

    Ok(())
}

fn apply_interactions<S: IStateAccessor>(
    state: &mut S,
    source: AccountId,
    fx_buf: AcctInteractionBuffer,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // Process transfers: pure value transfers with no message data
    for (dest, amount) in fx_buf.transfers {
        let payload = MsgPayload::new(amount, vec![]);
        account_processing::process_message(state, source, dest, payload, context)?;
    }

    // Process messages: carry both value and data
    for m in fx_buf.messages {
        account_processing::process_message(state, source, m.dest, m.payload, context)?;
    }

    Ok(())
}

/// Checks that a tx is valid based on conditions in its attachments.  Returns
/// false if any condition is not satisfied.
///
/// This DOES NOT perform any other validation on the tx.
fn check_tx_attachments(atch: &TransactionAttachment, context: &BlockContext<'_>) -> bool {
    // Check slot ranges.
    if let Some(min_slot) = atch.min_slot()
        && context.slot() < min_slot
    {
        return false;
    }

    if let Some(max_slot) = atch.max_slot()
        && context.slot() > max_slot
    {
        return false;
    }

    true
}
