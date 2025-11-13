//! EVM block implementation.

use strata_codec::{Codec, CodecError};
use strata_ee_acct_types::ExecBlock;

use super::{EvmBlockBody, EvmHeader};

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
