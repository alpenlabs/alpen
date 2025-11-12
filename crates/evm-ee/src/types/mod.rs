//! EVM Execution Environment types.
//!
//! This module defines the types needed for EVM block execution within the
//! ExecutionEnvironment trait framework.

use std::collections::BTreeMap;

use alloy_consensus::{BlockHeader, Header};
use itertools::Itertools;
use reth_primitives::TransactionSigned;
use reth_primitives_traits::SealedHeader;
use reth_trie::HashedPostState;
use revm::state::Bytecode;
use revm_primitives::{B256, map::HashMap};
use rsp_mpt::EthereumState;
use strata_codec::{Codec, CodecError};
use strata_ee_acct_types::{EnvResult, ExecBlock, ExecBlockBody, ExecHeader, ExecPartialState};

pub(crate) type Hash = [u8; 32];

/// Helper function to encode an RLP-encodable item with length prefix.
///
/// This encodes the item using RLP, then writes a u32 length prefix followed by the RLP bytes.
fn encode_rlp_with_length<T: alloy_rlp::Encodable>(
    item: &T,
    enc: &mut impl strata_codec::Encoder,
) -> Result<(), CodecError> {
    let rlp_encoded = alloy_rlp::encode(item);
    (rlp_encoded.len() as u32).encode(enc)?;
    enc.write_buf(&rlp_encoded)?;
    Ok(())
}

/// Helper function to decode an RLP-decodable item with length prefix.
///
/// This reads a u32 length prefix, then reads that many bytes and decodes them using RLP.
fn decode_rlp_with_length<T: alloy_rlp::Decodable>(
    dec: &mut impl strata_codec::Decoder,
) -> Result<T, CodecError> {
    let len = u32::decode(dec)? as usize;
    let mut buf = vec![0u8; len];
    dec.read_buf(&mut buf)?;

    alloy_rlp::Decodable::decode(&mut &buf[..])
        .map_err(|_| CodecError::MalformedField("RLP decode failed"))
}

/// Helper function to encode raw bytes with length prefix.
///
/// This writes a u32 length prefix followed by the raw bytes.
fn encode_bytes_with_length(
    bytes: &[u8],
    enc: &mut impl strata_codec::Encoder,
) -> Result<(), CodecError> {
    (bytes.len() as u32).encode(enc)?;
    enc.write_buf(bytes)?;
    Ok(())
}

/// Helper function to decode raw bytes with length prefix.
///
/// This reads a u32 length prefix, then reads that many bytes and returns them as a Vec<u8>.
fn decode_bytes_with_length(dec: &mut impl strata_codec::Decoder) -> Result<Vec<u8>, CodecError> {
    let len = u32::decode(dec)? as usize;
    let mut bytes = vec![0u8; len];
    dec.read_buf(&mut bytes)?;
    Ok(bytes)
}

/// Partial state for EVM execution using RSP's sparse Merkle Patricia Trie.
///
/// This represents a sparse state containing only the accounts and storage slots
/// that were accessed during execution. It's optimized for zero-knowledge proof
/// generation where including the full Ethereum state would be prohibitively expensive.
///
/// Also includes witness data needed for block execution:
/// - Contract bytecodes for executing contract calls
/// - Ancestor block headers for the BLOCKHASH opcode
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
    pub fn prepare_witness_db<'a>(
        &'a self,
        current_header: &Header,
    ) -> rsp_client_executor::io::TrieDB<'a> {
        // Seal the current block header and ancestor headers
        let current_sealed = SealedHeader::seal_slow(current_header.clone());
        let sealed_headers: Vec<SealedHeader> = std::iter::once(current_sealed)
            .chain(
                self.ancestor_headers
                    .values()
                    .map(|h| SealedHeader::seal_slow(h.clone())),
            )
            .collect();

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
        rsp_client_executor::io::TrieDB::new(&self.ethereum_state, block_hashes, bytecode_by_hash)
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

/// Write batch for EVM execution containing state changes.
///
/// This wraps Reth's HashedPostState which contains the differences (deltas)
/// in account states and storage slots after executing a block. It's used to
/// apply state changes to the sparse EthereumState.
///
/// Also stores execution metadata (state root, logs bloom) needed for completing
/// the block header.
#[derive(Clone, Debug)]
pub struct EvmWriteBatch {
    hashed_post_state: HashedPostState,
    /// The computed state root after applying the state changes
    state_root: Hash,
    /// The accumulated logs bloom from all receipts
    logs_bloom: revm_primitives::alloy_primitives::Bloom,
}

