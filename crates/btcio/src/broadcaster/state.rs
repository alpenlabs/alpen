use bitcoin::hashes::Hash;
use strata_db_types::l1_broadcast::{L1TxEntry, L1TxStatus};
use strata_primitives::{buf::Buf32, indexed::Indexed};
use strata_service::{ServiceState, TickMsg};
use tracing::*;

use super::{
    error::{BroadcasterError, BroadcasterResult},
    input::BroadcasterInputMessage,
    io::BroadcasterIoContext,
    processor::{fetch_unfinalized_entries, process_unfinalized_entries, update_state},
};
use crate::BtcioParams;

/// Transaction entry coupled with its broadcast DB index.
pub(crate) type IndexedEntry = Indexed<L1TxEntry, u64>;

/// In-memory broadcaster progress and pending-entry view.
pub(crate) struct BroadcasterState {
    /// Next index from which to read the next [`L1TxEntry`] to process.
    pub(crate) next_idx: u64,

    /// Unfinalized [`L1TxEntry`]s which the broadcaster will check for.
    pub(crate) unfinalized_entries: Vec<IndexedEntry>,
}

impl BroadcasterState {
    fn new(next_idx: u64, unfinalized_entries: Vec<IndexedEntry>) -> Self {
        Self {
            next_idx,
            unfinalized_entries,
        }
    }
}

/// Stateful service context used by [`super::service::BroadcasterService`].
///
/// This binds pure broadcaster state to concrete IO and runtime config.
pub(crate) struct BroadcasterServiceState<C> {
    /// In-memory broadcaster cursor and unfinalized entry set.
    pub(crate) inner: BroadcasterState,
    /// Runtime broadcaster config (e.g. reorg-safe confirmation depth).
    pub(crate) config: BtcioParams,
    /// Concrete IO context used for DB reads/writes and RPC calls.
    pub(crate) io: C,
}

impl<C> BroadcasterServiceState<C>
where
    C: BroadcasterIoContext,
{
    /// Builds initial service state by scanning persisted broadcaster entries.
    pub(crate) async fn try_new(io: C, config: BtcioParams) -> BroadcasterResult<Self> {
        let next_idx = io.get_next_tx_idx().await?;
        let unfinalized_entries = fetch_unfinalized_entries(&io, 0, next_idx).await?;

        Ok(Self {
            inner: BroadcasterState::new(next_idx, unfinalized_entries),
            config,
            io,
        })
    }

    /// Handles one input event and then runs one processing pass over unfinalized entries.
    pub(crate) async fn process_input(
        &mut self,
        input: TickMsg<BroadcasterInputMessage>,
    ) -> BroadcasterResult<()> {
        match input {
            TickMsg::Tick => {}
            TickMsg::Msg(BroadcasterInputMessage::NotifyNewEntry { idx, txentry }) => {
                self.handle_notify_new_entry(idx, txentry).await?;
            }
            TickMsg::Msg(BroadcasterInputMessage::NotifyReplacedEntry { txid }) => {
                self.handle_notify_replaced_entry(txid);
            }
        }

        self.process_unfinalized_entries().await
    }

    async fn process_unfinalized_entries(&mut self) -> BroadcasterResult<()> {
        let updated_entries = process_unfinalized_entries(
            self.inner.unfinalized_entries.iter(),
            &self.io,
            &self.config,
        )
        .await?;

        for entry in updated_entries.iter() {
            let idx = *entry.index();

            // The fee bumper can mark this entry `Replaced` in the DB between the read that
            // produced our in-memory copy and this write-back. Writing the stale status here
            // would resurrect a txid that a broadcast replacement has already superseded, so
            // re-read and leave replaced entries alone. The matching
            // `NotifyReplacedEntry` message drops it from `unfinalized_entries`.
            let replaced_concurrently =
                self.io.get_tx_entry(idx).await?.is_some_and(|persisted| {
                    matches!(persisted.status, L1TxStatus::Replaced { .. })
                });
            if replaced_concurrently {
                debug!(%idx, "skipping write-back for entry replaced by a fee bump");
                continue;
            }

            self.io
                .put_tx_entry_by_idx(idx, entry.item().clone())
                .await?;
        }

        update_state(&mut self.inner, updated_entries.into_iter(), &self.io).await
    }

    /// Drops the in-memory copy of an entry that a fee bump superseded.
    fn handle_notify_replaced_entry(&mut self, txid: Buf32) {
        let entries = &mut self.inner.unfinalized_entries;
        let before = entries.len();

        entries.retain(|entry| {
            entry
                .item()
                .try_to_tx()
                .map(|tx| Buf32::from(tx.compute_txid().to_byte_array()) != txid)
                .unwrap_or(true)
        });

        if entries.len() == before {
            // Normal when the entry already left the unfinalized set, e.g. it confirmed in the
            // same tick the replacement was built and was then dropped from tracking.
            debug!(?txid, "replaced entry was not tracked in memory");
        } else {
            info!(?txid, "stopped tracking entry superseded by a fee bump");
        }
    }

    /// Inserts or replaces a tracked unfinalized entry by index.
    pub(crate) async fn handle_notify_new_entry(
        &mut self,
        idx: u64,
        txentry: L1TxEntry,
    ) -> BroadcasterResult<()> {
        let txid = txentry
            .try_to_tx()
            .map_err(|e| BroadcasterError::Other(e.to_string()))?
            .compute_txid();
        info!(%idx, %txid, "received txentry");

        let state = &mut self.inner;
        if let Some(existing) = state
            .unfinalized_entries
            .iter_mut()
            .find(|entry| *entry.index() == idx)
        {
            *existing = IndexedEntry::new(idx, txentry);
        } else {
            state
                .unfinalized_entries
                .push(IndexedEntry::new(idx, txentry));
        }

        Ok(())
    }
}

