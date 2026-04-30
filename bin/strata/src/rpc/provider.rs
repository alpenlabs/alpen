//! Production [`OLRpcProvider`] implementation backed by real storage.

use std::sync::Arc;

use async_trait::async_trait;
use ssz::Decode;
use strata_acct_types::MessageEntry;
use strata_asm_common::AsmManifest;
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::CheckpointL1Ref;
use strata_db_types::{
    DbError, DbResult, MmrId,
    ol_state_index::{AccountUpdateRecord, InboxMessageRecord},
};
use strata_identifiers::{AccountId, Epoch, L1Height, OLBlockId, OLTxId};
use strata_ol_chain_types_new::{OLBlock, OLTransaction};
use strata_ol_mempool::{MempoolHandle, OLMempoolResult};
use strata_ol_rpc_types::OLRpcProvider;
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
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        self.storage
            .ol_checkpoint()
            .get_canonical_epoch_commitment_at_async(epoch)
            .await
    }

    async fn get_epoch_summary(
        &self,
        commitment: EpochCommitment,
    ) -> DbResult<Option<EpochSummary>> {
        self.storage
            .ol_checkpoint()
            .get_epoch_summary_async(commitment)
            .await
    }

    async fn get_checkpoint_l1_ref(
        &self,
        commitment: EpochCommitment,
    ) -> DbResult<Option<CheckpointL1Ref>> {
        self.storage
            .ol_checkpoint()
            .get_checkpoint_l1_ref_async(commitment)
            .await
    }

    async fn get_account_update_records(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<AccountUpdateRecord>>> {
        self.storage
            .ol_state_indexing()
            .get_account_update_records_async(epoch, account)
            .await
    }

    async fn get_account_inbox_records(
        &self,
        epoch: Epoch,
        account: AccountId,
    ) -> DbResult<Option<Vec<InboxMessageRecord>>> {
        self.storage
            .ol_state_indexing()
            .get_account_inbox_records_async(epoch, account)
            .await
    }

    async fn get_account_inbox_messages(
        &self,
        account_id: AccountId,
        start_idx: u64,
        end_idx_exclusive: u64,
    ) -> DbResult<Vec<MessageEntry>> {
        if end_idx_exclusive <= start_idx {
            return Ok(Vec::new());
        }

        let mmr_handle = self
            .storage
            .mmr_index()
            .as_ref()
            .get_handle(MmrId::SnarkMsgInbox(account_id));

        let mut messages = Vec::with_capacity((end_idx_exclusive - start_idx) as usize);
        for idx in start_idx..end_idx_exclusive {
            let preimage = mmr_handle.get(idx).await?;
            let message = MessageEntry::from_ssz_bytes(&preimage).map_err(|e| {
                DbError::Other(format!(
                    "failed to decode account inbox message at index {idx} for account {account_id}: {e}"
                ))
            })?;
            messages.push(message);
        }

        Ok(messages)
    }

    async fn get_account_creation_epoch(&self, account_id: AccountId) -> DbResult<Option<Epoch>> {
        self.storage
            .ol_state_indexing()
            .get_account_creation_epoch_async(account_id)
            .await
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

    fn get_l1_tip_height(&self) -> Option<L1Height> {
        Some(self.status_channel.get_l1_status().cur_height)
    }

    async fn submit_transaction(&self, tx: OLTransaction) -> OLMempoolResult<OLTxId> {
        self.mempool_handle.submit_transaction(tx).await
    }
}
