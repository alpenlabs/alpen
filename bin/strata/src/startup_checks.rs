use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use bitcoin::Network;
use bitcoind_async_client::{Client, corepc_types::model::GetBlockchainInfo, traits::Reader};
use strata_node_context::NodeContext;
use tracing::info;

#[async_trait]
pub(crate) trait StartupBitcoinClient {
    async fn get_blockchain_info_for_startup(&self) -> Result<GetBlockchainInfo>;
}

#[async_trait]
impl StartupBitcoinClient for Client {
    async fn get_blockchain_info_for_startup(&self) -> Result<GetBlockchainInfo> {
        Reader::get_blockchain_info(self).await.map_err(Into::into)
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

pub(crate) fn run_startup_checks(ctx: &NodeContext) -> Result<()> {
    ctx.executor()
        .handle()
        .block_on(run_bitcoin_connectivity_and_network_checks(
            ctx.bitcoin_client().as_ref(),
            ctx.config().bitcoind.network,
        ))?;

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
        blockchain_info_result: Result<GetBlockchainInfo>,
    }

    #[async_trait]
    impl StartupBitcoinClient for MockBitcoinClient {
        async fn get_blockchain_info_for_startup(&self) -> Result<GetBlockchainInfo> {
            match &self.blockchain_info_result {
                Ok(info) => Ok(info.clone()),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }
    }

    fn mock_client_ok(network: Network) -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: Ok(make_blockchain_info(network)),
        }
    }

    fn mock_client_unreachable() -> MockBitcoinClient {
        MockBitcoinClient {
            blockchain_info_result: Err(anyhow::anyhow!("connection refused")),
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
}
