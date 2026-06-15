//! EVM block header implementation.

use alloy_consensus::{BlockBody, Header, proofs::calculate_transaction_root};
use alpen_ee_da_types::EvmHeaderSummary;
use reth_primitives::TransactionSigned;
use revm_primitives::alloy_primitives::{Address, B64, B256, Bloom, Bytes, U256};
use strata_codec::{Codec, CodecError, encode_to_vec};
use strata_ee_acct_types::ExecHeader;
use strata_ee_chain_types::ExecHeaderSummary;

use super::Hash;
use crate::codec_shims::{decode_rlp_with_length, encode_rlp_with_length};

/// Block header for EVM execution.
///
/// Wraps Alloy's consensus Header type and implements the ExecHeader trait
/// to provide block metadata for the execution environment.
#[derive(Clone, Debug)]
pub struct EvmHeader {
    header: Header,
}

/// Header fields needed as inputs to EVM block execution.
///
/// This excludes commitments derived from the body or execution result, such as
/// the state root, receipts root, logs bloom, and gas used.
#[derive(Clone, Debug)]
pub struct EvmHeaderIntrinsics {
    parent_hash: B256,
    beneficiary: Address,
    difficulty: U256,
    number: u64,
    gas_limit: u64,
    timestamp: u64,
    extra_data: Bytes,
    mix_hash: B256,
    nonce: B64,
    base_fee_per_gas: Option<u64>,
    has_blob_gas_used: bool,
    excess_blob_gas: Option<u64>,
    parent_beacon_block_root: Option<B256>,
    has_requests_hash: bool,
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

impl EvmHeaderIntrinsics {
    /// Creates execution intrinsics from a full EVM header.
    pub fn from_header(header: &Header) -> Self {
        Self {
            parent_hash: header.parent_hash,
            beneficiary: header.beneficiary,
            difficulty: header.difficulty,
            number: header.number,
            gas_limit: header.gas_limit,
            timestamp: header.timestamp,
            extra_data: header.extra_data.clone(),
            mix_hash: header.mix_hash,
            nonce: header.nonce,
            base_fee_per_gas: header.base_fee_per_gas,
            has_blob_gas_used: header.blob_gas_used.is_some(),
            excess_blob_gas: header.excess_blob_gas,
            parent_beacon_block_root: header.parent_beacon_block_root,
            has_requests_hash: header.requests_hash.is_some(),
        }
    }

    /// Builds a header-shaped execution environment from intrinsics and body commitments.
    ///
    /// Execution-result fields are left empty because they are produced after
    /// executing the block and verified separately against the full header.
    pub(crate) fn build_execution_header(&self, body: &BlockBody<TransactionSigned>) -> Header {
        Header {
            parent_hash: self.parent_hash,
            ommers_hash: body.calculate_ommers_root(),
            beneficiary: self.beneficiary,
            state_root: B256::ZERO,
            transactions_root: calculate_transaction_root(&body.transactions),
            receipts_root: B256::ZERO,
            logs_bloom: Bloom::ZERO,
            difficulty: self.difficulty,
            number: self.number,
            gas_limit: self.gas_limit,
            gas_used: 0,
            timestamp: self.timestamp,
            extra_data: self.extra_data.clone(),
            mix_hash: self.mix_hash,
            nonce: self.nonce,
            base_fee_per_gas: self.base_fee_per_gas,
            withdrawals_root: body.calculate_withdrawals_root(),
            blob_gas_used: self.has_blob_gas_used.then_some(0),
            excess_blob_gas: self.excess_blob_gas,
            parent_beacon_block_root: self.parent_beacon_block_root,
            requests_hash: self.has_requests_hash.then_some(B256::ZERO),
        }
    }

    pub fn parent_hash(&self) -> B256 {
        self.parent_hash
    }

    pub fn beneficiary(&self) -> Address {
        self.beneficiary
    }

    pub fn difficulty(&self) -> U256 {
        self.difficulty
    }

    pub fn number(&self) -> u64 {
        self.number
    }

    pub fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn extra_data(&self) -> &Bytes {
        &self.extra_data
    }

    pub fn mix_hash(&self) -> B256 {
        self.mix_hash
    }

    pub fn nonce(&self) -> B64 {
        self.nonce
    }

    pub fn base_fee_per_gas(&self) -> Option<u64> {
        self.base_fee_per_gas
    }

    pub fn has_blob_gas_used(&self) -> bool {
        self.has_blob_gas_used
    }

    pub fn excess_blob_gas(&self) -> Option<u64> {
        self.excess_blob_gas
    }

    pub fn parent_beacon_block_root(&self) -> Option<B256> {
        self.parent_beacon_block_root
    }

    pub fn has_requests_hash(&self) -> bool {
        self.has_requests_hash
    }
}

impl ExecHeader for EvmHeader {
    type Intrinsics = EvmHeaderIntrinsics;

    fn get_intrinsics(&self) -> Self::Intrinsics {
        EvmHeaderIntrinsics::from_header(&self.header)
    }

    fn get_parent_id(&self) -> Hash {
        self.header.parent_hash.0.into()
    }

    fn get_state_root(&self) -> Hash {
        self.header.state_root.0.into()
    }

    fn get_exec_header_summary(&self) -> ExecHeaderSummary {
        let payload = EvmHeaderSummary {
            block_num: self.header.number,
            timestamp: self.header.timestamp,
            base_fee: self
                .header
                .base_fee_per_gas
                .expect("Alpen EVM headers must include base_fee_per_gas from genesis"),
            gas_used: self.header.gas_used,
            gas_limit: self.header.gas_limit,
        };
        ExecHeaderSummary::from_vec(encode_to_vec(&payload).expect("encode EVM header summary"))
            .expect("exec header summary fits the SSZ bound")
    }

    fn compute_block_id(&self) -> Hash {
        self.header.hash_slow().0.into()
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
