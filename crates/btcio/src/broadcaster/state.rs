use std::sync::Arc;

use bitcoin::Txid;
use strata_db::types::L1TxEntry;
use strata_primitives::indexed::Indexed;
use strata_storage::BroadcastDbOps;
use tracing::*;

use super::error::{BroadcasterError, BroadcasterResult};

pub type IndexedEntry = Indexed<L1TxEntry, u64>;

pub(crate) struct BroadcasterState {
    /// Next index from which we should next read the [`L1TxEntry`] to check and process
    pub(crate) next_idx: u64,

    /// Unfinalized [`L1TxEntry`]s which the broadcaster will check for.
    pub(crate) unfinalized_entries: Vec<IndexedEntry>,
}

impl BroadcasterState {
    /// Initialize the `[BroadcasterState]` by looking at all [`L1TxEntry`]s in database
    pub async fn initialize(ops: &Arc<BroadcastDbOps>) -> BroadcasterResult<Self> {
        Self::initialize_from_idx(ops, 0).await
    }

    /// Initialize the [`BroadcasterState`] by looking at [`L1TxEntry`]s in database starting from
    /// given `start_idx`
    pub async fn initialize_from_idx(
        ops: &Arc<BroadcastDbOps>,
        start_idx: u64,
    ) -> BroadcasterResult<Self> {
        let next_idx = ops.get_next_tx_idx_async().await?;

        let unfinalized_entries = filter_unfinalized_from_db(ops, start_idx, next_idx).await?;

        Ok(Self {
            next_idx,
            unfinalized_entries,
        })
    }

    /// Fetches entries from database based on the `next_idx` and updates the broadcaster state
    pub async fn update(
        &mut self,
        updated_entries: impl Iterator<Item = IndexedEntry>,
        ops: &Arc<BroadcastDbOps>,
    ) -> BroadcasterResult<()> {
        // Filter out finalized and invalid entries so that we don't have to process them again.
        let unfinalized_entries: Vec<_> = updated_entries
            .filter(|entry| !entry.item().is_finalized() && entry.item().is_valid())
            .collect();

        let next_idx = ops.get_next_tx_idx_async().await?;

        if next_idx < self.next_idx {
            return Err(BroadcasterError::Other(
                "Inconsistent db idx and state idx".to_string(),
            ));
        }
        let new_unfinalized_entries =
            filter_unfinalized_from_db(ops, self.next_idx, next_idx).await?;

        // Update state: include updated entries and new unfinalized entries
        self.unfinalized_entries = unfinalized_entries;
        self.unfinalized_entries.extend(new_unfinalized_entries);
        self.next_idx = next_idx;
        Ok(())
    }
}

