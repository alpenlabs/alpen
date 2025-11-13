use std::collections::HashMap;

use strata_db_types::{DbResult, traits::MempoolDatabase};
use strata_identifiers::OLTxId;

use super::schemas::MempoolTxSchema;
use crate::define_sled_database;

define_sled_database!(
    pub struct MempoolDBSled {
        tx_tree: MempoolTxSchema,
    }
);

impl MempoolDatabase for MempoolDBSled {
    fn put_tx_entry(&self, txid: &OLTxId, blob: &[u8]) -> DbResult<()> {
        self.tx_tree.insert(txid, &blob.to_vec())?;
        Ok(())
    }

    fn get_tx_entry(&self, txid: &OLTxId) -> DbResult<Option<Vec<u8>>> {
        Ok(self.tx_tree.get(txid)?)
    }

    fn get_tx_entries(&self, txids: &[OLTxId]) -> DbResult<HashMap<OLTxId, Vec<u8>>> {
        let mut result = HashMap::new();
        for txid in txids {
            if let Some(blob) = self.tx_tree.get(txid)? {
                result.insert(*txid, blob);
            }
        }
        Ok(result)
    }

    fn del_tx_entry(&self, txid: &OLTxId) -> DbResult<()> {
        self.tx_tree.remove(txid)?;
        Ok(())
    }

    fn del_tx_entries(&self, txids: &[OLTxId]) -> DbResult<()> {
        if txids.is_empty() {
            return Ok(());
        }

        // Use transaction with retry for atomic batch deletion
        self.config.with_retry((&self.tx_tree,), |(tx_tree,)| {
            for txid in txids {
                tx_tree.remove(txid)?;
            }
            Ok(())
        })
    }

    fn get_all_tx_ids(&self) -> DbResult<Vec<OLTxId>> {
        let mut tx_ids = Vec::new();
        for item in self.tx_tree.iter() {
            let (txid, _) = item?;
            tx_ids.push(txid);
        }
        Ok(tx_ids)
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
