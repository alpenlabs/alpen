use strata_db_types::types::L1TxEntry;
use strata_primitives::indexed::Indexed;
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
            self.io
                .put_tx_entry_by_idx(*entry.index(), entry.item().clone())
                .await?;
        }

        update_state(&mut self.inner, updated_entries.into_iter(), &self.io).await
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
    use strata_db_types::{traits::DatabaseBackend, types::L1TxStatus};
    use strata_l1_txfmt::MagicBytes;
    use strata_primitives::buf::Buf32;
    use strata_storage::{ops::l1tx_broadcast::Context, BroadcastDbOps};

    use super::*;
    use crate::{
        broadcaster::io::BroadcasterIo,
        test_utils::{gen_l1_tx_entry_with_status, TestBitcoinClient},
    };

    fn get_ops() -> Arc<BroadcastDbOps> {
        let pool = threadpool::Builder::new().num_threads(2).build();
        let db = get_test_sled_backend().broadcast_db();
        let ops = Context::new(db).into_ops(pool);
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
}
