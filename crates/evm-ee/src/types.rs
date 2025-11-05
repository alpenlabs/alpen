//! EVM Execution Environment types.
//!
//! This module defines the types needed for EVM block execution within the
//! ExecutionEnvironment trait framework.

use alloy_consensus::Header;
use reth_primitives::TransactionSigned;
use reth_trie::HashedPostState;
use revm::state::Bytecode;
use rsp_mpt::EthereumState;
use strata_codec::{Codec, CodecError};
use strata_ee_acct_types::{EnvResult, ExecBlock, ExecBlockBody, ExecHeader, ExecPartialState};

pub(crate) type Hash = [u8; 32];

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
    /// Contract bytecodes needed for execution
    /// FIXME: ensure bytecodes and ancestor_header are right to be placed in this struct
    bytecodes: Vec<Bytecode>,
    /// Ancestor block headers for BLOCKHASH opcode support
    ancestor_headers: Vec<Header>,
}

impl EvmPartialState {
    /// Creates a new EvmPartialState from an EthereumState with witness data.
    pub fn new(
        ethereum_state: EthereumState,
        bytecodes: Vec<Bytecode>,
        ancestor_headers: Vec<Header>,
    ) -> Self {
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

    /// Gets a reference to the bytecodes.
    pub fn bytecodes(&self) -> &[Bytecode] {
        &self.bytecodes
    }

    /// Gets a reference to the ancestor headers.
    pub fn ancestor_headers(&self) -> &[Header] {
        &self.ancestor_headers
    }
}

impl ExecPartialState for EvmPartialState {
    fn compute_state_root(&self) -> EnvResult<Hash> {
        let state_root = self.ethereum_state.state_root();
        Ok(state_root.into())
    }
}

impl Codec for EvmPartialState {
    fn encode(&self, _enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // TODO: Implement proper encoding for EthereumState
        // For now, we'll return an error as this needs RSP's serialization
        Err(CodecError::InvalidVariant(
            "EthereumState encoding not implemented",
        ))
    }

    fn decode(_dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // TODO: Implement proper decoding for EthereumState
        // For now, we'll return an error as this needs RSP's deserialization
        Err(CodecError::InvalidVariant(
            "EthereumState decoding not implemented",
        ))
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
    fn encode(&self, _enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // TODO: Implement proper encoding for HashedPostState
        Err(CodecError::InvalidVariant(
            "HashedPostState encoding not implemented",
        ))
    }

    fn decode(_dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // TODO: Implement proper decoding for HashedPostState
        Err(CodecError::InvalidVariant(
            "HashedPostState decoding not implemented",
        ))
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
    fn encode(&self, _enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // TODO: Implement proper encoding for Header using Alloy's encoding
        Err(CodecError::InvalidVariant(
            "Header encoding not implemented",
        ))
    }

    fn decode(_dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // TODO: Implement proper decoding for Header using Alloy's encoding
        Err(CodecError::InvalidVariant(
            "Header decoding not implemented",
        ))
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
    fn encode(&self, _enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // TODO: Implement proper encoding for transactions
        Err(CodecError::InvalidVariant(
            "BlockBody encoding not implemented",
        ))
    }

    fn decode(_dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // TODO: Implement proper decoding for transactions
        Err(CodecError::InvalidVariant(
            "BlockBody decoding not implemented",
        ))
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
    fn encode(&self, _enc: &mut impl strata_codec::Encoder) -> Result<(), CodecError> {
        // TODO: Implement proper encoding for block
        Err(CodecError::InvalidVariant("Block encoding not implemented"))
    }

    fn decode(_dec: &mut impl strata_codec::Decoder) -> Result<Self, CodecError> {
        // TODO: Implement proper decoding for block
        Err(CodecError::InvalidVariant("Block decoding not implemented"))
    }
}
