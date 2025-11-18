//! EVM block header implementation.

use alloy_consensus::Header;
use strata_codec::{Codec, CodecError};
use strata_ee_acct_types::ExecHeader;

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
