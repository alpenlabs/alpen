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
use tracing::{info, warn};

use super::error::{BroadcasterError, BroadcasterResult};

/// Classifies a bitcoind `-25` reject reason as benign (already accepted) or
/// not. Bitcoind's reject strings use hyphenated tokens (see
/// `validation.cpp`/`policy/policy.cpp`): `txn-already-in-mempool`,
/// `txn-already-known`, `txn-already-in-block-chain`. A space-separated check
/// like `"already in mempool"` does not match any of these and would route
/// every benign `-25` to `InvalidInputs`, causing spurious envelope rebuilds.
fn is_benign_minus25_message(msg: &str) -> bool {
    msg.contains("already-in-mempool")
        || msg.contains("already-known")
        || msg.contains("already-in-block-chain")
}

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

    /// Transaction is already accepted and present in mempool.
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
        let txid = tx.compute_txid();
        match self.rpc_client.send_raw_transaction(tx).await {
            Ok(_) => {
                info!(%txid, "sendrawtransaction accepted (Published)");
                Ok(PublishTxOutcome::Published)
            }
            Err(ClientError::Server(-25, msg)) => {
                // Bitcoind reuses code -25 for several distinct reject reasons.
                // "txn-already-in-mempool" / "txn-already-known" mean the tx is
                // already accepted; fold to AlreadyInMempool.
                // "bad-txns-inputs-missingorspent" means the chosen UTXO has
                // already been spent or evicted; the entry must be re-signed
                // against a fresh listunspent snapshot. Mapping it blindly to
                // AlreadyInMempool pins the entry at Published forever and
                // stalls the watcher's curr_payloadidx.
                if is_benign_minus25_message(&msg) {
                    warn!(%txid, %msg, "sendrawtransaction reports tx already accepted (AlreadyInMempool)");
                    Ok(PublishTxOutcome::AlreadyInMempool)
                } else {
                    warn!(%txid, %msg, "sendrawtransaction -25 with non-benign message (treated as InvalidInputs)");
                    Ok(PublishTxOutcome::InvalidInputs)
                }
            }
            Err(ClientError::Server(-22, msg)) => {
                warn!(%txid, %msg, "sendrawtransaction returned -22 (treated as InvalidInputs)");
                Ok(PublishTxOutcome::InvalidInputs)
            }
            Err(err) if err.is_missing_or_invalid_input() => {
                warn!(%txid, %err, "sendrawtransaction missing/invalid input (treated as InvalidInputs)");
                Ok(PublishTxOutcome::InvalidInputs)
            }
            Err(err @ ClientError::Status(500, _)) => {
                warn!(%txid, %err, "sendrawtransaction HTTP 500 (treated as RetryLater)");
                Ok(PublishTxOutcome::RetryLater {
                    reason: err.to_string(),
                })
            }
            Err(ClientError::Server(code, msg)) => {
                warn!(%txid, %code, %msg, "sendrawtransaction returned unhandled bitcoin server error");
                Err(BroadcasterError::Rpc(anyhow!(
                    "bitcoin server error {code}: {msg}"
                )))
            }
            Err(err) => {
                warn!(%txid, %err, "sendrawtransaction returned unexpected error");
                Err(BroadcasterError::Rpc(anyhow!(err)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_benign_minus25_message;

    #[test]
    fn benign_minus25_strings_match() {
        // Bitcoind reject reasons that mean "the tx is already accepted",
        // safe to fold to AlreadyInMempool. Strings come from bitcoind's
        // validation.cpp / policy.cpp.
        assert!(is_benign_minus25_message("txn-already-in-mempool"));
        assert!(is_benign_minus25_message("txn-already-known"));
        assert!(is_benign_minus25_message("txn-already-in-block-chain"));
    }

    #[test]
    fn missing_input_minus25_is_not_benign() {
        // The case that motivated this disambiguation: input UTXO is gone,
        // entry must be re-signed against a fresh listunspent.
        assert!(!is_benign_minus25_message("bad-txns-inputs-missingorspent"));
    }

    #[test]
    fn other_minus25_reasons_are_not_benign() {
        // Catch-all: anything we have not explicitly classified as benign
        // (e.g. RBF / fee policy violations) routes to InvalidInputs so the
        // watcher rebuilds rather than pinning at Published.
        assert!(!is_benign_minus25_message("txn-mempool-conflict"));
        assert!(!is_benign_minus25_message("min relay fee not met"));
    }

    #[test]
    fn space_separated_does_not_match_hyphenated() {
        // Space-separated forms do not appear in bitcoind reject strings.
        // Keep them non-benign so only real `already-*` tokens map to
        // AlreadyInMempool.
        assert!(!is_benign_minus25_message("already in mempool"));
        assert!(!is_benign_minus25_message("already known"));
    }
}
