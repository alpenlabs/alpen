//! EVM write batch implementation.

use std::collections::BTreeMap;

use reth_trie::HashedPostState;
use revm::state::Bytecode;
use revm_primitives::B256;
use strata_codec::{Codec, CodecError};

use crate::codec_shims::{
    decode_bytes_with_length, decode_hashed_post_state, encode_bytes_with_length,
    encode_hashed_post_state,
};

/// Write batch for EVM execution containing state changes.
///
/// This wraps Reth's HashedPostState which contains the differences (deltas)
/// in account states and storage slots after executing a block. It's used to
/// apply state changes to the sparse EthereumState.
#[derive(Clone, Debug)]
pub struct EvmWriteBatch {
    hashed_post_state: HashedPostState,
    /// Bytecodes deployed while executing the block, keyed by code hash.
    bytecodes: BTreeMap<B256, Bytecode>,
}

impl EvmWriteBatch {
    /// Creates a new EvmWriteBatch from execution state changes.
    pub fn new(hashed_post_state: HashedPostState, bytecodes: BTreeMap<B256, Bytecode>) -> Self {
        Self {
            hashed_post_state,
            bytecodes,
        }
    }

    /// Gets a reference to the underlying HashedPostState.
    pub fn hashed_post_state(&self) -> &HashedPostState {
        &self.hashed_post_state
    }

    /// Gets bytecodes deployed by the block.
    pub fn bytecodes(&self) -> &BTreeMap<B256, Bytecode> {
        &self.bytecodes
    }

    /// Consumes self and returns the underlying HashedPostState.
    pub fn into_hashed_post_state(self) -> HashedPostState {
        self.hashed_post_state
    }
}

impl Codec for EvmWriteBatch {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        encode_hashed_post_state(&self.hashed_post_state, enc)?;

        (self.bytecodes.len() as u32).encode(enc)?;
        for (hash, bytecode) in &self.bytecodes {
            encode_bytes_with_length(&bytecode.original_bytes(), enc)?;
            enc.write_buf(hash.as_slice())?;
        }

        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        let hashed_post_state = decode_hashed_post_state(dec)?;
        let bytecode_count = u32::decode(dec)? as usize;
        let mut bytecodes = BTreeMap::new();

        for _ in 0..bytecode_count {
            let bytes = decode_bytes_with_length(dec)?;
            let bytecode = Bytecode::new_raw_checked(bytes.into())
                .map_err(|_| CodecError::MalformedField("Bytecode decode failed"))?;

            let mut hash_bytes = [0u8; 32];
            dec.read_buf(&mut hash_bytes)?;
            bytecodes.insert(B256::from(hash_bytes), bytecode);
        }

        Ok(Self {
            hashed_post_state,
            bytecodes,
        })
    }
}
