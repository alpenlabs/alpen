//! EVM partial state implementation.

use std::collections::BTreeMap;

use alloy_consensus::{BlockHeader, Header, Sealable, Sealed};
use itertools::Itertools;
use revm::state::Bytecode;
use revm_primitives::{B256, map::HashMap};
use rsp_mpt::EthereumState;
use strata_codec::{Codec, CodecError};
use strata_ee_acct_types::{EnvResult, ExecPartialState};

use super::Hash;
use crate::{
    codec_shims::{
        decode_bytes_with_length, decode_rlp_with_length, encode_bytes_with_length,
        encode_rlp_with_length,
    },
    types::{EvmWriteBatch, WitnessDB},
};

/// Partial state for EVM block execution.
///
/// Contains the witness data needed to execute a block: the sparse Merkle Patricia Trie
/// state, contract bytecodes, and ancestor block headers for BLOCKHASH opcode support.
///
/// This struct pre-computes expensive operations (header hashing, block hash map) during
/// construction to avoid repeated work when preparing witness databases.
#[derive(Clone, Debug)]
pub struct EvmPartialState {
    /// The sparse Merkle Patricia Trie state from RSP
    ethereum_state: EthereumState,
    /// Contract bytecodes indexed by their hash for direct lookup during execution.
    /// BTreeMap is used (instead of HashMap) to ensure deterministic serialization order in Codec.
    bytecodes: BTreeMap<B256, Bytecode>,
    /// Ancestor block headers with pre-computed hashes, indexed by block number.
    /// Headers are sealed once during construction to avoid repeated hash computations.
    ancestor_headers: BTreeMap<u64, Sealed<Header>>,
    /// Pre-computed block hash lookup map for BLOCKHASH opcode.
    /// Built once during construction from sealed ancestor headers.
    block_hashes: HashMap<u64, B256>,
}

impl EvmPartialState {
    /// Creates a new EvmPartialState from an EthereumState with witness data.
    ///
    /// This performs expensive one-time operations optimized for zkVM execution:
    /// - Hashes all bytecodes once
    /// - Seals all ancestor headers (computes their hashes once)
    /// - Validates header chain integrity
    /// - Builds block_hashes lookup map once
    ///
    /// These operations are done once at construction to avoid repeated work
    /// during sequential block execution in zkVM.
    ///
    /// # Panics
    /// Panics if the header chain is invalid (block numbers or parent hashes don't match).
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

        // Seal ancestor headers once (compute hashes) and index by block number
        let ancestor_headers: BTreeMap<u64, Sealed<Header>> = ancestor_headers
            .into_iter()
            .map(|header| {
                let block_num = header.number;
                (block_num, header.seal_slow())
            })
            .collect();

        // Validate header chain and build block_hashes map once
        let mut block_hashes: HashMap<u64, B256> = HashMap::with_hasher(Default::default());
        for (child_sealed, parent_sealed) in ancestor_headers.values().tuple_windows() {
            // Validate block number continuity
            assert_eq!(
                parent_sealed.number() + 1,
                child_sealed.number(),
                "Invalid header block number: expected {}, got {}",
                parent_sealed.number() + 1,
                child_sealed.number()
            );

            // Validate parent hash matches
            let parent_hash = parent_sealed.hash();
            assert_eq!(
                parent_hash,
                child_sealed.parent_hash(),
                "Invalid header parent hash: expected {}, got {}",
                parent_hash,
                child_sealed.parent_hash()
            );

            // Insert parent's hash into block_hashes map
            block_hashes.insert(parent_sealed.number(), child_sealed.parent_hash());
        }

