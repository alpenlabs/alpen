//! EVM write batch implementation.

use std::collections::BTreeMap;

use reth_trie::HashedPostState;
use revm::state::Bytecode;
use revm_primitives::{B256, alloy_primitives::Bloom};
use strata_acct_types::Hash;
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
///
/// Also stores execution metadata (state root from header intrinsics, logs bloom)
/// needed for block header completion and state root verification during merge.
#[derive(Clone, Debug)]
pub struct EvmWriteBatch {
    hashed_post_state: HashedPostState,
    /// The state root extracted from block header intrinsics.
    ///
    /// This value is taken directly from `header_intrinsics.state_root` during
    /// block execution, NOT computed from the pre-state. Actual verification
    /// occurs in `merge_write_into_state` after the state is mutated, where
    /// we compute the real state root and compare it against this value.
    /// This approach avoids an expensive state clone in zkVM.
    intrinsics_state_root: Hash,
    /// The accumulated logs bloom from all receipts
    logs_bloom: Bloom,
    /// Bytecodes created while executing this block, keyed by code hash.
    created_bytecodes: BTreeMap<B256, Bytecode>,
}

impl EvmWriteBatch {
    /// Creates a new EvmWriteBatch from a HashedPostState and header intrinsics metadata.
    pub fn new(
        hashed_post_state: HashedPostState,
        intrinsics_state_root: Hash,
        logs_bloom: Bloom,
        created_bytecodes: Vec<Bytecode>,
    ) -> Self {
        let created_bytecodes = created_bytecodes
            .into_iter()
            .filter(|bytecode| !bytecode.original_bytes().is_empty())
            .map(|bytecode| (bytecode.hash_slow(), bytecode))
            .collect();

        Self {
            hashed_post_state,
            intrinsics_state_root,
            logs_bloom,
            created_bytecodes,
        }
    }

    /// Gets a reference to the underlying HashedPostState.
    pub fn hashed_post_state(&self) -> &HashedPostState {
        &self.hashed_post_state
    }

    /// Gets the state root from block header intrinsics.
    ///
    /// This value is verified against the actual computed state root
    /// during `merge_write_into_state`.
    pub fn intrinsics_state_root(&self) -> Hash {
        self.intrinsics_state_root
    }

    /// Gets the accumulated logs bloom.
    pub fn logs_bloom(&self) -> Bloom {
        self.logs_bloom
    }

    /// Gets the bytecodes created while executing this block.
    pub fn created_bytecodes(&self) -> impl Iterator<Item = &Bytecode> {
        self.created_bytecodes.values()
    }

    /// Consumes self and returns the underlying HashedPostState.
    pub fn into_hashed_post_state(self) -> HashedPostState {
        self.hashed_post_state
    }
}

impl Codec for EvmWriteBatch {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // Encode HashedPostState using custom deterministic encoding
        encode_hashed_post_state(&self.hashed_post_state, enc)?;

        // Encode intrinsics_state_root (32 bytes)
        enc.write_buf(&self.intrinsics_state_root.0)?;

        // Encode logs_bloom (256 bytes)
        enc.write_buf(self.logs_bloom.as_slice())?;

        // Encode bytecodes in deterministic hash order.
        (self.created_bytecodes.len() as u32).encode(enc)?;
        for (hash, bytecode) in &self.created_bytecodes {
            encode_bytes_with_length(&bytecode.original_bytes(), enc)?;
            enc.write_buf(hash.as_slice())?;
        }

        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // Decode HashedPostState using custom deterministic decoding
        let hashed_post_state = decode_hashed_post_state(dec)?;

        // Decode intrinsics_state_root (32 bytes)
        let mut intrinsics_state_root_bytes = [0u8; 32];
        dec.read_buf(&mut intrinsics_state_root_bytes)?;
        let intrinsics_state_root = Hash::new(intrinsics_state_root_bytes);

        // Decode logs_bloom (256 bytes)
        let mut logs_bloom_bytes = [0u8; 256];
        dec.read_buf(&mut logs_bloom_bytes)?;
        let logs_bloom = Bloom::from(logs_bloom_bytes);

        // Decode bytecodes with their pre-computed hashes.
        let bytecode_count = u32::decode(dec)? as usize;
        let mut created_bytecodes = BTreeMap::new();
        for _ in 0..bytecode_count {
            let bytes = decode_bytes_with_length(dec)?;
            let bytecode = Bytecode::new_raw_checked(bytes.into())
                .map_err(|_| CodecError::MalformedField("Bytecode decode failed"))?;

            let mut hash_bytes = [0u8; 32];
            dec.read_buf(&mut hash_bytes)?;
            let hash = B256::from(hash_bytes);

            created_bytecodes.insert(hash, bytecode);
        }

        Ok(Self {
            hashed_post_state,
            intrinsics_state_root,
            logs_bloom,
            created_bytecodes,
        })
    }
}
