use strata_db::{DbResult, errors::DbError, traits::L1BroadcastDatabase, types::L1TxEntry};
use strata_identifiers::Buf32;

use super::schemas::{BcastL1TxIdSchema, BcastL1TxSchema};
use crate::{
    define_sled_database,
    utils::{find_next_available_id, first, second},
};

define_sled_database!(
    pub struct L1BroadcastDBSled {
        tx_id_tree: BcastL1TxIdSchema,
        tx_tree: BcastL1TxSchema,
    }
);

impl L1BroadcastDBSled {
    fn get_next_idx(&self) -> DbResult<u64> {
        match self.tx_id_tree.last()? {
            Some((idx, _)) => Ok(idx + 1),
            None => Ok(0),
        }
    }
}

impl L1BroadcastDatabase for L1BroadcastDBSled {
    fn put_tx_entry(&self, txid: Buf32, txentry: L1TxEntry) -> DbResult<Option<u64>> {
        let next = self.get_next_idx()?;
        let nxt =
            self.config
                .with_retry((&self.tx_tree, &self.tx_id_tree), |(txtree, txidtree)| {
                    let nxt = find_next_available_id(&txidtree, next)?;
                    if txtree.get(&txid)?.is_none() {
                        txidtree.insert(&nxt, &txid)?;
                    }
                    txtree.insert(&txid, &txentry)?;
                    Ok(nxt)
                })?;
        Ok(Some(nxt))
    }

    fn put_tx_entry_by_idx(&self, idx: u64, txentry: L1TxEntry) -> DbResult<()> {
        if let Some(txid) = self.tx_id_tree.get(&idx)? {
            self.tx_tree.insert(&txid, &txentry)?;
            Ok(())
        } else {
            Err(DbError::Other(format!(
                "Entry does not exist for idx {idx:?}"
            )))
        }
    }

    fn del_tx_entry(&self, txid: Buf32) -> DbResult<bool> {
        let old_item = self.tx_tree.get(&txid)?;
        let exists = old_item.is_some();
        if exists {
            self.tx_tree.compare_and_swap(txid, old_item, None)?;
        }
        Ok(exists)
    }

    fn del_tx_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>> {
        let last_idx = self.tx_id_tree.last()?.map(first);
        let Some(last_idx) = last_idx else {
            return Ok(Vec::new());
        };

        if start_idx > last_idx {
            return Ok(Vec::new());
        }

        let deleted_indices =
            self.config
                .with_retry((&self.tx_tree, &self.tx_id_tree), |(txtree, txidtree)| {
                    let mut deleted_indices = Vec::new();
                    for idx in start_idx..=last_idx {
                        if let Some(txid) = txidtree.get(&idx)? {
                            txidtree.remove(&idx)?;
                            txtree.remove(&txid)?;
                            deleted_indices.push(idx);
                        }
                    }
                    Ok(deleted_indices)
                })?;
        Ok(deleted_indices)
    }

    fn get_tx_entry_by_id(&self, txid: Buf32) -> DbResult<Option<L1TxEntry>> {
        Ok(self.tx_tree.get(&txid)?)
    }

    fn get_next_tx_idx(&self) -> DbResult<u64> {
        self.get_next_idx()
    }

    fn get_txid(&self, idx: u64) -> DbResult<Option<Buf32>> {
        Ok(self.tx_id_tree.get(&idx)?)
    }

    fn get_tx_entry(&self, idx: u64) -> DbResult<Option<L1TxEntry>> {
        if let Some(txid) = self.get_txid(idx)? {
            Ok(self.tx_tree.get(&txid)?)
        } else {
            Err(DbError::Other(format!(
                "Entry does not exist for idx {idx:?}"
            )))
        }
    }

    fn get_last_tx_entry(&self) -> DbResult<Option<L1TxEntry>> {
        Ok(self.tx_tree.last()?.map(second))
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::l1_broadcast_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(L1BroadcastDBSled, l1_broadcast_db_tests);
}
