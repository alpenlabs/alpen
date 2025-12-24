//! MMR position calculation helpers and algorithm implementation
//!
//! This module provides:
//! - Bit manipulation utilities for navigating the MMR structure
//! - Pure MMR algorithm logic (computation without storage)
//!
//! # Note on Future Migration
//!
//! These helpers are pure MMR navigation utilities that don't depend on any
//! Alpen-specific logic. They are candidates for upstreaming to the `strata-merkle`
//! crate in the `strata-common` repository, where they would benefit the entire
//! ecosystem. For now, they live here in `strata-db-types` to avoid duplication
//! between the database layer and storage manager layer.
//!
//! # MMR Structure
//!
//! MMR (Merkle Mountain Range) uses post-order traversal numbering:
//!
//! ```text
//!       7
//!      /  \
//!     3    6
//!    / \  / \
//!   1  2 4  5
//!
//! Position: [0, 1, 2, 3, 4, 5, 6, 7]
//! Height:   [0, 0, 1, 0, 0, 1, 2, 0]
//! Leaves:   [0, 1, x, 2, 3, x, x, 4]  (x = internal node)
//! ```

use borsh::{BorshDeserialize, BorshSerialize};
use strata_merkle::{hasher::MerkleHasher, Sha256Hasher};
use strata_primitives::buf::Buf32;

use crate::{DbError, DbResult};

/// Convert leaf index to total MMR size (number of nodes)
///
/// Formula: 2 * leaves - peak_count
/// Peak count = number of set bits in binary representation of leaf count
///
/// # Examples
///
/// ```text
/// 1 leaf  (0b1)   -> 1 peak  -> size = 2*1 - 1 = 1
/// 2 leaves (0b10)  -> 1 peak  -> size = 2*2 - 1 = 3
/// 3 leaves (0b11)  -> 2 peaks -> size = 2*3 - 2 = 4
/// 7 leaves (0b111) -> 3 peaks -> size = 2*7 - 3 = 11
/// ```
pub fn leaf_index_to_mmr_size(index: u64) -> u64 {
    let leaves_count = index + 1;
    let peak_count = leaves_count.count_ones() as u64;
    2 * leaves_count - peak_count
}

/// Convert leaf index to MMR position
///
/// Uses bit manipulation to find the position in post-order traversal.
///
/// # Examples
///
/// ```text
/// leaf_index_to_pos(0) = 0  // First leaf
/// leaf_index_to_pos(1) = 1  // Second leaf
/// leaf_index_to_pos(2) = 3  // Third leaf (skip internal node at 2)
/// leaf_index_to_pos(3) = 4  // Fourth leaf
/// ```
pub fn leaf_index_to_pos(index: u64) -> u64 {
    leaf_index_to_mmr_size(index) - (index + 1).trailing_zeros() as u64 - 1
}

/// Calculate the height of a node at given position
///
/// Uses leading zeros to find the peak size, then performs binary subtraction
/// to find the height within that peak.
///
/// # Examples
///
/// ```text
/// pos_height_in_tree(0) = 0  // Leaf
/// pos_height_in_tree(1) = 0  // Leaf
/// pos_height_in_tree(2) = 1  // Internal node (parent of 0,1)
/// pos_height_in_tree(3) = 0  // Leaf
/// ```
pub fn pos_height_in_tree(mut pos: u64) -> u8 {
    if pos == 0 {
        return 0;
    }

    // Find the largest peak that fits before this position
    let mut peak_size = u64::MAX >> pos.leading_zeros();

    // Subtract peaks until we find which one contains this position
    while peak_size > 0 {
        if pos >= peak_size {
            pos -= peak_size;
        }
        peak_size >>= 1;
    }

    pos as u8
}

/// Calculate the offset to a node's parent
///
/// For a node at height h, parent is 2^(h+1) positions away
#[inline]
pub fn parent_offset(height: u8) -> u64 {
    2 << height
}

/// Calculate the offset to a node's sibling
///
/// For a node at height h, sibling is 2^(h+1) - 1 positions away
#[inline]
pub fn sibling_offset(height: u8) -> u64 {
    (2 << height) - 1
}

/// Get the position of a node's parent
///
/// # Arguments
///
/// * `pos` - Current node position
/// * `height` - Current node height
///
/// # Returns
///
/// Position of the parent node
pub fn parent_pos(pos: u64, height: u8) -> u64 {
    let next_height = pos_height_in_tree(pos + 1);
    if next_height > height {
        // Current node is a right sibling
        pos + 1
    } else {
        // Current node is a left sibling
        pos + parent_offset(height)
    }
}

