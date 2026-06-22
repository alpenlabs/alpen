//! MMR indexing types.
//!
//! [`MmrIndexDatabase`], [`NodePos`], batch-write structs, preconditions, etc.

use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_identifiers::{AccountId, Hash};
pub use strata_merkle_node_store::{LeafPos, NodePos};

#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Opaque serialized form of [`MmrId`], used as a database key.
pub type RawMmrId = Vec<u8>;

/// Identifier for a specific MMR instance in unified storage.
///
/// Each variant represents a different MMR type, with optional scoping
/// within that type (e.g., per-account MMRs).
#[derive(Debug, Clone, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize)]
pub enum MmrId {
    /// ASM manifest MMR (singleton, no account scope).
    Asm,
    /// Snark message inbox MMR (per-account scope).
    SnarkMsgInbox(AccountId),
    /// OL L1 block refs MMR (singleton, no account scope).
    L1BlockRefs,
}

impl MmrId {
    /// Serializes `MmrId` to bytes for use as a database key.
    ///
    /// Uses borsh encoding to ensure stable, deterministic serialization.
    pub fn to_bytes(&self) -> Vec<u8> {
        borsh::to_vec(&self).expect("MmrId serialization should not fail")
    }
}

/// Convert leaves count to total MMR size (number of nodes).
///
/// Formula: `2 * leaves - peak_count`, where `peak_count` is the number
/// of set bits in the binary representation of `leaves`.
pub fn num_leaves_to_mmr_size(leaves_count: u64) -> u64 {
    debug_assert!(
        leaves_count <= (u64::MAX / 2),
        "num_leaves_to_mmr_size would overflow"
    );
    let peak_count = leaves_count.count_ones() as u64;
    2 * leaves_count - peak_count
}

/// Scoped node reference for a specific MMR namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MmrNodePos {
    id: RawMmrId,
    pos: NodePos,
}

impl MmrNodePos {
    /// Constructs a scoped node reference.
    pub fn new(id: RawMmrId, pos: NodePos) -> Self {
        Self { id, pos }
    }

    /// Returns the MMR id for this node reference.
    pub fn id(&self) -> &RawMmrId {
        &self.id
    }

    /// Returns the node position for this node reference.
    pub fn pos(&self) -> NodePos {
        self.pos
    }
}

/// Value precondition for compare-and-set semantics in an atomic MMR batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MmrIndexPrecondition {
    /// Expected current node hash at `pos`. `None` means key absent.
    Node {
        pos: NodePos,
        expected: Option<Hash>,
    },
    /// Expected current preimage at `pos`. `None` means key absent.
    Preimage {
        pos: LeafPos,
        expected: Option<Vec<u8>>,
    },
}

/// Read-side table for one MMR namespace.
#[derive(Debug, Default, Clone)]
pub struct NodeTable {
    nodes: BTreeMap<NodePos, Hash>,
    preimages: BTreeMap<LeafPos, Vec<u8>>,
}

impl NodeTable {
    /// Inserts or overwrites a node hash in this table.
    pub fn put_node(&mut self, pos: NodePos, hash: Hash) {
        self.nodes.insert(pos, hash);
    }

    /// Inserts or overwrites a leaf preimage in this table.
    pub fn put_preimage(&mut self, pos: LeafPos, preimage: Vec<u8>) {
        self.preimages.insert(pos, preimage);
    }

    /// Iterates over stored node hashes.
    pub fn nodes(&self) -> impl Iterator<Item = (&NodePos, &Hash)> {
        self.nodes.iter()
    }

    /// Returns the stored node hash at `pos`, if present.
    pub fn get_node(&self, pos: NodePos) -> Option<&Hash> {
        self.nodes.get(&pos)
    }

    /// Iterates over stored preimages.
    pub fn preimages(&self) -> impl Iterator<Item = (&LeafPos, &Vec<u8>)> {
        self.preimages.iter()
    }

    /// Returns the stored preimage at `pos`, if present.
    pub fn get_preimage(&self, pos: LeafPos) -> Option<&Vec<u8>> {
        self.preimages.get(&pos)
    }
}

/// Read-side table grouped by MMR namespace.
#[derive(Debug, Default, Clone)]
pub struct MmrNodeTable {
    mmrs: BTreeMap<RawMmrId, NodeTable>,
}

