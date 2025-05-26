use std::{sync::Arc, time::Duration};

use bitcoin::{hashes::Hash, Txid};
use bitcoind_async_client::traits::{Broadcaster, Wallet};
use strata_db::types::{L1TxEntry, L1TxStatus};
use strata_primitives::params::Params;
use strata_storage::{ops::l1tx_broadcast, BroadcastDbOps};
use tokio::sync::mpsc::Receiver;
use tracing::*;

use crate::broadcaster::{
    error::{BroadcasterError, BroadcasterResult},
    state::{BroadcasterState, IndexedEntry},
};

/// Broadcasts the next blob to be sent
pub async fn broadcaster_task(
    rpc_client: Arc<impl Broadcaster + Wallet>,
    ops: Arc<l1tx_broadcast::BroadcastDbOps>,
    mut entry_receiver: Receiver<(u64, L1TxEntry)>,
    params: Arc<Params>,
    broadcast_poll_interval: u64,
) -> BroadcasterResult<()> {
    info!("Starting Broadcaster task");
    let interval = tokio::time::interval(Duration::from_millis(broadcast_poll_interval));
    tokio::pin!(interval);

    let mut state = BroadcasterState::initialize(&ops).await?;

    // Run indefinitely to watch/publish txs
    loop {
        tokio::select! {
            _ = interval.tick() => {}

            Some((idx, txentry)) = entry_receiver.recv() => {
                let txid: Txid = ops.get_txid_async(idx).await?.
                    ok_or(BroadcasterError::TxNotFound(idx))
                    .map(Into::into)?;
                info!(%idx, %txid, "Received txentry");
                state.unfinalized_entries.push(IndexedEntry::new(idx, txentry));
            }
        }

        // Process any unfinalized entries
        let updated_entries = process_unfinalized_entries(
            state.unfinalized_entries.iter(),
            ops.clone(),
            rpc_client.as_ref(),
            params.as_ref(),
        )
        .await
        .inspect_err(|e| {
            error!(%e, "broadcaster exiting");
        })?;

        // Update in db
        for entry in updated_entries.iter() {
            ops.put_tx_entry_by_idx_async(*entry.index(), entry.item().clone())
                .await?;
        }

        // Update the state.
        state.update(updated_entries.into_iter(), &ops).await?;
    }
}

/// Processes unfinalized entries and returns entries idxs that are updated.
async fn process_unfinalized_entries(
    unfinalized_entries: impl Iterator<Item = &IndexedEntry>,
    ops: Arc<BroadcastDbOps>,
    rpc_client: &(impl Broadcaster + Wallet),
    params: &Params,
) -> BroadcasterResult<Vec<IndexedEntry>> {
    let mut updated_entries = Vec::new();

    for entry in unfinalized_entries {
        let idx = *entry.index();
        let txentry = entry.item();
        let txid_raw = ops
            .get_txid_async(idx)
            .await?
            .ok_or(BroadcasterError::TxNotFound(idx))?;

        let txid = Txid::from_slice(txid_raw.0.as_slice())
            .map_err(|e| BroadcasterError::Other(e.to_string()))?;

        let span = debug_span!("process txentry", %idx, %txid);

        let _ = span.enter();
        debug!(current_status=?txentry.status);

        let updated_status = process_entry(rpc_client, txentry, &txid, params).await?;
        debug!(?updated_status);

        if let Some(status) = updated_status {
            let mut new_txentry = txentry.clone();
            new_txentry.status = status.clone();
            updated_entries.push(IndexedEntry::new(idx, new_txentry.clone()));
        }
    }
    Ok(updated_entries)
}

/// Takes in `[L1TxEntry]`, checks status and then either publishes or checks for confirmations and
/// returns its new status. Returns [`None`] if status is not changed.
async fn process_entry(
    rpc_client: &(impl Broadcaster + Wallet),
    txentry: &L1TxEntry,
    txid: &Txid,
    params: &Params,
) -> BroadcasterResult<Option<L1TxStatus>> {
    match txentry.status {
        L1TxStatus::Unpublished => publish_tx(rpc_client, txentry).await.map(Some),
        L1TxStatus::Published | L1TxStatus::Confirmed { confirmations: _ } => {
            check_tx_confirmations(rpc_client, txentry, txid, params)
                .await
                .map(Some)
        }
        L1TxStatus::Finalized { .. } => Ok(None),
        L1TxStatus::InvalidInputs => Ok(None),
    }
}

