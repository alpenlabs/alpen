use ssz_types::FixedBytes;
use strata_db_types::{
    DbError, DbResult,
    mmr_helpers::{MmrAlgorithm, MmrMetadata},
    traits::AccountMmrDatabase,
};
use strata_identifiers::AccountId;
use strata_merkle::CompactMmr64B32 as CompactMmr64;
use strata_primitives::buf::Buf32;

use super::schemas::{SnarkMsgMmrMetaSchema, SnarkMsgMmrNodeSchema};
use crate::define_sled_database;

define_sled_database!(
    pub struct SnarkMsgMmrDb {
        node_tree: SnarkMsgMmrNodeSchema,
        meta_tree: SnarkMsgMmrMetaSchema,
    }
);

impl SnarkMsgMmrDb {
    fn ensure_mmr_metadata(&self, account: AccountId) -> DbResult<()> {
        if self.meta_tree.get(&account)?.is_none() {
            let metadata = MmrMetadata::empty();
            self.meta_tree.insert(&account, &metadata)?;
        }
        Ok(())
    }

    fn load_mmr_metadata(&self, account: AccountId) -> DbResult<MmrMetadata> {
        self.meta_tree.get(&account)?.ok_or_else(|| {
            DbError::Other(format!("MMR metadata not found for account {}", account))
        })
    }

    fn get_mmr_node(&self, account: AccountId, pos: u64) -> DbResult<[u8; 32]> {
        self.node_tree
            .get(&(account, pos))?
            .map(|buf| buf.0)
            .ok_or(DbError::MmrLeafNotFoundForAccount(pos, account))
    }
}

impl AccountMmrDatabase for SnarkMsgMmrDb {
    fn append_leaf(&self, account: AccountId, hash: [u8; 32]) -> DbResult<u64> {
        self.ensure_mmr_metadata(account)?;

        self.config
            .with_retry((&self.node_tree, &self.meta_tree), |(nt, mt)| {
                let metadata = mt
                    .get(&account)?
                    .expect("MMR metadata must exist after ensure_mmr_metadata");

                let result = MmrAlgorithm::append_leaf(hash, &metadata, |pos| {
                    nt.get(&(account, pos))?
                        .map(|buf| buf.0)
                        .ok_or(DbError::MmrLeafNotFoundForAccount(pos, account))
                })
                .map_err(typed_sled::error::Error::abort)?;

                for (pos, node_hash) in result.nodes_to_write {
                    nt.insert(&(account, pos), &Buf32(node_hash))?;
                }

                mt.insert(&account, &result.new_metadata)?;

                Ok(result.leaf_index)
            })
    }

    fn get_node(&self, account: AccountId, pos: u64) -> DbResult<[u8; 32]> {
        self.get_mmr_node(account, pos)
    }

    fn mmr_size(&self, account: AccountId) -> DbResult<u64> {
        self.ensure_mmr_metadata(account)?;
        let metadata = self.load_mmr_metadata(account)?;
        Ok(metadata.mmr_size)
    }

    fn num_leaves(&self, account: AccountId) -> DbResult<u64> {
        self.ensure_mmr_metadata(account)?;
        let metadata = self.load_mmr_metadata(account)?;
        Ok(metadata.num_leaves)
    }

    fn peak_roots(&self, account: AccountId) -> Vec<[u8; 32]> {
        self.load_mmr_metadata(account)
            .map(|m| m.peak_roots.into_iter().map(|b| b.0).collect())
            .unwrap_or_default()
    }

    fn to_compact(&self, account: AccountId) -> CompactMmr64 {
        let metadata = self
            .load_mmr_metadata(account)
            .unwrap_or_else(|_| MmrMetadata::empty());

        let roots_vec: Vec<_> = metadata
            .peak_roots
            .iter()
            .map(|buf| FixedBytes::<32>::from(buf.0))
            .collect();

        CompactMmr64 {
            entries: metadata.num_leaves,
            cap_log2: 64,
            roots: roots_vec.into(),
        }
    }

    fn pop_leaf(&self, account: AccountId) -> DbResult<Option<[u8; 32]>> {
        self.ensure_mmr_metadata(account)?;

        self.config
            .with_retry((&self.node_tree, &self.meta_tree), |(nt, mt)| {
                let metadata = mt
                    .get(&account)?
                    .expect("MMR metadata must exist after ensure_mmr_metadata");

                let result = MmrAlgorithm::pop_leaf(&metadata, |pos| {
                    nt.get(&(account, pos))?
                        .map(|buf| buf.0)
                        .ok_or(DbError::MmrLeafNotFoundForAccount(pos, account))
                })
                .map_err(typed_sled::error::Error::abort)?;

                let Some(result) = result else {
                    return Ok(None);
                };

                for pos in result.nodes_to_remove {
                    nt.remove(&(account, pos))?;
                }

                mt.insert(&account, &result.new_metadata)?;

                Ok(Some(result.leaf_hash))
            })
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::mmr_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(SnarkMsgMmrDb, mmr_db_tests);
}