impl MmrNodeTable {
    /// Returns a mutable table for `mmr_id`, if present.
    pub fn get_table_mut(&mut self, mmr_id: &RawMmrId) -> Option<&mut NodeTable> {
        self.mmrs.get_mut(mmr_id)
    }

    /// Returns a mutable table for `mmr_id`, creating one if absent.
    pub fn get_or_create_table_mut(&mut self, mmr_id: RawMmrId) -> &mut NodeTable {
        self.mmrs.entry(mmr_id).or_default()
    }

    /// Iterates over all MMR namespace tables.
    pub fn mmrs(&self) -> impl Iterator<Item = (&RawMmrId, &NodeTable)> {
        self.mmrs.iter()
    }

    /// Returns the table for `mmr_id`, if present.
    pub fn get_table(&self, mmr_id: &RawMmrId) -> Option<&NodeTable> {
        self.mmrs.get(mmr_id)
    }
}

/// Single-MMR atomic write batch.
///
/// Used by manager code to build writes for one MMR namespace.
#[derive(Debug, Default, Clone)]
pub struct BatchWrite {
    expected_leaf_count: Option<u64>,
    leaf_count: Option<u64>,
    node_writes: BTreeMap<NodePos, Option<Hash>>,
    preimage_writes: BTreeMap<LeafPos, Option<Vec<u8>>>,
    preconditions: Vec<MmrIndexPrecondition>,
}

impl BatchWrite {
    /// Sets the expected current leaf count for CAS validation.
    pub fn set_expected_leaf_count(&mut self, count: u64) {
        self.expected_leaf_count = Some(count);
    }

    /// Returns the expected current leaf count, if provided.
    pub fn expected_leaf_count(&self) -> Option<u64> {
        self.expected_leaf_count
    }

    /// Sets the next leaf count to persist atomically with this batch.
    pub fn set_leaf_count(&mut self, count: u64) {
        self.leaf_count = Some(count);
    }

    /// Returns the next leaf count to persist, if provided.
    pub fn leaf_count(&self) -> Option<u64> {
        self.leaf_count
    }

    /// Writes or overwrites a node hash.
    pub fn put_node(&mut self, pos: NodePos, hash: Hash) {
        self.node_writes.insert(pos, Some(hash));
    }

    /// Deletes a node hash.
    pub fn del_node(&mut self, pos: NodePos) {
        self.node_writes.insert(pos, None);
    }

    /// Writes or overwrites preimage bytes for a leaf.
    pub fn put_preimage(&mut self, pos: LeafPos, preimage: Vec<u8>) {
        self.preimage_writes.insert(pos, Some(preimage));
    }

    /// Deletes preimage bytes for a leaf.
    pub fn del_preimage(&mut self, pos: LeafPos) {
        self.preimage_writes.insert(pos, None);
    }

    /// Adds a node precondition to this single-MMR batch.
    pub fn add_node_precond(&mut self, pos: NodePos, expected: Option<Hash>) {
        self.preconditions
            .push(MmrIndexPrecondition::Node { pos, expected });
    }

    /// Adds a preimage precondition to this single-MMR batch.
    pub fn add_preimage_precond(&mut self, pos: LeafPos, expected: Option<Vec<u8>>) {
        self.preconditions
            .push(MmrIndexPrecondition::Preimage { pos, expected });
    }

