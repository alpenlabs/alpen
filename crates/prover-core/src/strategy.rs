//! Concrete [`ProveStrategy`](crate::ProveStrategy) impls: native and remote.
//!
//! Both are sync/blocking — called inside `spawn_blocking` by the prover.
//! The `Host` type is captured at build time and erased via
//! `dyn ProveStrategy<H>`.

use std::sync::Arc;
#[cfg(feature = "remote")]
use std::time::Duration;

#[cfg(feature = "remote")]
use tokio::runtime::{Builder, Runtime};
use zkaleido::{ProofReceiptWithMetadata, ZkVmHost, ZkVmProgram};

#[cfg(feature = "remote")]
use crate::error::ProverError;
use crate::{
    error::{classify_prove_failure, ProverResult},
    traits::{ProofSpec, ProveContext, ProveStrategy},
};

/// Native execution: `ZkVmProgram::prove` directly.
pub(crate) struct NativeStrategy<Host> {
    host: Arc<Host>,
}

impl<Host> NativeStrategy<Host> {
    pub(crate) fn new(host: Host) -> Self {
        Self {
            host: Arc::new(host),
        }
    }
}

impl<H, Host> ProveStrategy<H> for NativeStrategy<Host>
where
    H: ProofSpec,
    Host: ZkVmHost + Send + Sync + 'static,
{
    fn prove(
        &self,
        input: &<H::Program as ZkVmProgram>::Input,
        _ctx: ProveContext,
    ) -> ProverResult<ProofReceiptWithMetadata> {
        // An out-of-process backend killed by SIGBUS (a full `/dev/shm`, say)
        // surfaces here as an error string, so classify before deciding whether
        // the task is worth retrying.
        H::Program::prove(input, self.host.as_ref())
            .map_err(|e| classify_prove_failure(e.to_string()))
    }
}

/// Remote execution: `start_proving` + poll + `get_proof`.
///
/// `ZkVmRemoteProver` exposes `!Send` futures, so each `prove()` drives them on
/// a `LocalSet`. Crucially those futures run on a **single long-lived runtime**
/// owned by the strategy, not a fresh runtime per call.
///
/// SP1 SDK >=6.2 caches its gRPC channel for the whole process, and a tonic
/// channel is just an mpsc handle to a background worker `tokio::spawn`'d on the
/// runtime that was active when the channel was first built. A per-call runtime
/// would be dropped after the first prove, killing that worker and leaving the
/// cached channel permanently unusable — every later call would then fail with
/// "Service was not ready: transport error". The shared runtime keeps the worker
/// alive for as long as the strategy exists (i.e. the process).
///
/// On crash recovery, if `ctx.saved` contains a serialized `ProofId`, we skip
/// `start_proving` and resume polling directly — no double submission.
#[cfg(feature = "remote")]
pub(crate) struct RemoteStrategy<Host> {
    host: Arc<Host>,
    poll_interval: Duration,
    /// Long-lived runtime shared across every `prove()` call so the SP1 gRPC
    /// channel's background worker (spawned on first use) outlives individual
    /// proves. See the type-level docs for why this must not be per-call.
    ///
    /// `Option` only so [`Drop`] can take it and shut it down without blocking;
    /// it is `Some` for the entire normal lifetime.
    rt: Option<Runtime>,
}

