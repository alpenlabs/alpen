//! High-level manager for MMR index database access.

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::sync::Arc;

use strata_db_types::mmr_index::MmrIndexDatabase;
use strata_db_types::{
    num_leaves_to_mmr_size, DbError, DbResult, LeafPos, MmrBatchWrite, MmrId, MmrNodePos,
    MmrNodeTable, NodePos, NodeTable, RawMmrId,
};
use strata_identifiers::Hash;
use strata_merkle::{MerkleHasher, MerkleProofB32 as MerkleProof, Sha256Hasher};
use strata_merkle_node_store::{
    assemble_proof, iter_prune_after_positions, peak_positions, proof_positions, write_plan,
};
use tokio::runtime::Handle;
use tokio::task::spawn_blocking;

use crate::ops::mmr_index::MmrIndexOps;

/// Read-only view of MMR state at a specific leaf count.
#[derive(Debug, Clone)]
pub struct MmrStateView {
    pub leaf_count: u64,
    pub peaks: Vec<Hash>,
}

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

/// One append operation for a specific MMR namespace.
#[derive(Debug, Clone)]
pub struct MmrAppendRequest {
    pub mmr_id: MmrId,
    pub hash: Hash,
    pub preimage: Option<Vec<u8>>,
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

    fn get_leaf_count_for_mmr_blocking(&self, mmr_id: &RawMmrId) -> DbResult<u64> {
        self.ops.get_leaf_count_blocking(mmr_id.clone())
    }