/// Get the position of a node's sibling
///
/// # Arguments
///
/// * `pos` - Current node position
/// * `height` - Current node height
///
/// # Returns
///
/// Position of the sibling node
pub fn sibling_pos(pos: u64, height: u8) -> u64 {
    let next_height = pos_height_in_tree(pos + 1);
    if next_height > height {
        // Current node is a right sibling
        pos - sibling_offset(height)
    } else {
        // Current node is a left sibling
        pos + sibling_offset(height)
    }
}

/// Get all peak positions for a given MMR size
///
/// Peaks are the roots of the sub-trees in the MMR forest.
///
/// # Examples
///
/// ```text
/// MMR with 7 nodes (4 leaves):
///       6
///      /  \
///     2    5
///    / \  / \
///   0  1 3  4
///
/// Peaks: [6]  (single peak at height 2)
/// ```
pub fn get_peaks(mmr_size: u64) -> Vec<u64> {
    if mmr_size == 0 {
        return vec![];
    }

    let mut peaks = Vec::new();
    let mut pos = 0u64;
    let mut remaining = mmr_size;

    // Find each peak by subtracting the largest complete tree that fits
    while remaining > 0 {
        // Find the largest complete binary tree that fits in remaining nodes
        // A complete tree of height h has 2^(h+1) - 1 nodes
        // We need to find the largest h where 2^(h+1) - 1 <= remaining

        // Start with the highest bit of remaining as an estimate
        let mut height = 63 - remaining.leading_zeros();

        // Calculate the tree size for this height
        let mut tree_size = (1u64 << (height + 1)) - 1;

        // If tree is too big, reduce height
        while tree_size > remaining {
            height -= 1;
            tree_size = (1u64 << (height + 1)) - 1;
        }

        // The peak is at position pos + tree_size - 1 (0-indexed)
        let peak_pos = pos + tree_size - 1;
        peaks.push(peak_pos);

        // Move to next tree
        pos += tree_size;
        remaining -= tree_size;
    }

    peaks
}

/// Find which peak a given position belongs to
///
/// Returns the position of the peak that contains the given position.
///
/// # Errors
///
/// Returns `DbError::Other` if the position is beyond mmr_size
pub fn find_peak_for_pos(pos: u64, mmr_size: u64) -> DbResult<u64> {
    let peaks = get_peaks(mmr_size);

    for &peak_pos in &peaks {
        if pos <= peak_pos {
            return Ok(peak_pos);
        }
    }

    Err(DbError::Other(format!(
        "Position {} not found in MMR of size {}",
        pos, mmr_size
    )))
}

/// Metadata for an MMR instance
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MmrMetadata {
    pub num_leaves: u64,
    pub mmr_size: u64,
    pub peak_roots: Vec<Buf32>,
}

impl MmrMetadata {
    pub fn empty() -> Self {
        Self {
            num_leaves: 0,
            mmr_size: 0,
            peak_roots: Vec::new(),
        }
    }
}

/// Result of appending a leaf to the MMR
#[derive(Debug)]
pub struct AppendLeafResult {
    pub leaf_index: u64,
    pub nodes_to_write: Vec<(u64, [u8; 32])>,
    pub new_metadata: MmrMetadata,
}

/// Result of popping a leaf from the MMR
#[derive(Debug)]
pub struct PopLeafResult {
    pub leaf_hash: [u8; 32],
    pub nodes_to_remove: Vec<u64>,
    pub new_metadata: MmrMetadata,
}

/// Pure MMR algorithm logic separated from storage
///
/// This struct provides the core MMR algorithms (append, pop) as pure functions
/// that compute what changes need to be made without actually making them.
/// Storage implementations apply these changes within their own transaction context.
#[derive(Debug)]
pub struct MmrAlgorithm;