    /// Iterates over pending node hash writes in position order.
    pub fn node_puts(&self) -> impl Iterator<Item = (NodePos, Hash)> + '_ {
        self.node_writes
            .iter()
            .filter_map(|(&pos, opt)| opt.map(|h| (pos, h)))
    }

    /// Iterates over pending node hash deletions in position order.
    pub fn node_dels(&self) -> impl Iterator<Item = NodePos> + '_ {
        self.node_writes
            .iter()
            .filter_map(|(&pos, opt)| opt.is_none().then_some(pos))
    }

    /// Iterates over pending preimage writes in position order.
    // NOTE: Returns `&Vec<u8>` (instead of `&[u8]`) so typed-sled insert callers
    // can pass preimages through without allocating a temporary `Vec<u8>`.
    pub fn preimage_puts(&self) -> impl Iterator<Item = (LeafPos, &Vec<u8>)> + '_ {
        // NOTE: Returns `&Vec<u8>` because preimage schema `Value = Vec<u8>` and
        // typed-sled transaction insert requires `&Value` (`&Vec<u8>` here).
        // Returning `&[u8]` would force call-site allocations via `.to_vec()`.
        self.preimage_writes
            .iter()
            .filter_map(|(&pos, opt)| opt.as_ref().map(|v| (pos, v)))
    }

    /// Iterates over pending preimage deletions in position order.
    pub fn preimage_dels(&self) -> impl Iterator<Item = LeafPos> + '_ {
        self.preimage_writes
            .iter()
            .filter_map(|(&pos, opt)| opt.is_none().then_some(pos))
    }

    /// Iterates over preconditions in insertion order.
    pub fn preconditions(&self) -> &[MmrIndexPrecondition] {
        &self.preconditions
    }
}

/// Multi-MMR atomic write batch.
#[derive(Debug, Default, Clone)]
pub struct MmrBatchWrite {
    batches: BTreeMap<RawMmrId, BatchWrite>,
}

impl MmrBatchWrite {
    /// Returns a mutable reference to the [`BatchWrite`] for `mmr_id`,
    /// creating an empty one if absent.
    pub fn entry(&mut self, mmr_id: RawMmrId) -> &mut BatchWrite {
        self.batches.entry(mmr_id).or_default()
    }

    /// Adds a node precondition under `mmr_id`.
    pub fn add_node_precond(&mut self, mmr_id: RawMmrId, pos: NodePos, expected: Option<Hash>) {
        self.entry(mmr_id).add_node_precond(pos, expected);
    }

    /// Adds a preimage precondition under `mmr_id`.
    pub fn add_preimage_precond(
        &mut self,
        mmr_id: RawMmrId,
        pos: LeafPos,
        expected: Option<Vec<u8>>,
    ) {
        self.entry(mmr_id).add_preimage_precond(pos, expected);
    }

    /// Iterates over per-namespace batches.
    pub fn batches(&self) -> impl Iterator<Item = (&RawMmrId, &BatchWrite)> {
        self.batches.iter()
    }

    /// Builds write-batch preconditions from a pre-fetched node table.
    pub fn from_preconds_table(table: MmrNodeTable) -> Self {
        let mut out = Self::default();

        for (mmr_id, node_table) in table.mmrs {
            for (pos, hash) in node_table.nodes {
                out.add_node_precond(mmr_id.clone(), pos, Some(hash));
            }

            for (pos, preimage) in node_table.preimages {
                out.add_preimage_precond(mmr_id.clone(), pos, Some(preimage));
            }
        }

        out
    }
}

