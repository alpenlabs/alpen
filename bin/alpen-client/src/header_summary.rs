//! [`RethHeaderSummaryProvider`] — Reth-backed [`HeaderSummaryProvider`] for DA blobs.
//!
//! The DA blob pipeline needs an [`EvmHeaderSummary`] for each batch so that
//! verifiers can reconstruct EVM chain metadata (block number, timestamp,
//! base fee, gas used/limit, block hash, state root). [`RethHeaderSummaryProvider`]
//! satisfies the [`HeaderSummaryProvider`] trait by reading headers directly
//! from the Reth [`HeaderProvider`](reth_provider::HeaderProvider).
//!
//! This adapter lives in the binary crate because it depends on
//! `reth_provider::HeaderProvider`, which is only available where the Reth node
//! is assembled. The generic DA providers that consume it live in
//! [`alpen_ee_da`].

use alpen_ee_common::{EvmHeaderSummary, HeaderSummaryProvider};
use strata_acct_types::Hash;

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
        block_hash: Hash::from(header.hash_slow().0),
        state_root: Hash::from(header.state_root.0),
    })
}

#[cfg(test)]
mod tests {
    use alloy_primitives::B256;
    use reth_primitives::Header;

    use super::*;

    /// Header → EvmHeaderSummary mapping: verifies each field comes from the
    /// right source on the reth header, catching swaps (e.g. `block_hash`
    /// vs `state_root`) that would compile but corrupt DA blobs.
    #[test]
    fn summarize_header_maps_fields_correctly() {
        let state_root = B256::repeat_byte(0xAA);
        let expected_state_root_bytes = state_root.0;

        let header = Header {
            number: 12345,
            timestamp: 1_700_000_000,
            base_fee_per_gas: Some(1_000_000_000),
            gas_used: 15_000_000,
            gas_limit: 30_000_000,
            state_root,
            ..Default::default()
        };
        let expected_block_hash_bytes = header.hash_slow().0;

        let summary = summarize_header(&header).expect("mapping must succeed");

        assert_eq!(summary.block_num, 12345);
        assert_eq!(summary.timestamp, 1_700_000_000);
        assert_eq!(summary.base_fee, 1_000_000_000);
        assert_eq!(summary.gas_used, 15_000_000);
        assert_eq!(summary.gas_limit, 30_000_000);
        assert_eq!(
            summary.block_hash,
            Hash::from(expected_block_hash_bytes),
            "block_hash must come from header.hash_slow(), not state_root"
        );
        assert_eq!(
            summary.state_root,
            Hash::from(expected_state_root_bytes),
            "state_root must come from header.state_root, not hash_slow()"
        );
        assert_ne!(
            summary.block_hash, summary.state_root,
            "distinct sources must produce distinct values — guards against field-swap regressions"
        );
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
