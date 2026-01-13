use std::cmp::min;

use futures::stream::{self, Stream, StreamExt};
use ssz::Decode;
use ssz_types::VariableList;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_new::OLBlock;
use strata_ol_state_types as _;
use strata_primitives as _;
use strata_rpc_api_new::{OLClientRpcClient, OLFullNodeRpcClient};
use tracing::error;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("missing block: {0}")]
    MissingBlock(OLBlockId),
    #[error("failed to deserialize block: {0}")]
    Deserialization(String),
    #[error("network: {0}")]
    Network(String),
}

#[derive(Debug)]
pub struct PeerSyncStatus {
    tip_block: OLBlockCommitment,
}

impl PeerSyncStatus {
    pub fn tip_block(&self) -> &OLBlockCommitment {
        &self.tip_block
    }

    pub fn tip_block_id(&self) -> &OLBlockId {
        self.tip_block.blkid()
    }

    pub fn tip_height(&self) -> u64 {
        self.tip_block.slot()
    }
}

#[async_trait::async_trait]
pub trait SyncClient {
    async fn get_sync_status(&self) -> Result<PeerSyncStatus, ClientError>;

    fn get_blocks_range(&self, start_height: u64, end_height: u64) -> impl Stream<Item = OLBlock>;

    async fn get_block_by_id(&self, block_id: &OLBlockId) -> Result<Option<OLBlock>, ClientError>;
}

#[derive(Debug)]
pub struct OLRpcSyncPeer<RPC: OLFullNodeRpcClient + Send + Sync> {
    rpc_client: RPC,
    download_batch_size: usize,
}

impl<RPC: OLFullNodeRpcClient + Send + Sync> OLRpcSyncPeer<RPC> {
    pub fn new(rpc_client: RPC, download_batch_size: usize) -> Self {
        Self {
            rpc_client,
            download_batch_size,
        }
    }

    async fn get_blocks(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> Result<Vec<OLBlock>, ClientError> {
        let bytes = self
            .rpc_client
            .get_raw_blocks_range(start_height, end_height)
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        let blks = VariableList::<OLBlock, 1000>::from_ssz_bytes(&bytes.0)
            .map_err(|e| ClientError::Deserialization(e.to_string()))?;
        Ok(blks.into())
    }
}

#[async_trait::async_trait]
impl<RPC: OLClientRpcClient + OLFullNodeRpcClient + Send + Sync> SyncClient for OLRpcSyncPeer<RPC> {
    async fn get_sync_status(&self) -> Result<PeerSyncStatus, ClientError> {
        let status = self
            .rpc_client
            .chain_status()
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        Ok(PeerSyncStatus {
            tip_block: status.latest,
        })
    }

    fn get_blocks_range(&self, start_height: u64, end_height: u64) -> impl Stream<Item = OLBlock> {
        let block_ranges = (start_height..=end_height)
            .step_by(self.download_batch_size)
            .map(move |s| (s, min(self.download_batch_size as u64 + s - 1, end_height)));

        stream::unfold(block_ranges, |mut block_ranges| async {
            let (start_height, end_height) = block_ranges.next()?;
            match self.get_blocks(start_height, end_height).await {
                Ok(blocks) => Some((stream::iter(blocks), block_ranges)),
                Err(err) => {
                    error!("failed to get blocks: {err}");
                    None
                }
            }
        })
        .flatten()
    }

    async fn get_block_by_id(&self, block_id: &OLBlockId) -> Result<Option<OLBlock>, ClientError> {
        let bytes = self
            .rpc_client
            .get_raw_block_by_id(*block_id)
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;

        let blk = OLBlock::from_ssz_bytes(&bytes.0)
            .map_err(|e| ClientError::Deserialization(e.to_string()))?;
        Ok(Some(blk))
    }
}