/// Storage-only MMR indexing database interface.
///
/// This interface intentionally contains only primitive reads and one
/// backend-agnostic atomic batch write entry point.
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:mmr_index")
)]
pub trait MmrIndexDatabase: Send + Sync + 'static {
    /// Returns the node hash for a namespace and node position.
    fn get_node(&self, mmr_id: RawMmrId, pos: NodePos) -> DbResult<Option<Hash>>;

    /// Returns optional preimage bytes for a namespace and leaf position.
    fn get_preimage(&self, mmr_id: RawMmrId, pos: LeafPos) -> DbResult<Option<Vec<u8>>>;

    /// Returns optional preimage bytes for a namespace and leaf range.
    ///
    /// The returned vector has one slot per leaf in `[start, end_exclusive)`.
    /// Missing preimages are returned as `None`.
    ///
    /// Empty ranges return an empty vector. Backends must reject reversed
    /// ranges with [`crate::DbError::MmrInvalidRange`].
    fn get_preimage_range(
        &self,
        mmr_id: RawMmrId,
        start: LeafPos,
        end_exclusive: LeafPos,
    ) -> DbResult<Vec<Option<Vec<u8>>>>;

    /// Returns the current leaf count for a namespace.
    ///
    /// Implementations should return `0` when the namespace has no leaves.
    fn get_leaf_count(&self, mmr_id: RawMmrId) -> DbResult<u64>;

    /// Fetches requested nodes and available parent path nodes in one read.
    ///
    /// If `preimages` is true, implementations should also include available
    /// preimages for requested leaf positions.
    // NOTE: Takes an owned Vec so generated async/chan wrappers can move the
    // argument into 'static worker closures without borrowing/lifetime issues.
    fn fetch_node_paths(&self, nodes: Vec<MmrNodePos>, preimages: bool) -> DbResult<MmrNodeTable>;

    /// Applies an atomic batch write with compare-and-set preconditions.
    ///
    /// If any precondition fails, no writes are applied.
    fn apply_update(&self, batch: MmrBatchWrite) -> DbResult<()>;
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn node_pos_strat() -> impl Strategy<Value = NodePos> {
        (0u8..=10u8, 0u64..1024).prop_map(|(h, i)| NodePos::new(h, i))
    }

    fn hash_strat() -> impl Strategy<Value = Hash> {
        prop::array::uniform32(0u8..).prop_map(Hash::from)
    }

    // NOTE: `NodePos`/`LeafPos` position math (parent/sibling/children/etc.) is
    // owned and tested by the `strata-merkle-node-store` crate. These tests
    // cover only the local batch/table types built on those positions.
    proptest! {
        #[test]
        fn prop_put_node_overwrites(
            pos in node_pos_strat(),
            h1 in hash_strat(),
            h2 in hash_strat(),
        ) {
            let mut batch = BatchWrite::default();
            batch.put_node(pos, h1);
            batch.put_node(pos, h2);
            let puts: Vec<_> = batch.node_puts().collect();
            prop_assert_eq!(puts.len(), 1);
            prop_assert_eq!(puts[0], (pos, h2));
            prop_assert_eq!(batch.node_dels().count(), 0);
        }

        #[test]
        fn prop_del_node_after_put_seen_as_del(pos in node_pos_strat(), h in hash_strat()) {
            let mut batch = BatchWrite::default();
            batch.put_node(pos, h);
            batch.del_node(pos);
            prop_assert_eq!(batch.node_puts().count(), 0);
            let dels: Vec<_> = batch.node_dels().collect();
            prop_assert_eq!(dels, vec![pos]);
        }

        #[test]
        fn prop_put_preimage_overwrites(
            idx in 0u64..1024u64,
            p1 in prop::collection::vec(0u8.., 0..32usize),
            p2 in prop::collection::vec(0u8.., 0..32usize),
        ) {
            let pos = LeafPos::new(idx);
            let mut batch = BatchWrite::default();
            batch.put_preimage(pos, p1);
            batch.put_preimage(pos, p2.clone());
            let puts: Vec<_> = batch.preimage_puts().collect();
            prop_assert_eq!(puts.len(), 1);
            prop_assert_eq!(puts[0], (pos, &p2));
            prop_assert_eq!(batch.preimage_dels().count(), 0);
        }

        #[test]
        fn prop_del_preimage_after_put_seen_as_del(
            idx in 0u64..1024u64,
            p in prop::collection::vec(0u8.., 0..32usize),
        ) {
            let pos = LeafPos::new(idx);
            let mut batch = BatchWrite::default();
            batch.put_preimage(pos, p);
            batch.del_preimage(pos);
            prop_assert_eq!(batch.preimage_puts().count(), 0);
            let dels: Vec<_> = batch.preimage_dels().collect();
            prop_assert_eq!(dels, vec![pos]);
        }

        #[test]
        fn prop_node_puts_sorted(positions in prop::collection::vec(node_pos_strat(), 1..8)) {
            let mut batch = BatchWrite::default();
            let h: Hash = [0u8; 32].into();
            for pos in &positions {
                batch.put_node(*pos, h);
            }
            let keys: Vec<NodePos> = batch.node_puts().map(|(p, _)| p).collect();
            let mut expected = keys.clone();
            expected.sort();
            expected.dedup();
            prop_assert_eq!(keys, expected);
        }
    }

    #[test]
    fn mmr_batch_write_entry_idempotent() {
        let mut batch = MmrBatchWrite::default();
        let mmr_id = vec![1u8];
        let pos = NodePos::new(0, 0);
        let hash: Hash = [2u8; 32].into();
        batch.entry(mmr_id.clone()).put_node(pos, hash);
        batch.entry(mmr_id.clone()).put_node(pos, hash);
        assert_eq!(batch.batches().count(), 1);
        assert_eq!(
            batch.batches().next().expect("batch").1.node_puts().count(),
            1
        );
    }

    #[test]
    fn batch_write_stores_preconditions_locally() {
        let mut batch = BatchWrite::default();
        let pos = NodePos::new(0, 0);
        let hash: Hash = [2u8; 32].into();

        batch.add_node_precond(pos, Some(hash));

        assert_eq!(batch.preconditions().len(), 1);
        assert!(matches!(
            batch.preconditions()[0],
            MmrIndexPrecondition::Node {
                pos: p,
                expected: Some(h)
            } if p == pos && h == hash
        ));
    }

    #[test]
    fn batch_write_leaf_count_fields_roundtrip() {
        let mut batch = BatchWrite::default();
        assert_eq!(batch.expected_leaf_count(), None);
        assert_eq!(batch.leaf_count(), None);

        batch.set_expected_leaf_count(9);
        batch.set_leaf_count(10);

        assert_eq!(batch.expected_leaf_count(), Some(9));
        assert_eq!(batch.leaf_count(), Some(10));
    }

    #[test]
    fn from_preconds_table_populates_expected_values() {
        let mmr_id = vec![7u8];
        let node_pos = NodePos::new(1, 3);
        let leaf_pos = LeafPos::new(9);
        let node_hash: Hash = [9u8; 32].into();
        let preimage = vec![1u8, 2u8, 3u8];

        let mut table = MmrNodeTable::default();
        table
            .get_or_create_table_mut(mmr_id.clone())
            .put_node(node_pos, node_hash);
        table
            .get_or_create_table_mut(mmr_id.clone())
            .put_preimage(leaf_pos, preimage.clone());

        let batch = MmrBatchWrite::from_preconds_table(table);
        let per_mmr = batch
            .batches()
            .find(|(id, _)| *id == &mmr_id)
            .expect("mmr batch")
            .1;

        assert_eq!(per_mmr.preconditions().len(), 2);

        assert!(per_mmr.preconditions().iter().any(|p| {
            matches!(
                p,
                MmrIndexPrecondition::Node {
                    pos,
                    expected: Some(hash)
                } if *pos == node_pos && *hash == node_hash
            )
        }));

        assert!(per_mmr.preconditions().iter().any(|p| {
            matches!(
                p,
                MmrIndexPrecondition::Preimage {
                    pos,
                    expected: Some(bytes)
                } if *pos == leaf_pos && *bytes == preimage
            )
        }));
    }

    #[test]
    fn mmr_batch_write_helpers_add_preconditions_under_mmr_scope() {
        let mmr_a = vec![1u8];
        let mmr_b = vec![2u8];
        let node_pos = NodePos::new(0, 3);
        let leaf_pos = LeafPos::new(7);
        let hash: Hash = [8u8; 32].into();
        let preimage = vec![10u8, 11u8];

        let mut batch = MmrBatchWrite::default();
        batch.add_node_precond(mmr_a.clone(), node_pos, Some(hash));
        batch.add_preimage_precond(mmr_b.clone(), leaf_pos, Some(preimage.clone()));

        let a_batch = batch
            .batches()
            .find(|(id, _)| *id == &mmr_a)
            .expect("mmr a batch")
            .1;
        assert!(a_batch.preconditions().iter().any(|p| {
            matches!(
                p,
                MmrIndexPrecondition::Node {
                    pos,
                    expected: Some(h)
                } if *pos == node_pos && *h == hash
            )
        }));

        let b_batch = batch
            .batches()
            .find(|(id, _)| *id == &mmr_b)
            .expect("mmr b batch")
            .1;
        assert!(b_batch.preconditions().iter().any(|p| {
            matches!(
                p,
                MmrIndexPrecondition::Preimage {
                    pos,
                    expected: Some(bytes)
                } if *pos == leaf_pos && *bytes == preimage
            )
        }));
    }
}

