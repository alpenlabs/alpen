use strata_db_types::{
    DbError, DbResult,
    mmr_helpers::{MmrAlgorithm, MmrMetadata},
    traits::{AsmDatabase, MmrDatabase},
};
use strata_merkle::CompactMmr64B32 as CompactMmr64;
use strata_primitives::{buf::Buf32, l1::L1BlockCommitment};
use strata_state::asm_state::AsmState;

use super::schemas::{
    AsmLogSchema, AsmManifestHashSchema, AsmMmrMetaSchema, AsmMmrNodeSchema, AsmStateSchema,
};
use crate::define_sled_database;

define_sled_database!(
    pub struct AsmDBSled {
        asm_state_tree: AsmStateSchema,
        // TODO(refactor) - it should operate on manifests instead of logs.
        asm_log_tree: AsmLogSchema,
        // TODO: MMR should be a separate sled db, should not belong here.
        mmr_node_tree: AsmMmrNodeSchema,
        mmr_meta_tree: AsmMmrMetaSchema,
        manifest_hash_tree: AsmManifestHashSchema,
    }
);

impl AsmDBSled {
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

    /// Get a node hash by position from typed sled tree
    fn get_mmr_node(&self, pos: u64) -> DbResult<[u8; 32]> {
        self.mmr_node_tree
            .get(&pos)?
            .map(|buf| buf.0)
            .ok_or(DbError::MmrLeafNotFound(pos))
    }
}

impl AsmDatabase for AsmDBSled {
    fn put_asm_state(&self, block: L1BlockCommitment, state: AsmState) -> DbResult<()> {
        self.config.with_retry(
            (&self.asm_state_tree, &self.asm_log_tree),
            |(state_tree, log_tree)| {
                state_tree.insert(&block, state.state())?;
                log_tree.insert(&block, state.logs())?;

                Ok(())
            },
        )?;

        Ok(())
    }

    fn get_asm_state(&self, block: L1BlockCommitment) -> DbResult<Option<AsmState>> {
        self.config.with_retry(
            (&self.asm_state_tree, &self.asm_log_tree),
            |(state_tree, log_tree)| {
                let state = state_tree.get(&block)?;
                let logs = log_tree.get(&block)?;

                Ok(state.and_then(|s| logs.map(|l| AsmState::new(s, l))))
            },
        )
    }

    fn get_latest_asm_state(&self) -> DbResult<Option<(L1BlockCommitment, AsmState)>> {
        // Relying on the lexicographical order of L1BlockCommitment.
        let state = self.asm_state_tree.last()?;
        let logs = self.asm_log_tree.last()?;

        // Assert that the block for the state and for the logs is the same.
        // It should be because we are putting it within transaction.
        Ok(state.and_then(|s| {
            logs.map(|l| {
                assert_eq!(s.0, l.0);
                (s.0, AsmState::new(s.1, l.1))
            })
        }))
    }

    fn get_asm_states_from(
        &self,
        from_block: L1BlockCommitment,
        max_count: usize,
    ) -> DbResult<Vec<(L1BlockCommitment, AsmState)>> {
        let mut result = Vec::new();

        // Use non-transactional iteration since we only need reads
        // Iterate through all blocks and filter those >= from_block
        for item in self.asm_state_tree.iter() {
            let (block, state) = item?;

            // Skip blocks before the starting block
            if block < from_block {
                continue;
            }

            // Get corresponding logs (also non-transactional read)
            if let Some(logs) = self.asm_log_tree.get(&block)? {
                result.push((block, AsmState::new(state, logs)));

                if result.len() >= max_count {
                    break;
                }
            }
        }

        Ok(result)
    }

    fn store_manifest_hash(&self, index: u64, hash: Buf32) -> DbResult<()> {
        self.manifest_hash_tree.insert(&index, &hash)?;
        Ok(())
    }

    fn get_manifest_hash(&self, index: u64) -> DbResult<Option<Buf32>> {
        Ok(self.manifest_hash_tree.get(&index)?)
    }
}

/// MMR (Merkle Mountain Range) Database Implementation
///
/// This implementation provides a persistent MMR data structure backed by Sled database.
/// The MMR is used to efficiently store and prove inclusion of L1 block manifest hashes.
///
/// ## Structure
///
/// - **Nodes**: Stored in `mmr_node_tree` indexed by position in the MMR
/// - **Metadata**: Stored in `mmr_meta_tree` containing:
///   - `num_leaves`: Total number of leaves in the MMR
///   - `mmr_size`: Total number of nodes (leaves + internal nodes)
///   - `peak_roots`: Cached root hashes of the current peaks
///
/// ## Key Features
///
/// - **Incremental updates**: New leaves can be appended efficiently
/// - **Persistent storage**: All nodes and metadata are persisted to disk
/// - **Proof generation**: Supports generating Merkle proofs for any leaf
///
/// ## Peak Ordering
///
/// Implementation of `MmrDatabase` for ASM manifest MMR
///
/// This provides a singleton MMR (single global instance) for storing
/// ASM manifest hashes.
///
/// Note: strata-merkle stores peaks in reverse order (right-to-left / by increasing height)
/// while `get_peaks()` returns them in left-to-right position order. This implementation
/// reverses the peaks to match strata-merkle's expected ordering.
impl MmrDatabase for AsmDBSled {
    fn append_leaf(&self, hash: [u8; 32]) -> DbResult<u64> {
        self.ensure_mmr_metadata()?;

        self.config.with_retry(
            (&self.mmr_node_tree, &self.mmr_meta_tree),
            |(node_tree, meta_tree)| {
                let metadata = meta_tree
                    .get(&())?
                    .expect("MMR metadata must exist after ensure_mmr_metadata");

                let result = MmrAlgorithm::append_leaf(hash, &metadata, |pos| {
                    node_tree
                        .get(&pos)?
                        .map(|buf| buf.0)
                        .ok_or(DbError::MmrLeafNotFound(pos))
                })
                .map_err(typed_sled::error::Error::abort)?;

                for (pos, node_hash) in result.nodes_to_write {
                    node_tree.insert(&pos, &Buf32(node_hash))?;
                }

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
        use ssz_types::FixedBytes;

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

                let result = MmrAlgorithm::pop_leaf(&metadata, |pos| {
                    node_tree
                        .get(&pos)?
                        .map(|buf| buf.0)
                        .ok_or(DbError::MmrLeafNotFound(pos))
                })
                .map_err(typed_sled::error::Error::abort)?;

                let Some(result) = result else {
                    return Ok(None);
                };

                for pos in result.nodes_to_remove {
                    node_tree.remove(&pos)?;
                }

                meta_tree.insert(&(), &result.new_metadata)?;

                Ok(Some(result.leaf_hash))
            },
        )
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::asm_state_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(AsmDBSled, asm_state_db_tests);
}
