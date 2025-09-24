use strata_asm_types::{L1BlockManifest, L1Tx, L1TxRef};
use strata_db::{DbResult, errors::DbError, traits::*};
use strata_primitives::l1::L1BlockId;
use typed_sled::batch::SledBatch;

use super::schemas::{L1BlockSchema, L1BlocksByHeightSchema, L1CanonicalBlockSchema, TxnSchema};
use crate::{
    define_sled_database,
    utils::{first, to_db_error},
};

define_sled_database!(
    pub struct L1DBSled {
        l1_blk_tree: L1BlockSchema,
        l1_canonical_tree: L1CanonicalBlockSchema,
        l1_blks_height_tree: L1BlocksByHeightSchema,
        txn_tree: TxnSchema,
    }
);

impl L1DBSled {
    pub fn get_latest_block(&self) -> DbResult<Option<(u64, L1BlockId)>> {
        Ok(self.l1_canonical_tree.last()?)
    }
}

impl L1Database for L1DBSled {
    fn put_block_data(&self, mf: L1BlockManifest) -> DbResult<()> {
        let blockid = mf.blkid();
        let height = mf.height();

        self.config
            .with_retry(
                (&self.l1_blk_tree, &self.txn_tree, &self.l1_blks_height_tree),
                |(bt, tt, bht)| {
                    let mut blocks_at_height = bht.get(&height)?.unwrap_or_default();
                    blocks_at_height.push(*blockid);

                    bt.insert(blockid, &mf)?;
                    tt.insert(blockid, mf.txs_vec())?;
                    bht.insert(&height, &blocks_at_height)?;

                    Ok(())
                },
            )
            .map_err(to_db_error)
    }

    fn set_canonical_chain_entry(&self, height: u64, blockid: L1BlockId) -> DbResult<()> {
        Ok(self.l1_canonical_tree.insert(&height, &blockid)?)
    }

    fn remove_canonical_chain_entries(&self, start_height: u64, end_height: u64) -> DbResult<()> {
        let mut batch = SledBatch::<L1CanonicalBlockSchema>::new();
        for height in (start_height..=end_height).rev() {
            batch.remove(height)?;
        }
        // Execute the batch
        self.l1_canonical_tree.apply_batch(batch)?;
        Ok(())
    }

    fn prune_to_height(&self, end_height: u64) -> DbResult<()> {
        let earliest = self.l1_blks_height_tree.first()?.map(first);
        let Some(start_height) = earliest else {
            // empty db
            return Ok(());
        };

        self.config
            .with_retry(
                (
                    &self.l1_blk_tree,
                    &self.txn_tree,
                    &self.l1_blks_height_tree,
                    &self.l1_canonical_tree,
                ),
                |(bt, tt, bht, ct)| {
                    for height in start_height..=end_height {
                        let blocks = bht.get(&height)?.unwrap_or_default();

                        bht.remove(&height)?;
                        ct.remove(&height)?;
                        for blockid in blocks {
                            bt.remove(&blockid)?;
                            tt.remove(&blockid)?;
                        }
                    }

                    Ok(())
                },
            )
            .map_err(to_db_error)?;
        Ok(())
    }

    fn get_tx(&self, tx_ref: L1TxRef) -> DbResult<Option<L1Tx>> {
        let (blockid, txindex) = tx_ref.into();
        let txs = self
            .l1_blk_tree
            .get(&blockid)?
            .and_then(|mf| self.txn_tree.get(mf.blkid()).transpose())
            .transpose()?;

        // we only save subset of transaction in a block, while the txindex refers to
        // original position in txblock.
        // TODO: txs should be hashmap with original index
        Ok(txs.and_then(|txs| txs.into_iter().find(|tx| tx.proof().position() == txindex)))
    }

    fn get_canonical_chain_tip(&self) -> DbResult<Option<(u64, L1BlockId)>> {
        self.get_latest_block()
    }

    fn get_block_txs(&self, blockid: L1BlockId) -> DbResult<Option<Vec<L1TxRef>>> {
        let Some(txs) = self.txn_tree.get(&blockid)? else {
            return Err(DbError::MissingL1BlockManifest(blockid));
        };
        let txrefs = txs
            .into_iter()
            .map(|tx| L1TxRef::from((blockid, tx.proof().position())))
            .collect::<Vec<L1TxRef>>();

        Ok(Some(txrefs))
    }

    fn get_canonical_blockid_range(
        &self,
        start_idx: u64,
        end_idx: u64,
    ) -> DbResult<Vec<L1BlockId>> {
        let mut result = Vec::new();
        for height in start_idx..end_idx {
            if let Some(blockid) = self.l1_canonical_tree.get(&height)? {
                result.push(blockid);
            }
        }
        Ok(result)
    }

    fn get_canonical_blockid_at_height(&self, height: u64) -> DbResult<Option<L1BlockId>> {
        Ok(self.l1_canonical_tree.get(&height)?)
    }

    fn get_block_manifest(&self, blockid: L1BlockId) -> DbResult<Option<L1BlockManifest>> {
        Ok(self.l1_blk_tree.get(&blockid)?)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::l1_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(L1DBSled, l1_db_tests);
}