#[cfg(test)]
mod mmr_math_tests {
    use strata_merkle::{
        CompactMmr64, MerkleProof as GenericMerkleProof, Mmr, MmrState, Sha256Hasher,
    };

    use super::num_leaves_to_mmr_size;
    use crate::errors::MmrError;

    fn leaf_index_to_pos(index: u64) -> u64 {
        debug_assert!(index < u64::MAX, "leaf_index_to_pos index overflow");
        let next_index = index + 1;
        num_leaves_to_mmr_size(next_index) - next_index.trailing_zeros() as u64 - 1
    }

    fn pos_height_in_tree(mut pos: u64) -> u8 {
        if pos == 0 {
            return 0;
        }

        let mut peak_size = u64::MAX >> pos.leading_zeros();
        while peak_size > 0 {
            if pos >= peak_size {
                pos -= peak_size;
            }
            peak_size >>= 1;
        }

        pos as u8
    }

    #[inline]
    fn sibling_offset(height: u8) -> u64 {
        debug_assert!(height < 63, "sibling_offset shift overflow");
        (2u64 << height) - 1
    }

    fn parent_pos(pos: u64, height: u8) -> u64 {
        debug_assert!(pos < u64::MAX, "parent_pos pos + 1 overflow");
        debug_assert!(height < 63, "parent_pos shift overflow");
        let next_height = pos_height_in_tree(pos + 1);
        if next_height > height {
            pos + 1
        } else {
            let delta = 2u64 << height;
            debug_assert!(pos <= u64::MAX - delta, "parent_pos addition overflow");
            pos + delta
        }
    }

