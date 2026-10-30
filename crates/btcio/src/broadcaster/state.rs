use bitcoin::hashes::Hash;
use strata_db_types::l1_broadcast::{L1TxEntry, L1TxStatus};
use strata_primitives::{buf::Buf32, indexed::Indexed};
use strata_service::{ServiceState, TickMsg};
use tracing::*;

use super::{
    error::BroadcasterResult,
    input::BroadcasterInputMessage,
    io::BroadcasterIoContext,
    processor::{fetch_unfinalized_entries, process_unfinalized_entries, update_state},
};
use crate::{tx_entry::L1TxEntryExt, BtcioParams};

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
                self.handle_notify_replaced_entry(txid).await?;
            }
        }

        self.process_unfinalized_entries().await
    }

    async fn process_unfinalized_entries(&mut self) -> BroadcasterResult<()> {
        let processed = process_unfinalized_entries(
            self.inner.unfinalized_entries.iter(),
            &self.io,
            &self.config,
        )
        .await?;

        for entry in processed.updated.iter() {
            let idx = *entry.index();

            // The fee bumper can mark this entry `Replaced` in the DB between the read that
            // produced our in-memory copy and this write-back. Writing the stale status here
            // would resurrect a txid that a broadcast replacement has already superseded, so
            // re-read and leave replaced entries alone. The matching
            // `NotifyReplacedEntry` message drops it from `unfinalized_entries`.
            if self.is_replaced_in_db(idx).await? {
                debug!(%idx, "skipping write-back for entry replaced by a fee bump");
                continue;
            }

            self.io
                .put_tx_entry_by_idx(idx, entry.item().clone())
                .await?;
        }

        update_state(
            &mut self.inner,
            processed.updated.into_iter(),
            &self.io,
            processed.resync_required,
        )
        .await
    }

    /// Reports whether the persisted row at `idx` is marked [`L1TxStatus::Replaced`].
    ///
    /// The fee bumper writes that status straight to the DB, and an adoption in this service can
    /// reverse it, so the persisted row rather than the in-memory copy says whether an entry is
    /// still superseded. A missing row reads as not replaced.
    async fn is_replaced_in_db(&self, idx: u64) -> BroadcasterResult<bool> {
        Ok(self
            .io
            .get_tx_entry(idx)
            .await?
            .is_some_and(|persisted| matches!(persisted.status, L1TxStatus::Replaced { .. })))
    }

    /// Drops the in-memory copies of an entry that a fee bump superseded.
    ///
    /// The notice can be stale by the time it is handled. If the replacement is rejected while its
    /// original is mined or still in the mempool, a pass that runs before this notice reverses
    /// the replacement and re-admits the original. Dropping it then would leave it untracked
    /// until a restart: the cursor is already past its index, so it would not reach `Finalized`
    /// and a reorg on it would go unnoticed. The persisted row decides, and only a row still
    /// marked `Replaced` is dropped.
    async fn handle_notify_replaced_entry(&mut self, txid: Buf32) -> BroadcasterResult<()> {
        let Some(idx) = self
            .inner
            .unfinalized_entries
            .iter()
            .find(|entry| {
                entry
                    .item()
                    .try_to_tx()
                    .is_ok_and(|tx| Buf32::from(tx.compute_txid().to_byte_array()) == txid)
            })
            .map(|entry| *entry.index())
        else {
            // Normal when the entry already left the unfinalized set, e.g. a resync between the
            // swap and this notice skipped its `Replaced` row.
            debug!(?txid, "replaced entry was not tracked in memory");
            return Ok(());
        };

        if !self.is_replaced_in_db(idx).await? {
            info!(?txid, %idx, "persisted row is no longer replaced; keeping entry tracked");
            return Ok(());
        }

        // An entry admitted by `NotifyNewEntry` is read back by the cursor in the same pass, so it
        // can be tracked twice. Drop every copy.
        self.inner
            .unfinalized_entries
            .retain(|entry| *entry.index() != idx);
        info!(?txid, %idx, "stopped tracking entry superseded by a fee bump");
        Ok(())
    }

    /// Inserts or replaces a tracked unfinalized entry by index.
    pub(crate) async fn handle_notify_new_entry(
        &mut self,
        idx: u64,
        txentry: L1TxEntry,
    ) -> BroadcasterResult<()> {
        let txid = txentry.try_to_tx()?.compute_txid();
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
    use std::{iter::once, sync::Arc};

    use bitcoin::FeeRate;
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::{backend::DatabaseBackend, common::L1TxId, l1_broadcast::L1TxStatus};
    use strata_l1_txfmt::MagicBytes;
    use strata_primitives::buf::Buf32;
    use strata_storage::BroadcastDbOps;
    use tokio::runtime::Handle;

    use super::*;
    use crate::{
        broadcaster::{io::BroadcasterIo, L1BroadcastHandle},
        test_utils::{
            compute_entry_txid, gen_l1_tx_entry_with_status, gen_replacement_tx_entry,
            SendRawTransactionMode, TestBitcoinClient,
        },
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
        BroadcasterIo::new(
            Arc::new(client),
            ops,
            FeeRate::from_sat_per_vb(1_000).unwrap(),
        )
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
            false,
        )
        .await
        .unwrap();

        assert_eq!(service_state.inner.next_idx, idx1.unwrap() + 1);
        assert_eq!(service_state.inner.unfinalized_entries.len(), 4);

        let unf_entries = service_state.inner.unfinalized_entries;
        assert!(!unf_entries.iter().any(|e| e.item().is_finalized()));
        assert!(unf_entries.iter().all(|e| e.item().is_trackable()));
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

        let entry_txid = compute_entry_txid(&entry);
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

    /// Regression: an original can be mined before its replacement's first broadcast. The pass
    /// that handles `NotifyNewEntry` then sees the replacement rejected, adopts the original and
    /// re-admits it, so the `NotifyReplacedEntry` queued behind it must not drop it again. The
    /// cursor is past the original's index, so nothing would poll it: it would sit at `Confirmed`
    /// until a restart instead of reaching `Finalized`.
    #[tokio::test]
    async fn original_adopted_before_its_replaced_notice_stays_tracked() {
        assert_adopted_original_reaches_finalized(0).await;
    }

    /// Regression: ticks can run between the swap and the notices. The first reads the
    /// replacement in through the cursor and the second adopts the original, so both notices
    /// arrive stale, and `NotifyReplacedEntry` must still leave the original tracked.
    #[tokio::test]
    async fn original_adopted_by_ticks_before_its_notices_stays_tracked() {
        assert_adopted_original_reaches_finalized(2).await;
    }

    /// Replaces an original that is already mined, so the replacement's first broadcast is refused
    /// for spent inputs. Runs `ticks_before_notices` ticks, feeds the service both notices in the
    /// order the handle queued them, and checks that the adopted original reaches `Finalized`.
    async fn assert_adopted_original_reaches_finalized(ticks_before_notices: usize) {
        let ops = get_ops();
        let original = gen_l1_tx_entry_with_status(L1TxStatus::Published);
        let original_txid = compute_entry_txid(&original);
        let original_idx = ops
            .put_tx_entry_async(original_txid, original.clone())
            .await
            .unwrap()
            .expect("test: original index");

        // One confirmation for every txid: the original is mined, and the replacement's first
        // broadcast is refused because its inputs are spent.
        let client = TestBitcoinClient::new(1)
            .with_send_raw_transaction_mode(SendRawTransactionMode::MissingOrInvalidInput);
        let mut service_state =
            BroadcasterServiceState::try_new(make_io(ops.clone(), client), get_test_btcio_params())
                .await
                .unwrap();

        let (handle, mut receiver) = L1BroadcastHandle::new_for_test(ops.clone());
        let (replacement, replacement_txid) = gen_replacement_tx_entry(&original);
        assert!(handle
            .put_replacement_tx_entry(original_txid, replacement_txid, replacement)
            .await
            .unwrap());

        for _ in 0..ticks_before_notices {
            service_state.process_input(TickMsg::Tick).await.unwrap();
        }
        drop(handle);
        while let Some(notice) = receiver.recv().await {
            service_state
                .process_input(TickMsg::Msg(notice))
                .await
                .unwrap();
        }

        assert_eq!(
            ops.get_tx_entry_by_id_async(replacement_txid)
                .await
                .unwrap()
                .unwrap()
                .status,
            L1TxStatus::Replaced {
                by: L1TxId::from(original_txid.0)
            },
            "the rejected replacement must hand the chain back to the original"
        );
        assert_eq!(
            tracked_copies(&service_state, original_idx),
            1,
            "the adopted original must stay tracked"
        );

        // The original reaches reorg-safe depth.
        service_state.io = make_io(ops.clone(), TestBitcoinClient::new(6));
        service_state.process_input(TickMsg::Tick).await.unwrap();

        assert_eq!(
            ops.get_tx_entry_async(original_idx)
                .await
                .unwrap()
                .unwrap()
                .status,
            L1TxStatus::Finalized {
                confirmations: 6,
                block_hash: Buf32::zero(),
                block_height: 100,
            }
        );
    }

    /// Regression: an entry admitted through `NotifyNewEntry` is also read back by the cursor in
    /// the same pass, so it is tracked twice. A replaced notice that drops only one copy leaves the
    /// superseded transaction polled, and possibly re-published against its own replacement, until
    /// a restart.
    #[tokio::test]
    async fn accepted_replacement_drops_every_tracked_copy_of_the_original() {
        let ops = get_ops();
        let mut service_state = BroadcasterServiceState::try_new(
            make_io(ops.clone(), TestBitcoinClient::new(0)),
            get_test_btcio_params(),
        )
        .await
        .unwrap();
        let (handle, mut receiver) = L1BroadcastHandle::new_for_test(ops.clone());

        let original = gen_l1_tx_entry_with_status(L1TxStatus::Unpublished);
        let original_txid = compute_entry_txid(&original);
        let original_idx = handle
            .put_tx_entry(original_txid, original.clone())
            .await
            .unwrap();
        let notice = receiver.recv().await.expect("test: original notice");
        service_state
            .process_input(TickMsg::Msg(notice))
            .await
            .unwrap();
        assert_eq!(
            tracked_copies(&service_state, original_idx),
            2,
            "test: the notice and the cursor should both admit the original"
        );

        // The replacement is accepted, so no adoption resyncs the tracked set from the DB.
        let (replacement, replacement_txid) = gen_replacement_tx_entry(&original);
        assert!(handle
            .put_replacement_tx_entry(original_txid, replacement_txid, replacement)
            .await
            .unwrap());
        drop(handle);
        while let Some(notice) = receiver.recv().await {
            service_state
                .process_input(TickMsg::Msg(notice))
                .await
                .unwrap();
        }

        assert_eq!(
            tracked_copies(&service_state, original_idx),
            0,
            "every copy of the superseded original must leave tracking"
        );
    }

    /// Counts the in-memory copies of the entry at `idx`.
    fn tracked_copies<C>(service_state: &BroadcasterServiceState<C>, idx: u64) -> usize {
        service_state
            .inner
            .unfinalized_entries
            .iter()
            .filter(|entry| *entry.index() == idx)
            .count()
    }

    /// A superseded ancestor that wins on-chain has to come back into the tracked set.
    ///
    /// It left `unfinalized_entries` when it was first replaced, and the incremental cursor only
    /// reads rows added since, so without the resync nothing polls it again: it stays `Confirmed`
    /// forever, never reaches `Finalized`, and a reorg on it goes unnoticed.
    #[tokio::test]
    async fn adopted_winner_is_tracked_again() {
        let (mut service_state, winner_idx, loser_idx, replaced_loser) = adoption_fixture().await;

        let io_ref = &service_state.io;
        update_state(
            &mut service_state.inner,
            once(IndexedEntry::new(loser_idx, replaced_loser)),
            io_ref,
            true,
        )
        .await
        .unwrap();

        assert!(
            service_state
                .inner
                .unfinalized_entries
                .iter()
                .any(|entry| *entry.index() == winner_idx),
            "adopted winner must be tracked again"
        );
    }

    /// Pins what the resync flag is actually buying: without it the winner stays dropped.
    #[tokio::test]
    async fn without_resync_the_adopted_winner_stays_untracked() {
        let (mut service_state, winner_idx, loser_idx, replaced_loser) = adoption_fixture().await;

        let io_ref = &service_state.io;
        update_state(
            &mut service_state.inner,
            once(IndexedEntry::new(loser_idx, replaced_loser)),
            io_ref,
            false,
        )
        .await
        .unwrap();

        assert!(
            !service_state
                .inner
                .unfinalized_entries
                .iter()
                .any(|entry| *entry.index() == winner_idx),
            "incremental cursor cannot recover the winner on its own"
        );
    }

    /// Two entries where the second replaced the first, then the first won on-chain: the winner is
    /// `Confirmed` in the DB but already dropped from memory, and the loser is `Replaced`.
    async fn adoption_fixture() -> (
        BroadcasterServiceState<impl BroadcasterIoContext>,
        u64,
        u64,
        L1TxEntry,
    ) {
        let ops = get_ops();
        let winner = gen_l1_tx_entry_with_status(L1TxStatus::Confirmed {
            confirmations: 1,
            block_hash: [1; 32].into(),
            block_height: 100,
        });
        let winner_idx = ops
            .put_tx_entry_async([20; 32].into(), winner)
            .await
            .unwrap()
            .expect("test: winner index");
        let loser = gen_l1_tx_entry_with_status(L1TxStatus::Published);
        let loser_idx = ops
            .put_tx_entry_async([21; 32].into(), loser.clone())
            .await
            .unwrap()
            .expect("test: loser index");

        let io = make_io(ops.clone(), TestBitcoinClient::new(0));
        let mut service_state = BroadcasterServiceState::try_new(io, get_test_btcio_params())
            .await
            .unwrap();
        assert_eq!(service_state.inner.unfinalized_entries.len(), 2);

        // The winner stopped being tracked when it was replaced.
        service_state
            .inner
            .unfinalized_entries
            .retain(|entry| *entry.index() == loser_idx);

        // Adoption reverses the chain, leaving the loser superseded by the winner.
        let mut replaced_loser = loser;
        replaced_loser.status = L1TxStatus::Replaced {
            by: L1TxId::from([20u8; 32]),
        };
        ops.put_tx_entry_async([21; 32].into(), replaced_loser.clone())
            .await
            .unwrap();

        (service_state, winner_idx, loser_idx, replaced_loser)
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
