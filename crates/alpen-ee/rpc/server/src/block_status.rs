//! Alpen EE RPC handler implementation for block-status methods.

use alloy_primitives::B256;
use alpen_ee_common::ConsensusHeads;
use alpen_ee_rpc_api::{AlpenEeRpcServer, BlockStatus, BlockStatusResponse};
use async_trait::async_trait;
use jsonrpsee::core::RpcResult;
use reth_node_builder::NodeTypesWithDB;
use reth_provider::{
    providers::{BlockchainProvider, ProviderNodeTypes},
    BlockHashReader, BlockNumReader, ProviderResult,
};
use tokio::sync::watch;

use crate::errors::{block_not_found_error, internal_error};

/// Resolve `block_hash` to its canonical block number on `provider`.
///
/// Returns `Ok(Some(n))` only when `block_hash` is the canonical hash at
/// height `n`. A hash that is known to the provider but belongs to an
/// orphaned / non-canonical branch returns `Ok(None)`.
fn canonical_block_number<N: NodeTypesWithDB + ProviderNodeTypes>(
    provider: &BlockchainProvider<N>,
    block_hash: B256,
) -> ProviderResult<Option<u64>> {
    let Some(block_number) = provider.block_number(block_hash)? else {
        return Ok(None);
    };
    let Some(canonical_hash) = provider.block_hash(block_number)? else {
        return Ok(None);
    };
    if canonical_hash == block_hash {
        Ok(Some(block_number))
    } else {
        Ok(None)
    }
}

/// RPC handler for [`AlpenEeRpcServer`].
///
/// Resolves block status by combining Reth's canonical-chain lookup with the
/// `OLTracker`-derived [`ConsensusHeads`]. Works on both sequencer and fullnode
/// because neither dependency is sequencer-specific.
#[derive(Debug)]
pub struct EeRpcServer<N: NodeTypesWithDB + ProviderNodeTypes> {
    provider: BlockchainProvider<N>,
    consensus_rx: watch::Receiver<ConsensusHeads>,
}

impl<N: NodeTypesWithDB + ProviderNodeTypes> EeRpcServer<N> {
    pub fn new(
        provider: BlockchainProvider<N>,
        consensus_rx: watch::Receiver<ConsensusHeads>,
    ) -> Self {
        Self {
            provider,
            consensus_rx,
        }
    }
}

#[async_trait]
impl<N> AlpenEeRpcServer for EeRpcServer<N>
where
    N: NodeTypesWithDB + ProviderNodeTypes + Send + Sync + 'static,
{
    async fn get_block_status(&self, block_hash: B256) -> RpcResult<BlockStatusResponse> {
        // Resolve target to a canonical block number. `block_number` alone
        // does not distinguish canonical blocks from orphaned ones stored in
        // the DB, so round-trip through `block_hash(number)` to verify.
        let target_num = match canonical_block_number(&self.provider, block_hash) {
            Ok(Some(n)) => n,
            Ok(None) => return Err(block_not_found_error()),
            Err(e) => return Err(internal_error(e.to_string())),
        };

        // Preserve genesis semantics: block 0 is always considered finalized.
        if target_num == 0 {
            return Ok(BlockStatusResponse {
                status: BlockStatus::Finalized,
            });
        }

        let heads = self.consensus_rx.borrow().clone();

        // Finalized check: skip when the head is unset or not canonical on
        // this node (transient during sync / reorg — OLTracker may still be
        // tracking a fork that Reth hasn't reorged to).
        let finalized_b256 = B256::from_slice(heads.finalized().as_slice());
        if !finalized_b256.is_zero() {
            match canonical_block_number(&self.provider, finalized_b256) {
                Ok(Some(fin_num)) if target_num <= fin_num => {
                    return Ok(BlockStatusResponse {
                        status: BlockStatus::Finalized,
                    });
                }
                Ok(_) => {}
                Err(e) => return Err(internal_error(e.to_string())),
            }
        }

        // Confirmed check.
        let confirmed_b256 = B256::from_slice(heads.confirmed().as_slice());
        if !confirmed_b256.is_zero() {
            match canonical_block_number(&self.provider, confirmed_b256) {
                Ok(Some(conf_num)) if target_num <= conf_num => {
                    return Ok(BlockStatusResponse {
                        status: BlockStatus::Confirmed,
                    });
                }
                Ok(_) => {}
                Err(e) => return Err(internal_error(e.to_string())),
            }
        }

        Ok(BlockStatusResponse {
            status: BlockStatus::Pending,
        })
    }
}
