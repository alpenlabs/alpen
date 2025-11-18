//! EVM block implementation.

use alloy_consensus::proofs::calculate_transaction_root;
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
        // Validate that the transactions root in the header matches the body's transactions
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
