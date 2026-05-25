//! EVM block header implementation.

use alloy_consensus::Header;
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

/// EVM header fields committed through the generic [`ExecHeaderSummary`].
///
/// NOTE: This mirrors `alpen_ee_common::EvmHeaderSummary`. The duplication
/// avoids making the core EVM EE proof types depend on `alpen-ee/common` for
/// one small codec payload. If more shared types appear in future,
/// move them into a small common EE types crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Codec)]
struct EvmHeaderSummaryPayload {
    block_num: u64,
    timestamp: u64,
    base_fee: u64,
    gas_used: u64,
    gas_limit: u64,
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

    fn get_parent_id(&self) -> Hash {
        self.header.parent_hash.0.into()
    }

    fn get_state_root(&self) -> Hash {
        self.header.state_root.0.into()
    }

    fn get_exec_header_summary(&self) -> ExecHeaderSummary {
        let payload = EvmHeaderSummaryPayload {
            block_num: self.header.number,
            timestamp: self.header.timestamp,
            base_fee: self
                .header
                .base_fee_per_gas
                .expect("Alpen EVM headers must include base_fee_per_gas from genesis"),
            gas_used: self.header.gas_used,
            gas_limit: self.header.gas_limit,
        };
        ExecHeaderSummary::new(encode_to_vec(&payload).expect("encode EVM header summary"))
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
