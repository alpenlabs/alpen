//! Chunk proof output types for inner/outer proof system.

use ssz_derive::{Decode, Encode};
use strata_snark_acct_types::{MessageEntry, ProofState, UpdateOutputs};

/// Public output committed by a chunk proof.
///
/// This is verified by the outer proof when aggregating chunks.
/// Uses SSZ encoding as per serialization guidelines for public proof interfaces.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ChunkProofOutput {
    /// Starting state of this chunk
    pub prev_state: ProofState,

    /// Ending state of this chunk
    pub new_state: ProofState,

    /// Messages processed in this chunk
    pub processed_messages: Vec<MessageEntry>,

    /// Outputs produced by this chunk (messages + transfers)
    pub outputs: UpdateOutputs,

    /// Application-specific extra data
    pub extra_data: Vec<u8>,
}

impl ChunkProofOutput {
    /// Create a new ChunkProofOutput
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
}
