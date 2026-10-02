//! Behavior tests for fixed hosts, remote recovery and configuration waits.

use std::{
    fmt,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use tokio::{
    sync::{Notify, Semaphore},
    task::yield_now,
    time::timeout,
};
use zkaleido::{
    ProofReceiptWithMetadata, ProofType, PublicValues, ZkVmEnvSerde, ZkVmHost, ZkVmInputBuilder,
    ZkVmInputResult, ZkVmProgram, ZkVmResult,
};
use zkaleido_native_adapter::NativeHost;

use super::*;
use crate::{AdmissionDecision, InMemoryReceiptStore, InputResolution, TaskAdmission, TaskRecord};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TestTask(u8);

impl fmt::Display for TestTask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<TestTask> for Vec<u8> {
    fn from(task: TestTask) -> Self {
        vec![task.0]
    }
}

impl TryFrom<Vec<u8>> for TestTask {
    type Error = ();

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        match bytes.as_slice() {
            [task] => Ok(Self(*task)),
            _ => Err(()),
        }
    }
}

struct TestProgram;

impl ZkVmProgram for TestProgram {
    type Input = u8;
    type Output = u16;

    fn name() -> String {
        "fixed_host_test".into()
    }

    fn proof_type() -> ProofType {
        ProofType::Core
    }

    fn prepare_input<'a, B: ZkVmInputBuilder<'a>>(input: &'a u8) -> ZkVmInputResult<B::Input> {
        B::new().write_serde(input)?.build()
    }

    fn process_output<H: ZkVmHost>(public_values: &PublicValues) -> ZkVmResult<u16> {
        H::extract_serde_public_output(public_values)
    }
}

#[derive(Clone)]
enum Resolution {
    Ready,
    CountedReady(Arc<AtomicUsize>),
    Blocked,
}

struct TestSpec(Resolution);

#[async_trait]
impl ProofSpec for TestSpec {
    type Task = TestTask;
    type Program = TestProgram;

    async fn resolve_input(&self, task: &TestTask) -> ProverResult<InputResolution<u8>> {
        Ok(match &self.0 {
            Resolution::Ready => InputResolution::Ready(task.0),
            Resolution::CountedReady(fetches) => {
                fetches.fetch_add(1, Ordering::SeqCst);
                InputResolution::Ready(task.0)
            }
            Resolution::Blocked => InputResolution::Blocked {
                reason: "dependency unavailable".into(),
                recheck_after: Some(Duration::ZERO),
            },
        })
    }
}

