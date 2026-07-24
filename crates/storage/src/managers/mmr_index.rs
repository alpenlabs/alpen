//! High-level manager for MMR index database access.

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::sync::Arc;

use ssz_types::FixedBytes;
use strata_db_types::mmr_index::MmrIndexDatabase;
use strata_db_types::{
    num_leaves_to_mmr_size, DbError, DbResult, LeafPos, MmrBatchWrite, MmrId, MmrNodePos,
    MmrNodeTable, NodePos, NodeTable, RawMmrId,
};
use strata_identifiers::Hash;
use strata_merkle::{MerkleHasher, MerkleProofB32 as MerkleProof, Mmr64B32, Sha256Hasher};
use strata_merkle_node_store::{
    assemble_proof, iter_prune_after_positions, peak_positions, proof_positions, write_plan,
};
use tokio::runtime::Handle;
use tokio::task::spawn_blocking;

use crate::ops::mmr_index::MmrIndexOps;

/// Retry behavior for optimistic CAS-style MMR updates.
#[derive(Debug, Clone, Copy)]
pub struct MmrIndexRetryConfig {
    pub max_precondition_retries: usize,
}

impl Default for MmrIndexRetryConfig {
    fn default() -> Self {
        Self {
            max_precondition_retries: 3,
        }
    }
}

/// Manager-level configuration.
#[derive(Debug, Clone, Copy, Default)]
pub struct MmrIndexManagerConfig {
    pub retry: MmrIndexRetryConfig,
}

/// Node writes an append applies, resolved against a prefetched snapshot.
#[derive(Debug, Clone)]
struct AppendPlan {
    /// Position the new leaf occupies.
    leaf_pos: LeafPos,

    /// Leaf count after the append.
    new_leaf_count: u64,

    /// The new leaf and every ancestor it recomputes.
    nodes_to_write: Vec<(NodePos, Hash)>,
}

/// Node deletions a pop applies, resolved against a prefetched snapshot.
#[derive(Debug, Clone)]
struct PopPlan {
    /// Position of the leaf being removed.
    leaf_pos: LeafPos,

    /// Hash of the removed leaf, returned to the caller.
    leaf_hash: Hash,

    /// The leaf and every ancestor that becomes unreachable.
    nodes_to_remove: Vec<NodePos>,
}

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
#[derive(Clone)]
pub struct MmrIndexManager {
    ops: Arc<MmrIndexOps>,
    config: MmrIndexManagerConfig,
}

impl MmrIndexManager {
    pub fn new(handle: Handle, db: Arc<impl MmrIndexDatabase + 'static>) -> Self {
        Self::with_config(handle, db, MmrIndexManagerConfig::default())
    }

    pub fn with_config(
        handle: Handle,
        db: Arc<impl MmrIndexDatabase + 'static>,
        config: MmrIndexManagerConfig,
    ) -> Self {
        let ops = Arc::new(MmrIndexOps::new(handle, db));
        Self { ops, config }
    }

    pub fn get_handle(&self, mmr_id: MmrId) -> MmrIndexHandle {
        MmrIndexHandle {
            mmr_id,
            ops: self.ops.clone(),
            max_retries: self.config.retry.max_precondition_retries.max(1),
        }
    }

    /// Applies a cross-MMR atomic update.
    pub fn apply_update_blocking(&self, batch: MmrBatchWrite) -> DbResult<()> {
        self.ops.apply_update_blocking(batch)
    }

    /// Applies a cross-MMR atomic update.
    pub async fn apply_update(&self, batch: MmrBatchWrite) -> DbResult<()> {
        self.ops.apply_update_async(batch).await
    }

    /// Lists MMR namespace identifiers in the index.
    pub fn list_mmr_ids_blocking(&self) -> DbResult<Vec<RawMmrId>> {
        self.ops.list_mmr_ids_blocking()
    }

    /// Lists MMR namespace identifiers in the index.
    pub async fn list_mmr_ids(&self) -> DbResult<Vec<RawMmrId>> {
        self.ops.list_mmr_ids_async().await
    }
}

#[expect(
    missing_debug_implementations,
    reason = "Inner ops type doesn't have Debug implementation"
)]
#[derive(Clone)]
pub struct MmrIndexHandle {
    mmr_id: MmrId,
    ops: Arc<MmrIndexOps>,
    max_retries: usize,
}

impl MmrIndexHandle {
    fn mmr_id_bytes(&self) -> RawMmrId {
        self.mmr_id.to_bytes()
    }

    pub fn get_leaf_count_blocking(&self) -> DbResult<u64> {
        self.ops.get_leaf_count_blocking(self.mmr_id_bytes())
    }

