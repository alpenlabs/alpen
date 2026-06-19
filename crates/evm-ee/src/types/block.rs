//! EVM block implementation.

use reth_consensus_common::validation::validate_body_against_header;
use strata_codec::impl_type_flat_struct;
use strata_ee_acct_types::ExecBlock;

use super::{EvmBlockBody, EvmHeader};

impl_type_flat_struct! {
    /// Full EVM block containing header and body.
    ///
    /// Represents a complete Ethereum block with header metadata and transaction body.
    /// This is the top-level block type used in the ExecutionEnvironment.
    #[derive(Clone, Debug)]
    pub struct EvmBlock {
        header: EvmHeader,
        body: EvmBlockBody,
    }
}

impl ExecBlock for EvmBlock {
    type Header = EvmHeader;
    type Body = EvmBlockBody;

    fn from_parts(header: Self::Header, body: Self::Body) -> Self {
        Self { header, body }
    }

    fn check_header_matches_body(header: &Self::Header, body: &Self::Body) -> bool {
        validate_body_against_header(body.body(), header.header()).is_ok()
    }

    fn get_header(&self) -> &Self::Header {
        &self.header
    }

    fn get_body(&self) -> &Self::Body {
        &self.body
    }
}
