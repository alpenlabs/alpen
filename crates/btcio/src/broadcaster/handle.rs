use std::{str, sync::Arc};

use hex::encode_to_slice;
use strata_db_types::{
    types::{L1TxEntry, L1TxId, L1TxStatus, TxNodeId, TxNodeRecord},
    DbResult,
};
use strata_primitives::buf::Buf32;
use strata_service::ServiceMonitor;
use strata_storage::BroadcastDbOps;
use tokio::sync::mpsc;
use tracing::*;

use super::{
    error::{BroadcasterError, BroadcasterResult},
    input::BroadcasterInputMessage,
    service::BroadcasterStatus,
};

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug impls"
)]
pub struct L1BroadcastHandle {
    ops: Arc<BroadcastDbOps>,
    sender: mpsc::Sender<BroadcasterInputMessage>,
    monitor: Option<ServiceMonitor<BroadcasterStatus>>,
}

impl L1BroadcastHandle {
    pub(crate) fn new(
        sender: mpsc::Sender<BroadcasterInputMessage>,
        ops: Arc<BroadcastDbOps>,
        monitor: Option<ServiceMonitor<BroadcasterStatus>>,
    ) -> Self {
        Self {
            ops,
            sender,
            monitor,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(ops: Arc<BroadcastDbOps>) -> Self {
        let (sender, _) = mpsc::channel::<BroadcasterInputMessage>(64);
        Self::new(sender, ops, None)
    }

    pub fn monitor(&self) -> Option<&ServiceMonitor<BroadcasterStatus>> {
        self.monitor.as_ref()
    }

    pub async fn get_tx_status(&self, txid: Buf32) -> DbResult<Option<L1TxStatus>> {
        Ok(self
            .ops
            .get_tx_entry_by_id_async(txid)
            .await?
            .map(|e| e.status))
    }

    /// Insert an entry to the database and notify the broadcaster service.
    ///
    /// # Notes
    ///
    /// The DB write happens on the caller task first. Notification send is fire-and-forget.
    pub async fn put_tx_entry(&self, txid: Buf32, txentry: L1TxEntry) -> BroadcasterResult<u64> {
        // NOTE: Reverse the txid to little endian so that it's consistent with block explorers.
        let mut bytes = txid.0;
        bytes.reverse();
        let mut hex_buf = [0u8; 64];
        encode_to_slice(bytes, &mut hex_buf).expect("buf: enc hex");
        // SAFETY: hex encoding always produces valid UTF-8
        let txid_le = unsafe { str::from_utf8_unchecked(&hex_buf) };
        trace!(txid = %txid_le, "insert_new_tx_entry");

        assert!(txentry.try_to_tx().is_ok(), "invalid tx entry {txentry:?}");

        let Some(idx) = self.ops.put_tx_entry_async(txid, txentry.clone()).await? else {
            error!(
                txid = %txid_le,
                "tx entry was persisted but storage returned no entry index"
            );
            return Err(BroadcasterError::MissingEntryIndex(txid));
        };

        if self
            .sender
            .send(BroadcasterInputMessage::NotifyNewEntry { idx, txentry })
            .await
            .is_err()
        {
            // Not really an error, it just means it's shutting down; we'll pick
            // it up when we restart by scanning persisted entries.
            warn!("L1 broadcaster service is unavailable");
        }

        Ok(idx)
    }

    pub async fn get_tx_entry_by_id_async(&self, txid: Buf32) -> DbResult<Option<L1TxEntry>> {
        self.ops.get_tx_entry_by_id_async(txid).await
    }

    pub async fn update_tx_entry_by_id_async(
        &self,
        txid: Buf32,
        txentry: L1TxEntry,
    ) -> DbResult<()> {
        let _ = self.ops.put_tx_entry_async(txid, txentry).await?;
        Ok(())
    }

    pub async fn get_active_tx_entry_by_id_async(
        &self,
        mut txid: Buf32,
    ) -> DbResult<Option<(Buf32, L1TxEntry)>> {
        for _ in 0..16 {
            let Some(entry) = self.get_tx_entry_by_id_async(txid).await? else {
                return Ok(None);
            };
            match entry.status {
                L1TxStatus::Replaced { by } => txid = Buf32(by.0),
                _ => return Ok(Some((txid, entry))),
            }
        }

        Ok(None)
    }

    pub async fn get_last_tx_entry(&self) -> DbResult<Option<L1TxEntry>> {
        self.ops.get_last_tx_entry_async().await
    }

    pub async fn get_tx_entry_by_idx_async(&self, idx: u64) -> DbResult<Option<L1TxEntry>> {
        self.ops.get_tx_entry_async(idx).await
    }

    pub async fn put_tx_entry_by_idx(&self, idx: u64, txentry: L1TxEntry) -> BroadcasterResult<()> {
        self.ops.put_tx_entry_by_idx_async(idx, txentry).await?;
        Ok(())
    }

    pub async fn put_tx_node(&self, node: TxNodeRecord) -> DbResult<()> {
        self.ops.put_tx_node_async(node.node_id, node).await
    }

    pub async fn get_tx_node(&self, node_id: TxNodeId) -> DbResult<Option<TxNodeRecord>> {
        self.ops.get_tx_node_async(node_id).await
    }

    pub async fn get_all_tx_nodes(&self) -> DbResult<Vec<TxNodeRecord>> {
        self.ops.get_all_tx_nodes_async().await
    }
}
