//! Abstract update types and related functionality.
//!
//! This does partially replicate transaction structures so that we have a clean
//! interface for what this crate's logic cares about.

use strata_acct_types::{MessageEntry, TxEffects};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{
    LedgerRefs, OutputMessage, OutputTransfer, ProofState, Seqno, UpdateOutputs,
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
    new_predicate: Option<PredicateKey>,
}

impl SnarkAccountUpdateData {
    pub fn new(
        seq_no: Seqno,
        new_proof_state: ProofState,
        processed_messages: Vec<MessageEntry>,
        ledger_refs: LedgerRefs,
        effects: TxEffects,
        extra_data: Vec<u8>,
        new_predicate: Option<PredicateKey>,
    ) -> Self {
        Self {
            seq_no,
            new_proof_state,
            processed_messages,
            ledger_refs,
            effects,
            extra_data,
            new_predicate,
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

    pub fn new_predicate(&self) -> Option<&PredicateKey> {
        self.new_predicate.as_ref()
    }
}

/// Converts [`TxEffects`] to [`UpdateOutputs`] for proof claim computation.
///
/// The snark proof verification requires [`UpdateOutputs`] format for the public
/// parameters. This converts the shared [`TxEffects`] type into the format
/// expected by [`UpdateProofPubParams`](strata_snark_acct_types::UpdateProofPubParams).
/// The declared predicate rotation is not part of [`TxEffects`], so it is
/// passed alongside; the resulting outputs carry it into the proof claim,
/// binding the tx's declaration to what the update proof committed to.
pub fn effects_to_update_outputs(
    effects: &TxEffects,
    new_predicate: Option<&PredicateKey>,
) -> UpdateOutputs {
    let transfers: Vec<OutputTransfer> = effects
        .transfers_iter()
        .map(|t| OutputTransfer::new(t.dest(), t.value()))
        .collect();

    let messages: Vec<OutputMessage> = effects
        .messages_iter()
        .map(|m| OutputMessage::new(m.dest(), m.payload().clone()))
        .collect();

    let mut outputs = UpdateOutputs::new_empty()
        .with_transfers(transfers)
        .with_messages(messages);
    outputs.set_new_predicate(new_predicate.cloned());
    outputs
}