    fn fetch_node_paths_blocking(
        &self,
        positions: impl IntoIterator<Item = NodePos>,
        preimages: bool,
    ) -> DbResult<MmrNodeTable> {
        let mmr_id = self.mmr_id_bytes();
        let scoped_positions = positions
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|pos| MmrNodePos::new(mmr_id.clone(), pos))
            .collect::<Vec<_>>();
        self.ops
            .fetch_node_paths_blocking(scoped_positions, preimages)
    }

    fn get_scoped_node_table(prefetched: &MmrNodeTable, mmr_id: &RawMmrId) -> NodeTable {
        prefetched.get_table(mmr_id).cloned().unwrap_or_default()
    }

    fn append_leaf_once_blocking(&self, hash: Hash, preimage: Option<Vec<u8>>) -> DbResult<u64> {
        let leaf_count = self.get_leaf_count_blocking()?;
        let mmr_id = self.mmr_id_bytes();

        let read_positions = compute_append_read_positions(leaf_count);
        let prefetched = self.fetch_node_paths_blocking(read_positions, false)?;
        let node_table = Self::get_scoped_node_table(&prefetched, &mmr_id);

        let plan = plan_append(hash, leaf_count, &node_table)?;

        let mut batch = MmrBatchWrite::from_preconds_table(prefetched);
        let mmr_batch = batch.entry(mmr_id);

        mmr_batch.add_node_precond(plan.leaf_pos.to_node_pos(), None);
        mmr_batch.set_expected_leaf_count(leaf_count);
        mmr_batch.set_leaf_count(plan.new_leaf_count);
        for (node_pos, node_hash) in plan.nodes_to_write {
            mmr_batch.put_node(node_pos, node_hash);
        }

        if let Some(preimage) = preimage {
            mmr_batch.add_preimage_precond(plan.leaf_pos, None);
            mmr_batch.put_preimage(plan.leaf_pos, preimage);
        }

        self.ops.apply_update_blocking(batch)?;
        Ok(plan.leaf_pos.index())
    }

    pub async fn append_leaf(&self, hash: Hash) -> DbResult<u64> {
        let this = self.clone();
        spawn_blocking(move || this.append_leaf_blocking(hash))
            .await
            .map_err(DbError::from)?
    }

    pub fn append_leaf_blocking(&self, hash: Hash) -> DbResult<u64> {
        run_with_precondition_retries(self.max_retries, || {
            self.append_leaf_once_blocking(hash, None)
        })
    }

    /// Appends a caller-provided leaf hash and stores its preimage bytes.
    pub fn append_leaf_with_preimage_blocking(
        &self,
        hash: Hash,
        preimage: Vec<u8>,
    ) -> DbResult<u64> {
        run_with_precondition_retries(self.max_retries, || {
            self.append_leaf_once_blocking(hash, Some(preimage.clone()))
        })
    }

    /// Ensures the leaf at `expected_idx` is exactly `expected_hash`, appending
    /// `(expected_hash, preimage)` if the slot is currently empty.
    ///
    /// Makes mirroring an in-state MMR write idempotent for crash-restart
    /// retries: callers can re-run their indexing pass without checking whether
    /// each leaf was already written.
    ///
    /// - `expected_idx < leaf_count`: slot occupied — succeeds if the stored hash matches, else
    ///   [`DbError::MmrLeafHashMismatch`].
    /// - `expected_idx == leaf_count`: appends.
    /// - `expected_idx > leaf_count`: [`DbError::MmrIndexOutOfRange`] (MMRs cannot have gaps).
    pub fn idempotent_append_leaf_with_preimage_blocking(
        &self,
        expected_idx: u64,
        expected_hash: Hash,
        preimage: Vec<u8>,
    ) -> DbResult<()> {
        let leaf_count = self.get_leaf_count_blocking()?;

        if expected_idx < leaf_count {
            let Some(existing_hash) = self.get_leaf_blocking(expected_idx)? else {
                return Err(DbError::MmrNodeNotFound(
                    LeafPos::new(expected_idx).to_node_pos(),
                ));
            };

            if existing_hash != expected_hash {
                return Err(DbError::MmrLeafHashMismatch {
                    idx: expected_idx,
                    expected: expected_hash,
                    got: existing_hash,
                });
            }

            return Ok(());
        }

        if expected_idx > leaf_count {
            return Err(DbError::MmrIndexOutOfRange {
                requested: expected_idx,
                cur: leaf_count,
            });
        }

        let appended_idx = self.append_leaf_with_preimage_blocking(expected_hash, preimage)?;
        if appended_idx != expected_idx {
            return Err(DbError::MmrIndexOutOfRange {
                requested: expected_idx,
                cur: appended_idx,
            });
        }
        Ok(())
    }

    /// Appends a caller-provided leaf hash and stores its preimage bytes.
    pub async fn append_leaf_with_preimage(&self, hash: Hash, preimage: Vec<u8>) -> DbResult<u64> {
        let this = self.clone();
        spawn_blocking(move || this.append_leaf_with_preimage_blocking(hash, preimage))
            .await
            .map_err(DbError::from)?
    }

    /// Appends a preimage and stores it as bytes in the preimage table.
    pub fn append_blocking(&self, preimage: Vec<u8>) -> DbResult<u64> {
        self.append_with_hasher_blocking::<Sha256Hasher>(preimage)
    }

    /// Appends a preimage and stores it as bytes in the preimage table.
    pub async fn append(&self, preimage: Vec<u8>) -> DbResult<u64> {
        self.append_with_hasher::<Sha256Hasher>(preimage).await
    }

    /// Appends a preimage using caller-provided hash function.
    pub fn append_with_hasher_blocking<H>(&self, preimage: Vec<u8>) -> DbResult<u64>
    where
        H: MerkleHasher<Hash = [u8; 32]>,
    {
        let hash = H::hash_leaf(&preimage).into();
        run_with_precondition_retries(self.max_retries, || {
            self.append_leaf_once_blocking(hash, Some(preimage.clone()))
        })
    }

    /// Appends a preimage using caller-provided hash function.
    pub async fn append_with_hasher<H>(&self, preimage: Vec<u8>) -> DbResult<u64>
    where
        H: MerkleHasher<Hash = [u8; 32]>,
    {
        let this = self.clone();
        spawn_blocking(move || this.append_with_hasher_blocking::<H>(preimage))
            .await
            .map_err(DbError::from)?
    }

    pub async fn pop_leaf(&self) -> DbResult<Option<Hash>> {
        let this = self.clone();
        spawn_blocking(move || this.pop_leaf_blocking())
            .await
            .map_err(DbError::from)?
    }

    pub fn pop_leaf_blocking(&self) -> DbResult<Option<Hash>> {
        run_with_precondition_retries(self.max_retries, || self.pop_leaf_once_blocking())
    }

    /// Truncates this MMR namespace to `target_leaf_count` leaves atomically.
    ///
    /// Returns [`DbError::MmrIndexOutOfRange`] when `target_leaf_count` is above
    /// the current count. Truncating to the current count is a no-op.
    pub fn truncate_to_leaf_count_blocking(&self, target_leaf_count: u64) -> DbResult<()> {
        run_with_precondition_retries(self.max_retries, || {
            self.truncate_to_leaf_count_once_blocking(target_leaf_count)
        })
    }

    /// Async variant of [`Self::truncate_to_leaf_count_blocking`].
    pub async fn truncate_to_leaf_count(&self, target_leaf_count: u64) -> DbResult<()> {
        let this = self.clone();
        spawn_blocking(move || this.truncate_to_leaf_count_blocking(target_leaf_count))
            .await
            .map_err(DbError::from)?
    }

    fn truncate_to_leaf_count_once_blocking(&self, target_leaf_count: u64) -> DbResult<()> {
        let leaf_count = self.get_leaf_count_blocking()?;
        if target_leaf_count == leaf_count {
            return Ok(());
        }
        if target_leaf_count > leaf_count {
            return Err(DbError::MmrIndexOutOfRange {
                requested: target_leaf_count,
                cur: leaf_count,
            });
        }

        let mmr_id = self.mmr_id_bytes();
        let nodes_to_remove: Vec<NodePos> =
            iter_prune_after_positions(target_leaf_count, leaf_count).collect();
        let leaves_to_remove: Vec<LeafPos> =
            (target_leaf_count..leaf_count).map(LeafPos::new).collect();
        let prefetched = self.fetch_node_paths_blocking(nodes_to_remove.iter().copied(), true)?;
        let node_table = Self::get_scoped_node_table(&prefetched, &mmr_id);
        // The DB fetch returns present path nodes; truncate requires every requested node.
        // Truncate only needs positions, so completeness is verified here at the fetch
        // boundary instead of inside the plan.
        for node_pos in &nodes_to_remove {
            if node_table.get_node(*node_pos).is_none() {
                return Err(DbError::MmrNodeNotFound(*node_pos));
            }
        }

        let mut batch = MmrBatchWrite::from_preconds_table(prefetched);
        let mmr_batch = batch.entry(mmr_id);

        for leaf_pos in leaves_to_remove {
            mmr_batch.add_preimage_precond(leaf_pos, node_table.get_preimage(leaf_pos).cloned());
            mmr_batch.del_preimage(leaf_pos);
        }

        for node_pos in nodes_to_remove {
            mmr_batch.del_node(node_pos);
        }

        mmr_batch.set_expected_leaf_count(leaf_count);
        mmr_batch.set_leaf_count(target_leaf_count);

        self.ops.apply_update_blocking(batch)
    }

    fn pop_leaf_once_blocking(&self) -> DbResult<Option<Hash>> {
        let leaf_count = self.get_leaf_count_blocking()?;
        if leaf_count == 0 {
            return Ok(None);
        }

        let mmr_id = self.mmr_id_bytes();
        let last_leaf = LeafPos::new(leaf_count - 1);

        // Requesting the last leaf is enough: `fetch_node_paths` walks upward
        // from it, so the ancestors this pop deletes arrive in the same read.
        let prefetched = self.fetch_node_paths_blocking([last_leaf.to_node_pos()], true)?;
        let node_table = Self::get_scoped_node_table(&prefetched, &mmr_id);
        let plan = plan_pop(leaf_count, &node_table)?;

        let mut batch = MmrBatchWrite::from_preconds_table(prefetched);
        let mmr_batch = batch.entry(mmr_id);

        // Guard against concurrent preimage writes when we delete this leaf's preimage.
        mmr_batch.add_preimage_precond(
            plan.leaf_pos,
            node_table.get_preimage(plan.leaf_pos).cloned(),
        );
        for node_pos in plan.nodes_to_remove {
            mmr_batch.del_node(node_pos);
        }
        mmr_batch.del_preimage(plan.leaf_pos);
        mmr_batch.set_expected_leaf_count(leaf_count);
        mmr_batch.set_leaf_count(leaf_count - 1);

        self.ops.apply_update_blocking(batch)?;
        Ok(Some(plan.leaf_hash))
    }

    pub fn get_leaf_blocking(&self, leaf_index: u64) -> DbResult<Option<Hash>> {
        self.get_node_blocking(LeafPos::new(leaf_index).to_node_pos())
    }

    pub fn get_node_blocking(&self, pos: NodePos) -> DbResult<Option<Hash>> {
        self.ops.get_node_blocking(self.mmr_id_bytes(), pos)
    }

    pub fn get_mmr_size_blocking(&self) -> DbResult<u64> {
        Ok(num_leaves_to_mmr_size(self.get_leaf_count_blocking()?))
    }

    pub async fn get_leaf_count(&self) -> DbResult<u64> {
        let this = self.clone();
        spawn_blocking(move || this.get_leaf_count_blocking())
            .await
            .map_err(DbError::from)?
    }

    /// Reads raw preimage bytes by leaf index.
    pub fn get_blocking(&self, index: u64) -> DbResult<Vec<u8>> {
        self.ops
            .get_preimage_blocking(self.mmr_id_bytes(), LeafPos::new(index))?
            .ok_or(DbError::MmrPayloadNotFound(LeafPos::new(index)))
    }

    /// Reads raw preimage bytes for `[start, end_exclusive)`.
    pub fn get_range_blocking(&self, start: u64, end_exclusive: u64) -> DbResult<Vec<Vec<u8>>> {
        let len = end_exclusive
            .checked_sub(start)
            .ok_or(DbError::MmrInvalidRange {
                start,
                end: end_exclusive,
            })?;

        if len == 0 {
            return Ok(Vec::new());
        }

        let capacity = usize::try_from(len).map_err(|_| DbError::MmrInvalidRange {
            start,
            end: end_exclusive,
        })?;

        let preimages = self.ops.get_preimage_range_blocking(
            self.mmr_id_bytes(),
            LeafPos::new(start),
            LeafPos::new(end_exclusive),
        )?;
        let mut out = Vec::with_capacity(capacity);
        for (offset, preimage) in preimages.into_iter().enumerate() {
            let idx = start + offset as u64;
            out.push(preimage.ok_or(DbError::MmrPayloadNotFound(LeafPos::new(idx)))?);
        }
        Ok(out)
    }

    /// Reads raw preimage bytes by leaf index.
    pub async fn get(&self, index: u64) -> DbResult<Vec<u8>> {
        let this = self.clone();
        spawn_blocking(move || {
            this.ops
                .get_preimage_blocking(this.mmr_id_bytes(), LeafPos::new(index))?
                .ok_or(DbError::MmrPayloadNotFound(LeafPos::new(index)))
        })
        .await
        .map_err(DbError::from)?
    }

    /// Reads raw preimage bytes for `[start, end_exclusive)`.
    pub async fn get_range(&self, start: u64, end_exclusive: u64) -> DbResult<Vec<Vec<u8>>> {
        let this = self.clone();
        spawn_blocking(move || this.get_range_blocking(start, end_exclusive))
            .await
            .map_err(DbError::from)?
    }

    /// Generates contiguous proofs with leaf-hash validation from one prefetch snapshot.
    pub fn generate_proofs_for(
        &self,
        start: u64,
        expected_hashes: &[Hash],
        at_leaf_count: u64,
    ) -> DbResult<Vec<MerkleProof>> {
        if expected_hashes.is_empty() {
            return Ok(Vec::new());
        }

        let end = start + expected_hashes.len() as u64 - 1;
        if end >= at_leaf_count {
            return Err(DbError::MmrIndexOutOfRange {
                requested: end,
                cur: at_leaf_count,
            });
        }

        let mut positions = collect_proof_positions_for_range(start, end, at_leaf_count);
        positions.extend((start..=end).map(|i| LeafPos::new(i).to_node_pos()));

        let prefetched = self.fetch_node_paths_blocking(positions, false)?;
        let node_table = Self::get_scoped_node_table(&prefetched, &self.mmr_id_bytes());

        for (offset, expected_hash) in expected_hashes.iter().enumerate() {
            let idx = start + offset as u64;
            let actual = node_table
                .get_node(LeafPos::new(idx).to_node_pos())
                .copied()
                .ok_or(DbError::MmrLeafNotFound(idx))?;
            if actual != *expected_hash {
                return Err(DbError::MmrLeafHashMismatch {
                    idx,
                    expected: *expected_hash,
                    got: actual,
                });
            }
        }

        assemble_proofs_from_table(start, end, at_leaf_count, &node_table)
    }

    /// Generates proofs for arbitrary leaf indices with hash validation from one prefetch snapshot.
    pub fn generate_proofs_for_indices(
        &self,
        indices_and_hashes: &[(u64, Hash)],
        at_leaf_count: u64,
    ) -> DbResult<Vec<MerkleProof>> {
        if indices_and_hashes.is_empty() {
            return Ok(Vec::new());
        }

        let mut positions = BTreeSet::new();
        for (idx, _) in indices_and_hashes {
            if *idx >= at_leaf_count {
                return Err(DbError::MmrIndexOutOfRange {
                    requested: *idx,
                    cur: at_leaf_count,
                });
            }
            positions.insert(LeafPos::new(*idx).to_node_pos());
            positions.extend(proof_positions(*idx, at_leaf_count));
        }

        let prefetched = self.fetch_node_paths_blocking(positions, false)?;
        let node_table = Self::get_scoped_node_table(&prefetched, &self.mmr_id_bytes());

        for (idx, expected_hash) in indices_and_hashes {
            let actual = node_table
                .get_node(LeafPos::new(*idx).to_node_pos())
                .copied()
                .ok_or(DbError::MmrLeafNotFound(*idx))?;
            if actual != *expected_hash {
                return Err(DbError::MmrLeafHashMismatch {
                    idx: *idx,
                    expected: *expected_hash,
                    got: actual,
                });
            }
        }

        indices_and_hashes
            .iter()
            .map(|(idx, _)| assemble_proof_from_table(*idx, at_leaf_count, &node_table))
            .collect()
    }

    /// Generates a proof at `at_leaf_count`.
    pub fn generate_proof_at(&self, leaf_index: u64, at_leaf_count: u64) -> DbResult<MerkleProof> {
        if leaf_index >= at_leaf_count {
            return Err(DbError::MmrIndexOutOfRange {
                requested: leaf_index,
                cur: at_leaf_count,
            });
        }

        let prefetched =
            self.fetch_node_paths_blocking(proof_positions(leaf_index, at_leaf_count), false)?;
        let node_table = Self::get_scoped_node_table(&prefetched, &self.mmr_id_bytes());
        assemble_proof_from_table(leaf_index, at_leaf_count, &node_table)
    }

    /// Reads the native MMR state at `at_leaf_count`.
    pub fn get_state_at_blocking(&self, at_leaf_count: u64) -> DbResult<Mmr64B32> {
        let mmr_id = self.mmr_id_bytes();
        let peak_node_positions: Vec<_> = peak_positions(at_leaf_count).collect();
        let prefetched =
            self.fetch_node_paths_blocking(peak_node_positions.iter().copied(), false)?;
        let node_table = Self::get_scoped_node_table(&prefetched, &mmr_id);

        let mut roots = Vec::with_capacity(peak_node_positions.len());
        for peak_pos in peak_node_positions {
            let peak_hash = node_table
                .get_node(peak_pos)
                .copied()
                .ok_or(DbError::MmrNodeNotFound(peak_pos))?;
            roots.push(FixedBytes::from(peak_hash.0));
        }
        // `peak_positions` yields highest-height first; `Mmr64B32` roots are
        // lowest-height first. Construct directly so all-zero roots are preserved.
        roots.reverse();

        Ok(Mmr64B32 {
            entries: at_leaf_count,
            // A `u64` leaf count has at most 64 set bits, so at most 64 peaks —
            // always within the list's capacity.
            roots: roots.try_into().expect("MMR has at most 64 peaks"),
        })
    }

    pub async fn get_state_at(&self, at_leaf_count: u64) -> DbResult<Mmr64B32> {
        let this = self.clone();
        spawn_blocking(move || this.get_state_at_blocking(at_leaf_count))
            .await
            .map_err(DbError::from)?
    }

    pub fn mmr_id(&self) -> &MmrId {
        &self.mmr_id
    }
}

