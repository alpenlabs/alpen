//! MMR index algorithm for the storage manager layer.
//!
//! This module is `NodePos`/`LeafPos`-native at its public boundary.

use std::collections::BTreeSet;

use strata_db_types::{errors::MmrError, DbError, DbResult, LeafPos, NodePos, NodeTable};
use strata_identifiers::Hash;
use strata_merkle::{MerkleHasher, MerkleProofB32 as MerkleProof, Sha256Hasher};

#[derive(Debug, Clone)]
pub(crate) struct AppendPlan {
    pub(crate) leaf_pos: LeafPos,
    pub(crate) nodes_to_write: Vec<(NodePos, Hash)>,
}

#[derive(Debug, Clone)]
pub(crate) struct PopPlan {
    pub(crate) leaf_pos: LeafPos,
    pub(crate) leaf_hash: Hash,
    pub(crate) nodes_to_remove: Vec<NodePos>,
}

fn compute_highest_mountain_size(leaves: u64) -> u64 {
    debug_assert!(
        leaves > 0,
        "compute_highest_mountain_size: leaves must be > 0"
    );
    1u64 << (63 - leaves.leading_zeros())
}

pub(crate) fn compute_peak_positions(leaf_count: u64) -> Vec<NodePos> {
    if leaf_count == 0 {
        return Vec::new();
    }

    let mut peaks = Vec::new();
    let mut start_leaf = 0u64;
    let mut remaining = leaf_count;

    while remaining > 0 {
        let size = compute_highest_mountain_size(remaining);
        let height = size.trailing_zeros() as u8;
        peaks.push(NodePos::new(height, start_leaf >> height));

        debug_assert!(
            start_leaf.checked_add(size).is_some(),
            "compute_peak_positions: start_leaf + size overflow"
        );
        start_leaf += size;
        remaining -= size;
    }

    peaks
}

fn compute_peak_for_leaf(leaf_index: u64, leaf_count: u64) -> Result<NodePos, MmrError> {
    if leaf_index >= leaf_count {
        return Err(MmrError::LeafNotFound(leaf_index));
    }

    let mut start_leaf = 0u64;
    let mut remaining = leaf_count;

    while remaining > 0 {
        let size = compute_highest_mountain_size(remaining);
        let end_leaf = start_leaf + size;
        if leaf_index < end_leaf {
            let height = size.trailing_zeros() as u8;
            return Ok(NodePos::new(height, start_leaf >> height));
        }

        start_leaf = end_leaf;
        remaining -= size;
    }

    Err(MmrError::LeafNotFound(leaf_index))
}

fn require_node_hash(table: &NodeTable, pos: NodePos) -> DbResult<[u8; 32]> {
    table
        .get_node(pos)
        .map(|h| h.0)
        .ok_or(DbError::MmrNodeNotFound(pos))
}

pub(crate) fn compute_append_fetch_positions(leaf_count: u64) -> Vec<NodePos> {
    let mut positions = Vec::new();
    let mut current_pos = LeafPos::new(leaf_count).node_pos();

    while !current_pos.is_left_child() {
        positions.push(current_pos.neighbor());
        current_pos = current_pos.parent_unchecked();
    }

    positions
}

pub(crate) fn compute_pop_fetch_positions(leaf_count: u64) -> Vec<NodePos> {
    if leaf_count == 0 {
        return Vec::new();
    }

    vec![LeafPos::new(leaf_count - 1).node_pos()]
}

pub(crate) fn compute_append_plan(
    hash: [u8; 32],
    leaf_count: u64,
    table: &NodeTable,
) -> DbResult<AppendPlan> {
    let leaf_pos = LeafPos::new(leaf_count);

    let mut nodes_to_write = vec![(leaf_pos.node_pos(), Hash::from(hash))];
    let mut current_pos = leaf_pos.node_pos();
    let mut current_hash = hash;

    while !current_pos.is_left_child() {
        let sibling_pos = current_pos.neighbor();
        let sibling_hash = require_node_hash(table, sibling_pos)?;
        let parent_hash = Sha256Hasher::hash_node(sibling_hash, current_hash);
        let parent_pos = current_pos.parent_unchecked();

        nodes_to_write.push((parent_pos, parent_hash.into()));

        current_pos = parent_pos;
        current_hash = parent_hash;
    }

    Ok(AppendPlan {
        leaf_pos,
        nodes_to_write,
    })
}