async fn check_tx_confirmations(
    rpc_client: &impl Wallet,
    txentry: &L1TxEntry,
    txid: &Txid,
    params: &Params,
) -> BroadcasterResult<L1TxStatus> {
    let txinfo_res = rpc_client.get_transaction(txid).await;
    debug!(?txentry.status, ?txinfo_res, "check get transaction");

    let reorg_safe_depth = params.rollup().l1_reorg_safe_depth.into();
    match txinfo_res {
        Ok(info) => match (info.confirmations, &txentry.status) {
            // If it was published and still 0 confirmations, set it to published
            (0, L1TxStatus::Published) => Ok(L1TxStatus::Published),

            // If it was confirmed before and now it is 0, L1 reorged.
            // So set it to Unpublished.
            (0, _) => Ok(L1TxStatus::Unpublished),

            (confirmations, _) if confirmations >= reorg_safe_depth => {
                Ok(L1TxStatus::Finalized { confirmations })
            }
            (confirmations, _) => Ok(L1TxStatus::Confirmed { confirmations }),
        },
        Err(e) => {
            // If for some reasons tx is not found even if it was already
            // published/confirmed, set it to unpublished.
            if e.is_tx_not_found() {
                Ok(L1TxStatus::Unpublished)
            } else {
                Err(BroadcasterError::Other(e.to_string()))
            }
        }
    }
}

async fn publish_tx(
    rpc_client: &impl Broadcaster,
    txentry: &L1TxEntry,
) -> BroadcasterResult<L1TxStatus> {
    let tx = txentry.try_to_tx().expect("could not deserialize tx");
    debug!("Publishing tx");
    match rpc_client.send_raw_transaction(&tx).await {
        Ok(_) => {
            info!("Successfully published tx");
            Ok(L1TxStatus::Published)
        }
        Err(err) if err.is_missing_or_invalid_input() => {
            warn!(?err, "tx excluded due to invalid inputs");

            Ok(L1TxStatus::InvalidInputs)
        }
        Err(err) => {
            warn!(?err, "errored while broadcasting");
            Err(BroadcasterError::Other(err.to_string()))
        }
    }
}

#[cfg(test)]
mod test {
    use bitcoin::{consensus, Transaction};
    use strata_db::traits::BroadcastDatabase;
    use strata_rocksdb::{
        broadcaster::db::{BroadcastDb, L1BroadcastDb},
        test_utils::get_rocksdb_tmp_instance,
    };
    use strata_storage::ops::l1tx_broadcast::Context;
    use strata_test_utils::l2::gen_params;

    use super::*;
    use crate::test_utils::{TestBitcoinClient, SOME_TX};

    fn get_db() -> Arc<impl BroadcastDatabase> {
        let (db, dbops) = get_rocksdb_tmp_instance().unwrap();
        let bcastdb = Arc::new(L1BroadcastDb::new(db, dbops));
        Arc::new(BroadcastDb::new(bcastdb))
    }

    fn get_ops() -> Arc<BroadcastDbOps> {
        let pool = threadpool::Builder::new().num_threads(2).build();
        let db = get_db();
        let ops = Context::new(db.l1_broadcast_db().clone()).into_ops(pool);
        Arc::new(ops)
    }

    fn gen_entry_with_status(st: L1TxStatus) -> L1TxEntry {
        let tx: Transaction = consensus::encode::deserialize_hex(SOME_TX).unwrap();
        let mut entry = L1TxEntry::from_tx(&tx);
        entry.status = st;
        entry
    }

    fn get_params() -> Arc<Params> {
        Arc::new(gen_params())
    }

