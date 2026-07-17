//! EE prover facade backed by separate chunk and acct paas provers.
//!
//! Chunk proof submission is driven by the chunk lifecycle as soon as chunks are sealed.
//! Acct proof submission is requested by the batch lifecycle after DA is complete. This facade
//! accepts the request only once the acct proof inputs are ready.
//!
//! `check_proof_status(batch_id)` peeks the typed
//! [`EeBatchProofDbManager`] first (proof present → `Ready`); on miss
//! it maps `acct_handle.get_status(BatchTask)` to
//! [`ProofGenerationStatus`].

use std::sync::Arc;

use alpen_ee_common::{
    BatchId, BatchProver, BatchStorage, ChunkId, ChunkProver, ChunkStatus, ChunkStorage, Proof,
    ProofGenerationStatus, ProofId, ProofRequestStatus,
};
use async_trait::async_trait;
use strata_paas::{ProofSpec, ProverError as PaasError, ProverHandle, TaskStatus};
use tracing::{debug, info, warn};

use super::{BatchTask, ChunkTask, EeBatchProofDbManager};

/// New-paas-backed EE prover facade.
///
/// Generic over the chunk and acct [`ProofSpec`]s (production uses
/// `ChunkSpec` / `AcctSpec`) so tests can drive the facade with lightweight
/// mock specs instead of the full storage/witness wiring.
pub(crate) struct PaasEeProver<CS, AS>
where
    CS: ProofSpec<Task = ChunkTask>,
    AS: ProofSpec<Task = BatchTask>,
{
    chunk_handle: ProverHandle<CS>,
    acct_handle: ProverHandle<AS>,
    batch_storage: Arc<dyn BatchStorage>,
    chunk_storage: Arc<dyn ChunkStorage>,
    batch_proofs: Arc<EeBatchProofDbManager>,
}