impl EvmWriteBatch {
    /// Creates a new EvmWriteBatch from a HashedPostState and computed metadata.
    pub fn new(
        hashed_post_state: HashedPostState,
        state_root: Hash,
        logs_bloom: revm_primitives::alloy_primitives::Bloom,
    ) -> Self {
        Self {
            hashed_post_state,
            state_root,
            logs_bloom,
        }
    }

    /// Gets a reference to the underlying HashedPostState.
    pub fn hashed_post_state(&self) -> &HashedPostState {
        &self.hashed_post_state
    }

    /// Gets the computed state root.
    pub fn state_root(&self) -> Hash {
        self.state_root
    }

    /// Gets the accumulated logs bloom.
    pub fn logs_bloom(&self) -> revm_primitives::alloy_primitives::Bloom {
        self.logs_bloom
    }

    /// Consumes self and returns the underlying HashedPostState.
    pub fn into_hashed_post_state(self) -> HashedPostState {
        self.hashed_post_state
    }
}

impl Codec for EvmWriteBatch {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // Encode HashedPostState using bincode (it has serde support)
        let hashed_post_state_bytes = bincode::serialize(&self.hashed_post_state)
            .map_err(|_| CodecError::MalformedField("HashedPostState serialize failed"))?;
        encode_bytes_with_length(&hashed_post_state_bytes, enc)?;

        // Encode state_root (32 bytes)
        enc.write_buf(&self.state_root)?;

        // Encode logs_bloom (256 bytes)
        enc.write_buf(self.logs_bloom.as_slice())?;

        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // Decode HashedPostState using bincode
        let hashed_post_state_bytes = decode_bytes_with_length(dec)?;
        let hashed_post_state = bincode::deserialize(&hashed_post_state_bytes)
            .map_err(|_| CodecError::MalformedField("HashedPostState deserialize failed"))?;

        // Decode state_root (32 bytes)
        let mut state_root = [0u8; 32];
        dec.read_buf(&mut state_root)?;

        // Decode logs_bloom (256 bytes)
        let mut logs_bloom_bytes = [0u8; 256];
        dec.read_buf(&mut logs_bloom_bytes)?;
        let logs_bloom = revm_primitives::alloy_primitives::Bloom::from(logs_bloom_bytes);

        Ok(Self {
            hashed_post_state,
            state_root,
            logs_bloom,
        })
    }
}

/// Block header for EVM execution.
///
/// Wraps Alloy's consensus Header type and implements the ExecHeader trait
/// to provide block metadata for the execution environment.
#[derive(Clone, Debug)]
pub struct EvmHeader {
    header: Header,
}

impl EvmHeader {
    /// Creates a new EvmHeader from an Alloy Header.
    pub fn new(header: Header) -> Self {
        Self { header }
    }

    /// Gets a reference to the underlying Header.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Returns the block number.
    pub fn block_number(&self) -> u64 {
        self.header.number
    }
}

impl ExecHeader for EvmHeader {
    type Intrinsics = Header;

    fn get_intrinsics(&self) -> Self::Intrinsics {
        self.header.clone()
    }

    fn get_state_root(&self) -> Hash {
        self.header.state_root.into()
    }

    fn compute_block_id(&self) -> Hash {
        self.header.hash_slow().into()
    }
}

impl Codec for EvmHeader {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // Use Alloy's RLP encoding (standard Ethereum format) with length prefix
        encode_rlp_with_length(&self.header, enc)
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // Decode using Alloy's RLP decoder with length prefix
        let header = decode_rlp_with_length(dec)?;
        Ok(Self { header })
    }
}

/// Block body for EVM execution containing transactions.
///
/// Contains the transaction list for a block. Uses reth-compatible TransactionSigned
/// for proper type compatibility with the execution engine.
#[derive(Clone, Debug)]
pub struct EvmBlockBody {
    // Store the full BlockBody using reth's TransactionSigned type
    body: alloy_consensus::BlockBody<TransactionSigned>,
}

impl EvmBlockBody {
    /// Creates a new EvmBlockBody from a vector of transactions.
    pub fn new(transactions: Vec<TransactionSigned>) -> Self {
        Self {
            body: alloy_consensus::BlockBody {
                transactions,
                ommers: vec![],
                withdrawals: None,
            },
        }
    }