#[derive(Clone, Copy)]
enum AdmissionBehavior {
    Admit,
    AwaitingConfiguration(&'static str),
    PermanentFailure,
    StorageFailure,
}

struct TestAdmission {
    behavior: AdmissionBehavior,
    checks: Arc<AtomicUsize>,
}

impl TestAdmission {
    fn new(behavior: AdmissionBehavior) -> Self {
        Self {
            behavior,
            checks: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl TaskAdmission<TestSpec> for TestAdmission {
    async fn check(&self, _: &TestTask) -> ProverResult<AdmissionDecision> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        match self.behavior {
            AdmissionBehavior::Admit => Ok(AdmissionDecision::Admit),
            AdmissionBehavior::AwaitingConfiguration(reason) => {
                Ok(AdmissionDecision::AwaitingConfiguration {
                    reason: reason.into(),
                    recheck_after: Some(Duration::ZERO),
                })
            }
            AdmissionBehavior::PermanentFailure => {
                Err(ProverError::permanent("task cannot be admitted"))
            }
            AdmissionBehavior::StorageFailure => {
                Err(ProverError::Storage("admission lookup failed".into()))
            }
        }
    }
}

fn recorded_native_host(version: u16, executions: Arc<AtomicUsize>) -> NativeHost {
    NativeHost::new_with_random_key(move |machine| {
        executions.fetch_add(1, Ordering::SeqCst);
        let input: u8 = machine.read_serde();
        machine.commit_serde(&(version * 100 + u16::from(input)));
    })
}

async fn wait_for_blocked(prover: &Prover<TestSpec>, task: u8) {
    timeout(Duration::from_secs(5), async {
        loop {
            if prover.get_status(&TestTask(task)).unwrap().is_blocked() {
                return;
            }
            yield_now().await;
        }
    })
    .await
    .unwrap();
}

fn native_host(version: u16) -> NativeHost {
    NativeHost::new_with_random_key(move |machine| {
        let input: u8 = machine.read_serde();
        machine.commit_serde(&(version * 100 + u16::from(input)));
    })
}

fn register(prover: &Prover<TestSpec>, task: u8) {
    prover
        .task_store
        .insert(TaskRecord::new(vec![task], TaskStatus::Pending))
        .unwrap();
}

fn output(prover: &Prover<TestSpec>, task: u8) -> u16 {
    let receipt = prover.get_receipt(&TestTask(task)).unwrap().unwrap();
    TestProgram::process_output::<NativeHost>(receipt.receipt().public_values()).unwrap()
}

#[tokio::test]
async fn independent_native_services_use_their_fixed_hosts() {
    let provers = [1, 2].map(|version| {
        Arc::new(
            ProverBuilder::new(TestSpec(Resolution::Ready))
                .receipt_store(InMemoryReceiptStore::new())
                .native(native_host(version)),
        )
    });
    for (prover, expected_output) in provers.iter().zip([100, 200]) {
        let result = timeout(Duration::from_secs(5), prover.execute(TestTask(0)))
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_completed());
        assert_eq!(
            prover.get_status(&TestTask(0)).unwrap(),
            TaskStatus::Completed
        );
        assert_eq!(output(prover, 0), expected_output);
    }
}

#[tokio::test]
async fn configuration_wait_preserves_budgets_and_metadata_until_restart() {
    let store = Arc::new(InMemoryTaskStore::new());
    let counts = AttemptCounts {
        retry: 2,
        resubmit: 1,
        recheck: 1,
    };
    let metadata = vec![7, 8, 9];
    let fetches = Arc::new(AtomicUsize::new(0));
    let executions = Arc::new(AtomicUsize::new(0));
    let prover = ProverBuilder::new(TestSpec(Resolution::CountedReady(fetches.clone())))
        .task_admission(TestAdmission::new(
            AdmissionBehavior::AwaitingConfiguration("program not installed"),
        ))
        .task_store(store.clone())
        .retry(RetryConfig {
            max_blocked_rechecks: 1,
            ..RetryConfig::default()
        })
        .native(recorded_native_host(1, executions.clone()));
    register(&prover, 0);
    store
        .update_status(
            &[0],
            TaskStatus::Blocked {
                reason: "previous wait".into(),
                counts,
            },
        )
        .unwrap();
    store.set_metadata(&[0], metadata.clone()).unwrap();
    for _ in 0..4 {
        prover.run_task(TestTask(0), vec![0]).await;
        let record = store.get(&[0]).unwrap().unwrap();
        assert_eq!(
            record.status(),
            &TaskStatus::Blocked {
                reason: "program not installed".into(),
                counts
            }
        );
        assert_eq!(record.metadata(), Some(metadata.as_slice()));
        assert!(record.retry_after_secs().is_some());
    }
    assert_eq!(fetches.load(Ordering::SeqCst), 0);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    drop(prover);

    // A new service over the same records picks up the parked task through its
    // ordinary retry scanner once the operator installs the missing program.
    let restarted = Arc::new(
        ProverBuilder::new(TestSpec(Resolution::CountedReady(fetches.clone())))
            .task_admission(TestAdmission::new(AdmissionBehavior::Admit))
            .task_store(store)
            .receipt_store(InMemoryReceiptStore::new())
            .native(recorded_native_host(2, executions.clone())),
    );
    restarted.tick().await;
    let results = timeout(
        Duration::from_secs(5),
        restarted.wait_for_tasks(&[TestTask(0)]),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(results[0].is_completed());
    assert_eq!(output(&restarted, 0), 200);
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ordinary_dependency_wait_still_exhausts_its_recheck_budget() {
    let prover = ProverBuilder::new(TestSpec(Resolution::Blocked))
        .retry(RetryConfig {
            max_blocked_rechecks: 1,
            ..RetryConfig::default()
        })
        .native(native_host(1));
    register(&prover, 0);
    prover.run_task(TestTask(0), vec![0]).await;
    assert_eq!(prover.get_status(&TestTask(0)).unwrap().counts().recheck, 1);
    prover.run_task(TestTask(0), vec![0]).await;
    assert_eq!(
        prover.get_status(&TestTask(0)).unwrap(),
        TaskStatus::PermanentFailure {
            error: "blocked dependency unresolved after 1 rechecks: dependency unavailable".into(),
        }
    );
}

#[cfg(feature = "remote")]
mod remote {
    use std::collections::HashMap;

    use serde::{de::DeserializeOwned, Serialize};
    use zkaleido::{
        ExecutionSummary, ProgramId, ProofMetadata, ProofReceiptWithMetadata, RemoteProofStatus,
        VerifyingKey, ZkVm, ZkVmExecutor, ZkVmOutputExtractor, ZkVmProver, ZkVmRemoteProver,
        ZkVmTypedVerifier, ZkVmVkProvider,
    };
    use zkaleido_native_adapter::NativeMachine;

    use super::*;
    use crate::{config::LocalRetryConfig, strategy::RemoteStrategy};

    #[derive(Debug, Default)]
    struct Backend {
        receipts: Mutex<HashMap<u8, ProofReceiptWithMetadata>>,
        submissions: AtomicUsize,
        polls: Mutex<Vec<(ProgramId, u8)>>,
        verifications: AtomicUsize,
    }

    #[derive(Debug, Clone)]
    struct RemoteHost {
        native: NativeHost,
        backend: Arc<Backend>,
    }

    impl RemoteHost {
        fn new(version: u16, backend: Arc<Backend>) -> Self {
            Self {
                native: native_host(version),
                backend,
            }
        }
    }

    impl ZkVmHost for RemoteHost {
        fn zkvm(&self) -> ZkVm {
            ZkVm::Native
        }
    }

    impl ZkVmExecutor for RemoteHost {
        type Input<'a> = <NativeHost as ZkVmExecutor>::Input<'a>;
        fn execute<'a>(&self, input: NativeMachine) -> ZkVmResult<ExecutionSummary> {
            self.native.execute(input)
        }
        fn get_elf(&self) -> &[u8] {
            self.native.get_elf()
        }
        fn program_id(&self) -> ProgramId {
            self.native.program_id()
        }
        fn save_trace(&self, trace_name: &str) {
            self.native.save_trace(trace_name);
        }
    }

    impl ZkVmProver for RemoteHost {
        type ZkVmProofReceipt = <NativeHost as ZkVmProver>::ZkVmProofReceipt;
        fn prove_inner<'a>(
            &self,
            input: NativeMachine,
            proof_type: ProofType,
        ) -> ZkVmResult<Self::ZkVmProofReceipt> {
            self.native.prove_inner(input, proof_type)
        }
    }

    impl ZkVmTypedVerifier for RemoteHost {
        type ZkVmProofReceipt = <NativeHost as ZkVmTypedVerifier>::ZkVmProofReceipt;
        fn verify_inner(&self, receipt: &Self::ZkVmProofReceipt) -> ZkVmResult<()> {
            self.backend.verifications.fetch_add(1, Ordering::SeqCst);
            self.native.verify_inner(receipt)
        }
    }

    impl ZkVmVkProvider for RemoteHost {
        fn vk(&self) -> VerifyingKey {
            self.native.vk()
        }
    }

    impl ZkVmOutputExtractor for RemoteHost {
        fn extract_serde_public_output<T: Serialize + DeserializeOwned>(
            values: &PublicValues,
        ) -> ZkVmResult<T> {
            NativeHost::extract_serde_public_output(values)
        }
    }

    #[async_trait]
    impl ZkVmRemoteProver for RemoteHost {
        type ProofId = TestTask;
        async fn start_proving<'a>(
            &self,
            input: NativeMachine,
            proof_type: ProofType,
        ) -> ZkVmResult<TestTask> {
            let id = self.backend.submissions.fetch_add(1, Ordering::SeqCst) as u8;
            self.backend
                .receipts
                .lock()
                .insert(id, self.prove(input, proof_type)?);
            Ok(TestTask(id))
        }
        async fn get_status(&self, id: &TestTask) -> ZkVmResult<RemoteProofStatus> {
            self.backend.polls.lock().push((self.program_id(), id.0));
            Ok(RemoteProofStatus::Completed)
        }
        async fn get_proof(&self, id: &TestTask) -> ZkVmResult<ProofReceiptWithMetadata> {
            let receipt = self.backend.receipts.lock().get(&id.0).unwrap().clone();
            // Match the SP1 adapter's metadata behavior: querying with the wrong
            // host still labels this receipt with the querying host's program ID.
            Ok(ProofReceiptWithMetadata::new(
                receipt.receipt().clone(),
                ProofMetadata::new(ZkVm::Native, self.program_id(), "test", ProofType::Core),
            ))
        }
    }

