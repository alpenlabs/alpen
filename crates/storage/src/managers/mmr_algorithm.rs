//! MMR index algorithm for the storage manager layer.
//!
//! This module is `NodePos`/`LeafPos`-native at its public boundary. The pure
//! position math and the append/proof walks are provided by
//! [`strata_merkle_node_store`]; this module adapts them to the prefetched
//! [`NodeTable`] snapshots and `DbError` contract the manager uses, and adds the
//! pop algorithm (which the node-store crate does not provide).

use std::{collections::BTreeSet, convert::Infallible};

use strata_db_types::{DbError, DbResult, LeafPos, NodePos, NodeTable};
use strata_identifiers::Hash;
use strata_merkle::{MerkleProofB32 as MerkleProof, Sha256Hasher};
use strata_merkle_node_store::{
    assemble_proof, proof_positions, write_plan, MmrError as NodeStoreMmrError,
};

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

/// Maps a node-store [`write_plan`] error into the manager's `DbError`.
///
/// `write_plan` over a consistent store can only fail with
/// [`NodeStoreMmrError::NodeMissing`] (a corrupt store, surfaced as
/// [`DbError::MmrNodeNotFound`]); the backend error is [`Infallible`] here. The
/// remaining variants are not reachable for an append but are mapped defensively
/// to keep the match exhaustive.
fn map_write_plan_err(err: NodeStoreMmrError<Infallible>) -> DbError {
    match err {
        NodeStoreMmrError::NodeMissing(pos) => DbError::MmrNodeNotFound(pos),
        NodeStoreMmrError::Backend(never) => match never {},
        NodeStoreMmrError::LeafOutOfRange { index, leaf_count }
        | NodeStoreMmrError::LeafGap { index, leaf_count } => DbError::MmrIndexOutOfRange {
            requested: index,
            cur: leaf_count,
        },
        NodeStoreMmrError::Pruned {
            index,
            pruned_before,
        } => DbError::MmrIndexOutOfRange {
            requested: index,
            cur: pruned_before,
        },
        NodeStoreMmrError::MaxCapacity => {
            DbError::Other("MMR has reached max capacity".to_string())
        }
    }
}

fn require_node_hash(table: &NodeTable, pos: NodePos) -> DbResult<[u8; 32]> {
    table
        .get_node(pos)
        .map(|h| h.0)
        .ok_or(DbError::MmrNodeNotFound(pos))
}

pub(crate) fn compute_append_fetch_positions(leaf_count: u64) -> Vec<NodePos> {
    // Appending leaf `leaf_count` recomputes exactly the ancestors on that
    // leaf's proof path in the resulting MMR, so its sibling positions are the
    // nodes the append must read.
    let new_count = leaf_count.checked_add(1).expect("MMR leaf count overflow");
    proof_positions(leaf_count, new_count)
}

pub(crate) fn compute_pop_fetch_positions(leaf_count: u64) -> Vec<NodePos> {
    if leaf_count == 0 {
        return Vec::new();
    }

    vec![LeafPos::new(leaf_count - 1).to_node_pos()]
}

pub(crate) fn compute_append_plan(
    hash: [u8; 32],
    leaf_count: u64,
    table: &NodeTable,
) -> DbResult<AppendPlan> {
    let leaf_pos = LeafPos::new(leaf_count);
    let new_count = leaf_count.checked_add(1).expect("MMR leaf count overflow");

    let writes = write_plan::<Sha256Hasher, Infallible>(leaf_count, hash, new_count, |pos| {
        Ok(table.get_node(pos).map(|h| h.0))
    })
    .map_err(map_write_plan_err)?;

    let nodes_to_write = writes
        .into_iter()
        .map(|(pos, node_hash)| (pos, Hash::from(node_hash)))
        .collect();

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
    let leaf_node_pos = leaf_pos.to_node_pos();
    let leaf_hash = require_node_hash(table, leaf_node_pos)?;
    nodes_to_remove.push(leaf_node_pos);

    let mut current_pos = leaf_node_pos;
    while !current_pos.is_left_child() {
        current_pos = current_pos.parent();
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
    if leaf_index >= leaf_count {
        return Err(DbError::MmrLeafNotFound(leaf_index));
    }

    let mut cohashes = Vec::new();
    for pos in proof_positions(leaf_index, leaf_count) {
        cohashes.push(require_node_hash(table, pos)?);
    }

    // `assemble_proof` yields the generic `MerkleProof<[u8; 32]>`; convert it to
    // the SSZ wire type (`MerkleProofB32`) the manager returns.
    Ok(MerkleProof::from_generic(&assemble_proof(
        leaf_index, cohashes,
    )))
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
    if leaf_index >= leaf_count {
        return Err(DbError::MmrLeafNotFound(leaf_index));
    }

    Ok(proof_positions(leaf_index, leaf_count))
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
