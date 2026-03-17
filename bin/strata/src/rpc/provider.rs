//! Production [`OLRpcProvider`] implementation backed by real storage.

use std::sync::Arc;

use async_trait::async_trait;
use strata_asm_common::AsmManifest;
use strata_db_types::DbResult;
use strata_identifiers::{AccountId, Epoch, L1Height, OLBlockId, OLTxId};
use strata_ol_chain_types_new::OLBlock;
use strata_ol_mempool::{MempoolHandle, OLMempoolResult, OLMempoolTransaction};
use strata_ol_rpc_types::{AccountExtraData, OLRpcProvider};
use strata_ol_state_types::OLState;
use strata_primitives::{OLBlockCommitment, epoch::EpochCommitment};
use strata_status::{OLSyncStatus, StatusChannel};
use strata_storage::NodeStorage;

/// Production provider that delegates to [`NodeStorage`], [`StatusChannel`],
/// and [`MempoolHandle`].
pub(crate) struct NodeRpcProvider {
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
    mempool_handle: Arc<MempoolHandle>,
}

impl NodeRpcProvider {
    pub(crate) fn new(
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
        mempool_handle: Arc<MempoolHandle>,
    ) -> Self {
        Self {
            storage,
            status_channel,
            mempool_handle,
        }
    }
}

#[async_trait]
impl OLRpcProvider for NodeRpcProvider {
    async fn get_canonical_block_at(&self, height: u64) -> DbResult<Option<OLBlockCommitment>> {
        self.storage
            .ol_block()
            .get_canonical_block_at_async(height)
            .await
    }

    async fn get_block_data(&self, id: OLBlockId) -> DbResult<Option<OLBlock>> {
        self.storage.ol_block().get_block_data_async(id).await
    }

    async fn get_toplevel_ol_state(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<Arc<OLState>>> {
        self.storage
            .ol_state()
            .get_toplevel_ol_state_async(commitment)
            .await
    }

    async fn get_canonical_epoch_commitment_at(
        &self,
        epoch: u64,
    ) -> DbResult<Option<EpochCommitment>> {
        self.storage
            .ol_checkpoint()
            .get_canonical_epoch_commitment_at_async(epoch)
            .await
    }

    async fn get_account_extra_data(
        &self,
        key: (AccountId, Epoch),
    ) -> DbResult<Option<AccountExtraData>> {
        self.storage
            .account()
            .get_account_extra_data_async(key)
            .await
    }

    async fn get_account_creation_epoch(&self, account_id: AccountId) -> DbResult<Option<Epoch>> {
        self.storage
            .account()
            .get_account_creation_epoch_blocking(account_id)
    }

    async fn get_block_manifest_at_height(
        &self,
        height: L1Height,
    ) -> DbResult<Option<AsmManifest>> {
        self.storage
            .l1()
            .get_block_manifest_at_height_async(height)
            .await
    }

    fn get_ol_sync_status(&self) -> Option<OLSyncStatus> {
        self.status_channel.get_ol_sync_status()
    }

    async fn submit_transaction(&self, tx: OLMempoolTransaction) -> OLMempoolResult<OLTxId> {
        self.mempool_handle.submit_transaction(tx).await
    }
}