impl MmrAlgorithm {
    /// Compute the result of appending a leaf to the MMR
    ///
    /// # Arguments
    ///
    /// * `hash` - The hash value to append as a new leaf
    /// * `metadata` - Current MMR metadata (num_leaves, mmr_size, peak_roots)
    /// * `get_node` - Closure to read existing nodes (for sibling lookups during merge)
    ///
    /// # Returns
    ///
    /// `AppendLeafResult` containing:
    /// - The leaf index of the newly appended leaf
    /// - All nodes to write (leaf + internal nodes from merging)
    /// - Updated metadata
    ///
    /// # Storage Independence
    ///
    /// This function doesn't know about transactions or storage backends.
    /// The caller provides a closure to read nodes and applies the result
    /// within their own transaction mechanism.
    pub fn append_leaf(
        hash: [u8; 32],
        metadata: &MmrMetadata,
        get_node: impl Fn(u64) -> DbResult<[u8; 32]>,
    ) -> DbResult<AppendLeafResult> {
        let leaf_index = metadata.num_leaves;
        let leaf_pos = leaf_index_to_pos(leaf_index);

        let mut nodes_to_write = Vec::new();

        // Store the leaf
        nodes_to_write.push((leaf_pos, hash));

        // Merge along the path to create internal nodes
        let mut current_pos = leaf_pos;
        let mut current_hash = hash;
        let mut current_height = 0u8;

        // Keep merging as long as we have a left sibling
        loop {
            let next_pos = current_pos + 1;
            let next_height = pos_height_in_tree(next_pos);

            // If next position is higher, current is a right sibling - we should merge
            if next_height > current_height {
                // Current is right sibling, get left sibling
                let sibling_position = sibling_pos(current_pos, current_height);
                let sibling_hash = get_node(sibling_position)?;

                // Create parent hash (left sibling, right sibling)
                let parent_hash = Sha256Hasher::hash_node(sibling_hash, current_hash);

                // Store parent
                nodes_to_write.push((next_pos, parent_hash));

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
        let new_num_leaves = metadata.num_leaves + 1;
        let peak_count = new_num_leaves.count_ones() as u64;
        let new_mmr_size = 2 * new_num_leaves - peak_count;

        // Calculate peak_roots
        let peak_positions = get_peaks(new_mmr_size);
        let new_peak_roots: Vec<Buf32> = peak_positions
            .iter()
            .rev()
            .map(|&pos| {
                // Check if this node is in our write list
                if let Some((_, hash)) = nodes_to_write.iter().find(|(p, _)| *p == pos) {
                    Ok(Buf32(*hash))
                } else {
                    get_node(pos).map(Buf32)
                }
            })
            .collect::<DbResult<Vec<_>>>()?;

        let new_metadata = MmrMetadata {
            num_leaves: new_num_leaves,
            mmr_size: new_mmr_size,
            peak_roots: new_peak_roots,
        };

        Ok(AppendLeafResult {
            leaf_index,
            nodes_to_write,
            new_metadata,
        })
    }

    /// Compute the result of popping the last leaf from the MMR
    ///
    /// # Arguments
    ///
    /// * `metadata` - Current MMR metadata
    /// * `get_node` - Closure to read existing nodes (for leaf hash and new peaks)
    ///
    /// # Returns
    ///
    /// `Some(PopLeafResult)` if there was a leaf to pop, containing:
    /// - The hash of the removed leaf
    /// - All node positions to remove
    /// - Updated metadata
    ///
    /// `None` if the MMR is empty
    pub fn pop_leaf(
        metadata: &MmrMetadata,
        get_node: impl Fn(u64) -> DbResult<[u8; 32]>,
    ) -> DbResult<Option<PopLeafResult>> {
        // Can't pop from empty MMR
        if metadata.num_leaves == 0 {
            return Ok(None);
        }

        let leaf_index = metadata.num_leaves - 1;
        let leaf_pos = leaf_index_to_pos(leaf_index);

        // Read the leaf hash before deletion
        let leaf_hash = get_node(leaf_pos)?;

        // Calculate the old MMR size (before the last leaf was added)
        let old_mmr_size = if metadata.num_leaves == 1 {
            0 // Empty MMR
        } else {
            leaf_index_to_mmr_size(metadata.num_leaves - 2)
        };

        // Collect nodes to remove: [old_mmr_size, current_mmr_size)
        let nodes_to_remove: Vec<u64> = (old_mmr_size..metadata.mmr_size).collect();

        // Update metadata
        let new_num_leaves = metadata.num_leaves - 1;
        let new_mmr_size = old_mmr_size;

        // Calculate peak_roots for the new size
        let new_peak_roots = if old_mmr_size > 0 {
            let peak_positions = get_peaks(old_mmr_size);
            peak_positions
                .into_iter()
                .rev()
                .map(|pos| get_node(pos).map(Buf32))
                .collect::<DbResult<Vec<_>>>()?
        } else {
            Vec::new()
        };

        let new_metadata = MmrMetadata {
            num_leaves: new_num_leaves,
            mmr_size: new_mmr_size,
            peak_roots: new_peak_roots,
        };

        Ok(Some(PopLeafResult {
            leaf_hash,
            nodes_to_remove,
            new_metadata,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leaf_index_to_mmr_size() {
        assert_eq!(leaf_index_to_mmr_size(0), 1); // 1 leaf -> 1 node
        assert_eq!(leaf_index_to_mmr_size(1), 3); // 2 leaves -> 3 nodes
        assert_eq!(leaf_index_to_mmr_size(2), 4); // 3 leaves -> 4 nodes
        assert_eq!(leaf_index_to_mmr_size(3), 7); // 4 leaves -> 7 nodes
        assert_eq!(leaf_index_to_mmr_size(6), 11); // 7 leaves -> 11 nodes
    }

    #[test]
    fn test_leaf_index_to_pos() {
        // First few leaves in MMR
        assert_eq!(leaf_index_to_pos(0), 0); // First leaf
        assert_eq!(leaf_index_to_pos(1), 1); // Second leaf
        assert_eq!(leaf_index_to_pos(2), 3); // Third leaf (2 is internal)
        assert_eq!(leaf_index_to_pos(3), 4); // Fourth leaf
    }

    #[test]
    fn test_pos_height_in_tree() {
        // Leaves have height 0
        assert_eq!(pos_height_in_tree(0), 0);
        assert_eq!(pos_height_in_tree(1), 0);
        assert_eq!(pos_height_in_tree(3), 0);
        assert_eq!(pos_height_in_tree(4), 0);

        // Internal nodes have height > 0
        assert_eq!(pos_height_in_tree(2), 1); // Parent of 0,1
        assert_eq!(pos_height_in_tree(5), 1); // Parent of 3,4
        assert_eq!(pos_height_in_tree(6), 2); // Parent of 2,5
    }

    #[test]
    fn test_parent_and_sibling_offsets() {
        assert_eq!(parent_offset(0), 2); // 2^1
        assert_eq!(parent_offset(1), 4); // 2^2
        assert_eq!(parent_offset(2), 8); // 2^3

        assert_eq!(sibling_offset(0), 1); // 2^1 - 1
        assert_eq!(sibling_offset(1), 3); // 2^2 - 1
        assert_eq!(sibling_offset(2), 7); // 2^3 - 1
    }

    #[test]
    fn test_parent_pos() {
        // Left sibling (pos 0, height 0) -> parent at 2
        assert_eq!(parent_pos(0, 0), 2);

        // Right sibling (pos 1, height 0) -> parent at 2
        assert_eq!(parent_pos(1, 0), 2);

        // Left sibling (pos 3, height 0) -> parent at 5
        assert_eq!(parent_pos(3, 0), 5);
    }

    #[test]
    fn test_sibling_pos() {
        // Left sibling (pos 0) -> right sibling at 1
        assert_eq!(sibling_pos(0, 0), 1);

        // Right sibling (pos 1) -> left sibling at 0
        assert_eq!(sibling_pos(1, 0), 0);

        // Left sibling (pos 3) -> right sibling at 4
        assert_eq!(sibling_pos(3, 0), 4);
    }

    #[test]
    fn test_get_peaks() {
        // 1 node (1 leaf): [0]
        assert_eq!(get_peaks(1), vec![0]);

        // 3 nodes (2 leaves): [2]
        assert_eq!(get_peaks(3), vec![2]);

        // 4 nodes (3 leaves): [2, 3]
        assert_eq!(get_peaks(4), vec![2, 3]);

        // 7 nodes (4 leaves): [6]
        assert_eq!(get_peaks(7), vec![6]);

        // 11 nodes (7 leaves): [6, 9, 10]
        assert_eq!(get_peaks(11), vec![6, 9, 10]);
    }

    #[test]
    fn test_find_peak_for_pos() {
        // 11 nodes: peaks [6, 9, 10]
        assert_eq!(find_peak_for_pos(0, 11).unwrap(), 6); // Leaf 0 is under peak 6
        assert_eq!(find_peak_for_pos(2, 11).unwrap(), 6); // Node 2 is under peak 6
        assert_eq!(find_peak_for_pos(6, 11).unwrap(), 6); // Peak 6 itself
        assert_eq!(find_peak_for_pos(7, 11).unwrap(), 9); // Leaf 7 is under peak 9
        assert_eq!(find_peak_for_pos(10, 11).unwrap(), 10); // Peak 10 itself
    }
}
