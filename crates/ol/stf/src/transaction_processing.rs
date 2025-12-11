//! Block transactional processing.

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SentMessage};
use strata_codec::encode_to_vec;
use strata_ledger_types::{
    IAccountState, IAccountStateMut, ISnarkAccountStateMut, IStateAccessor,
};
use strata_ol_chain_types_new::{
    OLLog, OLTransaction, OLTxSegment, SnarkAccountUpdateLogData, TransactionAttachment,
    TransactionPayload,
};
use strata_snark_acct_types::SnarkAccountUpdateContainer;

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
}

impl AcctInteractionBuffer {
    fn new_empty() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    fn add_sent_message(&mut self, sent_msg: SentMessage) {
        self.messages.push(sent_msg);
    }

    fn send_message_to(&mut self, dest: AccountId, payload: MsgPayload) {
        self.add_sent_message(SentMessage::new(dest, payload));
    }
}

fn process_update_tx<S: IStateAccessor>(
    state: &mut S,
    target: AccountId,
    update: &SnarkAccountUpdateContainer,
    context: &TxExecContext<'_>,
) -> ExecResult<()> {
    // We need to collect information for logging and effects from within the
    // closure, then apply them after.
    let seqno = update.operation().seq_no();
    let new_state = update.operation().new_state();
    let extra_data = update.operation().extra_data();
    let outputs = update.operation().outputs();

    // Update the account within the closure, collecting effects to apply after.
    let (fx_buf, account_serial) = state.update_account(target, |astate| -> ExecResult<_> {
        // 1. Make sure it's a snark account and get a mutable reference.
        let sastate = astate
            .as_snark_account_mut()
            .map_err(|_| ExecError::IncorrectTxTargetType)?;

        // 2. Call the snark account machinery to process the update.
        //
        // XXX This implementation is very limited because we don't want to support
        // the full snark account functionality yet.  We don't check anything, we
        // just update the fields as we're told to without authenticating anything.
        //
        // TODO make this the full implementation, this is where we'd call out to it
        // instead of just doing it here

        // This just calls the function to update the state as we would if we
        // actually were doing the real implementation.
        sastate.update_inner_state(
            new_state.inner_state(),
            new_state.next_inbox_msg_idx(),
            seqno.into(),
            extra_data,
        )?;

        // We also have to extract the effects here too, subtracting balance in the
        // process.
        let mut fx_buf = AcctInteractionBuffer::new_empty();
        for m in outputs.messages() {
            let coin = astate
                .take_balance(m.payload().value())
                .map_err(|_| ExecError::InsufficientAccountBalance(target, m.payload().value()))?;
            coin.safely_consume_unchecked(); // TODO track this better
            fx_buf.send_message_to(m.dest(), m.payload().clone());
        }

        // Capture the account serial for the log.
        let account_serial = astate.serial();

        Ok((fx_buf, account_serial))
    })??;

    // 3. Apply the effects (after the account update closure completes).
    apply_interactions(state, target, fx_buf, context.basic_context())?;

    // 4. Emit a log message.
    // According to the spec, the log should contain:
    // - new_msg_idx: The sequence number from the account state
    // - extra_data: The extra data from the update operation
    // TODO improve codec error handling here when more stuff is SSZed
    let log_data =
        SnarkAccountUpdateLogData::new(new_state.next_inbox_msg_idx, extra_data.to_vec()).ok_or(
            ExecError::Codec(strata_codec::CodecError::OverflowContainer),
        )?;

    // Encode the log data and emit it
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
    // Send the messages off to each of the targets.
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
