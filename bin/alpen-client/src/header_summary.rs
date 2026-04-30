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
        summarize_header(&header)
    }
}

/// Extracts the [`EvmHeaderSummary`] fields from a reth header.
///
/// Split out from the trait impl so the reth → DA mapping can be unit-tested
/// without constructing a full [`reth_provider::HeaderProvider`].
fn summarize_header(header: &reth_primitives::Header) -> eyre::Result<EvmHeaderSummary> {
    Ok(EvmHeaderSummary {
        block_num: header.number,
        timestamp: header.timestamp,
        base_fee: header.base_fee_per_gas.ok_or_else(|| {
            eyre::eyre!(
                "block {} missing base_fee_per_gas; \
                 Alpen is post-London from genesis so this should always be present",
                header.number
            )
        })?,
        gas_used: header.gas_used,
        gas_limit: header.gas_limit,
    })
}

#[cfg(test)]
mod tests {
    use reth_primitives::Header;

    use super::*;

    /// Header → EvmHeaderSummary mapping: verifies each field comes from the
    /// right source on the reth header.
    #[test]
    fn summarize_header_maps_fields_correctly() {
        let header = Header {
            number: 12345,
            timestamp: 1_700_000_000,
            base_fee_per_gas: Some(1_000_000_000),
            gas_used: 15_000_000,
            gas_limit: 30_000_000,
            ..Default::default()
        };
        let summary = summarize_header(&header).expect("mapping must succeed");

        assert_eq!(summary.block_num, 12345);
        assert_eq!(summary.timestamp, 1_700_000_000);
        assert_eq!(summary.base_fee, 1_000_000_000);
        assert_eq!(summary.gas_used, 15_000_000);
        assert_eq!(summary.gas_limit, 30_000_000);
    }

    #[test]
    fn summarize_header_errors_when_base_fee_missing() {
        let header = Header {
            number: 7,
            base_fee_per_gas: None,
            ..Default::default()
        };
        let err = summarize_header(&header).expect_err("should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("base_fee_per_gas") && msg.contains("block 7"),
            "error must identify missing field and block number, got: {msg}"
        );
    }
}
