//! Sled-backed mempool database implementation.

use strata_db_types::{DbResult, traits::MempoolDatabase, types::MempoolTxData};
use strata_identifiers::OLTxId;

use super::schemas::{MempoolTxEntry, MempoolTxSchema};
use crate::{define_sled_database, utils::to_db_error};

define_sled_database!(
    pub struct MempoolDBSled {
        tx_tree: MempoolTxSchema,
    }
);

impl MempoolDatabase for MempoolDBSled {
    fn put_tx(&self, data: MempoolTxData) -> DbResult<()> {
        let entry = MempoolTxEntry::new(data.tx_bytes, data.first_seen_slot, data.insertion_id);
        self.config
            .with_retry((&self.tx_tree,), |(tx_tree,)| {
                tx_tree.insert(&data.txid, &entry)?;
                Ok(())
            })
            .map_err(to_db_error)
    }

    fn get_tx(&self, txid: OLTxId) -> DbResult<Option<MempoolTxData>> {
        Ok(self.tx_tree.get(&txid)?.map(|entry| {
            let (tx_bytes, first_seen_slot, insertion_id) = entry.into_tuple();
            MempoolTxData::new(txid, tx_bytes, first_seen_slot, insertion_id)
        }))
    }

    fn get_all_txs(&self) -> DbResult<Vec<MempoolTxData>> {
        let mut result = Vec::new();
        for item in self.tx_tree.iter() {
            let (txid, entry) = item?;
            let (tx_bytes, first_seen_slot, insertion_id) = entry.into_tuple();
            result.push(MempoolTxData::new(
                txid,
                tx_bytes,
                first_seen_slot,
                insertion_id,
            ));
        }
        Ok(result)
    }

    fn del_tx(&self, txid: OLTxId) -> DbResult<bool> {
        let old_entry = self.tx_tree.get(&txid)?;
        let existed = old_entry.is_some();
        if existed {
            self.tx_tree.remove(&txid)?;
        }
        Ok(existed)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::mempool_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(MempoolDBSled, mempool_db_tests);
}
