//! Chunk operation data for inner proof verification.
//!
//! ChunkOperationData is similar to UpdateOperationData but scoped to a single chunk
//! and does NOT include ledger_refs (DA verification is outer proof's responsibility).

use ssz_derive::{Decode, Encode};
use strata_snark_acct_types::{MessageEntry, ProofState, UpdateOutputs};

/// Claims about what executing this chunk should produce.
///
/// Verified during chunk execution in the inner proof.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ChunkOperationData {
    /// Expected starting state for this chunk
    prev_state: ProofState,

    /// Expected ending state after executing chunk blocks
    new_state: ProofState,

    /// Expected messages to be processed in this chunk
    processed_messages: Vec<MessageEntry>,

    /// Expected outputs (messages + transfers) from this chunk
    outputs: UpdateOutputs,

    /// Application-specific extra data
    extra_data: Vec<u8>,
}

impl ChunkOperationData {
    /// Create a new ChunkOperationData
    pub fn new(
        prev_state: ProofState,
        new_state: ProofState,
        processed_messages: Vec<MessageEntry>,
        outputs: UpdateOutputs,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            prev_state,
            new_state,
            processed_messages,
            outputs,
            extra_data,
        }
    }

    /// Get the previous state
    pub fn prev_state(&self) -> &ProofState {
        &self.prev_state
    }

    /// Get the new state
    pub fn new_state(&self) -> &ProofState {
        &self.new_state
    }

    /// Get processed messages
    pub fn processed_messages(&self) -> &[MessageEntry] {
        &self.processed_messages
    }

    /// Get outputs
    pub fn outputs(&self) -> &UpdateOutputs {
        &self.outputs
    }

    /// Get extra data
    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    /// Consume and destructure into all components
    pub fn into_parts(
        self,
    ) -> (
        ProofState,
        ProofState,
        Vec<MessageEntry>,
        UpdateOutputs,
        Vec<u8>,
    ) {
        (
            self.prev_state,
            self.new_state,
            self.processed_messages,
            self.outputs,
            self.extra_data,
        )
    }
}