impl<C> ServiceState for BroadcasterServiceState<C>
where
    C: BroadcasterIoContext,
{
    fn name(&self) -> &str {
        "l1_broadcaster"
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::{backend::DatabaseBackend, common::L1TxId, l1_broadcast::L1TxStatus};
    use strata_l1_txfmt::MagicBytes;
    use strata_primitives::buf::Buf32;
    use strata_storage::BroadcastDbOps;
    use tokio::runtime::Handle;

    use super::*;
    use crate::{
        broadcaster::io::BroadcasterIo,
        test_utils::{gen_l1_tx_entry_with_status, SendRawTransactionMode, TestBitcoinClient},
    };

    fn get_ops() -> Arc<BroadcastDbOps> {
        let db = get_test_sled_backend().broadcast_db();
        let ops = BroadcastDbOps::new(Handle::current(), db);
        Arc::new(ops)
    }

    fn get_test_btcio_params() -> BtcioParams {
        BtcioParams::new(
            6,                         // l1_reorg_safe_depth
            MagicBytes::new(*b"ALPN"), // magic_bytes
            0,                         // genesis_l1_height
        )
    }

    fn make_io(
        ops: Arc<BroadcastDbOps>,
        client: TestBitcoinClient,
    ) -> BroadcasterIo<TestBitcoinClient> {
        BroadcasterIo::new(Arc::new(client), ops)
    }

    async fn populate_broadcast_db(ops: Arc<BroadcastDbOps>) -> Vec<(u64, L1TxEntry)> {
        // Make deterministic insertions keyed by [1;32]...[5;32].
        let entries = [
            gen_l1_tx_entry_with_status(L1TxStatus::Unpublished),
            gen_l1_tx_entry_with_status(L1TxStatus::Confirmed {
                confirmations: 1,
                block_hash: Buf32::zero(),
                block_height: 100,
            }),
            gen_l1_tx_entry_with_status(L1TxStatus::Finalized {
                confirmations: 1,
                block_hash: Buf32::zero(),
                block_height: 100,
            }),
            gen_l1_tx_entry_with_status(L1TxStatus::Published),
            gen_l1_tx_entry_with_status(L1TxStatus::InvalidInputs),
        ];

        let mut inserted = Vec::with_capacity(entries.len());
        for (offset, entry) in entries.into_iter().enumerate() {
            let key = [(offset + 1) as u8; 32];
            let idx = ops
                .put_tx_entry_async(key.into(), entry.clone())
                .await
                .unwrap()
                .expect("entry index should exist");
            inserted.push((idx, entry));
        }

        inserted
    }

    #[tokio::test]
    async fn test_initialize() {
        let ops = get_ops();

        let pop = populate_broadcast_db(ops.clone()).await;
        let [(i1, _e1), (i2, _e2), (i3, _e3), (i4, _e4), (i5, _e5)] = pop.as_slice() else {
            panic!("Invalid initialization");
        };

        let io = make_io(ops, TestBitcoinClient::new(0));
        let service_state = BroadcasterServiceState::try_new(io, get_test_btcio_params())
            .await
            .unwrap();
        let state = &service_state.inner;

        assert_eq!(state.next_idx, i5 + 1);

        assert!(state.unfinalized_entries.iter().any(|e| e.index() == i1));
        assert!(state.unfinalized_entries.iter().any(|e| e.index() == i2));
        assert!(state.unfinalized_entries.iter().any(|e| e.index() == i4));

        assert!(!state.unfinalized_entries.iter().any(|e| e.index() == i3));
        assert!(!state.unfinalized_entries.iter().any(|e| e.index() == i5));
    }

    #[tokio::test]
    async fn test_next_state() {
        let ops = get_ops();

        let entries = populate_broadcast_db(ops.clone()).await;
        assert_eq!(entries.len(), 5, "test: broadcast db init invalid");

        let io = make_io(ops.clone(), TestBitcoinClient::new(0));
        let mut service_state = BroadcasterServiceState::try_new(io, get_test_btcio_params())
            .await
            .unwrap();

        assert_eq!(
            service_state.inner.unfinalized_entries.len(),
            3,
            "Total 5 but should omit 2, one finalized and one invalid"
        );

        let mut updated_entries = service_state.inner.unfinalized_entries.clone();
        let entry = gen_l1_tx_entry_with_status(L1TxStatus::InvalidInputs);
        updated_entries.push(IndexedEntry::new(0, entry));

        let e = gen_l1_tx_entry_with_status(L1TxStatus::InvalidInputs);
        let _ = ops
            .put_tx_entry_async([7; 32].into(), e.clone())
            .await
            .unwrap();

        let e1 = gen_l1_tx_entry_with_status(L1TxStatus::Published);
        let idx1 = ops
            .put_tx_entry_async([8; 32].into(), e1.clone())
            .await
            .unwrap();
        let io_ref = &service_state.io;
        update_state(
            &mut service_state.inner,
            updated_entries.into_iter(),
            io_ref,
        )
        .await
        .unwrap();

        assert_eq!(service_state.inner.next_idx, idx1.unwrap() + 1);
        assert_eq!(service_state.inner.unfinalized_entries.len(), 4);

        let unf_entries = service_state.inner.unfinalized_entries;
        assert!(!unf_entries.iter().any(|e| e.item().is_finalized()));
        assert!(unf_entries.iter().all(|e| e.item().is_valid()));
    }

    #[tokio::test]
    async fn bitcoind_warmup_does_not_terminate_broadcaster_poll() {
        let ops = get_ops();
        let entries = populate_broadcast_db(ops.clone()).await;
        let client = TestBitcoinClient::new(0)
            .with_send_raw_transaction_mode(SendRawTransactionMode::BitcoindWarmup);
        let io = make_io(ops, client);
        let mut service_state = BroadcasterServiceState::try_new(io, get_test_btcio_params())
            .await
            .unwrap();

        service_state.process_input(TickMsg::Tick).await.unwrap();

        let statuses: Vec<_> = service_state
            .inner
            .unfinalized_entries
            .iter()
            .map(|entry| entry.item().status.clone())
            .collect();
        assert_eq!(
            statuses,
            vec![
                entries[0].1.status.clone(),
                entries[1].1.status.clone(),
                entries[3].1.status.clone(),
            ],
            "warmup must preserve broadcaster entries for the next poll"
        );
    }

    /// A fee bump marks the old entry `Replaced` in the DB directly. Without the matching
    /// notification the service would keep the stale `Published` copy in memory, re-publish the
    /// superseded txid, and write that status back over the persisted `Replaced`.
    #[tokio::test]
    async fn replaced_entry_is_dropped_from_memory_and_not_written_back() {
        let ops = get_ops();
        let txid: Buf32 = [9; 32].into();
        let entry = gen_l1_tx_entry_with_status(L1TxStatus::Published);
        let idx = ops
            .put_tx_entry_async(txid, entry.clone())
            .await
            .unwrap()
            .expect("entry index should exist");

        let io = make_io(ops.clone(), TestBitcoinClient::new(0));
        let mut service_state = BroadcasterServiceState::try_new(io, get_test_btcio_params())
            .await
            .unwrap();
        assert_eq!(service_state.inner.unfinalized_entries.len(), 1);

        // The fee bumper's DB write, made behind the service's back.
        let replacement_txid = L1TxId::from([10u8; 32]);
        let mut replaced = entry.clone();
        replaced.status = L1TxStatus::Replaced {
            by: replacement_txid,
        };
        ops.put_tx_entry_async(txid, replaced).await.unwrap();

        let entry_txid = Buf32::from(
            entry
                .try_to_tx()
                .expect("test: entry holds a valid tx")
                .compute_txid()
                .to_byte_array(),
        );
        service_state
            .process_input(TickMsg::Msg(BroadcasterInputMessage::NotifyReplacedEntry {
                txid: entry_txid,
            }))
            .await
            .unwrap();

        assert!(
            service_state.inner.unfinalized_entries.is_empty(),
            "replaced entry should no longer be tracked"
        );
        assert!(
            matches!(
                ops.get_tx_entry_async(idx).await.unwrap().unwrap().status,
                L1TxStatus::Replaced { .. }
            ),
            "persisted Replaced status must survive the processing pass"
        );
    }

    /// Even if the notification is lost, the write-back guard must not resurrect the old status.
    #[tokio::test]
    async fn write_back_skips_entries_replaced_concurrently() {
        let ops = get_ops();
        let txid: Buf32 = [11; 32].into();
        let entry = gen_l1_tx_entry_with_status(L1TxStatus::Published);
        let idx = ops
            .put_tx_entry_async(txid, entry.clone())
            .await
            .unwrap()
            .expect("entry index should exist");

        let io = make_io(ops.clone(), TestBitcoinClient::new(0));
        let mut service_state = BroadcasterServiceState::try_new(io, get_test_btcio_params())
            .await
            .unwrap();

        let mut replaced = entry;
        replaced.status = L1TxStatus::Replaced {
            by: L1TxId::from([12u8; 32]),
        };
        ops.put_tx_entry_async(txid, replaced).await.unwrap();

        // No NotifyReplacedEntry: the tick alone must leave the persisted status alone.
        service_state.process_input(TickMsg::Tick).await.unwrap();

        assert!(matches!(
            ops.get_tx_entry_async(idx).await.unwrap().unwrap().status,
            L1TxStatus::Replaced { .. }
        ));
    }
}
