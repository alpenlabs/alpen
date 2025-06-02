use std::sync::Arc;

use bitcoind_async_client::traits::{Broadcaster, Reader, Signer, Wallet};
use strata_db::{
    types::{L1TxEntry, L1TxStatus},
    DbResult,
};
use strata_primitives::{buf::Buf32, params::Params};
use strata_storage::BroadcastDbOps;
use strata_tasks::TaskExecutor;
use tokio::sync::mpsc;
use tracing::*;

use super::task::broadcaster_task;

#[expect(missing_debug_implementations)]
pub struct L1BroadcastHandle {
    ops: Arc<BroadcastDbOps>,
    sender: mpsc::Sender<(u64, L1TxEntry)>,
}

impl L1BroadcastHandle {
    pub fn new(sender: mpsc::Sender<(u64, L1TxEntry)>, ops: Arc<BroadcastDbOps>) -> Self {
        Self { ops, sender }
    }

    pub async fn get_tx_status(&self, txid: Buf32) -> DbResult<Option<L1TxStatus>> {
        Ok(self
            .ops
            .get_tx_entry_by_id_async(txid)
            .await?
            .map(|e| e.status))
    }

    /// Insert an entry to the database
    ///
    /// # Notes
    ///
    /// This function is infallible. If the entry already exists it will update with the new
    /// `txentry`.
    pub async fn put_tx_entry(&self, txid: Buf32, txentry: L1TxEntry) -> DbResult<Option<u64>> {
        trace!(%txid, "insert_new_tx_entry");
        assert!(txentry.try_to_tx().is_ok(), "invalid tx entry {txentry:?}");
        let Some(idx) = self.ops.put_tx_entry_async(txid, txentry.clone()).await? else {
            return Ok(None);
        };
        if self.sender.send((idx, txentry)).await.is_err() {
            // Not really an error, it just means it's shutting down, we'll pick
            // it up when we restart.
            warn!("L1 tx broadcast worker shutting down");
        }

        Ok(Some(idx))
    }

    pub async fn get_tx_entry_by_id_async(&self, txid: Buf32) -> DbResult<Option<L1TxEntry>> {
        self.ops.get_tx_entry_by_id_async(txid).await
    }

    pub async fn get_last_tx_entry(&self) -> DbResult<Option<L1TxEntry>> {
        self.ops.get_last_tx_entry_async().await
    }

    pub async fn get_tx_entry_by_idx_async(&self, idx: u64) -> DbResult<Option<L1TxEntry>> {
        self.ops.get_tx_entry_async(idx).await
    }
}

pub fn spawn_broadcaster_task<T>(
    executor: &TaskExecutor,
    l1_rpc_client: Arc<T>,
    broadcast_ops: Arc<BroadcastDbOps>,
    params: Arc<Params>,
    broadcast_poll_interval: u64,
) -> L1BroadcastHandle
where
    T: Reader + Broadcaster + Wallet + Signer + Send + Sync + 'static,
{
    let (broadcast_entry_tx, broadcast_entry_rx) = mpsc::channel::<(u64, L1TxEntry)>(64);
    let ops = broadcast_ops.clone();
    executor.spawn_critical_async("l1_broadcaster_task", async move {
        broadcaster_task(
            l1_rpc_client,
            ops,
            broadcast_entry_rx,
            params,
            broadcast_poll_interval,
        )
        .await
        .map_err(Into::into)
    });
    L1BroadcastHandle::new(broadcast_entry_tx, broadcast_ops)
}
