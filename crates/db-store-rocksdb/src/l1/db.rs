use std::sync::Arc;

use rockbound::{
    rocksdb::ReadOptions,
    schema::KeyEncoder,
    utils::{get_first, get_last},
    OptimisticTransactionDB, SchemaBatch, SchemaDBOperationsExt,
};
use strata_db::{errors::DbError, traits::*, DbResult};
use strata_primitives::l1::{L1BlockId, L1BlockManifest, L1Tx, L1TxRef};
use tracing::*;

use super::schemas::{L1BlockSchema, L1BlocksByHeightSchema, L1CanonicalBlockSchema, TxnSchema};
use crate::DbOpsConfig;

#[derive(Debug)]
pub struct L1Db {
    db: Arc<OptimisticTransactionDB>,
    ops: DbOpsConfig,
}

impl L1Db {
    // NOTE: db is expected to open all the column families defined in STORE_COLUMN_FAMILIES.
    // FIXME: Make it better/generic.
    pub fn new(db: Arc<OptimisticTransactionDB>, ops: DbOpsConfig) -> Self {
        Self { db, ops }
    }

    pub fn get_latest_block(&self) -> DbResult<Option<(u64, L1BlockId)>> {
        Ok(get_last::<L1CanonicalBlockSchema>(self.db.as_ref())?)
    }
}

impl L1Database for L1Db {
    fn put_block_data(&self, mf: L1BlockManifest) -> DbResult<()> {
        let blockid = mf.blkid();
        let height = mf.height();

        self.db
            .with_optimistic_txn(self.ops.txn_retry_count(), |txn| {
                let mut blocks_at_height = txn
                    .get_for_update::<L1BlocksByHeightSchema>(&height)?
                    .unwrap_or_default();
                blocks_at_height.push(*blockid);

                txn.put::<L1BlockSchema>(blockid, &mf)?;
                txn.put::<TxnSchema>(blockid, mf.txs_vec())?;
                txn.put::<L1BlocksByHeightSchema>(&height, &blocks_at_height)?;

                Ok::<(), DbError>(())
            })
            .map_err(|e: rockbound::TransactionError<_>| DbError::TransactionError(e.to_string()))
    }

    fn set_canonical_chain_entry(&self, height: u64, blockid: L1BlockId) -> DbResult<()> {
        self.db.put::<L1CanonicalBlockSchema>(&height, &blockid)?;
        Ok(())
    }

    fn remove_canonical_chain_entries(&self, start_height: u64, end_height: u64) -> DbResult<()> {
        let mut batch = SchemaBatch::new();
        for height in (start_height..=end_height).rev() {
            batch.delete::<L1CanonicalBlockSchema>(&height)?;
        }

        // Execute the batch
        self.db.write_schemas(batch)?;
        Ok(())
    }

    fn prune_to_height(&self, end_height: u64) -> DbResult<()> {
        let earliest =
            get_first::<L1BlocksByHeightSchema>(self.db.as_ref())?.map(|(height, _)| height);
        let Some(start_height) = earliest else {
            // empty db
            return Ok(());
        };

        for height in start_height..=end_height {
            self.db
                .with_optimistic_txn(self.ops.txn_retry_count(), |txn| {
                    let blocks = txn.get_for_update::<L1BlocksByHeightSchema>(&height)?;

                    txn.delete::<L1BlocksByHeightSchema>(&height)?;
                    txn.delete::<L1CanonicalBlockSchema>(&height)?;
                    for blockid in blocks.unwrap_or_default() {
                        txn.delete::<L1BlockSchema>(&blockid)?;
                        txn.delete::<TxnSchema>(&blockid)?;
                    }

                    Ok::<(), DbError>(())
                })
                .map_err(|e: rockbound::TransactionError<_>| {
                    DbError::TransactionError(e.to_string())
                })?;
        }
        Ok(())
    }

    fn get_tx(&self, tx_ref: L1TxRef) -> DbResult<Option<L1Tx>> {
        let (blockid, txindex) = tx_ref.into();
        let tx = self
            .db
            .get::<L1BlockSchema>(&blockid)
            .and_then(|mf_opt| match mf_opt {
                Some(mf) => {
                    let txs_opt = self.db.get::<TxnSchema>(mf.blkid())?;
                    // we only save subset of transaction in a block, while the txindex refers to
                    // original position in txblock.
                    // TODO: txs should be hashmap with original index
                    let tx = txs_opt.and_then(|txs| {
                        txs.iter()
                            .find(|tx| tx.proof().position() == txindex)
                            .cloned()
                    });
                    Ok(tx)
                }
                None => Ok(None),
            });
        Ok(tx?)
    }

    fn get_canonical_chain_tip(&self) -> DbResult<Option<(u64, L1BlockId)>> {
        self.get_latest_block()
    }

    fn get_block_txs(&self, blockid: L1BlockId) -> DbResult<Option<Vec<L1TxRef>>> {
        let Some(txs) = self.db.get::<TxnSchema>(&blockid)? else {
            warn!(%blockid, "missing L1 block body");
            return Err(DbError::MissingL1BlockManifest(blockid));
        };

        let txs_refs = txs
            .into_iter()
            .map(|tx| L1TxRef::from((blockid, tx.proof().position())))
            .collect::<Vec<L1TxRef>>();

        Ok(Some(txs_refs))
    }

    // TODO: This should not exist in database level and should be handled by downstream manager
    fn get_canonical_blockid_range(
        &self,
        start_idx: u64,
        end_idx: u64,
    ) -> DbResult<Vec<L1BlockId>> {
        let mut options = ReadOptions::default();
        options.set_iterate_lower_bound(
            KeyEncoder::<L1CanonicalBlockSchema>::encode_key(&start_idx)
                .map_err(|err| DbError::CodecError(err.to_string()))?,
        );
        options.set_iterate_upper_bound(
            KeyEncoder::<L1CanonicalBlockSchema>::encode_key(&end_idx)
                .map_err(|err| DbError::CodecError(err.to_string()))?,
        );

        let res = self
            .db
            .iter_with_opts::<L1CanonicalBlockSchema>(options)?
            .map(|item_result| item_result.map(|item| item.into_tuple().1))
            .collect::<Result<Vec<L1BlockId>, anyhow::Error>>()?;

        Ok(res)
    }

    fn get_canonical_blockid_at_height(&self, height: u64) -> DbResult<Option<L1BlockId>> {
        Ok(self.db.get::<L1CanonicalBlockSchema>(&height)?)
    }

    fn get_block_manifest(&self, blockid: L1BlockId) -> DbResult<Option<L1BlockManifest>> {
        Ok(self.db.get::<L1BlockSchema>(&blockid)?)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::l1_db_tests;

    use super::*;
    use crate::test_utils::get_rocksdb_tmp_instance;

    fn setup_db() -> L1Db {
        let (db, db_ops) = get_rocksdb_tmp_instance().unwrap();
        L1Db::new(db, db_ops)
    }

    l1_db_tests!(setup_db());
}
