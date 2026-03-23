use std::{future::Future, sync::Arc};

use anyhow::anyhow;
use bitcoin::{Transaction, Txid};
use bitcoind_async_client::{
    error::ClientError,
    traits::{Broadcaster, Wallet},
};
use strata_btc_types::BlockHashExt;
use strata_db_types::types::L1TxEntry;
use strata_primitives::{buf::Buf32, L1Height};
use strata_storage::BroadcastDbOps;

use super::error::{BroadcasterError, BroadcasterResult};

/// IO context abstraction for broadcaster service internals.
pub(crate) trait BroadcasterIoContext: Send + Sync + 'static {
    /// Returns the next write index in broadcaster database.
    fn get_next_tx_idx(&self) -> impl Future<Output = BroadcasterResult<u64>> + Send;

    /// Returns the broadcast entry at `idx`, or `None` if missing.
    fn get_tx_entry(
        &self,
        idx: u64,
    ) -> impl Future<Output = BroadcasterResult<Option<L1TxEntry>>> + Send;

    /// Persists `entry` at the existing broadcast index `idx`.
    fn put_tx_entry_by_idx(
        &self,
        idx: u64,
        entry: L1TxEntry,
    ) -> impl Future<Output = BroadcasterResult<()>> + Send;

    /// Fetches transaction observation data used for confirmation-state transitions.
    fn get_transaction<'a>(
        &'a self,
        txid: &'a Txid,
    ) -> impl Future<Output = BroadcasterResult<Option<TxConfirmationInfo>>> + Send + 'a;

    /// Attempts publication and classifies the outcome for retry/state logic.
    fn send_raw_transaction<'a>(
        &'a self,
        tx: &'a Transaction,
    ) -> impl Future<Output = BroadcasterResult<PublishTxOutcome>> + Send + 'a;
}

/// Minimal transaction view needed by broadcaster confirmation logic.
#[derive(Clone, Debug)]
pub(crate) struct TxConfirmationInfo {
    pub(crate) confirmations: i64,
    pub(crate) block_hash: Option<Buf32>,
    pub(crate) block_height: Option<L1Height>,
}

/// Broadcaster-level outcome of broadcasting a transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PublishTxOutcome {
    /// Transaction was accepted for broadcast.
    Published,

    /// Transaction was already accepted earlier and is already in mempool.
    AlreadyInMempool,

    /// Transaction has invalid/missing inputs and should be marked invalid.
    InvalidInputs,

    /// Transient failure; call sites should retry in a later poll pass.
    RetryLater { reason: String },
}

/// Production IO context wrapping concrete DB ops and Bitcoin RPC client.
pub(crate) struct BroadcasterIo<T> {
    rpc_client: Arc<T>,
    ops: Arc<BroadcastDbOps>,
}

impl<T> BroadcasterIo<T> {
    /// Creates a production IO adapter from RPC client and broadcast DB ops.
    #[expect(
        dead_code,
        reason = "constructor used once builder switches to service path"
    )]
    pub(crate) fn new(rpc_client: Arc<T>, ops: Arc<BroadcastDbOps>) -> Self {
        Self { rpc_client, ops }
    }
}

impl<T> BroadcasterIoContext for BroadcasterIo<T>
where
    T: Broadcaster + Wallet + Send + Sync + 'static,
{
    async fn get_next_tx_idx(&self) -> BroadcasterResult<u64> {
        Ok(self.ops.get_next_tx_idx_async().await?)
    }

    async fn get_tx_entry(&self, idx: u64) -> BroadcasterResult<Option<L1TxEntry>> {
        Ok(self.ops.get_tx_entry_async(idx).await?)
    }

    async fn put_tx_entry_by_idx(&self, idx: u64, entry: L1TxEntry) -> BroadcasterResult<()> {
        self.ops.put_tx_entry_by_idx_async(idx, entry).await?;
        Ok(())
    }

    async fn get_transaction<'a>(
        &'a self,
        txid: &'a Txid,
    ) -> BroadcasterResult<Option<TxConfirmationInfo>> {
        match self.rpc_client.get_transaction(txid).await {
            Ok(info) => Ok(Some(TxConfirmationInfo {
                confirmations: info.confirmations,
                block_hash: info.block_hash.map(|h| h.to_buf32()),
                block_height: info.block_height,
            })),
            Err(err) if err.is_tx_not_found() => Ok(None),
            Err(err) => Err(BroadcasterError::Rpc(anyhow!(err))),
        }
    }

    async fn send_raw_transaction<'a>(
        &'a self,
        tx: &'a Transaction,
    ) -> BroadcasterResult<PublishTxOutcome> {
        match self.rpc_client.send_raw_transaction(tx).await {
            Ok(_) => Ok(PublishTxOutcome::Published),
            Err(ClientError::Server(-25, _)) => Ok(PublishTxOutcome::AlreadyInMempool),
            Err(err)
                if err.is_missing_or_invalid_input()
                    || matches!(err, ClientError::Server(-22, _)) =>
            {
                Ok(PublishTxOutcome::InvalidInputs)
            }
            Err(err @ ClientError::Status(500, _)) => Ok(PublishTxOutcome::RetryLater {
                reason: err.to_string(),
            }),
            Err(err) => Err(BroadcasterError::Rpc(anyhow!(err))),
        }
    }
}
