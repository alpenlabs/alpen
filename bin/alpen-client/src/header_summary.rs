//! [`RethHeaderSummaryProvider`] — Reth-backed [`HeaderSummaryProvider`] for DA blobs.
//!
//! The DA blob pipeline needs an [`EvmHeaderSummary`] for each batch so that
//! verifiers can reconstruct EVM chain metadata (block number, timestamp,
//! base fee, gas used/limit). [`RethHeaderSummaryProvider`]
//! satisfies the [`HeaderSummaryProvider`] trait by reading headers directly
//! from the Reth [`HeaderProvider`](reth_provider::HeaderProvider).
//!
//! This adapter lives in the binary crate because it depends on
//! `reth_provider::HeaderProvider`, which is only available where the Reth node
//! is assembled. The generic DA providers that consume it live in
//! [`alpen_ee_da`].

use alpen_ee_common::{EvmHeaderSummary, HeaderSummaryProvider};

/// [`HeaderSummaryProvider`] backed by a Reth [`HeaderProvider`](reth_provider::HeaderProvider).
pub(crate) struct RethHeaderSummaryProvider<P> {
    provider: P,
}

impl<P> RethHeaderSummaryProvider<P> {
    pub(crate) fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl<P> HeaderSummaryProvider for RethHeaderSummaryProvider<P>
where
    P: reth_provider::HeaderProvider<Header = reth_primitives::Header> + Send + Sync,
{
    fn header_summary(&self, block_num: u64) -> eyre::Result<EvmHeaderSummary> {
        let header = self
            .provider
            .header_by_number(block_num)?
            .ok_or_else(|| eyre::eyre!("no header for block {block_num}"))?;
        Ok(EvmHeaderSummary {
            block_num: header.number,
            timestamp: header.timestamp,
            base_fee: header.base_fee_per_gas.ok_or_else(|| {
                eyre::eyre!(
                    "block {block_num} missing base_fee_per_gas; \
                     Alpen is post-London from genesis so this should always be present"
                )
            })?,
            gas_used: header.gas_used,
            gas_limit: header.gas_limit,
        })
    }
}
