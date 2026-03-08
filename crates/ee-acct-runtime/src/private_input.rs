use rkyv::{Archive, Deserialize, Serialize};
use rkyv_impl::archive_impl;
use ssz::{Decode, DecodeError, Encode};
use strata_ee_acct_types::CommitChainSegment;
use strata_ee_chain_types::ChunkTransition;

/// EE update private input.
///
/// This is intended to be passed separately from the snark account update
/// input.
#[derive(Clone, Debug, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct PrivateInput {
    /// Previous header that we already have in our state.
    raw_prev_header: Vec<u8>,

    /// Partial pre-state corresponding to the previous header.
    raw_partial_pre_state: Vec<u8>,

    /// Raw chunks we're going to process in the order we're processing them.
    chunks: Vec<ChunkInput>,
}

impl PrivateInput {
    pub fn new(
        raw_prev_header: Vec<u8>,
        raw_partial_pre_state: Vec<u8>,
        chunks: Vec<ChunkInput>,
    ) -> Self {
        Self {
            raw_prev_header,
            raw_partial_pre_state,
            chunks,
        }
    }

    pub fn chunks(&self) -> &[ChunkInput] {
        &self.chunks
    }
}

#[archive_impl]
impl PrivateInput {
    pub fn raw_prev_header(&self) -> &[u8] {
        &self.raw_prev_header
    }

    pub fn raw_partial_pre_state(&self) -> &[u8] {
        &self.raw_partial_pre_state
    }
}

impl ArchivedPrivateInput {
    pub fn chunks(&self) -> &[ArchivedChunkInput] {
        &self.chunks
    }
}

/// A chunk transition and its validity proof.
#[derive(Clone, Debug, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct ChunkInput {
    chunk_transition_ssz: Vec<u8>,
    proof: Vec<u8>,
}

impl ChunkInput {
    pub fn new(chunk_transition: ChunkTransition, proof: Vec<u8>) -> Self {
        Self {
            chunk_transition_ssz: chunk_transition.as_ssz_bytes(),
            proof,
        }
    }
}

#[archive_impl]
impl ChunkInput {
    pub fn chunk_transition_ssz(&self) -> &[u8] {
        &self.chunk_transition_ssz
    }

    pub fn proof(&self) -> &[u8] {
        &self.proof
    }

    /// Tries to decode the chunk transition as its type.
    pub fn try_decode_chunk_transition(&self) -> Result<ChunkTransition, DecodeError> {
        ChunkTransition::from_ssz_bytes(self.chunk_transition_ssz())
    }
}
