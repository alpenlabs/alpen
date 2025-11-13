use std::collections::HashMap;

use strata_db_types::{DbResult, traits::MempoolDatabase, types::MempoolTxMetadata};
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;

use super::schemas::MempoolTxSchema;
use crate::define_sled_database;

define_sled_database!(
    pub struct MempoolDBSled {
        tx_tree: MempoolTxSchema,
    }
);

impl MempoolDatabase for MempoolDBSled {
    fn put_tx_entry(
        &self,
        txid: &OLTxId,
        tx: &OLTransaction,
        metadata: &MempoolTxMetadata,
    ) -> DbResult<()> {
        self.tx_tree.insert(txid, &(tx.clone(), metadata.clone()))?;
        Ok(())
    }

    fn get_tx_entry(&self, txid: &OLTxId) -> DbResult<Option<(OLTransaction, MempoolTxMetadata)>> {
        Ok(self.tx_tree.get(txid)?)
    }

    fn get_tx_entries(
        &self,
        txids: &[OLTxId],
    ) -> DbResult<HashMap<OLTxId, (OLTransaction, MempoolTxMetadata)>> {
        let mut result = HashMap::new();
        for txid in txids {
            if let Some(entry) = self.tx_tree.get(txid)? {
                result.insert(*txid, entry);
            }
        }
        Ok(result)
    }

    fn del_tx_entry(&self, txid: &OLTxId) -> DbResult<()> {
        self.tx_tree.remove(txid)?;
        Ok(())
    }

    fn del_tx_entries(&self, txids: &[OLTxId]) -> DbResult<()> {
        for txid in txids {
            self.tx_tree.remove(txid)?;
        }
        Ok(())
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