    /// Appends one leaf per distinct MMR namespace in a single read+write cycle.
    ///
    /// This API aggregates all required node positions across MMRs, performs
    /// one batched `fetch_node_paths`, computes append plans in memory, then
    /// applies one atomic `apply_update`.
    fn append_many_once_blocking(&self, requests: &[MmrAppendRequest]) -> DbResult<Vec<u64>> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }

        let mut leaf_counts = Vec::with_capacity(requests.len());
        let mut scoped_fetch_positions = BTreeSet::new();
        let mut seen_mmr_ids = BTreeSet::new();

        for request in requests {
            let mmr_id = request.mmr_id.to_bytes();
            if !seen_mmr_ids.insert(mmr_id.clone()) {
                return Err(DbError::Other(
                    "append_many_blocking requires distinct MMR IDs in one call".to_string(),
                ));
            }

            let leaf_count = self.get_leaf_count_for_mmr_blocking(&mmr_id)?;
            leaf_counts.push((mmr_id.clone(), leaf_count));

            let read_positions = compute_append_read_positions(leaf_count);
            for pos in read_positions {
                scoped_fetch_positions.insert(MmrNodePos::new(mmr_id.clone(), pos));
            }
        }

        // One batched read for all MMR append dependencies.
        let scoped_positions = scoped_fetch_positions.into_iter().collect::<Vec<_>>();
        let prefetched = self
            .ops
            .fetch_node_paths_blocking(scoped_positions, false)?;

        let mut batch = MmrBatchWrite::from_preconds_table(prefetched.clone());
        let mut appended_indexes = Vec::with_capacity(requests.len());

        for (request, (mmr_id, leaf_count)) in requests.iter().zip(leaf_counts) {
            let node_table = MmrIndexHandle::get_scoped_node_table(&prefetched, &mmr_id);
            let plan = plan_append(request.hash, leaf_count, &node_table)?;

            let mmr_batch = batch.entry(mmr_id);
            mmr_batch.add_node_precond(plan.leaf_pos.to_node_pos(), None);
            mmr_batch.set_expected_leaf_count(leaf_count);
            mmr_batch.set_leaf_count(plan.new_leaf_count);

            for (node_pos, node_hash) in plan.nodes_to_write {
                mmr_batch.put_node(node_pos, node_hash);
            }

            if let Some(preimage) = request.preimage.clone() {
                mmr_batch.add_preimage_precond(plan.leaf_pos, None);
                mmr_batch.put_preimage(plan.leaf_pos, preimage);
            }

            appended_indexes.push(plan.leaf_pos.index());
        }

        // One batched write for all MMR updates.
        self.ops.apply_update_blocking(batch)?;
        Ok(appended_indexes)
    }

    /// Appends one leaf per distinct MMR namespace in a single read+write cycle.
    ///
    /// Retries boundedly on MMR precondition failures to handle concurrent writers.
    pub fn append_many_blocking(&self, requests: Vec<MmrAppendRequest>) -> DbResult<Vec<u64>> {
        run_with_precondition_retries(self.config.retry.max_precondition_retries, || {
            self.append_many_once_blocking(&requests)
        })
    }

    /// Async wrapper for [`Self::append_many_blocking`].
    pub async fn append_many(&self, requests: Vec<MmrAppendRequest>) -> DbResult<Vec<u64>> {
        let this = self.clone();
        spawn_blocking(move || this.append_many_blocking(requests))
            .await
            .map_err(DbError::from)?
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

    fn get_leaf_count_blocking(&self) -> DbResult<u64> {
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
        let leaf_count = self.get_num_leaves_blocking()?;

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

    pub fn get_num_leaves_blocking(&self) -> DbResult<u64> {
        self.get_leaf_count_blocking()
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

    /// Generates proofs for all leaves in `[start, end]` (both inclusive) at
    /// `at_leaf_count`.
    pub fn generate_proofs_at(
        &self,
        start: u64,
        end: u64,
        at_leaf_count: u64,
    ) -> DbResult<Vec<MerkleProof>> {
        if start > end {
            return Err(DbError::MmrInvalidRange { start, end });
        }

        if end >= at_leaf_count {
            return Err(DbError::MmrIndexOutOfRange {
                requested: end,
                cur: at_leaf_count,
            });
        }

        let prefetched = self.fetch_node_paths_blocking(
            collect_proof_positions_for_range(start, end, at_leaf_count),
            false,
        )?;
        let node_table = Self::get_scoped_node_table(&prefetched, &self.mmr_id_bytes());
        assemble_proofs_from_table(start, end, at_leaf_count, &node_table)
    }

    pub fn get_state_at(&self, at_leaf_count: u64) -> DbResult<MmrStateView> {
        let mmr_id = self.mmr_id_bytes();
        let peak_node_positions: Vec<_> = peak_positions(at_leaf_count).collect();
        let prefetched =
            self.fetch_node_paths_blocking(peak_node_positions.iter().copied(), false)?;
        let node_table = Self::get_scoped_node_table(&prefetched, &mmr_id);

        let mut peaks = Vec::with_capacity(peak_node_positions.len());
        for peak_pos in peak_node_positions {
            let peak_hash = node_table
                .get_node(peak_pos)
                .copied()
                .ok_or(DbError::MmrNodeNotFound(peak_pos))?;
            peaks.push(peak_hash);
        }

        Ok(MmrStateView {
            leaf_count: at_leaf_count,
            peaks,
        })
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
        assert_eq!(handle.get_num_leaves_blocking().expect("leaf count"), 7);
    }

    #[test]
    fn test_pop_removes_last_leaf_and_its_preimage() {
        let handle = setup_handle();
        let hashes = append_leaves_with_preimages(&handle, 7);

        // Leaf 6 is a lone peak at count 7; popping it crosses a peak boundary.
        let popped = handle.pop_leaf_blocking().expect("pop").expect("some leaf");

        assert_eq!(popped, hashes[6]);
        assert_eq!(handle.get_num_leaves_blocking().expect("leaf count"), 6);
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

        let popped_state = handle.get_state_at(6).expect("popped state");
        let reference_state = reference.get_state_at(6).expect("reference state");

        assert_eq!(popped_state.leaf_count, reference_state.leaf_count);
        assert_eq!(popped_state.peaks, reference_state.peaks);
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
}