    fn sibling_pos(pos: u64, height: u8) -> u64 {
        debug_assert!(pos < u64::MAX, "sibling_pos pos + 1 overflow");
        let next_height = pos_height_in_tree(pos + 1);
        if next_height > height {
            let offset = sibling_offset(height);
            debug_assert!(pos >= offset, "sibling_pos subtraction overflow");
            pos - offset
        } else {
            let offset = sibling_offset(height);
            debug_assert!(pos <= u64::MAX - offset, "sibling_pos addition overflow");
            pos + offset
        }
    }

    fn get_peaks(mmr_size: u64) -> Vec<u64> {
        if mmr_size == 0 {
            return vec![];
        }

        let mut peaks = Vec::new();
        let mut pos = 0u64;
        let mut remaining = mmr_size;

        while remaining > 0 {
            let mut height = 63 - remaining.leading_zeros();
            let mut tree_size = ((1u128 << (height + 1)) - 1) as u64;

            while tree_size > remaining {
                height -= 1;
                tree_size = ((1u128 << (height + 1)) - 1) as u64;
            }

            let peak_pos = pos + tree_size - 1;
            peaks.push(peak_pos);
            pos += tree_size;
            remaining -= tree_size;
        }

        peaks
    }

    fn find_peak_for_pos(pos: u64, max_size: u64) -> Result<u64, MmrError> {
        let peaks = get_peaks(max_size);
        for &peak_pos in &peaks {
            if pos <= peak_pos {
                return Ok(peak_pos);
            }
        }
        Err(MmrError::PositionOutOfBounds { pos, max_size })
    }

    #[test]
    fn test_leaf_index_to_mmr_size() {
        assert_eq!(num_leaves_to_mmr_size(1), 1);
        assert_eq!(num_leaves_to_mmr_size(2), 3);
        assert_eq!(num_leaves_to_mmr_size(3), 4);
        assert_eq!(num_leaves_to_mmr_size(4), 7);
        assert_eq!(num_leaves_to_mmr_size(7), 11);
    }

    #[test]
    fn test_leaf_index_to_pos() {
        assert_eq!(leaf_index_to_pos(0), 0);
        assert_eq!(leaf_index_to_pos(1), 1);
        assert_eq!(leaf_index_to_pos(2), 3);
        assert_eq!(leaf_index_to_pos(3), 4);
    }

    #[test]
    fn test_pos_height_in_tree() {
        assert_eq!(pos_height_in_tree(0), 0);
        assert_eq!(pos_height_in_tree(1), 0);
        assert_eq!(pos_height_in_tree(3), 0);
        assert_eq!(pos_height_in_tree(4), 0);
        assert_eq!(pos_height_in_tree(2), 1);
        assert_eq!(pos_height_in_tree(5), 1);
        assert_eq!(pos_height_in_tree(6), 2);
    }