    /// Creates a new EvmBlockBody from an alloy BlockBody.
    pub fn from_alloy_body(body: alloy_consensus::BlockBody<TransactionSigned>) -> Self {
        Self { body }
    }

    /// Gets a reference to the transactions.
    pub fn transactions(&self) -> &[TransactionSigned] {
        &self.body.transactions
    }

    /// Gets a reference to the full body.
    pub fn body(&self) -> &alloy_consensus::BlockBody<TransactionSigned> {
        &self.body
    }

    /// Returns the number of transactions in the block.
    pub fn transaction_count(&self) -> usize {
        self.body.transactions.len()
    }
}

impl ExecBlockBody for EvmBlockBody {}

impl Codec for EvmBlockBody {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // Encode transactions count
        let tx_count = self.body.transactions.len() as u32;
        tx_count.encode(enc)?;

        // Encode each transaction using RLP helper
        for tx in &self.body.transactions {
            encode_rlp_with_length(tx, enc)?;
        }

        // Encode withdrawals (optional)
        let has_withdrawals = self.body.withdrawals.is_some();
        has_withdrawals.encode(enc)?;

        if let Some(ref withdrawals) = self.body.withdrawals {
            let withdrawals_count = withdrawals.len() as u32;
            withdrawals_count.encode(enc)?;

            // Encode each withdrawal using RLP helper
            for withdrawal in withdrawals.iter() {
                encode_rlp_with_length(withdrawal, enc)?;
            }
        }

        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // Decode transactions count
        let tx_count = u32::decode(dec)? as usize;

        // Decode each transaction using RLP helper
        let mut transactions = Vec::with_capacity(tx_count);
        for _ in 0..tx_count {
            transactions.push(decode_rlp_with_length(dec)?);
        }

        // Decode withdrawals (optional)
        let has_withdrawals = bool::decode(dec)?;
        let withdrawals = if has_withdrawals {
            let withdrawals_count = u32::decode(dec)? as usize;
            let mut withdrawals_vec = Vec::with_capacity(withdrawals_count);

            // Decode each withdrawal using RLP helper
            for _ in 0..withdrawals_count {
                withdrawals_vec.push(decode_rlp_with_length(dec)?);
            }

            Some(withdrawals_vec)
        } else {
            None
        };

        Ok(Self {
            body: alloy_consensus::BlockBody {
                transactions,
                ommers: vec![], // Ommers are deprecated post-merge, always empty
                withdrawals: withdrawals.map(Into::into),
            },
        })
    }
}

/// Full EVM block containing header and body.
///
/// Represents a complete Ethereum block with header metadata and transaction body.
/// This is the top-level block type used in the ExecutionEnvironment.
#[derive(Clone, Debug)]
pub struct EvmBlock {
    header: EvmHeader,
    body: EvmBlockBody,
}

impl EvmBlock {
    /// Creates a new EvmBlock from a header and body.
    pub fn new(header: EvmHeader, body: EvmBlockBody) -> Self {
        Self { header, body }
    }

    /// Gets a reference to the block header.
    pub fn header(&self) -> &EvmHeader {
        &self.header
    }

    /// Gets a reference to the block body.
    pub fn body(&self) -> &EvmBlockBody {
        &self.body
    }
}

impl ExecBlock for EvmBlock {
    type Header = EvmHeader;
    type Body = EvmBlockBody;

    fn from_parts(header: Self::Header, body: Self::Body) -> Self {
        Self { header, body }
    }

    fn check_header_matches_body(header: &Self::Header, body: &Self::Body) -> bool {
        // Validate that the transactions root in the header matches the body's transactions
        // This uses Alloy's proofs::calculate_transaction_root to compute the root
        use alloy_consensus::proofs::calculate_transaction_root;

        let computed_tx_root = calculate_transaction_root(body.transactions());
        let header_tx_root = header.header().transactions_root;

        computed_tx_root == header_tx_root
    }

    fn get_header(&self) -> &Self::Header {
        &self.header
    }

    fn get_body(&self) -> &Self::Body {
        &self.body
    }
}

impl Codec for EvmBlock {
    fn encode(&self, enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // Encode header and body separately using their Codec implementations
        self.header.encode(enc)?;
        self.body.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // Decode header and body separately using their Codec implementations
        let header = EvmHeader::decode(dec)?;
        let body = EvmBlockBody::decode(dec)?;
        Ok(Self { header, body })
    }
}

#[cfg(test)]
mod tests;