        Self {
            ethereum_state,
            bytecodes,
            ancestor_headers,
            block_hashes,
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

    /// Gets a reference to the ancestor headers map (with pre-computed hashes).
    pub fn ancestor_headers(&self) -> &BTreeMap<u64, Sealed<Header>> {
        &self.ancestor_headers
    }

    /// Gets a reference to the pre-computed block hashes map.
    pub fn block_hashes(&self) -> &HashMap<u64, B256> {
        &self.block_hashes
    }

    // NOTE: same comment as `add_executed_block`
    pub fn add_bytecodes(&mut self, new_bytecodes: Vec<Bytecode>) {
        for bytecode in new_bytecodes {
            let hash = bytecode.hash_slow(); // Hash once
            // BTreeMap insert only adds if key doesn't exist
            self.bytecodes.entry(hash).or_insert(bytecode);
        }
    }

    /// Adds a newly executed block's header to the witness state.
    ///
    /// This is called after executing a block in a batch to make its hash
    /// available for BLOCKHASH opcode in subsequent blocks.
    // NOTE: not sure we we should be adding this in proof generation flow. Looks like we can
    // prepare all of this for whole batch while generating witness.
    pub fn add_executed_block(&mut self, header: Header) {
        let sealed = header.seal_slow(); // Hash once
        let block_num = sealed.number();
        let block_hash = sealed.hash();

        // Add to both maps for subsequent block execution
        self.ancestor_headers.insert(block_num, sealed);
        self.block_hashes.insert(block_num, block_hash);
    }

    /// Prepares witness database for block execution.
    ///
    /// Note: Current header validation should be done externally before calling this method.
    pub fn create_witness_db<'a>(&'a self) -> WitnessDB<'a> {
        // Simply create a view with references to pre-computed data
        WitnessDB::new(&self.ethereum_state, &self.block_hashes, &self.bytecodes)
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
        // Encode each bytecode: BOTH the bytes AND its pre-computed hash
        for (hash, bytecode) in &self.bytecodes {
            encode_bytes_with_length(&bytecode.original_bytes(), enc)?;
            enc.write_buf(hash.as_slice())?;
        }

        // Encode ancestor headers count
        (self.ancestor_headers.len() as u32).encode(enc)?;
        // Encode each sealed header: BOTH the header (RLP) AND its pre-computed hash
        for sealed_header in self.ancestor_headers.values() {
            encode_rlp_with_length(sealed_header.inner(), enc)?;
            enc.write_buf(sealed_header.hash().as_slice())?;
        }

        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // Decode EthereumState using bincode
        let ethereum_state_bytes = decode_bytes_with_length(dec)?;
        let ethereum_state = bincode::deserialize(&ethereum_state_bytes)
            .map_err(|_| CodecError::MalformedField("EthereumState deserialize failed"))?;

        // Decode bytecodes with their pre-computed hashes
        let bytecodes_count = u32::decode(dec)? as usize;
        let mut bytecodes = BTreeMap::new();
        for _ in 0..bytecodes_count {
            // Decode the bytecode bytes
            let bytes = decode_bytes_with_length(dec)?;
            let bytecode = Bytecode::new_raw_checked(bytes.into())
                .map_err(|_| CodecError::MalformedField("Bytecode decode failed"))?;

            // Decode the pre-computed hash (32 bytes, no length prefix needed)
            let mut hash_bytes = [0u8; 32];
            dec.read_buf(&mut hash_bytes)?;
            let hash = B256::from(hash_bytes);

            bytecodes.insert(hash, bytecode);
        }

        // Decode ancestor headers with their pre-computed hashes
        let headers_count = u32::decode(dec)? as usize;
        let mut ancestor_headers_sealed = Vec::with_capacity(headers_count);
        for _ in 0..headers_count {
            // Decode the header
            let header: Header = decode_rlp_with_length(dec)?;
            // Decode the pre-computed hash (32 bytes, no length prefix needed)
            let mut hash_bytes = [0u8; 32];
            dec.read_buf(&mut hash_bytes)?;
            let hash = B256::from(hash_bytes);
            // Reconstruct Sealed<Header> without hashing (zero cost!)
            ancestor_headers_sealed.push(Sealed::new_unchecked(header, hash));
        }

        // Build ancestor_headers BTreeMap directly from sealed headers
        let ancestor_headers: BTreeMap<u64, Sealed<Header>> = ancestor_headers_sealed
            .into_iter()
            .map(|sealed| (sealed.number(), sealed))
            .collect();

        // Validate header chain and build block_hashes map
        let mut block_hashes: HashMap<u64, B256> = HashMap::with_hasher(Default::default());
        for (child_sealed, parent_sealed) in ancestor_headers.values().tuple_windows() {
            // Validate block number continuity
            assert_eq!(
                parent_sealed.number() + 1,
                child_sealed.number(),
                "Invalid header block number: expected {}, got {}",
                parent_sealed.number() + 1,
                child_sealed.number()
            );

            // Validate parent hash matches
            let parent_hash = parent_sealed.hash();
            assert_eq!(
                parent_hash,
                child_sealed.parent_hash(),
                "Invalid header parent hash: expected {}, got {}",
                parent_hash,
                child_sealed.parent_hash()
            );

            // Insert parent's hash into block_hashes map
            block_hashes.insert(parent_sealed.number(), child_sealed.parent_hash());
        }

        Ok(Self {
            ethereum_state,
            bytecodes,
            ancestor_headers,
            block_hashes,
        })
    }
}