    #[test]
    fn test_sibling_offset() {
        assert_eq!(sibling_offset(0), 1);
        assert_eq!(sibling_offset(1), 3);
        assert_eq!(sibling_offset(2), 7);
    }

    #[test]
    fn test_parent_pos() {
        assert_eq!(parent_pos(0, 0), 2);
        assert_eq!(parent_pos(1, 0), 2);
        assert_eq!(parent_pos(3, 0), 5);
    }

    #[test]
    fn test_sibling_pos() {
        assert_eq!(sibling_pos(0, 0), 1);
        assert_eq!(sibling_pos(1, 0), 0);
        assert_eq!(sibling_pos(3, 0), 4);
    }

    #[test]
    fn test_get_peaks() {
        assert_eq!(get_peaks(1), vec![0]);
        assert_eq!(get_peaks(3), vec![2]);
        assert_eq!(get_peaks(4), vec![2, 3]);
        assert_eq!(get_peaks(7), vec![6]);
        assert_eq!(get_peaks(11), vec![6, 9, 10]);
    }

    #[test]
    fn test_find_peak_for_pos() {
        assert_eq!(find_peak_for_pos(0, 11).unwrap(), 6);
        assert_eq!(find_peak_for_pos(2, 11).unwrap(), 6);
        assert_eq!(find_peak_for_pos(6, 11).unwrap(), 6);
        assert_eq!(find_peak_for_pos(7, 11).unwrap(), 9);
        assert_eq!(find_peak_for_pos(10, 11).unwrap(), 10);
    }

    fn test_leaf_hash(i: u64) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[..8].copy_from_slice(&(i + 1).to_le_bytes());
        out
    }

    #[test]
    fn test_mmr_size_and_peak_heights_match_reference() {
        for leaf_count in 0u64..=128 {
            let mut mmr = CompactMmr64::<[u8; 32]>::new(64);
            for i in 0..leaf_count {
                Mmr::<Sha256Hasher>::add_leaf(&mut mmr, test_leaf_hash(i))
                    .expect("append to reference mmr");
            }

            let reference_peak_heights: Vec<u8> = mmr.iter_peaks().map(|(h, _)| h).collect();
            let reference_mmr_size: u64 = reference_peak_heights
                .iter()
                .map(|height| (1u64 << (u32::from(*height) + 1)) - 1)
                .sum();
            let mmr_size = num_leaves_to_mmr_size(leaf_count);

            assert_eq!(mmr_size, reference_mmr_size);

            let peaks = get_peaks(mmr_size);
            let mut peak_heights: Vec<u8> = peaks
                .iter()
                .map(|peak_pos| pos_height_in_tree(*peak_pos))
                .collect();
            let mut reference_peak_heights = reference_peak_heights;
            peak_heights.sort_unstable();
            reference_peak_heights.sort_unstable();
            assert_eq!(peak_heights, reference_peak_heights);
        }
    }

    #[test]
    fn test_leaf_proof_depths_match_reference() {
        for leaf_count in 1u64..=64 {
            let mut mmr = CompactMmr64::<[u8; 32]>::new(64);
            let mut proof_list: Vec<GenericMerkleProof<[u8; 32]>> = Vec::new();

            for i in 0..leaf_count {
                let leaf_hash = test_leaf_hash(i);
                let proof = Mmr::<Sha256Hasher>::add_leaf_updating_proof_list(
                    &mut mmr,
                    leaf_hash,
                    &mut proof_list,
                )
                .expect("append to reference mmr with proof updates");
                proof_list.push(proof);
            }

            let mmr_size = num_leaves_to_mmr_size(leaf_count);

            for (leaf_idx, proof) in proof_list.iter().enumerate() {
                let leaf_idx = leaf_idx as u64;
                let leaf_pos = leaf_index_to_pos(leaf_idx);
                assert!(leaf_pos < mmr_size);
                assert_eq!(pos_height_in_tree(leaf_pos), 0);

                let peak = find_peak_for_pos(leaf_pos, mmr_size).expect("peak for leaf position");
                let expected_depth = usize::from(pos_height_in_tree(peak));
                assert_eq!(proof.cohashes().len(), expected_depth);
            }
        }
    }
}
