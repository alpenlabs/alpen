use strata_db_types::{
    DbResult,
    mmr_helpers::{
        get_peaks, leaf_index_to_mmr_size, leaf_index_to_pos, pos_height_in_tree, sibling_pos,
    },
    traits::*,
};
use strata_merkle::{CompactMmr64B32 as CompactMmr64, Sha256Hasher, hasher::MerkleHasher};
use strata_primitives::{buf::Buf32, l1::L1BlockCommitment};
use strata_state::asm_state::AsmState;

use super::schemas::{
    AsmLogSchema, AsmManifestHashSchema, AsmMmrMetaSchema, AsmMmrNodeSchema, AsmStateSchema,
    MmrMetadata,
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
            let metadata = MmrMetadata {
                num_leaves: 0,
                mmr_size: 0,
                peak_roots: Vec::new(),
            };
            self.mmr_meta_tree.insert(&(), &metadata)?;
        }
        Ok(())
    }

    /// Load metadata from database
    fn load_mmr_metadata(&self) -> DbResult<MmrMetadata> {
        self.mmr_meta_tree
            .get(&())?
            .ok_or_else(|| strata_db_types::DbError::Other("MMR metadata not found".to_string()))
    }

    /// Hash two nodes together to create parent hash
    fn hash_mmr_nodes(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        Sha256Hasher::hash_node(*left, *right)
    }

    /// Get a node hash by position from typed sled tree
    fn get_mmr_node(&self, pos: u64) -> DbResult<[u8; 32]> {
        self.mmr_node_tree
            .get(&pos)?
            .map(|buf| buf.0)
            .ok_or_else(|| {
                strata_db_types::DbError::Other(format!("MMR node not found at position {}", pos))
            })
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

    fn store_manifest_hash(&self, index: u64, hash: [u8; 32]) -> DbResult<()> {
        self.manifest_hash_tree.insert(&index, &Buf32(hash))?;
        Ok(())
    }

    fn get_manifest_hash(&self, index: u64) -> DbResult<Option<[u8; 32]>> {
        Ok(self.manifest_hash_tree.get(&index)?.map(|buf| buf.0))
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
/// Note: strata-merkle stores peaks in reverse order (right-to-left / by increasing height)
/// while `get_peaks()` returns them in left-to-right position order. This implementation
/// reverses the peaks to match strata-merkle's expected ordering.
impl MmrDatabase for AsmDBSled {
    fn append_leaf(&self, hash: [u8; 32]) -> DbResult<u64> {
        // Ensure metadata exists
        self.ensure_mmr_metadata()?;

        self.config.with_retry(
            (&self.mmr_node_tree, &self.mmr_meta_tree),
            |(node_tree, meta_tree)| {
                let metadata = meta_tree.get(&())?;
                let mut metadata =
                    metadata.expect("MMR metadata must exist after ensure_mmr_metadata");

                let leaf_index = metadata.num_leaves;
                let leaf_pos = leaf_index_to_pos(leaf_index);

                // Store the leaf
                node_tree.insert(&leaf_pos, &Buf32(hash))?;

                // Merge along the path to create internal nodes
                let mut current_pos = leaf_pos;
                let mut current_hash = hash;
                let mut current_height = 0u8;

                // Keep merging as long as we have a left sibling
                loop {
                    // Calculate what the next position would be
                    let next_pos = current_pos + 1;
                    let next_height = pos_height_in_tree(next_pos);

                    // If next position is higher, current is a right sibling - we should merge
                    if next_height > current_height {
                        // Current is right sibling, get left sibling
                        let sibling_position = sibling_pos(current_pos, current_height);
                        let sibling_hash = node_tree
                            .get(&sibling_position)?
                            .expect("Sibling node must exist")
                            .0;

                        // Create parent hash (left sibling, right sibling)
                        let parent_hash = Self::hash_mmr_nodes(&sibling_hash, &current_hash);

                        // Store parent
                        node_tree.insert(&next_pos, &Buf32(parent_hash))?;

                        // Move up to parent
                        current_pos = next_pos;
                        current_hash = parent_hash;
                        current_height = next_height;
                    } else {
                        // Current is a left sibling (will be merged when right sibling comes)
                        // or we've reached a peak - stop here
                        break;
                    }
                }

                // Update metadata
                metadata.num_leaves += 1;
                let leaves_count = metadata.num_leaves;
                let peak_count = leaves_count.count_ones() as u64;
                metadata.mmr_size = 2 * leaves_count - peak_count;

                // Calculate and update peak_roots
                let peak_positions = get_peaks(metadata.mmr_size);
                let peaks: Vec<Buf32> = peak_positions
                    .iter()
                    .map(|&pos| {
                        node_tree
                            .get(&pos)
                            .map(|opt| opt.expect("Peak node must exist"))
                    })
                    .collect::<Result<Vec<Buf32>, _>>()?;
                metadata.peak_roots = peaks.into_iter().rev().collect();

                // Save metadata to database
                meta_tree.insert(&(), &metadata)?;

                Ok(leaf_index)
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
        // Directly construct CompactMmr64 from our stored peak roots
        use ssz_types::FixedBytes;

        let metadata = self.load_mmr_metadata().unwrap_or_else(|_| MmrMetadata {
            num_leaves: 0,
            mmr_size: 0,
            peak_roots: Vec::new(),
        });

        // Convert our Hash to FixedBytes<32> for SSZ
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
                let metadata = meta_tree.get(&())?;
                let mut metadata =
                    metadata.expect("MMR metadata must exist after ensure_mmr_metadata");

                // Can't pop from empty MMR
                if metadata.num_leaves == 0 {
                    return Ok(None);
                }

                // Get the hash of the leaf we're about to pop (before deletion)
                let leaf_index = metadata.num_leaves - 1;
                let leaf_pos = leaf_index_to_pos(leaf_index);
                let leaf_hash = node_tree.get(&leaf_pos)?.expect("Leaf node must exist").0;

                // Calculate the old MMR size (before the last leaf was added)
                let old_mmr_size = if metadata.num_leaves == 1 {
                    0 // Empty MMR
                } else {
                    leaf_index_to_mmr_size(metadata.num_leaves - 2)
                };

                // Delete all nodes created when the last leaf was added
                // These are nodes with positions: [old_mmr_size, current_mmr_size)
                for pos in old_mmr_size..metadata.mmr_size {
                    node_tree.remove(&pos)?;
                }

                // Update metadata
                metadata.num_leaves -= 1;
                metadata.mmr_size = old_mmr_size;

                // Calculate peak_roots for the new size
                metadata.peak_roots = if old_mmr_size > 0 {
                    let peak_positions = get_peaks(old_mmr_size);
                    let peaks: Vec<Buf32> = peak_positions
                        .iter()
                        .map(|&pos| {
                            node_tree
                                .get(&pos)
                                .map(|opt| opt.expect("Peak node must exist"))
                        })
                        .collect::<Result<Vec<Buf32>, _>>()?;
                    peaks.into_iter().rev().collect()
                } else {
                    Vec::new()
                };

                // Save metadata to database
                meta_tree.insert(&(), &metadata)?;

                Ok(Some(leaf_hash))
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