pub(crate) fn compute_pop_plan(leaf_count: u64, table: &NodeTable) -> DbResult<Option<PopPlan>> {
    if leaf_count == 0 {
        return Ok(None);
    }

    let leaf_pos = LeafPos::new(leaf_count - 1);
    let mut nodes_to_remove = Vec::new();
    let leaf_node_pos = leaf_pos.node_pos();
    let leaf_hash = require_node_hash(table, leaf_node_pos)?;
    nodes_to_remove.push(leaf_node_pos);

    let mut current_pos = leaf_node_pos;
    while !current_pos.is_left_child() {
        current_pos = current_pos.parent_unchecked();
        // Every removed node must exist in the pre-fetched table so we can
        // enforce a matching precondition before deletion.
        let _ = require_node_hash(table, current_pos)?;
        nodes_to_remove.push(current_pos);
    }

    Ok(Some(PopPlan {
        leaf_pos,
        leaf_hash: leaf_hash.into(),
        nodes_to_remove,
    }))
}

pub(crate) fn generate_proof(
    leaf_index: u64,
    leaf_count: u64,
    table: &NodeTable,
) -> DbResult<MerkleProof> {
    let peak_pos = compute_peak_for_leaf(leaf_index, leaf_count)?;

    let mut cohashes = Vec::new();
    let mut current_pos = LeafPos::new(leaf_index).node_pos();

    while current_pos != peak_pos {
        let sibling_pos = current_pos.neighbor();
        cohashes.push(require_node_hash(table, sibling_pos)?);
        current_pos = current_pos.parent_unchecked();
    }

    Ok(MerkleProof::from_cohashes(cohashes, leaf_index))
}

/// Generates proofs for all leaves in `[start, end]` (both inclusive).
pub(crate) fn generate_proofs(
    start: u64,
    end: u64,
    leaf_count: u64,
    table: &NodeTable,
) -> DbResult<Vec<MerkleProof>> {
    if start > end {
        return Err(DbError::MmrInvalidRange { start, end });
    }

    if end >= leaf_count {
        return Err(DbError::MmrLeafNotFound(end));
    }

    debug_assert!(end < u64::MAX, "generate_proofs: end + 1 overflow");
    let mut proofs = Vec::with_capacity((end - start + 1) as usize);
    for leaf_index in start..=end {
        proofs.push(generate_proof(leaf_index, leaf_count, table)?);
    }

    Ok(proofs)
}

pub(crate) fn compute_proof_fetch_positions(
    leaf_index: u64,
    leaf_count: u64,
) -> DbResult<Vec<NodePos>> {
    let peak_pos = compute_peak_for_leaf(leaf_index, leaf_count)?;
    let mut positions = Vec::new();
    let mut current_pos = LeafPos::new(leaf_index).node_pos();

    while current_pos != peak_pos {
        positions.push(current_pos.neighbor());
        current_pos = current_pos.parent_unchecked();
    }

    Ok(positions)
}

pub(crate) fn compute_proofs_fetch_positions(
    start: u64,
    end: u64,
    leaf_count: u64,
) -> DbResult<Vec<NodePos>> {
    if start > end {
        return Err(DbError::MmrInvalidRange { start, end });
    }

    if end >= leaf_count {
        return Err(DbError::MmrLeafNotFound(end));
    }

    let mut positions = BTreeSet::new();
    for leaf_index in start..=end {
        for pos in compute_proof_fetch_positions(leaf_index, leaf_count)? {
            positions.insert(pos);
        }
    }

    Ok(positions.into_iter().collect())
}
