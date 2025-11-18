//! EVM partial state implementation.

use std::collections::BTreeMap;

use alloy_consensus::{BlockHeader, Header, Sealable};
use itertools::Itertools;
use revm::state::Bytecode;
use revm_primitives::{B256, map::HashMap};
use rsp_client_executor::io::TrieDB;
use rsp_mpt::EthereumState;
use strata_codec::{Codec, CodecError};
use strata_ee_acct_types::{EnvResult, ExecPartialState};

use super::Hash;
use crate::codec_shims::{
    decode_bytes_with_length, decode_rlp_with_length, encode_bytes_with_length,
    encode_rlp_with_length,
};
use crate::types::EvmWriteBatch;

/// Partial state for EVM block execution.
///
/// Contains the witness data needed to execute a block: the sparse Merkle Patricia Trie
/// state, contract bytecodes, and ancestor block headers for BLOCKHASH opcode support.
#[derive(Clone, Debug)]
pub struct EvmPartialState {
    /// The sparse Merkle Patricia Trie state from RSP
    ethereum_state: EthereumState,
    /// Contract bytecodes indexed by their hash for direct lookup during execution.
    /// BTreeMap is used (instead of HashMap) to ensure deterministic serialization order in Codec.
    bytecodes: BTreeMap<B256, Bytecode>,
    /// Ancestor block headers indexed by block number for BLOCKHASH opcode support
    ancestor_headers: BTreeMap<u64, Header>,
}

impl EvmPartialState {
    /// Creates a new EvmPartialState from an EthereumState with witness data.
    ///
    /// The bytecodes and ancestor_headers are converted from vectors to BTrees
    /// for efficient lookup during block execution.
    pub fn new(
        ethereum_state: EthereumState,
        bytecodes: Vec<Bytecode>,
        ancestor_headers: Vec<Header>,
    ) -> Self {
        // Index bytecodes by their hash for O(log n) lookup
        let bytecodes = bytecodes
            .into_iter()
            .map(|code| (code.hash_slow(), code))
            .collect();

        // Index ancestor headers by block number for O(log n) lookup by BLOCKHASH opcode
        let ancestor_headers = ancestor_headers
            .into_iter()
            .map(|header| (header.number, header))
            .collect();

        Self {
            ethereum_state,
            bytecodes,
            ancestor_headers,
        }
    }

    /// Gets a reference to the underlying EthereumState.
    pub fn ethereum_state(&self) -> &EthereumState {
        &self.ethereum_state
    }

    /// Gets a mutable reference to the underlying EthereumState.
    pub fn ethereum_state_mut(&mut self) -> &mut EthereumState {
        &mut self.ethereum_state
    }

    /// Gets a reference to the bytecodes map.
    pub fn bytecodes(&self) -> &BTreeMap<B256, Bytecode> {
        &self.bytecodes
    }

    /// Gets a reference to the ancestor headers map.
    pub fn ancestor_headers(&self) -> &BTreeMap<u64, Header> {
        &self.ancestor_headers
    }

