use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use bitcoin::{BlockHash, Network};
use bitcoind_async_client::{Client, corepc_types::model::GetBlockchainInfo, traits::Reader};
use strata_btc_types::BlockHashExt;
use strata_node_context::NodeContext;
use strata_primitives::L1BlockCommitment;
use tracing::info;

#[async_trait]
pub(crate) trait StartupBitcoinClient {
    async fn get_blockchain_info_for_startup(&self) -> Result<GetBlockchainInfo>;
    async fn get_block_hash_for_startup(&self, height: u64) -> Result<BlockHash>;
}

#[async_trait]
impl StartupBitcoinClient for Client {
    async fn get_blockchain_info_for_startup(&self) -> Result<GetBlockchainInfo> {
        Reader::get_blockchain_info(self).await.map_err(Into::into)
    }

    async fn get_block_hash_for_startup(&self, height: u64) -> Result<BlockHash> {
        Reader::get_block_hash(self, height)
            .await
            .map_err(Into::into)
    }
}

pub(crate) async fn run_bitcoin_connectivity_and_network_checks(
    bitcoin_client: &impl StartupBitcoinClient,
    expected_network: Network,
) -> Result<()> {
    let chain_info = bitcoin_client
        .get_blockchain_info_for_startup()
        .await
        .context("startup: could not connect to bitcoind via getblockchaininfo")?;

    if chain_info.chain != expected_network {
        bail!(
            "startup: bitcoind network mismatch: expected {}, got {}",
            expected_network,
            chain_info.chain
        );
    }

    Ok(())
}

pub(crate) async fn verify_l1_anchor_block_on_restart(
    bitcoin_client: &impl StartupBitcoinClient,
    expected_l1_block: L1BlockCommitment,
) -> Result<()> {
    let actual_hash = bitcoin_client
        .get_block_hash_for_startup(expected_l1_block.height() as u64)
        .await
        .with_context(|| {
            format!(
                "startup: failed to fetch L1 block hash from bitcoind at height {}",
                expected_l1_block.height()
            )
        })?;

    let actual_block_id = actual_hash.to_l1_block_id();
    if actual_block_id != *expected_l1_block.blkid() {
        bail!(
            "startup: genesis L1 block hash mismatch at height {height}: expected {expected}, got {actual_block_id}",
            height = expected_l1_block.height(),
            expected = expected_l1_block.blkid(),
        );
    }

    Ok(())
}

pub(crate) fn run_startup_checks(ctx: &NodeContext) -> Result<()> {
    ctx.executor()
        .handle()
        .block_on(run_bitcoin_connectivity_and_network_checks(
            ctx.bitcoin_client().as_ref(),
            ctx.config().bitcoind.network,
        ))?;

    let has_persisted_client_state = ctx
        .storage()
        .client_state()
        .fetch_most_recent_state()
        .context("startup: failed to fetch most recent client state")?
        .is_some();
    if has_persisted_client_state {
        ctx.executor()
            .handle()
            .block_on(verify_l1_anchor_block_on_restart(
                ctx.bitcoin_client().as_ref(),
                ctx.ol_params().last_l1_block,
            ))?;
    }

    info!("startup: startup checks passed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bitcoin::{BlockHash, Network, Work, hashes::Hash};
    use bitcoind_async_client::corepc_types::model::GetBlockchainInfo;

    use super::*;

    fn make_blockchain_info(network: Network) -> GetBlockchainInfo {
        GetBlockchainInfo {
            chain: network,
            blocks: 100,
            headers: 100,
            best_block_hash: BlockHash::all_zeros(),
            difficulty: 1.0,
            median_time: 600,
            verification_progress: 1.0,
            initial_block_download: false,
            chain_work: Work::from_be_bytes([0; 32]),
            size_on_disk: 1_000_000,
            pruned: false,
            prune_height: None,
            automatic_pruning: None,
            prune_target_size: None,
            bits: None,
            target: None,
            time: None,
            signet_challenge: None,
            warnings: vec![],
            softforks: BTreeMap::new(),
        }
    }

    struct MockBitcoinClient {
        blockchain_info_result: Option<Result<GetBlockchainInfo>>,
        block_hash_result: Option<Result<BlockHash>>,
    }

    #[async_trait]
    impl StartupBitcoinClient for MockBitcoinClient {
        async fn get_blockchain_info_for_startup(&self) -> Result<GetBlockchainInfo> {
            match self.blockchain_info_result.as_ref().unwrap() {
                Ok(info) => Ok(info.clone()),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }

        async fn get_block_hash_for_startup(&self, _height: u64) -> Result<BlockHash> {
            match self.block_hash_result.as_ref().unwrap() {
                Ok(hash) => Ok(*hash),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }
    }

    fn mock_client_ok(network: Network) -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: Some(Ok(make_blockchain_info(network))),
            block_hash_result: None,
        }
    }

    fn mock_client_unreachable() -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: Some(Err(anyhow::anyhow!("connection refused"))),
            block_hash_result: None,
        }
    }

    fn mock_client_with_block_hash(hash: BlockHash) -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: None,
            block_hash_result: Some(Ok(hash)),
        }
    }

    fn mock_client_block_hash_unreachable() -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: None,
            block_hash_result: Some(Err(anyhow::anyhow!("block not found"))),
        }
    }

    #[tokio::test]
    async fn test_bitcoind_unreachable() {
        let client = mock_client_unreachable();

        let result = run_bitcoin_connectivity_and_network_checks(&client, Network::Regtest).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("could not connect to bitcoind"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn test_bitcoind_network_mismatch() {
        let client = mock_client_ok(Network::Bitcoin);

        let result = run_bitcoin_connectivity_and_network_checks(&client, Network::Regtest).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("network mismatch"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_bitcoind_network_matches() {
        let client = mock_client_ok(Network::Regtest);

        let result = run_bitcoin_connectivity_and_network_checks(&client, Network::Regtest).await;

        assert!(result.is_ok());
    }

    fn make_l1_block_commitment(height: u32, hash: BlockHash) -> L1BlockCommitment {
        let block_id = hash.to_l1_block_id();
        L1BlockCommitment::new(height, block_id)
    }

    #[tokio::test]
    async fn test_l1_anchor_block_hash_matches() {
        let hash = BlockHash::all_zeros();
        let commitment = make_l1_block_commitment(42, hash);
        let client = mock_client_with_block_hash(hash);

        let result = verify_l1_anchor_block_on_restart(&client, commitment).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_l1_anchor_block_hash_mismatch() {
        let expected_hash = BlockHash::all_zeros();
        let actual_hash = BlockHash::from_byte_array([1; 32]);
        let commitment = make_l1_block_commitment(42, expected_hash);
        let client = mock_client_with_block_hash(actual_hash);

        let result = verify_l1_anchor_block_on_restart(&client, commitment).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("genesis L1 block hash mismatch"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn test_l1_anchor_block_unreachable() {
        let hash = BlockHash::all_zeros();
        let commitment = make_l1_block_commitment(42, hash);
        let client = mock_client_block_hash_unreachable();

        let result = verify_l1_anchor_block_on_restart(&client, commitment).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("failed to fetch L1 block hash"),
            "unexpected error: {err}"
        );
    }
}
