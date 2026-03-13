//! High-level manager for MMR index database access.

use std::{collections::BTreeSet, sync::Arc};

use strata_db_types::{
    num_leaves_to_mmr_size, traits::MmrIndexDatabase, DbError, DbResult, LeafPos, MmrBatchWrite,
    MmrId, MmrNodePos, MmrNodeTable, NodePos, NodeTable, RawMmrId,
};
use strata_identifiers::Hash;
use strata_merkle::{MerkleHasher, MerkleProofB32 as MerkleProof, Sha256Hasher};
use threadpool::ThreadPool;
use tokio::task::spawn_blocking;

use super::mmr_algorithm;
use crate::ops::mmr_index::{Context, MmrIndexOps};

fn is_mmr_precondition_failed(err: &DbError) -> bool {
    matches!(err, DbError::MmrPreconditionFailed { .. })
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

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
#[derive(Clone)]
pub struct MmrIndexManager {
    ops: Arc<MmrIndexOps>,
    config: MmrIndexManagerConfig,
}

/// One append operation for a specific MMR namespace.
#[derive(Debug, Clone)]
pub struct MmrAppendRequest {
    pub mmr_id: MmrId,
    pub hash: Hash,
    pub preimage: Option<Vec<u8>>,
}

impl MmrIndexManager {
    pub fn new(pool: ThreadPool, db: Arc<impl MmrIndexDatabase + 'static>) -> Self {
        Self::with_config(pool, db, MmrIndexManagerConfig::default())
    }

    pub fn with_config(
        pool: ThreadPool,
        db: Arc<impl MmrIndexDatabase + 'static>,
        config: MmrIndexManagerConfig,
    ) -> Self {
        let ops = Arc::new(Context::new(db).into_ops(pool));
        Self { ops, config }
    }

    pub fn with_max_retries(
        pool: ThreadPool,
        db: Arc<impl MmrIndexDatabase + 'static>,
        max_retries: usize,
    ) -> Self {
        let config = MmrIndexManagerConfig {
            retry: MmrIndexRetryConfig {
                max_precondition_retries: max_retries,
            },
        };
        Self::with_config(pool, db, config)
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

            for pos in mmr_algorithm::compute_append_fetch_positions(leaf_count) {
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

        for (request, (mmr_id, leaf_count)) in requests.iter().zip(leaf_counts.into_iter()) {
            let node_table = MmrIndexHandle::get_scoped_node_table(&prefetched, &mmr_id);
            let append_plan =
                mmr_algorithm::compute_append_plan(request.hash.0, leaf_count, &node_table)?;

            let mmr_batch = batch.entry(mmr_id);
            mmr_batch.add_node_precond(append_plan.leaf_pos.node_pos(), None);
            mmr_batch.set_expected_leaf_count(leaf_count);
            mmr_batch.set_leaf_count(leaf_count + 1);

            for (node_pos, node_hash) in append_plan.nodes_to_write {
                mmr_batch.put_node(node_pos, node_hash);
            }

            if let Some(preimage) = request.preimage.clone() {
                mmr_batch.add_preimage_precond(append_plan.leaf_pos, None);
                mmr_batch.put_preimage(append_plan.leaf_pos, preimage);
            }

            appended_indexes.push(append_plan.leaf_pos.index());
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
            .map_err(|_| DbError::WorkerFailedStrangely)?
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

        let prefetched = self.fetch_node_paths_blocking(
            mmr_algorithm::compute_append_fetch_positions(leaf_count),
            false,
        )?;
        let node_table = Self::get_scoped_node_table(&prefetched, &mmr_id);
        let result = mmr_algorithm::compute_append_plan(hash.0, leaf_count, &node_table)?;

        let mut batch = MmrBatchWrite::from_preconds_table(prefetched);
        let mmr_batch = batch.entry(mmr_id);

        mmr_batch.add_node_precond(result.leaf_pos.node_pos(), None);
        mmr_batch.set_expected_leaf_count(leaf_count);
        mmr_batch.set_leaf_count(leaf_count + 1);
        for (node_pos, node_hash) in result.nodes_to_write {
            mmr_batch.put_node(node_pos, node_hash);
        }

        if let Some(preimage) = preimage {
            mmr_batch.add_preimage_precond(result.leaf_pos, None);
            mmr_batch.put_preimage(result.leaf_pos, preimage);
        }

        self.ops.apply_update_blocking(batch)?;
        Ok(result.leaf_pos.index())
    }

    pub async fn append_leaf(&self, hash: Hash) -> DbResult<u64> {
        let this = self.clone();
        spawn_blocking(move || this.append_leaf_blocking(hash))
            .await
            .map_err(|_| DbError::WorkerFailedStrangely)?
    }

    pub fn append_leaf_blocking(&self, hash: Hash) -> DbResult<u64> {
        run_with_precondition_retries(self.max_retries, || {
            self.append_leaf_once_blocking(hash, None)
        })
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
            .map_err(|_| DbError::WorkerFailedStrangely)?
    }

    pub async fn pop_leaf(&self) -> DbResult<Option<Hash>> {
        let this = self.clone();
        spawn_blocking(move || this.pop_leaf_blocking())
            .await
            .map_err(|_| DbError::WorkerFailedStrangely)?
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
        let prefetched = self.fetch_node_paths_blocking(
            mmr_algorithm::compute_pop_fetch_positions(leaf_count),
            true,
        )?;
        let node_table = Self::get_scoped_node_table(&prefetched, &mmr_id);

        let Some(result) = mmr_algorithm::compute_pop_plan(leaf_count, &node_table)? else {
            return Ok(None);
        };

        let mut batch = MmrBatchWrite::from_preconds_table(prefetched);
        let mmr_batch = batch.entry(mmr_id);

        // Guard against concurrent preimage writes when we delete this leaf's preimage.
        mmr_batch.add_preimage_precond(
            result.leaf_pos,
            node_table.get_preimage(result.leaf_pos).cloned(),
        );
        for node_pos in result.nodes_to_remove {
            mmr_batch.del_node(node_pos);
        }
        mmr_batch.del_preimage(result.leaf_pos);
        mmr_batch.set_expected_leaf_count(leaf_count);
        mmr_batch.set_leaf_count(leaf_count - 1);

        self.ops.apply_update_blocking(batch)?;
        Ok(Some(result.leaf_hash))
    }

    pub fn get_leaf_blocking(&self, leaf_index: u64) -> DbResult<Option<Hash>> {
        self.get_node_blocking(LeafPos::new(leaf_index).node_pos())
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

    /// Reads raw preimage bytes by leaf index.
    pub async fn get(&self, index: u64) -> DbResult<Vec<u8>> {
        let this = self.clone();
        spawn_blocking(move || {
            this.ops
                .get_preimage_blocking(this.mmr_id_bytes(), LeafPos::new(index))?
                .ok_or(DbError::MmrPayloadNotFound(LeafPos::new(index)))
        })
        .await
        .map_err(|_| DbError::WorkerFailedStrangely)?
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

        let mut positions =
            mmr_algorithm::compute_proofs_fetch_positions(start, end, at_leaf_count)?;
        positions.extend((start..=end).map(|i| LeafPos::new(i).node_pos()));

        let prefetched = self.fetch_node_paths_blocking(positions, false)?;
        let node_table = Self::get_scoped_node_table(&prefetched, &self.mmr_id_bytes());

        for (offset, expected_hash) in expected_hashes.iter().enumerate() {
            let idx = start + offset as u64;
            let actual = node_table
                .get_node(LeafPos::new(idx).node_pos())
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

        mmr_algorithm::generate_proofs(start, end, at_leaf_count, &node_table)
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
            positions.insert(LeafPos::new(*idx).node_pos());
            for pos in mmr_algorithm::compute_proof_fetch_positions(*idx, at_leaf_count)? {
                positions.insert(pos);
            }
        }

        let prefetched = self.fetch_node_paths_blocking(positions, false)?;
        let node_table = Self::get_scoped_node_table(&prefetched, &self.mmr_id_bytes());

        for (idx, expected_hash) in indices_and_hashes {
            let actual = node_table
                .get_node(LeafPos::new(*idx).node_pos())
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
            .map(|(idx, _)| mmr_algorithm::generate_proof(*idx, at_leaf_count, &node_table))
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

        let prefetched = self.fetch_node_paths_blocking(
            mmr_algorithm::compute_proof_fetch_positions(leaf_index, at_leaf_count)?,
            false,
        )?;
        let node_table = Self::get_scoped_node_table(&prefetched, &self.mmr_id_bytes());
        mmr_algorithm::generate_proof(leaf_index, at_leaf_count, &node_table)
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
            mmr_algorithm::compute_proofs_fetch_positions(start, end, at_leaf_count)?,
            false,
        )?;
        let node_table = Self::get_scoped_node_table(&prefetched, &self.mmr_id_bytes());
        mmr_algorithm::generate_proofs(start, end, at_leaf_count, &node_table)
    }

    pub fn get_state_at(&self, at_leaf_count: u64) -> DbResult<MmrStateView> {
        let mmr_id = self.mmr_id_bytes();
        let peak_positions = mmr_algorithm::compute_peak_positions(at_leaf_count);
        let prefetched = self.fetch_node_paths_blocking(peak_positions.iter().copied(), false)?;
        let node_table = Self::get_scoped_node_table(&prefetched, &mmr_id);

        let mut peaks = Vec::with_capacity(peak_positions.len());
        for peak_pos in peak_positions {
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
