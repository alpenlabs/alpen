//! Abstract update types and related functionality.
//!
//! This does partially replicate transaction structures so that we have a clean
//! interface for what this crate's logic cares about.

use strata_acct_types::{ITxEffects, MessageEntry, TxEffects};
use strata_identifiers::Buf32;
use strata_snark_acct_types::{
    ISnarkAccountUpdateData, LedgerRefs, OutputMessage, OutputTransfer, ProofState, Seqno,
    UpdateOutputs,
};

/// Update data extracted from the transaction.
#[derive(Debug)]
pub struct SnarkAccountUpdateData {
    seq_no: Seqno,
    new_proof_state: ProofState,
    processed_messages: Vec<MessageEntry>,
    ledger_refs: LedgerRefs,
    effects: TxEffects,
    extra_data: Vec<u8>,
}

impl SnarkAccountUpdateData {
    pub fn new(
        seq_no: Seqno,
        new_proof_state: ProofState,
        processed_messages: Vec<MessageEntry>,
        ledger_refs: LedgerRefs,
        effects: TxEffects,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            seq_no,
            new_proof_state,
            processed_messages,
            ledger_refs,
            effects,
            extra_data,
        }
    }

    pub fn seq_no(&self) -> Seqno {
        self.seq_no
    }

    pub fn new_proof_state(&self) -> &ProofState {
        &self.new_proof_state
    }

    pub fn processed_messages(&self) -> &[MessageEntry] {
        &self.processed_messages
    }

    pub fn ledger_refs(&self) -> &LedgerRefs {
        &self.ledger_refs
    }

    pub fn effects(&self) -> &TxEffects {
        &self.effects
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }
}

impl<'u> ISnarkAccountUpdateData for &'u SnarkAccountUpdateData {
    type Message = &'u MessageEntry;
    type LedgerRefs = &'u LedgerRefs;
    type Effects = &'u TxEffects;

    fn seq_no(&self) -> Seqno {
        self.seq_no
    }

    fn new_inner_state(&self) -> Buf32 {
        self.new_proof_state.inner_state()
    }

    fn new_next_msg_idx(&self) -> u64 {
        self.new_proof_state.next_inbox_msg_idx()
    }

    fn num_messasges(&self) -> u64 {
        self.processed_messages.len() as u64
    }

    fn messages_iter(&self) -> impl Iterator<Item = Self::Message> {
        self.processed_messages.iter()
    }

    fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    fn ledger_refs(&self) -> Self::LedgerRefs {
        &self.ledger_refs
    }

    fn effects(&self) -> Self::Effects {
        &self.effects
    }
}

/// Converts [`TxEffects`] to [`UpdateOutputs`] for proof claim computation.
///
/// The snark proof verification requires [`UpdateOutputs`] format for the public
/// parameters. This converts the shared [`TxEffects`] type into the format
/// expected by [`UpdateProofPubParams`](strata_snark_acct_types::UpdateProofPubParams).
pub fn effects_to_update_outputs(effects: impl ITxEffects) -> UpdateOutputs {
    let transfers: Vec<OutputTransfer> = effects
        .transfers_iter()
        .map(|t| OutputTransfer::new(t.dest(), t.value()))
        .collect();

    let messages: Vec<OutputMessage> = effects
        .messages_iter()
        .map(|m| OutputMessage::new(m.dest(), m.payload().clone()))
        .collect();

    UpdateOutputs::new(transfers, messages)
}
