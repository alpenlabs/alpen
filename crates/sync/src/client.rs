use std::cmp::min;

use futures::stream::{self, Stream, StreamExt};
use strata_primitives::l2::L2BlockCommitment;
use strata_rpc_api::StrataApiClient;
use strata_state::{block::L2BlockBundle, id::L2BlockId};
use tracing::error;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("missing block: {0}")]
    MissingBlock(L2BlockId),
    #[error("failed to deserialize block: {0}")]
    Deserialization(String),
    #[error("network: {0}")]
    Network(String),
}

#[derive(Debug)]
pub struct PeerSyncStatus {
    tip_block: L2BlockCommitment,
}

impl PeerSyncStatus {
    pub const fn tip_block(&self) -> &L2BlockCommitment {
        &self.tip_block
    }

    pub const fn tip_block_id(&self) -> &L2BlockId {
        self.tip_block.blkid()
    }

    pub const fn tip_height(&self) -> u64 {
        self.tip_block.slot()
    }
}

#[async_trait::async_trait]
pub trait SyncClient {
    async fn get_sync_status(&self) -> Result<PeerSyncStatus, ClientError>;

    fn get_blocks_range(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> impl Stream<Item = L2BlockBundle>;

    async fn get_block_by_id(
        &self,
        block_id: &L2BlockId,
    ) -> Result<Option<L2BlockBundle>, ClientError>;
}

#[derive(Debug)]
pub struct RpcSyncPeer<RPC: StrataApiClient + Send + Sync> {
    rpc_client: RPC,
    download_batch_size: usize,
}

impl<RPC: StrataApiClient + Send + Sync> RpcSyncPeer<RPC> {
    #[expect(clippy::missing_const_for_fn)]
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
    ) -> Result<Vec<L2BlockBundle>, ClientError> {
        let bytes = self
            .rpc_client
            .get_raw_bundles(start_height, end_height)
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;

        borsh::from_slice(&bytes.0).map_err(|err| ClientError::Deserialization(err.to_string()))
    }
}

#[async_trait::async_trait]
impl<RPC: StrataApiClient + Send + Sync> SyncClient for RpcSyncPeer<RPC> {
    async fn get_sync_status(&self) -> Result<PeerSyncStatus, ClientError> {
        let status = self
            .rpc_client
            .sync_status()
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?;
        Ok(PeerSyncStatus {
            tip_block: L2BlockCommitment::new(status.tip_height, status.tip_block_id),
        })
    }

    fn get_blocks_range(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> impl Stream<Item = L2BlockBundle> {
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

    async fn get_block_by_id(
        &self,
        block_id: &L2BlockId,
    ) -> Result<Option<L2BlockBundle>, ClientError> {
        let bytes = self
            .rpc_client
            .get_raw_bundle_by_id(*block_id)
            .await
            .map_err(|e| ClientError::Network(e.to_string()))?
            .ok_or(ClientError::MissingBlock(*block_id))?;

        borsh::from_slice(&bytes.0).map_err(|err| ClientError::Deserialization(err.to_string()))
    }
}