    fn prove_with_saved(
        host: RemoteHost,
        saved: Option<Vec<u8>>,
    ) -> (ProverResult<ProofReceiptWithMetadata>, Option<Vec<u8>>) {
        let strategy = RemoteStrategy::new(host, Duration::ZERO, LocalRetryConfig::default());
        let persisted = Arc::new(Mutex::new(None));
        let observed = persisted.clone();
        let result = <RemoteStrategy<_> as ProveStrategy<TestSpec>>::prove(
            &strategy,
            &0,
            ProveContext::new(saved, move |metadata| *observed.lock() = Some(metadata)),
        );
        let metadata = persisted.lock().clone();
        (result, metadata)
    }

    #[tokio::test]
    async fn independent_remote_services_use_their_fixed_hosts() {
        let backend = Arc::new(Backend::default());
        let provers = [1, 2].map(|version| {
            Arc::new(
                ProverBuilder::new(TestSpec(Resolution::Ready))
                    .receipt_store(InMemoryReceiptStore::new())
                    .remote_with_interval(
                        RemoteHost::new(version, backend.clone()),
                        Duration::ZERO,
                    ),
            )
        });
        for (prover, expected_output) in provers.iter().zip([100, 200]) {
            let result = timeout(Duration::from_secs(5), prover.execute(TestTask(0)))
                .await
                .unwrap()
                .unwrap();
            assert!(result.is_completed());
            assert_eq!(
                prover.get_status(&TestTask(0)).unwrap(),
                TaskStatus::Completed
            );
            assert_eq!(output(prover, 0), expected_output);
        }
        assert_eq!(backend.submissions.load(Ordering::SeqCst), 2);
        assert_eq!(backend.verifications.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn opaque_request_id_roundtrips_and_resumes_after_restart() {
        let backend = Arc::new(Backend::default());
        let host = RemoteHost::new(1, backend.clone());
        let (first_result, saved) = prove_with_saved(host.clone(), None);
        let first_receipt = first_result.unwrap();
        let saved = saved.unwrap();
        assert_eq!(saved, vec![0]);
        assert_eq!(TestTask::try_from(saved.clone()).unwrap(), TestTask(0));

        // Reconstructing the strategy resumes the opaque request ID without
        // resubmission or an additional local proof verification.
        let (resumed_result, replacement) = prove_with_saved(host.clone(), Some(saved));
        assert_eq!(resumed_result.unwrap(), first_receipt);
        assert!(replacement.is_none());
        assert_eq!(backend.submissions.load(Ordering::SeqCst), 1);
        assert_eq!(
            backend.polls.lock().as_slice(),
            &[(host.program_id(), 0), (host.program_id(), 0)]
        );
        assert_eq!(backend.verifications.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn malformed_opaque_request_id_starts_a_fresh_request() {
        let backend = Arc::new(Backend::default());
        let host = RemoteHost::new(1, backend.clone());
        let (result, saved) = prove_with_saved(host.clone(), None);
        result.unwrap();
        let mut saved = saved.unwrap();
        saved.push(0);
        let (result, replacement) = prove_with_saved(host, Some(saved));
        result.unwrap();
        assert_eq!(replacement, Some(vec![1]));
        assert_eq!(backend.submissions.load(Ordering::SeqCst), 2);
        assert_eq!(backend.polls.lock()[1].1, 1);
        assert_eq!(backend.verifications.load(Ordering::SeqCst), 0);
    }
}

mod configuration_logging {
    use tracing::{
        instrument::WithSubscriber,
        span::{Attributes, Id, Record},
        Event, Level, Metadata, Subscriber,
    };

    use super::*;

    #[derive(Clone)]
    struct ErrorCounter(Arc<AtomicUsize>);

    impl Subscriber for ErrorCounter {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }
        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn event(&self, event: &Event<'_>) {
            if *event.metadata().level() == Level::ERROR {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
    }

    #[tokio::test]
    async fn configuration_wait_alerts_once_per_cause_across_rechecks_and_restart() {
        let errors = ErrorCounter(Arc::new(AtomicUsize::new(0)));
        let store = Arc::new(InMemoryTaskStore::new());
        for (reason, expected_errors) in [
            ("program not installed", 1),
            ("program not installed", 1),
            ("program has incompatible parameters", 2),
        ] {
            let prover = ProverBuilder::new(TestSpec(Resolution::Ready))
                .task_admission(TestAdmission::new(
                    AdmissionBehavior::AwaitingConfiguration(reason),
                ))
                .task_store(store.clone())
                .native(native_host(1));
            if store.get(&[0]).unwrap().is_none() {
                register(&prover, 0);
            }
            for _ in 0..3 {
                prover
                    .run_task(TestTask(0), vec![0])
                    .with_subscriber(errors.clone())
                    .await;
                assert_eq!(errors.0.load(Ordering::SeqCst), expected_errors);
            }
        }
    }
}

#[tokio::test]
async fn admission_defers_fresh_recovered_and_retried_tasks_before_input_fetch() {
    let previous_counts = AttemptCounts {
        retry: 2,
        resubmit: 1,
        recheck: 1,
    };
    for initial_status in [
        None,
        Some(TaskStatus::Pending),
        Some(TaskStatus::Proving {
            counts: previous_counts,
        }),
        Some(TaskStatus::TransientFailure {
            counts: previous_counts,
            error: "previous failure".into(),
        }),
    ] {
        let store = Arc::new(InMemoryTaskStore::new());
        let fetches = Arc::new(AtomicUsize::new(0));
        let executions = Arc::new(AtomicUsize::new(0));
        let admission = TestAdmission::new(AdmissionBehavior::AwaitingConfiguration(
            "program not installed",
        ));
        let checks = admission.checks.clone();
        let prover = Arc::new(
            ProverBuilder::new(TestSpec(Resolution::CountedReady(fetches.clone())))
                .task_admission(admission)
                .task_store(store.clone())
                .retry(RetryConfig::default())
                .native(recorded_native_host(1, executions.clone())),
        );
        let mut expected_counts = initial_status
            .as_ref()
            .map_or(AttemptCounts::default(), TaskStatus::counts);
        if let Some(status) = initial_status {
            let recovering_in_progress = status.is_in_progress();
            store.insert(TaskRecord::new(vec![0], status)).unwrap();
            store.set_metadata(&[0], vec![9]).unwrap();
            store.set_retry_after(&[0], 0).unwrap();
            prover.tick().await;
            if recovering_in_progress {
                // Crash recovery accounts for the interrupted attempt before
                // the retry scanner invokes admission.
                expected_counts.retry += 1;
                assert_eq!(checks.load(Ordering::SeqCst), 0);
                store.set_retry_after(&[0], 0).unwrap();
                prover.tick().await;
            }
        } else {
            prover.submit(TestTask(0)).await.unwrap();
        }
        wait_for_blocked(&prover, 0).await;
        assert_eq!(
            prover.get_status(&TestTask(0)).unwrap().counts(),
            expected_counts
        );
        assert_eq!(checks.load(Ordering::SeqCst), 1);
        assert_eq!(fetches.load(Ordering::SeqCst), 0);
        assert_eq!(executions.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn admission_failures_use_normal_classification_without_fetching_inputs() {
    for (behavior, permanent) in [
        (AdmissionBehavior::PermanentFailure, true),
        (AdmissionBehavior::StorageFailure, false),
    ] {
        let fetches = Arc::new(AtomicUsize::new(0));
        let executions = Arc::new(AtomicUsize::new(0));
        let prover = ProverBuilder::new(TestSpec(Resolution::CountedReady(fetches.clone())))
            .task_admission(TestAdmission::new(behavior))
            .retry(RetryConfig::default())
            .native(recorded_native_host(1, executions.clone()));
        register(&prover, 0);
        prover.run_task(TestTask(0), vec![0]).await;
        let status = prover.get_status(&TestTask(0)).unwrap();
        if permanent {
            assert_eq!(
                status,
                TaskStatus::PermanentFailure {
                    error: "task cannot be admitted".into()
                }
            );
        } else {
            assert_eq!(
                status,
                TaskStatus::TransientFailure {
                    counts: AttemptCounts {
                        retry: 1,
                        ..AttemptCounts::default()
                    },
                    error: "storage: admission lookup failed".into(),
                }
            );
        }
        assert_eq!(fetches.load(Ordering::SeqCst), 0);
        assert_eq!(executions.load(Ordering::SeqCst), 0);
    }
}

struct SlowAdmission {
    entered: Arc<Notify>,
    release: Arc<Semaphore>,
    checks: Arc<AtomicUsize>,
}

#[async_trait]
impl TaskAdmission<TestSpec> for SlowAdmission {
    async fn check(&self, _: &TestTask) -> ProverResult<AdmissionDecision> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        self.release.acquire().await.unwrap().forget();
        Ok(AdmissionDecision::Admit)
    }
}

#[tokio::test]
async fn slow_admission_claims_the_attempt_before_repeated_retry_ticks() {
    let store = Arc::new(InMemoryTaskStore::new());
    store
        .insert(TaskRecord::new(
            vec![0],
            TaskStatus::Blocked {
                reason: "previous wait".into(),
                counts: AttemptCounts::default(),
            },
        ))
        .unwrap();
    store.set_retry_after(&[0], 0).unwrap();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Semaphore::new(0));
    let checks = Arc::new(AtomicUsize::new(0));
    let fetches = Arc::new(AtomicUsize::new(0));
    let executions = Arc::new(AtomicUsize::new(0));
    let prover = Arc::new(
        ProverBuilder::new(TestSpec(Resolution::CountedReady(fetches.clone())))
            .task_admission(SlowAdmission {
                entered: entered.clone(),
                release: release.clone(),
                checks: checks.clone(),
            })
            .task_store(store)
            .native(recorded_native_host(1, executions.clone())),
    );
    prover.tick().await;
    timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    assert_eq!(
        prover.get_status(&TestTask(0)).unwrap(),
        TaskStatus::Proving {
            counts: AttemptCounts::default()
        }
    );
    for _ in 0..4 {
        prover.tick().await;
        yield_now().await;
    }
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    assert_eq!(fetches.load(Ordering::SeqCst), 0);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    release.add_permits(1);
    let results = timeout(
        Duration::from_secs(5),
        prover.wait_for_tasks(&[TestTask(0)]),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(results[0].is_completed());
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

struct CountingReceiptHook(Arc<AtomicUsize>);

#[async_trait]
impl ReceiptHook<TestSpec> for CountingReceiptHook {
    async fn on_receipt(&self, _: &TestTask, _: &ProofReceiptWithMetadata) -> ProverResult<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn stored_receipt_finishes_without_admission_or_new_proving() {
    let receipts = Arc::new(InMemoryReceiptStore::new());
    let receipt = TestProgram::prove(&0, &native_host(1)).unwrap();
    receipts.put(&[0], &receipt).unwrap();
    let admission = TestAdmission::new(AdmissionBehavior::AwaitingConfiguration(
        "program not installed",
    ));
    let checks = admission.checks.clone();
    let fetches = Arc::new(AtomicUsize::new(0));
    let executions = Arc::new(AtomicUsize::new(0));
    let hooks = Arc::new(AtomicUsize::new(0));
    let prover = Arc::new(
        ProverBuilder::new(TestSpec(Resolution::CountedReady(fetches.clone())))
            .task_admission(admission)
            .receipt_store(receipts)
            .receipt_hook(CountingReceiptHook(hooks.clone()))
            .native(recorded_native_host(1, executions.clone())),
    );
    let result = timeout(Duration::from_secs(5), prover.execute(TestTask(0)))
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_completed());
    assert_eq!(checks.load(Ordering::SeqCst), 0);
    assert_eq!(fetches.load(Ordering::SeqCst), 0);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert_eq!(hooks.load(Ordering::SeqCst), 1);
    assert_eq!(prover.get_receipt(&TestTask(0)).unwrap(), Some(receipt));
}