/// Returns unfinalized but valid [`L1TxEntry`]s from db starting from index `from` until `to`
/// non-inclusive.
async fn filter_unfinalized_from_db(
    ops: &Arc<BroadcastDbOps>,
    from: u64,
    to: u64,
) -> BroadcasterResult<Vec<IndexedEntry>> {
    let mut unfinalized_entries = Vec::new();
    for idx in from..to {
        let Some(txentry) = ops.get_tx_entry_async(idx).await? else {
            break;
        };

        let status = &txentry.status;
        let txid = ops.get_txid_async(idx).await?.map(Txid::from);
        debug!(?idx, ?txid, ?status, "TxEntry");

        if txentry.is_valid() && !txentry.is_finalized() {
            unfinalized_entries.push(IndexedEntry::new(idx, txentry));
        }
    }
    Ok(unfinalized_entries)
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod test {
    use bitcoin::{consensus, Transaction};
    use strata_db::{traits::BroadcastDatabase, types::L1TxStatus};
    use strata_rocksdb::{
        broadcaster::db::{BroadcastDb, L1BroadcastDb},
        test_utils::get_rocksdb_tmp_instance,
    };
    use strata_storage::ops::l1tx_broadcast::Context;

    use super::*;
    use crate::test_utils::SOME_TX;

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

    async fn populate_broadcast_db(ops: Arc<BroadcastDbOps>) -> Vec<(u64, L1TxEntry)> {
        // Make some insertions
        let e1 = gen_entry_with_status(L1TxStatus::Unpublished);
        let i1 = ops
            .put_tx_entry_async([1; 32].into(), e1.clone())
            .await
            .unwrap();

        let e2 = gen_entry_with_status(L1TxStatus::Confirmed { confirmations: 1 });
        let i2 = ops
            .put_tx_entry_async([2; 32].into(), e2.clone())
            .await
            .unwrap();

        let e3 = gen_entry_with_status(L1TxStatus::Finalized { confirmations: 1 });
        let i3 = ops
            .put_tx_entry_async([3; 32].into(), e3.clone())
            .await
            .unwrap();

        let e4 = gen_entry_with_status(L1TxStatus::Published);
        let i4 = ops
            .put_tx_entry_async([4; 32].into(), e4.clone())
            .await
            .unwrap();

        let e5 = gen_entry_with_status(L1TxStatus::InvalidInputs);
        let i5 = ops
            .put_tx_entry_async([5; 32].into(), e5.clone())
            .await
            .unwrap();
        vec![
            (i1.unwrap(), e1),
            (i2.unwrap(), e2),
            (i3.unwrap(), e3),
            (i4.unwrap(), e4),
            (i5.unwrap(), e5),
        ]
    }

    #[tokio::test]
    async fn test_initialize() {
        // Insert entries to db
        let ops = get_ops();

        let pop = populate_broadcast_db(ops.clone()).await;
        let [(i1, _e1), (i2, _e2), (i3, _e3), (i4, _e4), (i5, _e5)] = pop.as_slice() else {
            panic!("Invalid initialization");
        };
        // Now initialize state
        let state = BroadcasterState::initialize(&ops).await.unwrap();

        assert_eq!(state.next_idx, i5 + 1);

        // state should contain all except reorged, invalid or  finalized entries
        let unfin_entries = state.unfinalized_entries;
        assert!(unfin_entries.iter().any(|e| e.index() == i1));
        assert!(unfin_entries.iter().any(|e| e.index() == i2));
        assert!(unfin_entries.iter().any(|e| e.index() == i4));

        assert!(!unfin_entries.iter().any(|e| e.index() == i3));
        assert!(!unfin_entries.iter().any(|e| e.index() == i5));
    }

    #[tokio::test]
    async fn test_next_state() {
        // Insert entries to db
        let ops = get_ops();

        let entries = populate_broadcast_db(ops.clone()).await;
        assert_eq!(entries.len(), 5, "test: broadcast db init invalid");
        // Now initialize state
        let mut state = BroadcasterState::initialize(&ops).await.unwrap();

        // Check for valid unfinalized entries in state.
        assert_eq!(
            state.unfinalized_entries.len(),
            3,
            "Total 5 but should omit 2, one finalized and one invalid"
        );

        // Get unfinalized entries where one entry is modified, another is removed
        let mut unfinalized_entries = state.unfinalized_entries.clone();
        let entry = gen_entry_with_status(L1TxStatus::InvalidInputs);
        unfinalized_entries.push(IndexedEntry::new(0, entry));

        // Insert two more items to db, one invalid and one published. Note the new idxs than used
        // in populate db.
        let e = gen_entry_with_status(L1TxStatus::InvalidInputs);
        let _ = ops
            .put_tx_entry_async([7; 32].into(), e.clone())
            .await
            .unwrap();

        let e1 = gen_entry_with_status(L1TxStatus::Published); // this should be in new state
        let idx1 = ops
            .put_tx_entry_async([8; 32].into(), e1.clone())
            .await
            .unwrap();
        // Compute next state
        //
        state
            .update(unfinalized_entries.into_iter(), &ops)
            .await
            .unwrap();

        assert_eq!(state.next_idx, idx1.unwrap() + 1);
        // Original 5, 3 added, 2 invalid, 1 finalized. Ignores finalized and invalid
        assert_eq!(state.unfinalized_entries.len(), 4);

        // Check no invalid and finalized entries in state
        let unf_entries = state.unfinalized_entries;
        assert!(!unf_entries.iter().any(|e| e.item().is_finalized()));
        assert!(unf_entries.iter().all(|e| e.item().is_valid()));
    }
}