fn is_mmr_precondition_failed(err: &DbError) -> bool {
    matches!(err, DbError::MmrPreconditionFailed { .. })
}

/// Collects the deduplicated proof-path positions for leaves `[start, end]`.
///
/// Proof paths of nearby leaves share ancestors, so the union is prefetched
/// once instead of per leaf. The caller validates the range.
fn collect_proof_positions_for_range(start: u64, end: u64, at_leaf_count: u64) -> Vec<NodePos> {
    (start..=end)
        .flat_map(|leaf_index| proof_positions(leaf_index, at_leaf_count))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Returns the node positions an append reads.
///
/// Appending leaf `leaf_count` recomputes that leaf's ancestors, and
/// recomputing an ancestor requires the sibling it is combined with — the same
/// siblings an inclusion proof for that leaf collects. So the append's read set
/// is the leaf's proof path in the resulting MMR.
fn compute_append_read_positions(leaf_count: u64) -> Vec<NodePos> {
    proof_positions(leaf_count, compute_appended_leaf_count(leaf_count))
}

/// Returns the leaf count an append produces.
fn compute_appended_leaf_count(leaf_count: u64) -> u64 {
    leaf_count.checked_add(1).expect("MMR leaf count overflow")
}

/// Plans the node writes for appending `hash` against a prefetched snapshot.
fn plan_append(hash: Hash, leaf_count: u64, table: &NodeTable) -> DbResult<AppendPlan> {
    let new_leaf_count = compute_appended_leaf_count(leaf_count);

    // `write_plan` reads each sibling on the new leaf's proof path. Ensure they
    // are all present up front so a corrupt store surfaces as `MmrNodeNotFound`
    // here — the same guarantee `plan_pop` makes — which leaves `write_plan`
    // itself infallible (its only failure is a missing node).
    for pos in compute_append_read_positions(leaf_count) {
        require_node_hash(table, pos)?;
    }

    let writes =
        write_plan::<Sha256Hasher, Infallible>(leaf_count, hash.0, new_leaf_count, |pos| {
            Ok(table.get_node(pos).map(|h| h.0))
        })
        .unwrap_or_else(|err| {
            unreachable!("append planning read a node the presence check did not cover: {err}")
        });

    Ok(AppendPlan {
        leaf_pos: LeafPos::new(leaf_count),
        new_leaf_count,
        nodes_to_write: writes
            .into_iter()
            .map(|(pos, node_hash)| (pos, Hash::from(node_hash)))
            .collect(),
    })
}

/// Plans the node deletions for popping the last leaf of `leaf_count`.
///
/// Every deleted node must be present in `table` so the batch can attach a
/// matching precondition before removing it.
fn plan_pop(leaf_count: u64, table: &NodeTable) -> DbResult<PopPlan> {
    let new_leaf_count = leaf_count - 1;
    let leaf_pos = LeafPos::new(new_leaf_count);

    // Truncating to `new_leaf_count` leaves removes exactly the nodes present at
    // `leaf_count` but not at `new_leaf_count`.
    let nodes_to_remove: Vec<NodePos> =
        iter_prune_after_positions(new_leaf_count, leaf_count).collect();

    let leaf_hash = require_node_hash(table, leaf_pos.to_node_pos())?;
    for node_pos in &nodes_to_remove {
        require_node_hash(table, *node_pos)?;
    }

    Ok(PopPlan {
        leaf_pos,
        leaf_hash: Hash::from(leaf_hash),
        nodes_to_remove,
    })
}

/// Reads a node hash out of a prefetched snapshot.
///
/// A position that was prefetched but is absent means the store is missing a
/// node the MMR requires, which is corruption rather than a normal miss.
fn require_node_hash(table: &NodeTable, pos: NodePos) -> DbResult<[u8; 32]> {
    table
        .get_node(pos)
        .map(|h| h.0)
        .ok_or(DbError::MmrNodeNotFound(pos))
}

/// Assembles the inclusion proof for `leaf_index` from a prefetched snapshot.
fn assemble_proof_from_table(
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

/// Assembles proofs for all leaves in `[start, end]` (both inclusive).
fn assemble_proofs_from_table(
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

    (start..=end)
        .map(|leaf_index| assemble_proof_from_table(leaf_index, leaf_count, table))
        .collect()
}

fn run_with_precondition_retries<T, F>(max_retries: usize, mut run: F) -> DbResult<T>
where
    F: FnMut() -> DbResult<T>,
{
    let max_retries = max_retries.max(1);
    let mut last_precondition_err: Option<DbError> = None;

    for attempt in 0..max_retries {
        match run() {
            Ok(value) => return Ok(value),
            Err(err) if is_mmr_precondition_failed(&err) => {
                last_precondition_err = Some(err);
                if attempt + 1 < max_retries {
                    continue;
                }
                break;
            }
            Err(err) => return Err(err),
        }
    }

    Err(DbError::RetriesExhausted {
        attempts: max_retries,
        last_error: Box::new(last_precondition_err.unwrap_or_else(|| {
            DbError::Other("MMR precondition retry loop ended without a captured error".to_string())
        })),
    })
}

#[cfg(test)]
mod tests {
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_merkle::{Mmr, Mmr64B32, MmrState};

    use super::*;

    fn setup_handle() -> MmrIndexHandle {
        setup_manager().get_handle(MmrId::Asm)
    }

    fn setup_manager() -> MmrIndexManager {
        let handle = crate::test_runtime_handle();
        let backend = get_test_sled_backend();
        MmrIndexManager::new(handle, backend.mmr_index_db())
    }

    #[test]
    fn test_every_indexed_namespace_is_discoverable() {
        let manager = setup_manager();
        let asm_handle = manager.get_handle(MmrId::Asm);
        let l1_handle = manager.get_handle(MmrId::L1BlockRefs);

        asm_handle
            .append_leaf_blocking(Hash::from([0x11; 32]))
            .expect("append asm leaf");
        l1_handle
            .append_leaf_blocking(Hash::from([0x22; 32]))
            .expect("append l1 leaf");

        let mut ids = manager.list_mmr_ids_blocking().expect("list mmr ids");
        ids.sort();

        let mut expected = vec![MmrId::Asm.to_bytes(), MmrId::L1BlockRefs.to_bytes()];
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn test_truncate_noops_at_current_leaf_count() {
        let handle = setup_handle();
        handle
            .append_blocking(vec![0x11])
            .expect("append first preimage");
        handle
            .append_blocking(vec![0x22])
            .expect("append second preimage");

        let before = handle
            .get_state_at_blocking(2)
            .expect("state before truncate");
        handle
            .truncate_to_leaf_count_blocking(2)
            .expect("truncate to current count");
        let after = handle
            .get_state_at_blocking(2)
            .expect("state after truncate");

        assert_eq!(before, after);
        assert_eq!(handle.get_leaf_count_blocking().expect("leaf count"), 2);
        assert_eq!(handle.get_blocking(0).expect("first preimage"), vec![0x11]);
        assert_eq!(handle.get_blocking(1).expect("second preimage"), vec![0x22]);
    }

    #[test]
    fn test_truncate_rejects_target_above_current_leaf_count() {
        let handle = setup_handle();
        handle.append_blocking(vec![0x11]).expect("append preimage");

        let err = handle
            .truncate_to_leaf_count_blocking(2)
            .expect_err("target above current count should fail");

        assert!(matches!(
            err,
            DbError::MmrIndexOutOfRange {
                requested: 2,
                cur: 1
            }
        ));
        assert_eq!(handle.get_leaf_count_blocking().expect("leaf count"), 1);
        assert!(handle.get_leaf_blocking(0).expect("leaf").is_some());
    }

    #[test]
    fn test_truncate_removes_tail_and_preserves_multi_peak_prefix() {
        let manager = setup_manager();
        let handle = manager.get_handle(MmrId::Asm);
        let prefix_handle = manager.get_handle(MmrId::L1BlockRefs);
        let payloads = (0u8..9).map(|byte| vec![byte]).collect::<Vec<_>>();
        let target_leaf_count = 6;

        for payload in payloads.iter().cloned() {
            handle.append_blocking(payload).expect("append full MMR");
        }
        for payload in payloads[..target_leaf_count].iter().cloned() {
            prefix_handle
                .append_blocking(payload)
                .expect("append prefix MMR");
        }
        let prefix_leaf_hashes = (0..target_leaf_count as u64)
            .map(|idx| {
                handle
                    .get_leaf_blocking(idx)
                    .expect("read prefix leaf")
                    .expect("prefix leaf should exist")
            })
            .collect::<Vec<_>>();

        handle
            .truncate_to_leaf_count_blocking(target_leaf_count as u64)
            .expect("truncate to prefix");

        assert_eq!(
            handle.get_leaf_count_blocking().expect("leaf count"),
            target_leaf_count as u64
        );
        assert_eq!(
            handle
                .get_state_at_blocking(target_leaf_count as u64)
                .expect("truncated state"),
            prefix_handle
                .get_state_at_blocking(target_leaf_count as u64)
                .expect("fresh prefix state"),
        );
        for (idx, expected_hash) in prefix_leaf_hashes.into_iter().enumerate() {
            assert_eq!(
                handle
                    .get_leaf_blocking(idx as u64)
                    .expect("read preserved leaf"),
                Some(expected_hash)
            );
            assert_eq!(
                handle
                    .get_blocking(idx as u64)
                    .expect("read prefix preimage"),
                payloads[idx]
            );
        }
        for idx in target_leaf_count..payloads.len() {
            assert_eq!(
                handle
                    .get_leaf_blocking(idx as u64)
                    .expect("read removed leaf"),
                None
            );
            assert!(matches!(
                handle.get_blocking(idx as u64),
                Err(DbError::MmrPayloadNotFound(pos)) if pos == LeafPos::new(idx as u64)
            ));
        }

        let next_payload = vec![0xff];
        handle
            .append_blocking(next_payload.clone())
            .expect("append after truncate");
        prefix_handle
            .append_blocking(next_payload)
            .expect("append to fresh prefix");
        assert_eq!(
            handle.get_state_at_blocking(7).expect("state after append"),
            prefix_handle
                .get_state_at_blocking(7)
                .expect("fresh prefix after append"),
        );
    }

    #[test]
    fn test_truncate_can_empty_namespace() {
        let handle = setup_handle();
        for byte in 0u8..3 {
            handle.append_blocking(vec![byte]).expect("append preimage");
        }

        handle
            .truncate_to_leaf_count_blocking(0)
            .expect("truncate to empty");

        assert_eq!(handle.get_leaf_count_blocking().expect("leaf count"), 0);
        assert_eq!(
            handle.get_state_at_blocking(0).expect("empty state"),
            Mmr64B32::new_empty(),
        );
        assert_eq!(
            handle.get_leaf_blocking(0).expect("removed first leaf"),
            None
        );
        assert!(matches!(
            handle.get_blocking(0),
            Err(DbError::MmrPayloadNotFound(pos)) if pos == LeafPos::new(0)
        ));
        assert_eq!(
            handle
                .append_blocking(vec![0xaa])
                .expect("append after empty truncate"),
            0
        );
    }

    #[test]
    fn test_truncate_handles_hash_only_leaves() {
        let handle = setup_handle();
        let hashes = [
            Hash::from([0x11; 32]),
            Hash::from([0x22; 32]),
            Hash::from([0x33; 32]),
        ];

        for hash in hashes {
            handle
                .append_leaf_blocking(hash)
                .expect("append hash-only leaf");
        }

        handle
            .truncate_to_leaf_count_blocking(1)
            .expect("truncate hash-only leaves");

        assert_eq!(handle.get_leaf_count_blocking().expect("leaf count"), 1);
        assert_eq!(
            handle.get_leaf_blocking(0).expect("read preserved leaf"),
            Some(hashes[0])
        );
        assert_eq!(
            handle.get_leaf_blocking(1).expect("read removed leaf"),
            None
        );
        assert!(matches!(
            handle.get_blocking(0),
            Err(DbError::MmrPayloadNotFound(pos)) if pos == LeafPos::new(0)
        ));
        assert_eq!(
            handle
                .append_leaf_blocking(Hash::from([0x44; 32]))
                .expect("append after hash-only truncate"),
            1
        );
    }

    #[test]
    fn test_range_read_returns_contiguous_preimages() {
        let handle = setup_handle();
        let payloads = [vec![0x11], vec![0x22], vec![0x33]];

        for payload in payloads.iter().cloned() {
            handle.append_blocking(payload).expect("append preimage");
        }

        assert_eq!(
            handle.get_range_blocking(0, 3).expect("get full range"),
            payloads
        );
        assert_eq!(
            handle.get_range_blocking(1, 2).expect("get single range"),
            vec![payloads[1].clone()]
        );
    }

    #[tokio::test]
    async fn test_async_range_read_returns_contiguous_preimages() {
        let handle = setup_handle();
        let payloads = [vec![0xaa], vec![0xbb]];

        for payload in payloads.iter().cloned() {
            handle.append_blocking(payload).expect("append preimage");
        }

        assert_eq!(
            handle.get_range(0, 2).await.expect("get async range"),
            payloads
        );
    }

    #[tokio::test]
    async fn test_async_state_read_matches_blocking_read_for_multi_peak_mmr() {
        let handle = setup_handle();
        for payload in (0u8..6).map(|byte| vec![byte]) {
            handle.append_blocking(payload).expect("append preimage");
        }

        assert_eq!(
            handle.get_leaf_count().await.expect("async leaf count"),
            handle
                .get_leaf_count_blocking()
                .expect("blocking leaf count")
        );
        assert_eq!(
            handle.get_state_at(6).await.expect("async state"),
            handle.get_state_at_blocking(6).expect("blocking state"),
        );
    }

    #[test]
    fn test_empty_range_reads_nothing() {
        let handle = setup_handle();

        assert_eq!(
            handle
                .get_range_blocking(7, 7)
                .expect("empty range should succeed"),
            Vec::<Vec<u8>>::new()
        );
    }

    #[test]
    fn test_range_read_fails_when_a_preimage_is_missing() {
        let handle = setup_handle();
        handle
            .append_blocking(vec![0x11])
            .expect("append preimage at 0");
        handle
            .append_leaf_blocking(Hash::from([0x22; 32]))
            .expect("append leaf without preimage at 1");
        handle
            .append_blocking(vec![0x33])
            .expect("append preimage at 2");

        let err = handle
            .get_range_blocking(0, 3)
            .expect_err("missing preimage should fail");
        assert!(matches!(
            err,
            DbError::MmrPayloadNotFound(pos) if pos == LeafPos::new(1)
        ));
    }

    #[test]
    fn test_inverted_range_read_is_rejected() {
        let handle = setup_handle();

        let err = handle
            .get_range_blocking(4, 2)
            .expect_err("invalid range should fail");
        assert!(matches!(err, DbError::MmrInvalidRange { start: 4, end: 2 }));
    }

    /// Appends `count` leaves carrying preimage `[i]`, returning their hashes.
    ///
    /// A count of 7 gives a three-peak MMR (4 + 2 + 1), so tests exercise the
    /// peak-crossing cases rather than a single perfect tree.
    fn append_leaves_with_preimages(handle: &MmrIndexHandle, count: u64) -> Vec<Hash> {
        (0..count)
            .map(|index| {
                let preimage = vec![index as u8];
                handle
                    .append_blocking(preimage.clone())
                    .expect("append preimage");
                Hash::from(Sha256Hasher::hash_leaf(&preimage))
            })
            .collect()
    }

    /// Builds an in-memory MMR over `leaf_hashes` to check proofs against.
    fn build_reference_mmr(leaf_hashes: &[Hash]) -> Mmr64B32 {
        let mut mmr = Mmr64B32::new_empty();
        for hash in leaf_hashes {
            Mmr::<Sha256Hasher>::add_leaf(&mut mmr, hash.0).expect("append reference leaf");
        }
        mmr
    }

    #[test]
    fn test_append_stores_leaf_and_preimage_together() {
        let handle = setup_handle();
        let hashes = append_leaves_with_preimages(&handle, 7);

        // The node write and the preimage write share one commit, so neither is
        // observable without the other.
        for (index, hash) in hashes.iter().enumerate() {
            let index = index as u64;
            assert_eq!(
                handle.get_leaf_blocking(index).expect("read leaf"),
                Some(*hash),
                "leaf {index} hash"
            );
            assert_eq!(
                handle.get_blocking(index).expect("read preimage"),
                vec![index as u8],
                "leaf {index} preimage"
            );
        }
        assert_eq!(handle.get_leaf_count_blocking().expect("leaf count"), 7);
    }

    #[test]
    fn test_pop_removes_last_leaf_and_its_preimage() {
        let handle = setup_handle();
        let hashes = append_leaves_with_preimages(&handle, 7);

        // Leaf 6 is a lone peak at count 7; popping it crosses a peak boundary.
        let popped = handle.pop_leaf_blocking().expect("pop").expect("some leaf");

        assert_eq!(popped, hashes[6]);
        assert_eq!(handle.get_leaf_count_blocking().expect("leaf count"), 6);
        assert_eq!(handle.get_leaf_blocking(6).expect("read popped leaf"), None);
        assert!(matches!(
            handle
                .get_blocking(6)
                .expect_err("popped preimage should be gone"),
            DbError::MmrPayloadNotFound(pos) if pos == LeafPos::new(6)
        ));

        // Surviving leaves and their preimages are untouched.
        for (index, hash) in hashes.iter().enumerate().take(6) {
            let index = index as u64;
            assert_eq!(
                handle.get_leaf_blocking(index).expect("read leaf"),
                Some(*hash)
            );
            assert_eq!(
                handle.get_blocking(index).expect("read preimage"),
                vec![index as u8]
            );
        }
    }

    #[test]
    fn test_pop_leaves_state_equal_to_a_shorter_mmr() {
        let handle = setup_handle();
        append_leaves_with_preimages(&handle, 7);
        handle.pop_leaf_blocking().expect("pop").expect("some leaf");

        // An MMR built directly at the shortened length must be indistinguishable.
        let reference = setup_handle();
        append_leaves_with_preimages(&reference, 6);

        let popped_state = handle.get_state_at_blocking(6).expect("popped state");
        let reference_state = reference.get_state_at_blocking(6).expect("reference state");

        assert_eq!(popped_state, reference_state);
    }

    #[test]
    fn test_generated_proofs_verify_against_an_equivalent_mmr() {
        let handle = setup_handle();
        let hashes = append_leaves_with_preimages(&handle, 7);
        let reference = build_reference_mmr(&hashes);

        for (index, hash) in hashes.iter().enumerate() {
            let proof = handle
                .generate_proof_at(index as u64, 7)
                .expect("generate proof");
            assert!(
                reference.verify(&proof, &hash.0),
                "proof for leaf {index} should verify"
            );
        }
    }

    #[test]
    fn test_generated_proofs_do_not_verify_for_the_wrong_leaf() {
        let handle = setup_handle();
        let hashes = append_leaves_with_preimages(&handle, 7);
        let reference = build_reference_mmr(&hashes);

        let proof = handle.generate_proof_at(2, 7).expect("generate proof");

        assert!(
            !reference.verify(&proof, &hashes[3].0),
            "leaf 2's proof should not verify leaf 3"
        );
    }

    #[test]
    fn test_proof_requests_past_leaf_count_are_rejected() {
        let handle = setup_handle();
        append_leaves_with_preimages(&handle, 7);

        assert!(matches!(
            handle
                .generate_proof_at(7, 7)
                .expect_err("index at leaf count should fail"),
            DbError::MmrIndexOutOfRange {
                requested: 7,
                cur: 7
            }
        ));
    }

    #[test]
    fn test_proofs_for_a_contiguous_range_verify() {
        let handle = setup_handle();
        let hashes = append_leaves_with_preimages(&handle, 7);
        let reference = build_reference_mmr(&hashes);

        let proofs = handle
            .generate_proofs_for(2, &hashes[2..5], 7)
            .expect("generate contiguous proofs");

        assert_eq!(proofs.len(), 3);
        for (offset, proof) in proofs.iter().enumerate() {
            let index = 2 + offset;
            assert!(
                reference.verify(proof, &hashes[index].0),
                "proof for leaf {index} should verify"
            );
        }
    }

    #[test]
    fn test_proofs_are_refused_when_a_claimed_leaf_hash_is_wrong() {
        let handle = setup_handle();
        let hashes = append_leaves_with_preimages(&handle, 7);
        let mut claimed = hashes[2..5].to_vec();
        claimed[1] = Hash::from([0xff; 32]);

        assert!(matches!(
            handle
                .generate_proofs_for(2, &claimed, 7)
                .expect_err("mismatched hash should fail"),
            DbError::MmrLeafHashMismatch { idx: 3, .. }
        ));
    }

    #[test]
    fn test_a_proof_range_past_leaf_count_is_rejected() {
        let handle = setup_handle();
        let hashes = append_leaves_with_preimages(&handle, 7);

        assert!(matches!(
            handle
                .generate_proofs_for(5, &hashes[5..7], 6)
                .expect_err("range past leaf count should fail"),
            DbError::MmrIndexOutOfRange {
                requested: 6,
                cur: 6
            }
        ));
    }

    #[test]
    fn test_proofs_for_arbitrary_leaf_indices_verify() {
        let handle = setup_handle();
        let hashes = append_leaves_with_preimages(&handle, 7);
        let reference = build_reference_mmr(&hashes);
        let requested = [(5u64, hashes[5]), (0, hashes[0]), (3, hashes[3])];

        let proofs = handle
            .generate_proofs_for_indices(&requested, 7)
            .expect("generate indexed proofs");

        assert_eq!(proofs.len(), requested.len());
        for (proof, (index, hash)) in proofs.iter().zip(requested) {
            assert!(
                reference.verify(proof, &hash.0),
                "proof for leaf {index} should verify"
            );
        }
    }

    #[test]
    fn test_a_requested_leaf_index_past_leaf_count_is_rejected() {
        let handle = setup_handle();
        let hashes = append_leaves_with_preimages(&handle, 7);

        assert!(matches!(
            handle
                .generate_proofs_for_indices(&[(1, hashes[1]), (7, hashes[0])], 7)
                .expect_err("index at leaf count should fail"),
            DbError::MmrIndexOutOfRange {
                requested: 7,
                cur: 7
            }
        ));
    }

    #[test]
    fn test_state_matches_a_natively_built_multi_peak_mmr() {
        let handle = setup_handle();
        // Seven leaves give three peaks (4 + 2 + 1), so both multi-peak
        // reconstruction and native root order are exercised.
        let leaves: Vec<Hash> = (0..7u8).map(|i| Hash::from([i + 1; 32])).collect();
        for leaf in &leaves {
            handle.append_leaf_blocking(*leaf).expect("append leaf");
        }

        let reference = build_reference_mmr(&leaves);
        assert_eq!(reference.roots.len(), 3, "fixture should have three peaks");

        let state = handle.get_state_at_blocking(7).expect("state");
        // Whole-value equality also pins root order (lowest-height first), not
        // the highest-first order the peaks are read in.
        assert_eq!(state, reference);
    }

    #[test]
    fn test_state_reconstructs_an_all_zero_leaf_hash() {
        let handle = setup_handle();
        let zero = Hash::from([0u8; 32]);
        handle.append_leaf_blocking(zero).expect("append zero leaf");

        // Construct the expected shape directly because the native mutation API
        // treats zero roots as unset.
        let state = handle
            .get_state_at_blocking(1)
            .expect("state with zero leaf");
        assert_eq!(state.entries, 1);
        assert_eq!(state.roots.len(), 1);
        assert_eq!(state.roots[0], FixedBytes::from([0u8; 32]));
    }
}
