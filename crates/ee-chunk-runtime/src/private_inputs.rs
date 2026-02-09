//! Generic input definitions.

use rkyv::{Archive, Deserialize, Serialize};
use rkyv_impl::archive_impl;
use ssz::{Decode, DecodeError, Encode};
use strata_acct_types::Hash;
use strata_codec::CodecError;
use strata_ee_acct_types::{ExecBlock, ExecutionEnvironment};
use strata_ee_chain_types::{ChunkTransition, ExecInputs, ExecOutputs};

/// Private inputs we expose to the runtime.
#[derive(Debug, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct PrivateInput {
    chunk_transition_ssz: Vec<u8>,
    raw_chunk: RawChunkData,
    raw_prev_header: Vec<u8>,
    raw_partial_pre_state: Vec<u8>,
}

impl PrivateInput {
    pub fn new(
        chunk_transition: ChunkTransition,
        raw_chunk: RawChunkData,
        raw_prev_header: Vec<u8>,
        raw_partial_pre_state: Vec<u8>,
    ) -> Self {
        Self {
            chunk_transition_ssz: chunk_transition.as_ssz_bytes(),
            raw_chunk,
            raw_prev_header,
            raw_partial_pre_state,
        }
    }

    pub fn raw_chunk(&self) -> &RawChunkData {
        &self.raw_chunk
    }
}

#[archive_impl]
impl PrivateInput {
    pub fn chunk_transition_ssz(&self) -> &[u8] {
        &self.chunk_transition_ssz
    }

    pub fn raw_prev_header(&self) -> &[u8] {
        &self.raw_prev_header
    }

    pub fn raw_partial_pre_state(&self) -> &[u8] {
        &self.raw_partial_pre_state
    }

    /// Tries to decode the chunk transition as its type.
    pub fn try_decode_chunk_transition(&self) -> Result<ChunkTransition, DecodeError> {
        ChunkTransition::from_ssz_bytes(self.chunk_transition_ssz())
    }

    /// Tries to decode the raw prev header for an execution environment.
    pub fn try_decode_prev_header<E: ExecutionEnvironment>(
        &self,
    ) -> Result<<E::Block as ExecBlock>::Header, CodecError> {
        strata_codec::decode_buf_exact(self.raw_prev_header())
    }

    /// Tries to decode the raw partial prestate for an execution environment.
    pub fn try_decode_pre_state<E: ExecutionEnvironment>(
        &self,
    ) -> Result<E::PartialState, CodecError> {
        strata_codec::decode_buf_exact(self.raw_partial_pre_state())
    }
}

impl ArchivedPrivateInput {
    pub fn raw_chunk(&self) -> &ArchivedRawChunkData {
        &self.raw_chunk
    }
}

#[derive(Debug, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct RawChunkData {
    blocks: Vec<RawBlockData>,
    prev_exec_blkid: [u8; 32],
}

impl RawChunkData {
    pub fn new(blocks: Vec<RawBlockData>, prev_exec_blkid: Hash) -> Self {
        Self {
            blocks,
            prev_exec_blkid: prev_exec_blkid.0,
        }
    }

    pub fn blocks(&self) -> &[RawBlockData] {
        &self.blocks
    }
}

#[archive_impl]
impl RawChunkData {
    pub fn prev_exec_blkid(&self) -> Hash {
        self.prev_exec_blkid.into()
    }
}

impl ArchivedRawChunkData {
    pub fn blocks(&self) -> &[ArchivedRawBlockData] {
        &self.blocks
    }
}

#[derive(Debug, Archive, Deserialize, Serialize)]
#[rkyv(derive(Debug))]
pub struct RawBlockData {
    /// Raw encoded block that we can pass to `Codec::decode`.
    raw_block: Vec<u8>,

    /// SSZed `ExecInputs`.
    exec_inputs_ssz: Vec<u8>,

    /// SSZed `ExecOutputs`.
    exec_outputs_ssz: Vec<u8>,
}

impl RawBlockData {
    pub fn new(raw_block: Vec<u8>, exec_inputs: ExecInputs, exec_outputs: ExecOutputs) -> Self {
        Self {
            raw_block,
            exec_inputs_ssz: exec_inputs.as_ssz_bytes(),
            exec_outputs_ssz: exec_outputs.as_ssz_bytes(),
        }
    }

    /// Constructs a new instance from a execution block (and IO) by encoding it.
    pub fn from_block<E: ExecutionEnvironment>(
        block: &E::Block,
        exec_inputs: ExecInputs,
        exec_outputs: ExecOutputs,
    ) -> Result<Self, CodecError> {
        Ok(Self::new(
            strata_codec::encode_to_vec(block)?,
            exec_inputs,
            exec_outputs,
        ))
    }
}

#[archive_impl]
impl RawBlockData {
    pub fn raw_block(&self) -> &[u8] {
        &self.raw_block
    }

    pub fn exec_inputs_ssz(&self) -> &[u8] {
        &self.exec_inputs_ssz
    }

    pub fn exec_outputs_ssz(&self) -> &[u8] {
        &self.exec_outputs_ssz
    }

    /// Tries to decode the raw block for an execution environment.
    pub fn try_decode_block<E: ExecutionEnvironment>(&self) -> Result<E::Block, CodecError> {
        strata_codec::decode_buf_exact::<E::Block>(self.raw_block())
    }

    /// Tries to decode the [`ExecInputs`] as its type.
    pub fn try_decode_exec_inputs(&self) -> Result<ExecInputs, DecodeError> {
        ExecInputs::from_ssz_bytes(self.exec_inputs_ssz())
    }

    /// Tries to decode the [`ExecOutputs`] as its type.
    pub fn try_decode_exec_outputs(&self) -> Result<ExecOutputs, DecodeError> {
        ExecOutputs::from_ssz_bytes(self.exec_outputs_ssz())
    }
}
