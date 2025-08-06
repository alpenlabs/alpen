use std::sync::Arc;

use strata_db::{DbResult, errors::DbError, traits::L1BroadcastDatabase, types::L1TxEntry};
use strata_primitives::buf::Buf32;
use typed_sled::{SledDb, SledTree};

use super::schemas::{BcastL1TxIdSchema, BcastL1TxSchema};
use crate::{
    SledDbConfig,
    utils::{find_next_available_id, second},
};

#[derive(Debug)]
pub struct L1BroadcastDBSled {
    tx_id_tree: SledTree<BcastL1TxIdSchema>,
    tx_tree: SledTree<BcastL1TxSchema>,
    config: SledDbConfig,
}

impl L1BroadcastDBSled {
    pub fn new(db: Arc<SledDb>, config: SledDbConfig) -> DbResult<Self> {
        Ok(Self {
            tx_id_tree: db.get_tree()?,
            tx_tree: db.get_tree()?,
            config,
        })
    }

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

    fn setup_db() -> L1BroadcastDBSled {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = SledDb::new(db).unwrap();
        let config = SledDbConfig::new_with_constant_backoff(3, 100);
        L1BroadcastDBSled::new(sled_db.into(), config).unwrap()
    }

    l1_broadcast_db_tests!(setup_db());
}