    #[tokio::test]
    async fn test_handle_unpublished_entry() {
        let ops = get_ops();
        let e = gen_entry_with_status(L1TxStatus::Unpublished);

        // Add tx to db
        ops.put_tx_entry_async([1; 32].into(), e.clone())
            .await
            .unwrap();

        // This client will return confirmations to be 0
        let client = TestBitcoinClient::new(0);
        let cl = Arc::new(client);

        let txid = Txid::from_slice([1; 32].as_slice()).unwrap();
        let res = process_entry(cl.as_ref(), &e, &txid, get_params().as_ref())
            .await
            .unwrap();
        assert_eq!(
            res,
            Some(L1TxStatus::Published),
            "Status should be if tx is published"
        );
    }

    #[tokio::test]
    async fn test_handle_published_entry() {
        let ops = get_ops();
        let e = gen_entry_with_status(L1TxStatus::Published);

        // Add tx to db
        ops.put_tx_entry_async([1; 32].into(), e.clone())
            .await
            .unwrap();

        // This client will return confirmations to be 0
        let client = TestBitcoinClient::new(0);
        let cl = Arc::new(client);
        let params = get_params();

        let txid = Txid::from_slice([1; 32].as_slice()).unwrap();
        let res = process_entry(cl.as_ref(), &e, &txid, params.as_ref())
            .await
            .unwrap();
        assert_eq!(
            res,
            Some(L1TxStatus::Published),
            "Status should not change if no confirmations for a published tx"
        );

        let reorg_depth: u64 = params.rollup().l1_reorg_safe_depth.into();
        // This client will return confirmations to be finality_depth - 1
        let client = TestBitcoinClient::new(reorg_depth - 1);
        let cl = Arc::new(client);

        let res = process_entry(cl.as_ref(), &e, &txid, params.as_ref())
            .await
            .unwrap();
        assert_eq!(
            res,
            Some(L1TxStatus::Confirmed {
                confirmations: cl.confs
            }),
            "Status should be confirmed if 0 < confirmations < finality_depth"
        );

        // This client will return confirmations to be finality_depth
        let client = TestBitcoinClient::new(reorg_depth);
        let cl = Arc::new(client);

        let res = process_entry(cl.as_ref(), &e, &txid, params.as_ref())
            .await
            .unwrap();
        assert_eq!(
            res,
            Some(L1TxStatus::Finalized {
                confirmations: cl.confs
            }),
            "Status should be confirmed if confirmations >= finality_depth"
        );
    }

    #[tokio::test]
    async fn test_handle_confirmed_entry() {
        let ops = get_ops();
        let e = gen_entry_with_status(L1TxStatus::Confirmed { confirmations: 1 });

        // Add tx to db
        ops.put_tx_entry_async([1; 32].into(), e.clone())
            .await
            .unwrap();

        // This client will return confirmations to be 0
        let client = TestBitcoinClient::new(0);
        let cl = Arc::new(client);

        let params = get_params();
        let txid = Txid::from_slice([1; 32].as_slice()).unwrap();
        let res = process_entry(cl.as_ref(), &e, &txid, params.as_ref())
            .await
            .unwrap();
        assert_eq!(
            res,
            Some(L1TxStatus::Unpublished),
            "Status should revert to reorged if previously confirmed tx has 0 confirmations"
        );

        let reorg_depth = params.rollup().l1_reorg_safe_depth as u64;
        // This client will return confirmations to be finality_depth - 1
        let client = TestBitcoinClient::new(reorg_depth - 1);
        let cl = Arc::new(client);

        let res = process_entry(cl.as_ref(), &e, &txid, params.as_ref())
            .await
            .unwrap();
        assert_eq!(
            res,
            Some(L1TxStatus::Confirmed {
                confirmations: cl.confs
            }),
            "Status should be confirmed if 0 < confirmations < finality_depth"
        );

        // This client will return confirmations to be finality_depth
        let client = TestBitcoinClient::new(reorg_depth);
        let cl = Arc::new(client);

        let res = process_entry(cl.as_ref(), &e, &txid, params.as_ref())
            .await
            .unwrap();
        assert_eq!(
            res,
            Some(L1TxStatus::Finalized {
                confirmations: cl.confs
            }),
            "Status should be confirmed if confirmations >= finality_depth"
        );
    }

