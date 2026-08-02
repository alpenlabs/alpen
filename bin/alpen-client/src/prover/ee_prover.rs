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

/// Reconciled view of a chunk's paas task record.
///
/// Produced by [`PaasEeProver::classify_chunk_task`] so that the storage-writing
/// reconciler and the read-only status poll share one interpretation of the
/// paas [`TaskStatus`] plus receipt presence.
enum ChunkTaskState {
    /// No usable task record: none was ever submitted, or a stale terminal
    /// record was removed so the chunk can be re-proved.
    Absent,
    /// A task record exists that can still make progress on its own.
    InFlight,
    /// The task record is terminally failed and will not retry without
    /// operator action.
    FailedPermanently(String),
    /// The task completed and its receipt is present.
    Ready,
}

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

    /// Classifies a chunk's paas task record into a [`ChunkTaskState`].
    ///
    /// This is the single interpretation of a chunk task shared by
    /// [`Self::reconcile_chunk_task_status`] and
    /// [`ChunkProver::check_proof_status`]; keeping the two matches in one
    /// place is what stops them from drifting apart.
    ///
    /// A `Completed` task whose receipt is missing gets special treatment. Such
    /// a task can never make progress on its own: `submit` short-circuits on
    /// the existing record and the receipt is only written by a fresh proving
    /// run, so a receipt-less `Completed` task would wedge the chunk in a
    /// `ProofReady`/`ProofPending` oscillation while the acct proof waits
    /// forever. The stale record is therefore reset here so the chunk lifecycle
    /// resubmits it, and the chunk reports [`ChunkTaskState::Absent`]. If the
    /// record turned out not to be terminal after all (a concurrent retry
    /// picked it up), `reset_task` leaves it alone and the chunk reports
    /// [`ChunkTaskState::InFlight`] instead, because that in-flight attempt
    /// will regenerate the receipt on its own.
    fn classify_chunk_task(&self, task: &ChunkTask) -> eyre::Result<ChunkTaskState> {
        let chunk_id = task.0;
        match self.chunk_handle.get_status(task) {
            Ok(TaskStatus::Completed) => {
                if self
                    .chunk_handle
                    .get_receipt(task)
                    .map_err(|e| eyre::eyre!("get chunk receipt {chunk_id:?}: {e}"))?
                    .is_some()
                {
                    return Ok(ChunkTaskState::Ready);
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
                    Ok(ChunkTaskState::Absent)
                } else {
                    debug!(
                        ?chunk_id,
                        "chunk task no longer terminal; leaving in-flight attempt to regenerate \
                         receipt"
                    );
                    Ok(ChunkTaskState::InFlight)
                }
            }
            Ok(TaskStatus::PermanentFailure { error }) => {
                Ok(ChunkTaskState::FailedPermanently(error))
            }
            Ok(TaskStatus::Pending)
            | Ok(TaskStatus::Proving { .. })
            | Ok(TaskStatus::TransientFailure { .. }) => Ok(ChunkTaskState::InFlight),
            Err(PaasError::TaskNotFound(_)) => Ok(ChunkTaskState::Absent),
            Err(e) => Err(eyre::eyre!("get_status({chunk_id:?}): {e}")),
        }
    }

    /// Reconciles the stored chunk status with its paas task record.
    ///
    /// Writes the chunk status implied by [`Self::classify_chunk_task`] and
    /// returns `true` when a usable task record exists, meaning the caller must
    /// not submit a new one.
    async fn reconcile_chunk_task_status(&self, task: ChunkTask) -> eyre::Result<bool> {
        let chunk_id = task.0;
        match self.classify_chunk_task(&task)? {
            // No usable task; the caller submits a fresh one.
            ChunkTaskState::Absent => Ok(false),
            ChunkTaskState::Ready => {
                self.chunk_storage
                    .update_chunk_status(chunk_id, ChunkStatus::ProofReady(task.proof_id()))
                    .await?;
                Ok(true)
            }
            ChunkTaskState::FailedPermanently(error) => {
                warn!(
                    ?chunk_id,
                    reason = %error,
                    "chunk proof task failed permanently; the chunk is wedged in ProofPending and \
                     its batch cannot reach ProofReady until an operator resets the task with \
                     `strata-dbtool ee-delete-chunk-receipt`"
                );
                self.chunk_storage
                    .update_chunk_status(chunk_id, ChunkStatus::ProofPending(task.to_string()))
                    .await?;
                Ok(true)
            }
            ChunkTaskState::InFlight => {
                // Benign race with `ChunkReceiptHook`: the hook can write
                // `ProofReady` between the classification above and this write,
                // which then stamps the chunk back to `ProofPending`. The chunk
                // stays in the pending work page, so the next lifecycle tick
                // calls `check_proof_status`, re-derives `Ready` from paas and
                // restores `ProofReady`. Costs one tick, never a re-proof.
                self.chunk_storage
                    .update_chunk_status(chunk_id, ChunkStatus::ProofPending(task.to_string()))
                    .await?;
                Ok(true)
            }
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
        if self
            .batch_storage
            .get_batch_by_id(batch_id)
            .await?
            .is_none()
        {
            return Err(eyre::eyre!(
                "cannot request acct proof for missing batch {batch_id}"
            ));
        }

        let Some(chunk_ids) = self.chunk_storage.get_batch_chunks(batch_id).await? else {
            debug!(%batch_id, "acct proof inputs not ready: batch chunk links missing");
            return Ok(false);
        };

        if chunk_ids.is_empty() {
            // A stored-but-empty chunk-link row is an upstream invariant
            // violation. The chunk builder links chunks only at batch seal
            // (`handle_batch_boundary`), where it force-seals the open
            // accumulator first, so every batch it links carries at least one
            // chunk. An empty row therefore means the link was written
            // against a corrupt builder state, and no batch tick can repair
            // it: the chunks are simply not recorded anywhere.
            //
            // The error is deliberate rather than an `Ok(false)` wait. There
            // is nothing to wait for, `AcctSpec::fetch_input` would
            // permanent-fail on the same input, and the batch lifecycle
            // re-enters this path every tick, so the error repeats until an
            // operator restores the invariant. That repetition is the alert.
            return Err(eyre::eyre!(
                "cannot request acct proof for batch {batch_id}: empty chunk list"
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
                    // Receipt presence is the gate condition, because
                    // `AcctSpec::fetch_input` reads the chunk receipt directly
                    // and needs nothing else from the task record. Also
                    // demanding `Completed` would be stricter than the
                    // consumer and would demote a healthy chunk during the
                    // window where `ChunkReceiptHook` has already stored the
                    // receipt but the task is not marked `Completed` yet.
                    if self
                        .chunk_handle
                        .get_receipt(&task)
                        .map_err(|e| eyre::eyre!("get chunk receipt {chunk_id:?}: {e}"))?
                        .is_some()
                    {
                        continue;
                    }

                    // No receipt, so this chunk cannot feed the acct proof.
                    // Run the shared classifier for its documented side
                    // effect: a receipt-less `Completed` record can never
                    // regenerate the receipt on its own, so the classifier
                    // resets it and the chunk lifecycle resubmits. No
                    // classification is `Ready` without a receipt, so the
                    // batch always waits here.
                    self.classify_chunk_task(&task)?;
                    debug!(
                        %batch_id,
                        ?chunk_id,
                        "acct proof inputs not ready: chunk receipt missing; chunk re-proves"
                    );
                    self.chunk_storage
                        .update_chunk_status(chunk_id, ChunkStatus::ProofPending(task.to_string()))
                        .await?;
                    return Ok(false);
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

        if self.reconcile_chunk_task_status(task).await? {
            return Ok(());
        }

        info!(?chunk_id, "submitting chunk proof task");

        self.chunk_handle
            .submit(task)
            .await
            .map_err(|e| eyre::eyre!("submit chunk task {chunk_id:?}: {e}"))?;

        // Benign race with `ChunkReceiptHook`: with a fast native host the hook
        // can store the receipt and write `ProofReady` before this stamp lands,
        // which then overwrites it back to `ProofPending`. The chunk stays in
        // the pending work page, so the next lifecycle tick calls
        // `check_proof_status`, re-derives `Ready` from paas and restores
        // `ProofReady`. Costs one tick, never a re-proof.
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

        // Read-only map of the shared classification. The one write it can
        // trigger is the stale-task reset inside `classify_chunk_task`, which
        // is what turns a receipt-less `Completed` task back into `NotStarted`
        // so the sealed path resubmits it.
        let task = ChunkTask(chunk_id);
        Ok(match self.classify_chunk_task(&task)? {
            ChunkTaskState::Ready => ProofGenerationStatus::Ready {
                proof_id: task.proof_id(),
            },
            ChunkTaskState::FailedPermanently(reason) => ProofGenerationStatus::Failed { reason },
            // The task record is still live, so reporting `NotStarted` here
            // would make the lifecycle log a bogus "PaaS task is missing" warn
            // and pointlessly resubmit.
            ChunkTaskState::InFlight => ProofGenerationStatus::Pending,
            ChunkTaskState::Absent => ProofGenerationStatus::NotStarted,
        })
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
        let task = BatchTask(batch_id);
        match self.acct_handle.get_status(&task) {
            Ok(TaskStatus::Completed) => {
                // Re-probe the proof DB before touching the record. The
                // hazard is the *read* order, not the write order:
                // `AcctReceiptHook` writes the proof row and only then marks
                // the task `Completed`, but this reader probed `has_proof`
                // above and `get_status` here, in that same direction. A
                // batch that finished proving between the two reads therefore
                // shows up as "Completed with no proof" even though it
                // succeeded, and resetting it would delete the record of a
                // healthy batch. This second probe closes that window.
                //
                // Not covered by a test: the in-memory fixtures drive both
                // reads from one thread, so the interleaving cannot be staged
                // deterministically without a seam that exists only for the
                // test.
                if self.batch_proofs.has_proof(batch_id) {
                    return Ok(ProofGenerationStatus::Ready {
                        proof_id: EeBatchProofDbManager::proof_id_for(batch_id),
                    });
                }

                // The proof is genuinely absent, so it was removed underneath
                // us (`strata-dbtool ee-delete-acct-proof` deletes the proof
                // and deliberately leaves the task record) or the DB lost it.
                // Same wedge as the receipt-less chunk task: `submit`
                // short-circuits on the existing record, so the proof can
                // never come back on its own. Reset the stale record and
                // report `NotStarted`, which the batch lifecycle answers by
                // re-requesting the acct proof.
                let removed = self
                    .acct_handle
                    .reset_task(&task)
                    .map_err(|e| eyre::eyre!("reset acct task {batch_id}: {e}"))?;
                if removed {
                    warn!(
                        %batch_id,
                        "acct task Completed but proof missing from the batch proof DB; reset the \
                         task so the batch lifecycle re-requests the acct proof"
                    );
                    Ok(ProofGenerationStatus::NotStarted)
                } else {
                    debug!(
                        %batch_id,
                        "acct task no longer terminal; leaving in-flight attempt to regenerate the \
                         proof"
                    );
                    Ok(ProofGenerationStatus::Pending)
                }
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
    use alpen_ee_database::open_for_node;
    use reth_tasks::TaskManager;
    use strata_acct_types::Hash;
    use strata_paas::{
        InMemoryReceiptStore, InMemoryTaskStore, ProverBuilder, ProverResult, ProverServiceBuilder,
        ReceiptStore, TaskRecord, TaskStore,
    };
    use strata_proofimpl_alpen_chunk::EeChunkProgram;
    use zkaleido::{
        ProgramId, Proof, ProofMetadata, ProofReceipt, ProofReceiptWithMetadata, ProofType,
        PublicValues, ZkVm, ZkVmHost, ZkVmInputBuilder, ZkVmInputResult, ZkVmProgram, ZkVmResult,
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
        acct_task_store: Arc<InMemoryTaskStore>,
        batch_id: BatchId,
        chunk_id: ChunkId,
        // Dropping the manager kills the service loops; direct handle
        // accessors keep working, but hold it for the test's lifetime anyway.
        _task_manager: TaskManager,
        _db_dir: tempfile::TempDir,
    }

    /// A receipt with no proof in it: the facade only ever checks presence.
    fn dummy_receipt() -> ProofReceiptWithMetadata {
        ProofReceiptWithMetadata::new(
            ProofReceipt::new(Proof::new(Vec::new()), PublicValues::new(Vec::new())),
            ProofMetadata::new(
                ZkVm::Native,
                ProgramId([0u8; 32]),
                "test".to_string(),
                ProofType::Core,
            ),
        )
    }

    /// One batch (idx 1) with one chunk in `ProofReady`, a `Completed` chunk
    /// task record, and an **empty** chunk receipt store.
    async fn receiptless_ready_chunk_fixture() -> Fixture {
        chunk_fixture(false).await
    }

    /// The same shape with the chunk receipt actually present, i.e. a batch
    /// whose acct proof inputs are fully ready.
    async fn ready_chunk_fixture() -> Fixture {
        chunk_fixture(true).await
    }

    async fn chunk_fixture(with_chunk_receipt: bool) -> Fixture {
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
            .insert(TaskRecord::new(
                chunk_task_key.clone(),
                TaskStatus::Completed,
            ))
            .unwrap();

        let chunk_receipt_store = Arc::new(InMemoryReceiptStore::new());
        if with_chunk_receipt {
            chunk_receipt_store
                .put(&chunk_task_key, &dummy_receipt())
                .unwrap();
        }

        let task_manager = TaskManager::current();
        let executor = ServiceExecutor::from_reth(task_manager.executor());

        let chunk_prover = ProverBuilder::new(MockChunkSpec)
            .task_store(chunk_task_store.clone())
            .receipt_store(chunk_receipt_store)
            .native(EeChunkProgram::native_host());
        let chunk_handle = ProverServiceBuilder::new(chunk_prover)
            .launch(&executor)
            .await
            .unwrap();

        let acct_task_store = Arc::new(InMemoryTaskStore::new());
        let acct_prover = ProverBuilder::new(MockAcctSpec)
            .task_store(acct_task_store.clone())
            .native(EeChunkProgram::native_host());
        let acct_handle = ProverServiceBuilder::new(acct_prover)
            .launch(&executor)
            .await
            .unwrap();

        let db_dir = tempfile::tempdir().unwrap();
        let dbs = open_for_node(db_dir.path().to_path_buf(), 3).await.unwrap();
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
            acct_task_store,
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

    /// A permanently failed chunk task is reported as `Failed` and leaves the
    /// chunk in `ProofPending` (there is no terminal chunk status), and the
    /// facade must not resubmit on top of the still-present record.
    #[tokio::test]
    async fn permanently_failed_chunk_task_keeps_chunk_proof_pending() {
        let fx = receiptless_ready_chunk_fixture().await;

        let chunk_task_key: Vec<u8> = ChunkTask(fx.chunk_id).into();
        fx.chunk_task_store
            .update_status(
                &chunk_task_key,
                TaskStatus::PermanentFailure {
                    error: "guest panicked".to_string(),
                },
            )
            .unwrap();
        fx.storage
            .update_chunk_status(fx.chunk_id, ChunkStatus::ProofPending("task".to_string()))
            .await
            .unwrap();

        let proof_status = ChunkProver::check_proof_status(&fx.prover, fx.chunk_id)
            .await
            .unwrap();
        assert!(matches!(
            proof_status,
            ProofGenerationStatus::Failed { reason } if reason == "guest panicked"
        ));

        ChunkProver::request_proof_generation(&fx.prover, fx.chunk_id)
            .await
            .unwrap();

        let (_, chunk_status) = fx
            .storage
            .get_chunk_by_id(fx.chunk_id)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(chunk_status, ChunkStatus::ProofPending(_)));
        // Presence alone would not discriminate: `submit` short-circuits on
        // any existing record, so a stray resubmit would leave a record
        // behind too. The status is what a resubmit would change, because a
        // fresh submission inserts `Pending` (and a reset removes the record
        // outright).
        let record = fx
            .chunk_task_store
            .get(&chunk_task_key)
            .unwrap()
            .expect("a permanently failed record must survive until an operator resets it");
        assert!(
            matches!(record.status(), TaskStatus::PermanentFailure { error } if error == "guest panicked"),
            "the terminal record must be left untouched, got {:?}",
            record.status()
        );
    }

    /// The acct mirror of the receipt-less chunk case: a `Completed` acct task
    /// whose proof row is gone (an operator ran `ee-delete-acct-proof`, which
    /// leaves the task record behind) can never regenerate the proof, because
    /// `submit` short-circuits on the record. It must be reset and reported as
    /// `NotStarted` so the batch lifecycle re-requests it.
    #[tokio::test]
    async fn check_proof_status_resets_completed_acct_task_with_missing_proof() {
        let fx = receiptless_ready_chunk_fixture().await;

        let acct_task_key: Vec<u8> = BatchTask(fx.batch_id).into();
        fx.acct_task_store
            .insert(TaskRecord::new(
                acct_task_key.clone(),
                TaskStatus::Completed,
            ))
            .unwrap();

        let proof_status = BatchProver::check_proof_status(&fx.prover, fx.batch_id)
            .await
            .unwrap();
        assert!(
            matches!(proof_status, ProofGenerationStatus::NotStarted),
            "expected NotStarted, got {proof_status:?}"
        );
        assert!(
            fx.acct_task_store.get(&acct_task_key).unwrap().is_none(),
            "the stale Completed acct task must be removed so the batch can resubmit"
        );
    }

    /// The accept path of the acct gate: every chunk of the batch is
    /// `ProofReady` with its receipt present and no acct task exists yet, so
    /// the facade submits the acct proof task and then reports it as existing.
    #[tokio::test]
    async fn acct_gate_submits_when_chunk_receipts_are_present() {
        let fx = ready_chunk_fixture().await;

        let status = BatchProver::request_proof_generation(&fx.prover, fx.batch_id)
            .await
            .unwrap();
        assert!(
            matches!(status, ProofRequestStatus::Submitted),
            "expected Submitted, got {status:?}"
        );

        let acct_task_key: Vec<u8> = BatchTask(fx.batch_id).into();
        assert!(
            fx.acct_task_store.get(&acct_task_key).unwrap().is_some(),
            "submission must leave an acct task record behind"
        );

        // The chunk is left alone: the gate accepted it, so nothing reverts
        // it to `ProofPending`.
        let (_, chunk_status) = fx
            .storage
            .get_chunk_by_id(fx.chunk_id)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(chunk_status, ChunkStatus::ProofReady(_)));

        let status = BatchProver::request_proof_generation(&fx.prover, fx.batch_id)
            .await
            .unwrap();
        assert!(
            matches!(status, ProofRequestStatus::AlreadyExists),
            "expected AlreadyExists on the second request, got {status:?}"
        );
    }

    /// The gate condition is receipt presence, not task status. `ChunkReceiptHook`
    /// stores the receipt and writes `ProofReady` before the task is marked
    /// `Completed`, so a batch tick landing in that window sees a stored receipt
    /// under a still-`Proving` record. `AcctSpec::fetch_input` only needs the
    /// receipt, so the gate must accept the chunk and leave it `ProofReady`
    /// instead of demoting a healthy chunk into a needless re-proof.
    #[tokio::test]
    async fn acct_gate_accepts_chunk_with_receipt_before_task_completes() {
        let fx = ready_chunk_fixture().await;

        let chunk_task_key: Vec<u8> = ChunkTask(fx.chunk_id).into();
        fx.chunk_task_store
            .update_status(&chunk_task_key, TaskStatus::Proving { retry_count: 0 })
            .unwrap();

        let status = BatchProver::request_proof_generation(&fx.prover, fx.batch_id)
            .await
            .unwrap();
        assert!(
            matches!(status, ProofRequestStatus::Submitted),
            "expected Submitted, got {status:?}"
        );

        let (_, chunk_status) = fx
            .storage
            .get_chunk_by_id(fx.chunk_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(chunk_status, ChunkStatus::ProofReady(_)),
            "a chunk whose receipt is stored must not be demoted, got {chunk_status:?}"
        );
    }
}
