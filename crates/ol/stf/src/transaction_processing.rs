//! Block transactional processing.

use strata_acct_types::{
    AccountId, AccountTypeId, AcctError, BitcoinAmount, MsgPayload, SentMessage, SentTransfer,
};
use strata_codec::{CodecError, encode_to_vec};
use strata_ledger_types::{
    IAccountState, IAccountStateMut, ISnarkAccountState, ISnarkAccountStateMut, IStateAccessor,
};
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
    check_tx_attachment(tx.attachment(), state)?;

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
    transfers: Vec<SentTransfer>,
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
        self.transfers.push(SentTransfer::new(dest, amount));
    }
}

impl LedgerInterface for AcctInteractionBuffer {
    type Error = ExecError;

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
    // Step 1: Read account state outside closure for verification
    let account_state = state
        .get_account_state(target)?
        .ok_or(ExecError::UnknownAccount(target))?;
    let acc_serial = account_state.serial();
    let snark_acct_state = account_state
        .as_snark_account()
        .map_err(|_| ExecError::IncorrectTxTargetType)?;
    let cur_balance = account_state.balance();

    // Step 2: Verify the update (needs state.asm_manifests_mmr())
    snark_sys::verify_update_correctness(state, target, snark_acct_state, update, cur_balance)?;

    let operation = update.operation();

    // Validate sequence number
    let current_seqno = *snark_acct_state.seqno().inner();
    check_snark_account_seq_no(target, operation.seq_no(), current_seqno)?;

    // Step 3: Mutate and collect effects (inside closure)
    let fx_buf = state.update_account(target, |astate| -> ExecResult<_> {
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
        let snrk_acct_state = astate
            .as_snark_account_mut()
            .map_err(|_| ExecError::IncorrectTxTargetType)?;

        let new_seqno = operation.seq_no().saturating_add(1);
        snrk_acct_state.update_inner_state(
            operation.new_proof_state().inner_state(),
            operation.new_proof_state().next_inbox_msg_idx(),
            new_seqno.into(),
            operation.extra_data(),
        )?;

        // Collect effects using snark-acct-sys
        let mut fx_buf = AcctInteractionBuffer::new_empty();
        snark_sys::apply_update_outputs(&mut fx_buf, update)?;

        Ok(fx_buf)
    })??;

    // Step 4: Apply effects
    apply_interactions(state, target, fx_buf, context.basic_context())?;

    Ok(())
}

fn apply_interactions<S: IStateAccessor>(
    state: &mut S,
    source: AccountId,
    fx_buf: AcctInteractionBuffer,
    context: &BasicExecContext<'_>,
) -> ExecResult<()> {
    // Process transfers: pure value transfers with no message data
    for t in fx_buf.transfers {
        account_processing::process_transfer(state, source, t.dest, t.value, context)?;
    }

    // Process messages: carry both value and data
    for m in fx_buf.messages {
        account_processing::process_message(state, source, m.dest, m.payload, context)?;
    }

    Ok(())
}

/// Checks that a transaction's slot bounds are valid for the current slot in state.
///
/// Returns:
/// - `Ok(())` if transaction is valid for current slot
/// - `Err(TransactionExpired)` if `max_slot` is set and `current_slot > max_slot`
/// - `Err(TransactionNotMature)` if `min_slot` is set and `current_slot < min_slot`
///
/// This can be used by mempool for early rejection and by block assembly/STF for validation.
pub(crate) fn check_slot_bounds<S: IStateAccessor>(
    attachment: &TransactionAttachment,
    state: &S,
) -> ExecResult<()> {
    let current_slot = state.cur_slot();

    // Check min_slot (transaction not yet valid)
    if let Some(min_slot) = attachment.min_slot()
        && current_slot < min_slot
    {
        return Err(ExecError::TransactionNotMature(min_slot, current_slot));
    }

    // Check max_slot (transaction expired)
    if let Some(max_slot) = attachment.max_slot()
        && current_slot > max_slot
    {
        return Err(ExecError::TransactionExpired(max_slot, current_slot));
    }

    Ok(())
}

/// Validates transaction sequence number using next-expected semantics.
pub fn check_snark_account_seq_no(
    account: AccountId,
    tx_seq_no: u64,
    expected_seq_no: u64,
) -> ExecResult<()> {
    if tx_seq_no != expected_seq_no {
        return Err(ExecError::InvalidSequenceNumber(
            account,
            expected_seq_no,
            tx_seq_no,
        ));
    }
    Ok(())
}

/// Gets an account state, returning an error if it doesn't exist.
///
/// Returns Ok(account_state) if account exists.
/// Returns Err(UnknownAccount) if account doesn't exist.
///
/// This helper is used by mempool and block assembly for account existence validation.
pub fn get_account_state<S: IStateAccessor>(
    state: &S,
    account: AccountId,
) -> ExecResult<&S::AccountState> {
    state
        .get_account_state(account)?
        .ok_or(ExecError::UnknownAccount(account))
}

/// Gets the current sequence number for a Snark account.
///
/// Returns Ok(seq_no) if account exists and is a Snark account.
/// Returns Err if account doesn't exist or is not a Snark account.
///
/// This helper is used by mempool and block assembly for sequence number validation.
pub fn get_snark_account_seq_no<S: IStateAccessor>(
    state: &S,
    account: AccountId,
) -> ExecResult<u64> {
    let account_state = get_account_state(state, account)?;

    if account_state.ty() != AccountTypeId::Snark {
        return Err(ExecError::IncorrectTxTargetType);
    }

    let snark_state = account_state.as_snark_account()?;

    Ok(*snark_state.seqno().inner())
}

/// Checks that a tx is valid based on conditions in its attachments.
///
/// This DOES NOT perform any other validation on the tx.
pub fn check_tx_attachment<S: IStateAccessor>(
    attachment: &TransactionAttachment,
    state: &S,
) -> ExecResult<()> {
    check_slot_bounds(attachment, state)
}