    /// The updated status should be Finalized for a finalized tx.
    #[tokio::test]
    async fn test_handle_finalized_entry() {
        let ops = get_ops();
        let e = gen_entry_with_status(L1TxStatus::Finalized { confirmations: 1 });

        // Add tx to db
        ops.put_tx_entry_async([1; 32].into(), e.clone())
            .await
            .unwrap();

        let params = get_params();
        let reorg_depth = params.rollup().l1_reorg_safe_depth as u64;
        // This client will return confirmations to be Finality depth
        let client = TestBitcoinClient::new(reorg_depth);
        let cl = Arc::new(client);

        let txid = Txid::from_slice([1; 32].as_slice()).unwrap();
        let res = process_entry(cl.as_ref(), &e, &txid, params.as_ref())
            .await
            .unwrap();
        assert_eq!(
            res, None,
            "Status should not change for finalized tx. Should remain the same."
        );

        // This client will return confirmations to be 0
        // NOTE: this should not occur in practice though
        let client = TestBitcoinClient::new(0);
        let cl = Arc::new(client);

        let res = process_entry(cl.as_ref(), &e, &txid, params.as_ref())
            .await
            .unwrap();
        assert_eq!(
            res, None,
            "Status should not change for finalized tx. Should remain the same."
        );
    }

    #[tokio::test]
    async fn test_handle_excluded_entry() {
        let ops = get_ops();
        let e = gen_entry_with_status(L1TxStatus::InvalidInputs);

        // Add tx to db
        ops.put_tx_entry_async([1; 32].into(), e.clone())
            .await
            .unwrap();

        let params = get_params();
        let reorg_depth = params.rollup().l1_reorg_safe_depth as u64;
        // This client will return confirmations to be Finality depth
        let client = TestBitcoinClient::new(reorg_depth);
        let cl = Arc::new(client);

        let txid = Txid::from_slice([1; 32].as_slice()).unwrap();
        let res = process_entry(cl.as_ref(), &e, &txid, params.as_ref())
            .await
            .unwrap();
        assert_eq!(
            res, None,
            "Status should not change for excluded tx. Should remain the same."
        );

        // This client will return confirmations to be 0
        // NOTE: this should not occur in practice for a finalized tx though
        let client = TestBitcoinClient::new(0);
        let cl = Arc::new(client);

        let res = process_entry(cl.as_ref(), &e, &txid, params.as_ref())
            .await
            .unwrap();
        assert_eq!(
            res, None,
            "Status should not change for excluded tx. Should remain the same."
        );
    }

    #[tokio::test]
    async fn test_process_unfinalized_entries() {
        let ops = get_ops();
        // Add a couple of txs
        let e1 = gen_entry_with_status(L1TxStatus::Unpublished);
        let i1 = ops.put_tx_entry_async([1; 32].into(), e1).await.unwrap();
        let e2 = gen_entry_with_status(L1TxStatus::InvalidInputs);
        let _i2 = ops.put_tx_entry_async([2; 32].into(), e2).await.unwrap();

        let e3 = gen_entry_with_status(L1TxStatus::Published);
        let i3 = ops.put_tx_entry_async([3; 32].into(), e3).await.unwrap();

        let state = BroadcasterState::initialize(&ops).await.unwrap();

        let params = get_params();
        let reorg_depth = params.rollup().l1_reorg_safe_depth as u64;
        // This client will make the published tx finalized
        let client = TestBitcoinClient::new(reorg_depth);
        let cl = Arc::new(client);

        let updated_entries = process_unfinalized_entries(
            state.unfinalized_entries.iter(),
            ops,
            cl.as_ref(),
            params.as_ref(),
        )
        .await
        .unwrap();

        assert_eq!(
            updated_entries
                .iter()
                .find(|e| *e.index() == i1.unwrap())
                .map(|e| e.item().status.clone())
                .unwrap(),
            L1TxStatus::Published,
            "unpublished tx should be published"
        );
        assert_eq!(
            updated_entries
                .iter()
                .find(|e| *e.index() == i3.unwrap())
                .map(|e| e.item().status.clone())
                .unwrap(),
            L1TxStatus::Finalized {
                confirmations: cl.confs
            },
            "published tx should be finalized"
        );
    }
}