#[cfg(feature = "remote")]
impl<Host> Drop for RemoteStrategy<Host> {
    fn drop(&mut self) {
        // The default `Runtime` drop performs a *blocking* shutdown, which
        // panics if it runs inside an async context — and these strategies are
        // dropped during async service teardown (the bins build remote provers
        // inside the tokio runtime). `shutdown_background` releases the runtime
        // without blocking, so it is safe to drop from any context.
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

#[cfg(feature = "remote")]
impl<Host> RemoteStrategy<Host> {
    pub(crate) fn new(host: Host, poll_interval: Duration) -> Self {
        // Multi-thread (not current-thread): concurrent fanned-out `prove()`
        // calls each run on their own `spawn_blocking` thread and `block_on`
        // this runtime simultaneously; a current-thread runtime would serialize
        // them. The remote prove future itself runs on the calling thread via
        // the per-call `LocalSet`; these worker threads only drive the shared
        // gRPC channel and its IO, so a small pool is plenty.
        let rt = Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("paas-remote-prover")
            .enable_all()
            .build()
            .expect("build remote-prover runtime");
        Self {
            host: Arc::new(host),
            poll_interval,
            rt: Some(rt),
        }
    }
}

#[cfg(feature = "remote")]
impl<H, Host> ProveStrategy<H> for RemoteStrategy<Host>
where
    H: ProofSpec,
    Host: zkaleido::ZkVmRemoteHost + Send + Sync + 'static,
{
    fn prove(
        &self,
        input: &<H::Program as ZkVmProgram>::Input,
        mut ctx: ProveContext,
    ) -> ProverResult<ProofReceiptWithMetadata> {
        use tokio::task::LocalSet;

        let local = LocalSet::new();
        let host = self.host.clone();
        let poll_interval = self.poll_interval;

        // Drive on the strategy's long-lived runtime (see the type-level docs):
        // a fresh per-call runtime would kill SP1's process-cached gRPC channel
        // worker after the first prove. `rt` is `Some` for the whole lifetime;
        // it is only taken in `Drop`.
        let rt = self
            .rt
            .as_ref()
            .expect("remote-prover runtime present outside Drop");
        local.block_on(rt, async move {
            // Try to resume from saved metadata (prior crash).
            let proof_id = if let Some(saved) = ctx.saved.take() {
                match Host::ProofId::try_from(saved) {
                    Ok(id) => {
                        tracing::info!(%id, "resuming remote proof from saved metadata");
                        id
                    }
                    Err(_) => {
                        tracing::warn!("failed to deserialize saved ProofId, starting fresh");
                        return self.start_fresh::<H>(input, &host, &mut ctx).await;
                    }
                }
            } else {
                // Fresh submission.
                let id = self.submit_proof::<H>(input, &host).await?;
                ctx.persist(id.clone().into());
                id
            };

            // Poll until completion.
            self.poll_until_done::<H>(&host, &proof_id, poll_interval, &mut ctx)
                .await
        })
    }
}

#[cfg(feature = "remote")]
impl<Host> RemoteStrategy<Host>
where
    Host: zkaleido::ZkVmRemoteHost + Send + Sync + 'static,
{
    async fn submit_proof<H: ProofSpec>(
        &self,
        input: &<H::Program as ZkVmProgram>::Input,
        host: &Host,
    ) -> ProverResult<Host::ProofId> {
        let prepared = <H::Program as ZkVmProgram>::prepare_input::<Host::Input<'_>>(input)
            .map_err(|e| ProverError::PermanentFailure(e.to_string()))?;

        let proof_id = host
            .start_proving(prepared, H::Program::proof_type())
            .await
            .map_err(|e| ProverError::TransientFailure(e.to_string()))?;

        tracing::info!(%proof_id, "remote proof submitted");
        Ok(proof_id)
    }

    async fn start_fresh<H: ProofSpec>(
        &self,
        input: &<H::Program as ZkVmProgram>::Input,
        host: &Host,
        ctx: &mut ProveContext,
    ) -> ProverResult<ProofReceiptWithMetadata> {
        let proof_id = self.submit_proof::<H>(input, host).await?;
        ctx.persist(proof_id.clone().into());
        self.poll_until_done::<H>(host, &proof_id, self.poll_interval, ctx)
            .await
    }

    async fn poll_until_done<H: ProofSpec>(
        &self,
        host: &Host,
        proof_id: &Host::ProofId,
        poll_interval: Duration,
        ctx: &mut ProveContext,
    ) -> ProverResult<ProofReceiptWithMetadata> {
        use tokio::time::sleep;
        use zkaleido::RemoteProofStatus;

        loop {
            let status = host
                .get_status(proof_id)
                .await
                .map_err(|e| ProverError::TransientFailure(e.to_string()))?;

            match status {
                RemoteProofStatus::Completed => {
                    tracing::info!(%proof_id, "remote proof completed");
                    break;
                }
                RemoteProofStatus::Failed(reason) => {
                    // A remote worker killed by SIGBUS is no more permanent
                    // than a local one; classify the reason the same way.
                    let err = classify_prove_failure(format!("remote proof failed: {reason}"));
                    if err.is_transient() {
                        // `Failed` is terminal for *this* remote job, so a
                        // retry that resumed the persisted id would re-poll a
                        // dead job until the retry budget is gone. Forget the
                        // id: the retry then submits a fresh proving job,
                        // which is the whole point of calling this transient.
                        tracing::warn!(
                            %proof_id,
                            %reason,
                            "remote proof failed transiently; forgetting proof id so the retry \
                             submits a new remote job"
                        );
                        ctx.clear();
                    }
                    return Err(err);
                }
                RemoteProofStatus::Requested | RemoteProofStatus::InProgress => {
                    sleep(poll_interval).await;
                }
            }
        }

        // Retrieve the receipt.
        let receipt = host
            .get_proof(proof_id)
            .await
            .map_err(|e| ProverError::PermanentFailure(e.to_string()))?;

        // Verify output is well-formed.
        let _ =
            <H::Program as ZkVmProgram>::process_output::<Host>(receipt.receipt().public_values())
                .map_err(|e| ProverError::PermanentFailure(e.to_string()))?;

        Ok(receipt)
    }
}

#[cfg(all(test, feature = "remote"))]
mod tests {
    use std::{
        fmt,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        },
    };

    use async_trait::async_trait;
    use serde::{de::DeserializeOwned, Serialize};
    use zkaleido::{
        AggregationInput, ExecutionSummary, ProgramId, ProofType, PublicValues,
        RemoteProofFailureReason, RemoteProofStatus, VerifyingKey, ZkVm, ZkVmError, ZkVmExecutor,
        ZkVmHost, ZkVmInputBuilder, ZkVmInputResult, ZkVmOutputExtractor, ZkVmProofError,
        ZkVmProver, ZkVmRemoteProver, ZkVmResult, ZkVmTypedVerifier, ZkVmVkProvider,
    };

    use super::*;
    use crate::traits::ProofSpec;

    /// Input builder that carries nothing: the mock host never runs a program.
    struct MockInputBuilder;

    impl ZkVmInputBuilder<'_> for MockInputBuilder {
        type Input = ();
        type ZkVmProofReceipt = MockReceipt;

        fn new() -> Self {
            Self
        }

        fn write_buf(&mut self, _item: &[u8]) -> ZkVmInputResult<&mut Self> {
            Ok(self)
        }

        fn write_serde<T: serde::Serialize>(&mut self, _item: &T) -> ZkVmInputResult<&mut Self> {
            Ok(self)
        }

        fn write_proof(&mut self, _item: &AggregationInput) -> ZkVmInputResult<&mut Self> {
            Ok(self)
        }

        fn build(&mut self) -> ZkVmInputResult<Self::Input> {
            Ok(())
        }
    }