    /// Prepares witness database for block execution.
    ///
    /// This builds the necessary maps (block_hashes, bytecode_by_hash) from the witness data
    /// and creates a TrieDB ready for EVM execution.
    ///
    /// # Panics
    /// Panics if the header chain is invalid (block numbers or parent hashes don't match).
    pub fn prepare_witness_db<'a>(&'a self, current_header: &Header) -> TrieDB<'a> {
        // Seal the current block header and ancestor headers by reference (no clones)
        let current_sealed = current_header.seal_ref_slow();
        let sealed_headers = std::iter::once(current_sealed)
            .chain(self.ancestor_headers.values().map(|h| h.seal_ref_slow()))
            .collect::<Vec<_>>();

        // Verify and build block_hashes from sealed headers for BLOCKHASH opcode support
        // This validates the header chain integrity by checking that each parent's computed hash
        // matches the child's parent_hash field
        let mut block_hashes: HashMap<u64, B256> = HashMap::with_hasher(Default::default());
        for (child_header, parent_header) in sealed_headers.iter().tuple_windows() {
            // Validate block number continuity
            assert_eq!(
                parent_header.number() + 1,
                child_header.number(),
                "Invalid header block number: expected {}, got {}",
                parent_header.number() + 1,
                child_header.number()
            );

            // Validate parent hash matches
            let parent_header_hash = parent_header.hash();
            assert_eq!(
                parent_header_hash,
                child_header.parent_hash(),
                "Invalid header parent hash: expected {}, got {}",
                parent_header_hash,
                child_header.parent_hash()
            );

            block_hashes.insert(parent_header.number(), child_header.parent_hash());
        }

        // Build bytecode_by_hash from bytecodes for contract execution
        // Since bytecodes are already indexed by hash in the BTreeMap, we can iterate directly
        let bytecode_by_hash: HashMap<B256, &revm::state::Bytecode> = self
            .bytecodes
            .iter()
            .map(|(hash, code)| (*hash, code))
            .collect();

        // Create and return TrieDB
        TrieDB::new(&self.ethereum_state, block_hashes, bytecode_by_hash)
    }

    /// Computes the new state root by merging hashed post state changes into this state.
    ///
    /// This clones the current state, applies the changes from the hashed post state,
    /// and computes the resulting state root.
    pub fn compute_state_root_with_changes(
        &self,
        hashed_post_state: &reth_trie::HashedPostState,
    ) -> revm_primitives::B256 {
        let mut updated_state = self.ethereum_state.clone();
        updated_state.update(hashed_post_state);
        updated_state.state_root()
    }

    /// Merges a write batch into this state by applying the hashed post state changes.
    ///
    /// This updates the internal EthereumState with the changes from the write batch.
    pub fn merge_write_batch(&mut self, wb: &EvmWriteBatch) {
        self.ethereum_state.update(wb.hashed_post_state());
    }
}

impl ExecPartialState for EvmPartialState {
    fn compute_state_root(&self) -> EnvResult<Hash> {
        let state_root = self.ethereum_state.state_root();
        Ok(state_root.into())
    }
}

impl Codec for EvmPartialState {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // Encode EthereumState using bincode (it has serde support and is a complex MPT structure)
        let ethereum_state_bytes = bincode::serialize(&self.ethereum_state)
            .map_err(|_| CodecError::MalformedField("EthereumState serialize failed"))?;
        encode_bytes_with_length(&ethereum_state_bytes, enc)?;

        // Encode bytecodes count
        (self.bytecodes.len() as u32).encode(enc)?;
        // Encode each bytecode as raw bytes (iterate over BTreeMap values)
        for bytecode in self.bytecodes.values() {
            encode_bytes_with_length(&bytecode.original_bytes(), enc)?;
        }

        // Encode ancestor headers count
        (self.ancestor_headers.len() as u32).encode(enc)?;
        // Encode each header using RLP helper (iterate over BTreeMap values)
        for header in self.ancestor_headers.values() {
            encode_rlp_with_length(header, enc)?;
        }

        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // Decode EthereumState using bincode
        let ethereum_state_bytes = decode_bytes_with_length(dec)?;
        let ethereum_state = bincode::deserialize(&ethereum_state_bytes)
            .map_err(|_| CodecError::MalformedField("EthereumState deserialize failed"))?;

        // Decode bytecodes
        let bytecodes_count = u32::decode(dec)? as usize;
        let mut bytecodes = Vec::with_capacity(bytecodes_count);
        for _ in 0..bytecodes_count {
            let bytes = decode_bytes_with_length(dec)?;
            let bytecode = Bytecode::new_raw_checked(bytes.into())
                .map_err(|_| CodecError::MalformedField("Bytecode decode failed"))?;
            bytecodes.push(bytecode);
        }

        // Decode ancestor headers
        let headers_count = u32::decode(dec)? as usize;
        let mut ancestor_headers = Vec::with_capacity(headers_count);
        for _ in 0..headers_count {
            ancestor_headers.push(decode_rlp_with_length(dec)?);
        }

        // Use the constructor which will convert Vecs to BTrees
        Ok(Self::new(ethereum_state, bytecodes, ancestor_headers))
    }
}