impl<CS, AS> PaasEeProver<CS, AS>
where
    CS: ProofSpec<Task = ChunkTask>,
    AS: ProofSpec<Task = BatchTask>,
{
    pub(crate) fn new(
        chunk_handle: ProverHandle<CS>,
        acct_handle: ProverHandle<AS>,
        batch_storage: Arc<dyn BatchStorage>,
        chunk_storage: Arc<dyn ChunkStorage>,
        batch_proofs: Arc<EeBatchProofDbManager>,
    ) -> Self {
        Self {
            chunk_handle,
            acct_handle,
            batch_storage,
            chunk_storage,
            batch_proofs,
        }
    }

    /// Guards against a Completed chunk task whose receipt is missing.
    ///
    /// Such a task can never make progress on its own: `submit` short-circuits
    /// on the existing record and the receipt is only written by a fresh
    /// proving run, so a receipt-less Completed task would wedge the chunk in
    /// a `ProofReady`/`ProofPending` oscillation while the acct proof waits
    /// forever. Resets the stale task so the chunk lifecycle resubmits it.
    ///
    /// Returns `true` when the receipt is missing (task reset, chunk must
    /// re-prove), `false` when the receipt is present.
    fn reset_receiptless_chunk_task(&self, task: &ChunkTask) -> eyre::Result<bool> {
        let chunk_id = task.0;
        if self
            .chunk_handle
            .get_receipt(task)
            .map_err(|e| eyre::eyre!("get chunk receipt {chunk_id:?}: {e}"))?
            .is_some()
        {
            return Ok(false);
        }

        let removed = self
            .chunk_handle
            .reset_task(task)
            .map_err(|e| eyre::eyre!("reset chunk task {chunk_id:?}: {e}"))?;
        if removed {
            warn!(
                ?chunk_id,
                "chunk task Completed but receipt missing; reset task for re-proof"
            );
        } else {
            debug!(
                ?chunk_id,
                "chunk task no longer terminal; leaving in-flight attempt to regenerate receipt"
            );
        }
        Ok(true)
    }

    async fn observe_existing_chunk_task(&self, task: ChunkTask) -> eyre::Result<bool> {
        let chunk_id = task.0;
        match self.chunk_handle.get_status(&task) {
            Ok(TaskStatus::Completed) => {
                if self.reset_receiptless_chunk_task(&task)? {
                    // Report "no usable task" so the caller submits a fresh one.
                    return Ok(false);
                }
                self.chunk_storage
                    .update_chunk_status(chunk_id, ChunkStatus::ProofReady(task.proof_id()))
                    .await?;
                Ok(true)
            }
            Ok(TaskStatus::PermanentFailure { error }) => {
                warn!(
                    ?chunk_id,
                    reason = %error,
                    "chunk proof task failed permanently; leaving chunk ProofPending until the task
                     is reset"
                );
                self.chunk_storage
                    .update_chunk_status(chunk_id, ChunkStatus::ProofPending(task.to_string()))
                    .await?;
                Ok(true)
            }
            Ok(TaskStatus::Pending)
            | Ok(TaskStatus::Proving { .. })
            | Ok(TaskStatus::TransientFailure { .. }) => {
                self.chunk_storage
                    .update_chunk_status(chunk_id, ChunkStatus::ProofPending(task.to_string()))
                    .await?;
                Ok(true)
            }
            Err(PaasError::TaskNotFound(_)) => Ok(false),
            Err(e) => Err(eyre::eyre!("get_status({chunk_id:?}): {e}")),
        }
    }

    fn observe_existing_batch_task(&self, batch_id: BatchId) -> eyre::Result<bool> {
        if self.batch_proofs.has_proof(batch_id) {
            return Ok(true);
        }

        match self.acct_handle.get_status(&BatchTask(batch_id)) {
            Ok(_) => Ok(true),
            Err(PaasError::TaskNotFound(_)) => Ok(false),
            Err(e) => Err(eyre::eyre!("get_status({batch_id}): {e}")),
        }
    }

    async fn acct_proof_inputs_ready(&self, batch_id: BatchId) -> eyre::Result<bool> {
        let Some((batch, _status)) = self.batch_storage.get_batch_by_id(batch_id).await? else {
            return Err(eyre::eyre!(
                "cannot request acct proof for missing batch {batch_id}"
            ));
        };

        let Some(chunk_ids) = self.chunk_storage.get_batch_chunks(batch_id).await? else {
            debug!(%batch_id, "acct proof inputs not ready: batch chunk links missing");
            return Ok(false);
        };

        if chunk_ids.is_empty() {
            if batch.idx() == 0 {
                return Ok(true);
            }

            return Err(eyre::eyre!(
                "cannot request acct proof for non-genesis batch {batch_id}: empty chunk list"
            ));
        }

        for chunk_id in chunk_ids {
            let Some((_chunk, status)) = self.chunk_storage.get_chunk_by_id(chunk_id).await? else {
                warn!(
                    %batch_id,
                    ?chunk_id,
                    "acct proof inputs not ready: batch references missing chunk"
                );
                return Ok(false);
            };

            match status {
                ChunkStatus::ProofReady(_) => {
                    let task = ChunkTask(chunk_id);
                    if self.reset_receiptless_chunk_task(&task)? {
                        debug!(
                            %batch_id,
                            ?chunk_id,
                            "acct proof inputs not ready: chunk receipt missing; chunk re-proves"
                        );
                        self.chunk_storage
                            .update_chunk_status(
                                chunk_id,
                                ChunkStatus::ProofPending(task.to_string()),
                            )
                            .await?;
                        return Ok(false);
                    }
                }
                status => {
                    debug!(
                        %batch_id,
                        ?chunk_id,
                        ?status,
                        "acct proof inputs not ready: chunk proof not ready"
                    );
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }
}

#[async_trait]
impl<CS, AS> ChunkProver for PaasEeProver<CS, AS>
where
    CS: ProofSpec<Task = ChunkTask>,
    AS: ProofSpec<Task = BatchTask>,
{
    async fn request_proof_generation(&self, chunk_id: ChunkId) -> eyre::Result<()> {
        let task = ChunkTask(chunk_id);
        let Some((_chunk, status)) = self.chunk_storage.get_chunk_by_id(chunk_id).await? else {
            return Err(eyre::eyre!(
                "cannot submit chunk proof task for missing chunk {chunk_id:?}"
            ));
        };
        match status {
            ChunkStatus::ProofReady(_) => return Ok(()),
            ChunkStatus::Sealed | ChunkStatus::ProofPending(_) => {}
        }

        if self.observe_existing_chunk_task(task).await? {
            return Ok(());
        }

        info!(?chunk_id, "submitting chunk proof task");

        self.chunk_handle
            .submit(task)
            .await
            .map_err(|e| eyre::eyre!("submit chunk task {chunk_id:?}: {e}"))?;

        self.chunk_storage
            .update_chunk_status(chunk_id, ChunkStatus::ProofPending(task.to_string()))
            .await?;

        Ok(())
    }

    async fn check_proof_status(&self, chunk_id: ChunkId) -> eyre::Result<ProofGenerationStatus> {
        if let Some((_chunk, status)) = self.chunk_storage.get_chunk_by_id(chunk_id).await? {
            match status {
                ChunkStatus::ProofReady(proof_id) => {
                    return Ok(ProofGenerationStatus::Ready { proof_id });
                }
                ChunkStatus::Sealed | ChunkStatus::ProofPending(_) => {}
            }
        }

        let task = ChunkTask(chunk_id);
        match self.chunk_handle.get_status(&task) {
            Ok(TaskStatus::Completed) => {
                if self.reset_receiptless_chunk_task(&task)? {
                    // Route back through the sealed path so the task is
                    // resubmitted and the receipt regenerated.
                    return Ok(ProofGenerationStatus::NotStarted);
                }
                Ok(ProofGenerationStatus::Ready {
                    proof_id: task.proof_id(),
                })
            }
            Ok(TaskStatus::PermanentFailure { error }) => {
                Ok(ProofGenerationStatus::Failed { reason: error })
            }
            Ok(TaskStatus::Pending)
            | Ok(TaskStatus::Proving { .. })
            | Ok(TaskStatus::TransientFailure { .. }) => Ok(ProofGenerationStatus::Pending),
            Err(PaasError::TaskNotFound(_)) => Ok(ProofGenerationStatus::NotStarted),
            Err(e) => {
                warn!(?chunk_id, %e, "chunk_handle.get_status failed");
                Err(eyre::eyre!("get_status({chunk_id:?}): {e}"))
            }
        }
    }
}

#[async_trait]
impl<CS, AS> BatchProver for PaasEeProver<CS, AS>
where
    CS: ProofSpec<Task = ChunkTask>,
    AS: ProofSpec<Task = BatchTask>,
{
    async fn request_proof_generation(
        &self,
        batch_id: BatchId,
    ) -> eyre::Result<ProofRequestStatus> {
        if !self.acct_proof_inputs_ready(batch_id).await? {
            return Ok(ProofRequestStatus::WaitingForInputs);
        }

        if self.observe_existing_batch_task(batch_id)? {
            return Ok(ProofRequestStatus::AlreadyExists);
        }

        info!(%batch_id, "submitting acct proof task");

        self.acct_handle
            .submit(BatchTask(batch_id))
            .await
            .map_err(|e| eyre::eyre!("submit acct task {batch_id}: {e}"))?;

        Ok(ProofRequestStatus::Submitted)
    }

    async fn check_proof_status(&self, batch_id: BatchId) -> eyre::Result<ProofGenerationStatus> {
        // Source of truth: the typed batch proof DB (the acct hook writes
        // there). Present ⇒ Ready.
        if self.batch_proofs.has_proof(batch_id) {
            return Ok(ProofGenerationStatus::Ready {
                proof_id: EeBatchProofDbManager::proof_id_for(batch_id),
            });
        }

        // Else map paas's task lifecycle status. `TaskNotFound` ⇒ NotStarted
        // (we never submitted, or we're in a fresh process and haven't yet
        // recovered).
        match self.acct_handle.get_status(&BatchTask(batch_id)) {
            Ok(TaskStatus::Completed) => {
                // Completed but not in the proof DB? Hook hasn't fired yet
                // or the DB lost its entry. Treat as Pending so the
                // lifecycle keeps polling.
                debug!(%batch_id, "acct task Completed but proof not yet in DB; reporting Pending");
                Ok(ProofGenerationStatus::Pending)
            }
            Ok(TaskStatus::PermanentFailure { error }) => {
                Ok(ProofGenerationStatus::Failed { reason: error })
            }
            Ok(TaskStatus::Pending)
            | Ok(TaskStatus::Proving { .. })
            | Ok(TaskStatus::TransientFailure { .. }) => Ok(ProofGenerationStatus::Pending),
            Err(PaasError::TaskNotFound(_)) => Ok(ProofGenerationStatus::NotStarted),
            Err(e) => {
                warn!(%batch_id, %e, "acct_handle.get_status failed");
                Err(eyre::eyre!("get_status({batch_id}): {e}"))
            }
        }
    }

    async fn get_proof(&self, proof_id: ProofId) -> eyre::Result<Option<Proof>> {
        Ok(self.batch_proofs.get_proof_by_id(proof_id))
    }
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{Batch, BatchStatus, Chunk, InMemoryStorage, ProofRequestStatus};
    use alpen_ee_database::init_db_storage;
    use reth_tasks::TaskManager;
    use strata_acct_types::Hash;
    use strata_paas::{
        InMemoryReceiptStore, InMemoryTaskStore, ProverBuilder, ProverResult, ProverServiceBuilder,
        TaskRecord, TaskStore,
    };
    use strata_proofimpl_alpen_chunk::EeChunkProgram;
    use zkaleido::{
        ProofType, PublicValues, ZkVmHost, ZkVmInputBuilder, ZkVmInputResult, ZkVmProgram,
        ZkVmResult,
    };

    use super::*;
    use crate::service_executor::ServiceExecutor;

    /// Program stub — the specs below never let a task reach proving.
    struct MockProgram;

    impl ZkVmProgram for MockProgram {
        type Input = ();
        type Output = ();

        fn name() -> String {
            "mock".to_string()
        }

        fn proof_type() -> ProofType {
            ProofType::Core
        }

        fn prepare_input<'a, B>(_input: &'a Self::Input) -> ZkVmInputResult<B::Input>
        where
            B: ZkVmInputBuilder<'a>,
        {
            B::new().build()
        }

        fn process_output<H>(_public_values: &PublicValues) -> ZkVmResult<Self::Output>
        where
            H: ZkVmHost,
        {
            Ok(())
        }
    }

    struct MockChunkSpec;

    #[async_trait]
    impl ProofSpec for MockChunkSpec {
        type Task = ChunkTask;
        type Program = MockProgram;

        async fn fetch_input(&self, _task: &Self::Task) -> ProverResult<()> {
            Err(PaasError::TransientFailure(
                "mock chunk spec never proves".to_string(),
            ))
        }
    }

    struct MockAcctSpec;

    #[async_trait]
    impl ProofSpec for MockAcctSpec {
        type Task = BatchTask;
        type Program = MockProgram;

        async fn fetch_input(&self, _task: &Self::Task) -> ProverResult<()> {
            Err(PaasError::TransientFailure(
                "mock acct spec never proves".to_string(),
            ))
        }
    }

    fn hash_from_u8(value: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = 1;
        bytes[31] = value;
        Hash::from(bytes)
    }

    struct Fixture {
        prover: PaasEeProver<MockChunkSpec, MockAcctSpec>,
        storage: Arc<InMemoryStorage>,
        chunk_task_store: Arc<InMemoryTaskStore>,
        batch_id: BatchId,
        chunk_id: ChunkId,
        // Dropping the manager kills the service loops; direct handle
        // accessors keep working, but hold it for the test's lifetime anyway.
        _task_manager: TaskManager,
        _db_dir: tempfile::TempDir,
    }

    /// One batch (idx 1) with one chunk in `ProofReady`, a `Completed` chunk
    /// task record, and an **empty** chunk receipt store.
    async fn receiptless_ready_chunk_fixture() -> Fixture {
        let storage = Arc::new(InMemoryStorage::new_empty());

        let batch = Batch::new(1, hash_from_u8(1), hash_from_u8(2), 10, Vec::new()).unwrap();
        let batch_id = batch.id();
        let chunk = Chunk::new(0, hash_from_u8(1), hash_from_u8(2), 10, 1, Vec::new());
        let chunk_id = chunk.id();

        storage
            .batch_id_to_idx
            .write()
            .unwrap()
            .insert(batch_id, batch.idx());
        storage
            .batches
            .write()
            .unwrap()
            .insert(batch.idx(), (batch, BatchStatus::Sealed));
        storage
            .chunk_id_to_idx
            .write()
            .unwrap()
            .insert(chunk_id, chunk.idx());
        storage.chunks.write().unwrap().insert(
            chunk.idx(),
            (chunk, ChunkStatus::ProofReady(hash_from_u8(2))),
        );
        storage
            .batch_chunks
            .write()
            .unwrap()
            .insert(batch_id, vec![chunk_id]);

        let chunk_task_store = Arc::new(InMemoryTaskStore::new());
        let chunk_task_key: Vec<u8> = ChunkTask(chunk_id).into();
        chunk_task_store
            .insert(TaskRecord::new(chunk_task_key, TaskStatus::Completed))
            .unwrap();

        let task_manager = TaskManager::current();
        let executor = ServiceExecutor::from_reth(task_manager.executor());

        let chunk_prover = ProverBuilder::new(MockChunkSpec)
            .task_store(chunk_task_store.clone())
            .receipt_store(InMemoryReceiptStore::new())
            .native(EeChunkProgram::native_host());
        let chunk_handle = ProverServiceBuilder::new(chunk_prover)
            .launch(&executor)
            .await
            .unwrap();

        let acct_prover = ProverBuilder::new(MockAcctSpec)
            .task_store(InMemoryTaskStore::new())
            .native(EeChunkProgram::native_host());
        let acct_handle = ProverServiceBuilder::new(acct_prover)
            .launch(&executor)
            .await
            .unwrap();

        let db_dir = tempfile::tempdir().unwrap();
        let dbs = init_db_storage(db_dir.path(), 3).unwrap();
        let batch_proofs = Arc::new(EeBatchProofDbManager::new(dbs.prover_db()));

        let prover = PaasEeProver::new(
            chunk_handle,
            acct_handle,
            storage.clone(),
            storage.clone(),
            batch_proofs,
        );

        Fixture {
            prover,
            storage,
            chunk_task_store,
            batch_id,
            chunk_id,
            _task_manager: task_manager,
            _db_dir: db_dir,
        }
    }

    /// A `ProofReady` chunk whose receipt is missing must not wedge the batch
    /// in a `ProofReady`/`ProofPending` oscillation: the acct gate reverts the
    /// chunk to `ProofPending` AND resets the stale `Completed` task so the
    /// chunk lifecycle resubmits it.
    #[tokio::test]
    async fn acct_gate_reverts_receiptless_ready_chunk_and_resets_task() {
        let fx = receiptless_ready_chunk_fixture().await;

        let status = BatchProver::request_proof_generation(&fx.prover, fx.batch_id)
            .await
            .unwrap();
        assert!(matches!(status, ProofRequestStatus::WaitingForInputs));

        let (_, chunk_status) = fx
            .storage
            .get_chunk_by_id(fx.chunk_id)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(chunk_status, ChunkStatus::ProofPending(_)));

        let chunk_task_key: Vec<u8> = ChunkTask(fx.chunk_id).into();
        assert!(
            fx.chunk_task_store.get(&chunk_task_key).unwrap().is_none(),
            "stale Completed task must be removed so the chunk can resubmit"
        );

        // The chunk lifecycle's next poll now routes back through the sealed
        // path (resubmission) instead of flipping the chunk ProofReady again.
        let proof_status = ChunkProver::check_proof_status(&fx.prover, fx.chunk_id)
            .await
            .unwrap();
        assert!(matches!(proof_status, ProofGenerationStatus::NotStarted));
    }

    /// `check_proof_status` itself must not report a receipt-less Completed
    /// task as Ready (that is the other half of the oscillation).
    #[tokio::test]
    async fn check_proof_status_resets_completed_task_with_missing_receipt() {
        let fx = receiptless_ready_chunk_fixture().await;

        // Put the chunk in ProofPending so status falls through to the task.
        fx.storage
            .update_chunk_status(fx.chunk_id, ChunkStatus::ProofPending("task".to_string()))
            .await
            .unwrap();

        let proof_status = ChunkProver::check_proof_status(&fx.prover, fx.chunk_id)
            .await
            .unwrap();
        assert!(matches!(proof_status, ProofGenerationStatus::NotStarted));

        let chunk_task_key: Vec<u8> = ChunkTask(fx.chunk_id).into();
        assert!(fx.chunk_task_store.get(&chunk_task_key).unwrap().is_none());
    }
}
