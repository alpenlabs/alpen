//! [`RethGasDataProvider`] — Reth-backed [`BlockDataProvider`] for gas-limit
//! batch sealing.

use alloy_consensus::BlockHeader;
use alpen_ee_sequencer::sealing_policy::{
    gas_limit_policy::{GasBlockData, GasLimitPolicy},
    BlockDataProvider,
};
use async_trait::async_trait;
use reth_provider::HeaderProvider;
use strata_acct_types::Hash;

/// Data provider that reads `gas_used` from reth block headers.
#[derive(Debug)]
pub(crate) struct RethGasDataProvider<P> {
    provider: P,
}

impl<P> RethGasDataProvider<P> {
    pub(crate) fn new(provider: P) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl<P> BlockDataProvider<GasLimitPolicy> for RethGasDataProvider<P>
where
    P: HeaderProvider + Send + Sync,
{
    async fn get_block_data(&self, hash: Hash) -> eyre::Result<Option<GasBlockData>> {
        let block_hash = hash.0.into();
        let Some(header) = self.provider.header(block_hash)? else {
            return Ok(None);
        };
        Ok(Some(GasBlockData {
            gas_used: header.gas_used(),
        }))
    }
}
