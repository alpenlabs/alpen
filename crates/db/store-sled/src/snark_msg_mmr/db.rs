use ssz_types::FixedBytes;
use strata_db_types::{
    DbError, DbResult,
    mmr_helpers::{MmrAlgorithm, MmrMetadata},
    traits::MmrDatabase,
};
use strata_merkle::CompactMmr64B32 as CompactMmr64;
use strata_primitives::buf::Buf32;

use super::schemas::{SnarkMsgMmrMetaSchema, SnarkMsgMmrNodeSchema};
use crate::define_sled_database;

// TODO: Since the mmr db is just parametric on the node schema and meta scehma, we can possibliy
// generalize this as well. Leave it for next ticket.
define_sled_database!(
    pub struct SnarkMsgMmrDb {
        mmr_node_tree: SnarkMsgMmrNodeSchema,
        mmr_meta_tree: SnarkMsgMmrMetaSchema,
    }
);

impl SnarkMsgMmrDb {
    /// Initialize MMR metadata if not present
    fn ensure_mmr_metadata(&self) -> DbResult<()> {
        if self.mmr_meta_tree.get(&())?.is_none() {
            let metadata = MmrMetadata::empty();
            self.mmr_meta_tree.insert(&(), &metadata)?;
        }
        Ok(())
    }

    /// Load metadata from database
    fn load_mmr_metadata(&self) -> DbResult<MmrMetadata> {
        self.mmr_meta_tree
            .get(&())?
            .ok_or_else(|| DbError::Other("MMR metadata not found".to_string()))
    }

    /// Get a node hash by position
    fn get_mmr_node(&self, pos: u64) -> DbResult<[u8; 32]> {
        self.mmr_node_tree
            .get(&pos)?
            .map(|buf| buf.0)
            .ok_or_else(|| DbError::Other(format!("MMR node not found at position {}", pos)))
    }
}

impl MmrDatabase for SnarkMsgMmrDb {
    fn append_leaf(&self, hash: [u8; 32]) -> DbResult<u64> {
        self.ensure_mmr_metadata()?;

        self.config.with_retry(
            (&self.mmr_node_tree, &self.mmr_meta_tree),
            |(node_tree, meta_tree)| {
                let metadata = meta_tree
                    .get(&())?
                    .expect("MMR metadata must exist after ensure_mmr_metadata");

                // Use the algorithm to compute what to write
                // Closure reads directly from node_tree and converts errors to DbError
                let result = MmrAlgorithm::append_leaf(hash, &metadata, |pos| {
                    node_tree
                        .get(&pos)
                        .map_err(DbError::from)?
                        .map(|buf| buf.0)
                        .ok_or_else(|| {
                            DbError::Other(format!("MMR node not found at position {}", pos))
                        })
                })
                .map_err(typed_sled::error::Error::abort)?;

                // Apply the writes
                for (pos, node_hash) in result.nodes_to_write {
                    node_tree.insert(&pos, &Buf32(node_hash))?;
                }

                // Save updated metadata
                meta_tree.insert(&(), &result.new_metadata)?;

                Ok(result.leaf_index)
            },
        )
    }

    fn get_node(&self, pos: u64) -> DbResult<[u8; 32]> {
        self.get_mmr_node(pos)
    }

    fn mmr_size(&self) -> DbResult<u64> {
        self.ensure_mmr_metadata()?;
        let metadata = self.load_mmr_metadata()?;
        Ok(metadata.mmr_size)
    }

    fn num_leaves(&self) -> DbResult<u64> {
        self.ensure_mmr_metadata()?;
        let metadata = self.load_mmr_metadata()?;
        Ok(metadata.num_leaves)
    }

    fn peak_roots(&self) -> Vec<[u8; 32]> {
        self.load_mmr_metadata()
            .map(|m| m.peak_roots.into_iter().map(|b| b.0).collect())
            .unwrap_or_default()
    }

    fn to_compact(&self) -> CompactMmr64 {
        let metadata = self
            .load_mmr_metadata()
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

    fn pop_leaf(&self) -> DbResult<Option<[u8; 32]>> {
        self.ensure_mmr_metadata()?;

        self.config.with_retry(
            (&self.mmr_node_tree, &self.mmr_meta_tree),
            |(node_tree, meta_tree)| {
                let metadata = meta_tree
                    .get(&())?
                    .expect("MMR metadata must exist after ensure_mmr_metadata");

                // Use the algorithm to compute what to delete
                // Closure reads directly from node_tree and converts errors to DbError
                let result = match MmrAlgorithm::pop_leaf(&metadata, |pos| {
                    node_tree
                        .get(&pos)
                        .map_err(strata_db_types::DbError::from)?
                        .map(|buf| buf.0)
                        .ok_or_else(|| {
                            strata_db_types::DbError::Other(format!(
                                "MMR node not found at position {}",
                                pos
                            ))
                        })
                })
                .map_err(typed_sled::error::Error::abort)?
                {
                    Some(r) => r,
                    None => return Ok(None),
                };

                // Delete the nodes
                for pos in result.nodes_to_remove {
                    node_tree.remove(&pos)?;
                }

                // Save updated metadata
                meta_tree.insert(&(), &result.new_metadata)?;

                Ok(Some(result.leaf_hash))
            },
        )
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