    /// Host-specific receipt wrapper required by the zkaleido host traits.
    #[derive(Debug, Clone)]
    struct MockReceipt(ProofReceiptWithMetadata);

    impl TryFrom<ProofReceiptWithMetadata> for MockReceipt {
        type Error = ZkVmProofError;

        fn try_from(receipt: ProofReceiptWithMetadata) -> Result<Self, Self::Error> {
            Ok(Self(receipt))
        }
    }

    impl TryFrom<MockReceipt> for ProofReceiptWithMetadata {
        type Error = ZkVmProofError;

        fn try_from(receipt: MockReceipt) -> Result<Self, Self::Error> {
            Ok(receipt.0)
        }
    }

    /// Opaque remote job handle. Distinct ids let the test tell a resumed
    /// poll apart from a freshly submitted job.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockProofId(u64);

    impl fmt::Display for MockProofId {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "mock-proof-{}", self.0)
        }
    }

    impl From<MockProofId> for Vec<u8> {
        fn from(id: MockProofId) -> Self {
            id.0.to_be_bytes().to_vec()
        }
    }

    impl TryFrom<Vec<u8>> for MockProofId {
        type Error = ();

        fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
            let arr: [u8; 8] = bytes.as_slice().try_into().map_err(|_| ())?;
            Ok(Self(u64::from_be_bytes(arr)))
        }
    }

    /// Remote host whose jobs always end in `Failed` with a configurable
    /// reason, counting submissions so a test can assert whether a retry
    /// resumed the old job or started a new one.
    #[derive(Debug, Clone)]
    struct FailingRemoteHost {
        reason: RemoteProofFailureReason,
        submissions: Arc<AtomicUsize>,
        polls: Arc<AtomicUsize>,
    }

    impl FailingRemoteHost {
        fn new(reason: RemoteProofFailureReason) -> Self {
            Self {
                reason,
                submissions: Arc::new(AtomicUsize::new(0)),
                polls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    // Only the remote-proving surface is exercised; the local proving and
    // verification surface is never reached from `RemoteStrategy::prove`.
    impl ZkVmExecutor for FailingRemoteHost {
        type Input<'a> = MockInputBuilder;

        fn execute<'a>(&self, _input: ()) -> ZkVmResult<ExecutionSummary> {
            unreachable!("remote strategy never executes locally")
        }

        fn get_elf(&self) -> &[u8] {
            &[]
        }

        fn program_id(&self) -> ProgramId {
            ProgramId([0u8; 32])
        }

        fn save_trace(&self, _trace_name: &str) {}
    }

    impl ZkVmProver for FailingRemoteHost {
        type ZkVmProofReceipt = MockReceipt;

        fn prove_inner<'a>(&self, _input: (), _proof_type: ProofType) -> ZkVmResult<MockReceipt> {
            unreachable!("remote strategy never proves locally")
        }
    }

    impl ZkVmTypedVerifier for FailingRemoteHost {
        type ZkVmProofReceipt = MockReceipt;

        fn verify_inner(&self, _receipt: &MockReceipt) -> ZkVmResult<()> {
            unreachable!("remote strategy never verifies")
        }
    }

    impl ZkVmVkProvider for FailingRemoteHost {
        fn vk(&self) -> VerifyingKey {
            VerifyingKey::new(Vec::new())
        }
    }

    impl ZkVmOutputExtractor for FailingRemoteHost {
        fn extract_serde_public_output<T: Serialize + DeserializeOwned>(
            _public_values: &PublicValues,
        ) -> ZkVmResult<T> {
            Err(ZkVmError::ProofVerificationError(
                "no output in this mock".to_string(),
            ))
        }
    }

    impl ZkVmHost for FailingRemoteHost {
        fn zkvm(&self) -> ZkVm {
            ZkVm::Native
        }
    }

    #[async_trait]
    impl ZkVmRemoteProver for FailingRemoteHost {
        type ProofId = MockProofId;

        async fn start_proving<'a>(
            &self,
            _input: (),
            _proof_type: ProofType,
        ) -> ZkVmResult<MockProofId> {
            let n = self.submissions.fetch_add(1, Ordering::SeqCst) + 1;
            // Ids of freshly submitted jobs start at 100 so they cannot be
            // confused with the pre-seeded "crashed run" id.
            Ok(MockProofId(100 + n as u64))
        }

        async fn get_status(&self, _id: &MockProofId) -> ZkVmResult<RemoteProofStatus> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            Ok(RemoteProofStatus::Failed(self.reason.clone()))
        }

        async fn get_proof(&self, _id: &MockProofId) -> ZkVmResult<ProofReceiptWithMetadata> {
            unreachable!("the mock host never completes a proof")
        }
    }

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

        fn process_output<Host>(_public_values: &PublicValues) -> ZkVmResult<Self::Output>
        where
            Host: ZkVmHost,
        {
            Ok(())
        }
    }

    struct MockSpec;

    #[async_trait]
    impl ProofSpec for MockSpec {
        type Task = String;
        type Program = MockProgram;

        async fn fetch_input(&self, _task: &String) -> ProverResult<()> {
            unreachable!("the strategy is driven directly in these tests")
        }
    }

    /// Stands in for the task record's metadata column: `persist` writes it,
    /// `clear` wipes it, and the next attempt reads it back as `ctx.saved`.
    #[derive(Default)]
    struct MetadataCell(Mutex<Option<Vec<u8>>>);

    impl MetadataCell {
        fn seeded(id: MockProofId) -> Arc<Self> {
            Arc::new(Self(Mutex::new(Some(id.into()))))
        }

        fn read(&self) -> Option<Vec<u8>> {
            self.0.lock().unwrap().clone()
        }

        fn context(self: &Arc<Self>) -> ProveContext {
            let saved = self.read();
            let cell = self.clone();
            ProveContext::new(saved, move |data| *cell.0.lock().unwrap() = data)
        }
    }

    fn strategy(host: FailingRemoteHost) -> RemoteStrategy<FailingRemoteHost> {
        RemoteStrategy::new(host, Duration::from_millis(1))
    }

    /// The regression this guards: a transient remote failure used to leave
    /// the dead remote job id in the task record, so every retry re-polled
    /// the same terminally failed job and no re-proof ever happened.
    #[test]
    fn transient_remote_failure_forgets_the_job_id_so_the_retry_resubmits() {
        let host = FailingRemoteHost::new(RemoteProofFailureReason::Other(
            "prove failed: exit status: signal: 7 (SIGBUS)".to_string(),
        ));
        let submissions = host.submissions.clone();
        let strategy = strategy(host);
        let metadata = MetadataCell::seeded(MockProofId(7));

        // Attempt 1 resumes the saved job, which is terminally failed.
        let err = ProveStrategy::<MockSpec>::prove(&strategy, &(), metadata.context())
            .expect_err("the mock host always fails");
        assert!(
            err.is_transient(),
            "expected a transient failure, got {err}"
        );
        assert_eq!(
            submissions.load(Ordering::SeqCst),
            0,
            "a resumed attempt must not submit a new job"
        );
        assert!(
            metadata.read().is_none(),
            "the dead remote job id must be forgotten so the retry starts fresh"
        );

        // Attempt 2 is what the retry scheduler does next: with no saved id
        // it submits a brand new remote job instead of re-polling the dead one.
        let err = ProveStrategy::<MockSpec>::prove(&strategy, &(), metadata.context())
            .expect_err("the mock host always fails");
        assert!(
            err.is_transient(),
            "expected a transient failure, got {err}"
        );
        assert_eq!(
            submissions.load(Ordering::SeqCst),
            1,
            "the retry must submit a fresh remote proving job"
        );
        assert!(metadata.read().is_none());
    }

    /// A genuinely permanent remote failure stays terminal and keeps its id,
    /// so nothing resubmits behind the operator's back.
    #[test]
    fn permanent_remote_failure_keeps_the_job_id() {
        let host = FailingRemoteHost::new(RemoteProofFailureReason::Unexecutable);
        let submissions = host.submissions.clone();
        let strategy = strategy(host);
        let metadata = MetadataCell::seeded(MockProofId(7));

        let err = ProveStrategy::<MockSpec>::prove(&strategy, &(), metadata.context())
            .expect_err("the mock host always fails");
        assert!(
            err.is_permanent(),
            "expected a permanent failure, got {err}"
        );
        assert_eq!(submissions.load(Ordering::SeqCst), 0);
        assert_eq!(
            metadata.read(),
            Some(MockProofId(7).into()),
            "a permanent failure must not disturb the persisted job id"
        );
    }

    /// The fresh-submission path persists the id before polling, so a crash
    /// between submit and completion can still resume it.
    #[test]
    fn fresh_submission_persists_the_job_id_before_polling() {
        let host = FailingRemoteHost::new(RemoteProofFailureReason::Unexecutable);
        let polls = host.polls.clone();
        let strategy = strategy(host);
        let metadata: Arc<MetadataCell> = Arc::new(MetadataCell::default());

        let err = ProveStrategy::<MockSpec>::prove(&strategy, &(), metadata.context())
            .expect_err("the mock host always fails");
        assert!(
            err.is_permanent(),
            "expected a permanent failure, got {err}"
        );
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        assert_eq!(
            metadata.read(),
            Some(MockProofId(101).into()),
            "the newly submitted job id must be persisted for crash recovery"
        );
    }
}
