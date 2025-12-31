//! High-level procedures for processing updates.
//!
//! This module contains update/batch-level operations and generic utilities
//! shared between chunk and update processing.
//!
//! The main function is [`apply_update_operation_unconditionally`], which is used
//! outside the proof, after verifying the proof, to update our view of the
//! state, presumably with information extracted from DA.  This does not require
//! understanding the execution environment.
//!
//! For chunk-level processing, see [`crate::chunk_processing`].

use strata_acct_types::{AccountId, BitcoinAmount, Hash};
use strata_codec::decode_buf_exact;
use strata_ee_acct_types::{
    DecodedEeMessageData, EeAccountState, EnvError, EnvResult, MessageDecodeResult,
    PendingInputEntry, UpdateExtraData,
};
use strata_ee_chain_types::SubjectDepositData;
use strata_snark_acct_types::{MessageEntry, UpdateInputData};
use tree_hash::{Sha256Hasher, TreeHash};

/// Meta fields extracted from a message.
#[derive(Debug)]
pub(crate) struct MsgMeta {
    #[expect(dead_code, reason = "for future use")]
    pub(crate) source: AccountId,
    #[expect(dead_code, reason = "for future use")]
    pub(crate) incl_epoch: u32,
    pub(crate) value: BitcoinAmount,
}

/// Decoded message and its metadata.
#[derive(Debug)]
pub struct MsgData {
    pub(crate) meta: MsgMeta,
    pub(crate) message: DecodedEeMessageData,
}

impl MsgData {
    pub(crate) fn from_entry(m: &MessageEntry) -> MessageDecodeResult<Self> {
        let message = DecodedEeMessageData::decode_raw(m.payload_buf())?;
        let meta = MsgMeta {
            source: m.source(),
            incl_epoch: m.incl_epoch(),
            value: m.payload_value(),
        };

        Ok(Self { meta, message })
    }

    /// Creates a new `MsgData` for testing purposes.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn new_for_test(
        source: AccountId,
        incl_epoch: u32,
        value: BitcoinAmount,
        message: DecodedEeMessageData,
    ) -> Self {
        Self {
            meta: MsgMeta {
                source,
                incl_epoch,
                value,
            },
            message,
        }
    }

    pub fn value(&self) -> BitcoinAmount {
        self.meta.value
    }

    pub fn decoded_message(&self) -> &DecodedEeMessageData {
        &self.message
    }
}

/// Applies the effects of an update, but does not check the messages.  It's
/// assumed we have a proof attesting to the validity that transitively attests
/// to this.
///
/// This is used in clients after they have a proof for an update to reconstruct
/// the actual state proven by the proof.
pub fn apply_update_operation_unconditionally(
    astate: &mut EeAccountState,
    operation: &UpdateInputData,
) -> EnvResult<()> {
    let extra =
        decode_buf_exact(operation.extra_data()).map_err(|_| EnvError::MalformedExtraData)?;

    // 1. Apply the changes from the messages.
    for (i, inp) in operation.processed_messages().iter().enumerate() {
        let Some(msg) = MsgData::from_entry(inp).ok() else {
            continue;
        };

        apply_message(astate, &msg).map_err(make_inp_err_indexer(i))?;
    }

    // 2. Apply the extra data.
    apply_extra_data(astate, &extra)?;

    // 3. Verify the final EE state matches `new_state`.
    verify_acct_state_matches(astate, &operation.new_state().inner_state())?;

    Ok(())
}

/// Applies state changes from the message.
pub(crate) fn apply_message(astate: &mut EeAccountState, msg: &MsgData) -> EnvResult<()> {
    if !msg.meta.value.is_zero() {
        astate.add_tracked_balance(msg.meta.value);
    }

    match &msg.message {
        DecodedEeMessageData::Deposit(data) => {
            let deposit_data = SubjectDepositData::new(*data.dest_subject(), msg.meta.value);
            astate.add_pending_input(PendingInputEntry::Deposit(deposit_data));
        }

        DecodedEeMessageData::SubjTransfer(_data) => {
            // TODO
        }

        DecodedEeMessageData::Commit(_data) => {
            // Just ignore this one for now because we're not handling it.
            // TODO improve
        }
    }

    Ok(())
}

/// Applies account state changes described by [`UpdateExtraData`].
///
/// This is a **generic utility** used at multiple levels:
/// - **Block level**: After building a single block
/// - **Chunk level**: After processing a chunk of blocks
/// - **Update/Batch level**: After processing a full update
///
/// Updates the execution tip block ID and removes pending entries based on the extra data.
pub fn apply_extra_data(state: &mut EeAccountState, extra: &UpdateExtraData) -> EnvResult<()> {
    // 1. Update final execution head block.
    state.set_last_exec_blkid(*extra.new_tip_blkid());

    // 2. Update queues.
    state.remove_pending_inputs(*extra.processed_inputs() as usize);
    state.remove_pending_fincls(*extra.processed_fincls() as usize);

    Ok(())
}

pub(crate) fn verify_acct_state_matches(
    astate: &EeAccountState,
    exp_new_state: &Hash,
) -> Result<(), EnvError> {
    // Compute SSZ tree_hash_root
    let computed_root = TreeHash::<Sha256Hasher>::tree_hash_root(astate);
    let computed_hash = Hash::from(computed_root.0);

    if computed_hash != *exp_new_state {
        return Err(EnvError::InvalidBlock);
    }
    Ok(())
}

fn maybe_index_inp_err(e: EnvError, idx: usize) -> EnvError {
    match e {
        EnvError::MalformedCoinput => EnvError::MalformedCoinputIdx(idx),
        EnvError::MismatchedCoinput => EnvError::MismatchedCoinputIdx(idx),
        EnvError::InconsistentCoinput => EnvError::InconsistentCoinputIdx(idx),
        _ => e,
    }
}

pub(crate) fn make_inp_err_indexer(idx: usize) -> impl Fn(EnvError) -> EnvError {
    move |e| maybe_index_inp_err(e, idx)
}
