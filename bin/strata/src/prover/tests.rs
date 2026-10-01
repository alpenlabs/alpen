//! Exercises checkpoint admission separately from witness assembly and task recovery.

use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use strata_asm_checkpoint_types::{CheckpointInitConfig, PendingPredicateTransition};
use strata_asm_common::{SectionState, SectionStateExt};
use strata_asm_proto_checkpoint::{CheckpointState, CheckpointSubprotocol};
use strata_checkpoint_types::EpochSummary;
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_db_tests::asm_tests::make_test_asm_state;
use strata_db_types::asm::AsmExecOutput;
use strata_identifiers::{
    Buf32, EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
};
use strata_ol_chain_types_v1::OLBlockV1;
use strata_ol_checkpoint_artifacts::{
    CheckpointArtifactRegistry, LoadedCheckpointPredicates, native_checkpoint_registry,
};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_container::{OLStateContainer, test_utils::create_test_container_with_staged};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{IStateAccessorMut, OLSpecId, OLSpecVersions};
use strata_ol_stf::{BlockComponents, BlockContext, BlockInfo, construct_block};
use strata_ol_stf_v1::test_utils::{
    EPOCH_RUNNER_GENESIS_TIMESTAMP, EPOCH_RUNNER_SLOT_TIMESTAMP_STEP, make_empty_manifest,
    make_genesis_state, to_ol_block,
};
use strata_paas::{
    AdmissionDecision, AttemptCounts, FailureAction, InMemoryReceiptStore, InputResolution,
    ProofSpec, Prover, ProverBuilder, TaskAdmission, TaskRecord, TaskStatus, TaskStore,
};
use strata_predicate::PredicateKey;
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use strata_storage::{NodeStorage, VersionedTaskStore, create_node_storage, test_runtime_handle};
use tokio::time::timeout;

use super::{
    admission::CheckpointAdmission,
    errors::ProverError,
    spec::{CheckpointSpec, CheckpointTask, checkpoint_task_spec},
};

struct EpochFixture {
    storage: Arc<NodeStorage>,
    parent: OLBlockV1,
    terminal: OLBlockV1,
    start_state: OLStateContainer,
    task: CheckpointTask,
}

impl EpochFixture {
    fn new() -> Self {
        let storage =
            Arc::new(create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap());
        let runtime_params = OLRuntimeParams::test_default();
        let mut state = make_genesis_state();
        let genesis_info = BlockInfo::new_genesis(EPOCH_RUNNER_GENESIS_TIMESTAMP);
        let genesis = construct_block(
            OLSpecId::V1,
            &mut state,
            BlockContext::new(&genesis_info, None),
            BlockComponents::new_manifests(vec![make_empty_manifest(1, 0)]).as_terminal(),
            &runtime_params,
        )
        .unwrap();
        let parent = to_ol_block(genesis.completed_block());
        let start_state = state.to_container();
        let terminal_manifest = make_empty_manifest(2, 0);
        let block_info = BlockInfo::new(
            EPOCH_RUNNER_GENESIS_TIMESTAMP + EPOCH_RUNNER_SLOT_TIMESTAMP_STEP,
            1,
            1,
        );
        let output = construct_block(
            OLSpecId::V1,
            &mut state,
            BlockContext::new(&block_info, Some(parent.header())),
            BlockComponents::new_manifests(vec![terminal_manifest.clone()]).as_terminal(),
            &runtime_params,
        )
        .unwrap();
        let terminal = to_ol_block(output.completed_block());
        let summary = EpochSummary::new(
            1,
            terminal.header().compute_block_commitment(),
            parent.header().compute_block_commitment(),
            L1BlockCommitment::new(terminal_manifest.height(), *terminal_manifest.blkid()),
            *terminal.header().state_root(),
        );
        for block in [&parent, &terminal] {
            storage
                .ol_block()
                .put_block_data_blocking(block.clone())
                .unwrap();
        }
        storage
            .ol_state()
            .put_toplevel_ol_state_blocking(
                parent.header().compute_block_commitment(),
                start_state.clone(),
            )
            .unwrap();
        storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(summary)
            .unwrap();
        Self {
            storage,
            parent,
            terminal,
            start_state,
            task: CheckpointTask(summary.get_epoch_commitment()),
        }
    }

    fn resolve(&self) -> InputResolution<CheckpointProverInput> {
        self.resolve_on(OLSpecId::V1)
    }

    fn admission(&self, predicates: LoadedCheckpointPredicates) -> AdmissionDecision {
        let admission =
            CheckpointAdmission::new(Arc::clone(&self.storage), predicates, OLSpecId::V1);
        test_runtime_handle()
            .block_on(admission.check(&self.task))
            .unwrap()
    }

    fn resolve_on(&self, assigned_spec: OLSpecId) -> InputResolution<CheckpointProverInput> {
        let spec = CheckpointSpec::new(
            Arc::clone(&self.storage),
            OLRuntimeParams::test_default(),
            assigned_spec,
        );
        test_runtime_handle()
            .block_on(spec.resolve_input(&self.task))
            .unwrap()
    }

    fn discard_epoch_blocks(&self) {
        self.storage
            .ol_block()
            .del_block_data_blocking(self.terminal.header().compute_blkid())
            .unwrap();
    }

    fn set_start_spec(&self, staged_version: u32) {
        let mut state = MemoryStateBaseLayer::from_container(self.start_state.clone());
        state.set_spec_versions(OLSpecVersions::new(OLSpecId::V1, staged_version).unwrap());
        self.storage
            .ol_state()
            .put_toplevel_ol_state_blocking(
                self.parent.header().compute_block_commitment(),
                state.into_container(),
            )
            .unwrap();
    }

    fn put_asm_checkpoint(&self, active: PredicateKey, pending: Option<PredicateKey>) {
        let mut checkpoint = CheckpointState::init(CheckpointInitConfig {
            sequencer_key: Buf32::zero(),
            checkpoint_predicate: active,
            genesis_l1_height: 1,
            genesis_ol_blkid: self.parent.header().compute_blkid(),
        });
        if let Some(predicate) = pending {
            checkpoint.queue_predicate_transition(PendingPredicateTransition::new(predicate, 10));
        }
        let mut anchor = make_test_asm_state().state().clone();
        anchor.sections =
            vec![SectionState::from_state::<CheckpointSubprotocol>(&checkpoint).unwrap()]
                .try_into()
                .unwrap();
        let height = self
            .storage
            .l1()
            .get_canonical_chain_tip()
            .unwrap()
            .map_or(1, |(height, _)| height + 1);
        let block = L1BlockCommitment::new(
            height,
            L1BlockId::from(Buf32::from([u8::try_from(height).unwrap(); 32])),
        );
        self.storage
            .l1()
            .extend_canonical_chain(block.blkid(), block.height())
            .unwrap();
        self.storage
            .asm()
            .put_state_blocking(block, AsmExecOutput::new(anchor, vec![]))
            .unwrap();
    }
}

fn loaded_predicates() -> LoadedCheckpointPredicates {
    native_checkpoint_registry(OLRuntimeParams::test_default()).to_predicates()
}

#[test]
fn admission_waits_for_missing_artifact_without_assembling_a_witness() {
    let fixture = EpochFixture::new();
    // Absence of the block would reject the task if input assembly reached DA first.
    fixture.discard_epoch_blocks();
    let empty = CheckpointArtifactRegistry::<()>::empty().to_predicates();
    assert!(matches!(
        fixture.admission(empty),
        AdmissionDecision::AwaitingConfiguration { .. }
    ));
}

#[test]
fn unsupported_staged_spec_rejects_witness_without_v1_fallback() {
    let fixture = EpochFixture::new();
    fixture.set_start_spec(2);
    assert!(matches!(
        checkpoint_task_spec(&fixture.storage, fixture.task),
        Err(ProverError::UnsupportedSpec(version)) if version.raw() == 2
    ));
    fixture.discard_epoch_blocks();
    assert!(matches!(
        fixture.resolve(),
        InputResolution::Rejected { .. }
    ));
}

#[test]
fn exact_epoch_start_selects_v1_even_when_newer_local_state_stages_unknown_spec() {
    let fixture = EpochFixture::new();
    let ahead = OLBlockCommitment::new(20, OLBlockId::from(Buf32::from([4; 32])));
    fixture
        .storage
        .ol_state()
        .put_toplevel_ol_state_blocking(ahead, create_test_container_with_staged(2))
        .unwrap();
    assert_eq!(
        checkpoint_task_spec(&fixture.storage, fixture.task).unwrap(),
        OLSpecId::V1,
    );
    let InputResolution::Ready(input) = fixture.resolve() else {
        panic!("a supported historical epoch must remain provable");
    };
    assert_eq!(input.start_state, fixture.start_state);
    assert_eq!(input.parent, *fixture.parent.header());
    assert_eq!(input.blocks, vec![fixture.terminal]);
    CheckpointProgram::execute(&input, OLSpecId::V1, OLRuntimeParams::test_default()).unwrap();
}

#[test]
fn missing_pending_artifact_does_not_block_valid_epoch_input() {
    let fixture = EpochFixture::new();
    let predicates = loaded_predicates();
    fixture.put_asm_checkpoint(
        predicates.predicate(OLSpecId::V1).unwrap().clone(),
        Some(PredicateKey::never_accept()),
    );
    assert!(matches!(
        fixture.admission(predicates),
        AdmissionDecision::Admit
    ));
    let InputResolution::Ready(input) = fixture.resolve() else {
        panic!("missing future artifacts must not stop supported epochs");
    };
    CheckpointProgram::execute(&input, OLSpecId::V1, OLRuntimeParams::test_default()).unwrap();
}

#[test]
fn task_uses_its_loaded_artifact_while_its_vk_is_pending_in_asm() {
    let fixture = EpochFixture::new();
    fixture.put_asm_checkpoint(
        PredicateKey::never_accept(),
        Some(CheckpointProgram::test_predicate_key()),
    );
    assert_eq!(
        checkpoint_task_spec(&fixture.storage, fixture.task).unwrap(),
        OLSpecId::V1,
    );
    assert!(matches!(
        fixture.admission(loaded_predicates()),
        AdmissionDecision::Admit
    ));
    let InputResolution::Ready(input) = fixture.resolve() else {
        panic!("ASM lag must not block an epoch whose artifact is loaded");
    };
    CheckpointProgram::execute(&input, OLSpecId::V1, OLRuntimeParams::test_default()).unwrap();
}

#[test]
fn corrected_registry_resolves_previously_unavailable_epoch_from_same_storage() {
    let fixture = EpochFixture::new();
    let empty = CheckpointArtifactRegistry::<()>::empty().to_predicates();
    assert!(matches!(
        fixture.admission(empty),
        AdmissionDecision::AwaitingConfiguration { .. }
    ));
    assert!(matches!(
        fixture.admission(loaded_predicates()),
        AdmissionDecision::Admit
    ));
    let InputResolution::Ready(input) = fixture.resolve() else {
        panic!("supplying the required artifact must unblock the same epoch");
    };
    CheckpointProgram::execute(&input, OLSpecId::V1, OLRuntimeParams::test_default()).unwrap();
}

#[test]
fn fixed_host_service_rejects_an_epoch_assigned_to_another_spec() {
    let fixture = EpochFixture::new();
    fixture.discard_epoch_blocks();
    // V0 is only an incorrect service assignment. No V0 host is built or executed.
    assert!(matches!(
        fixture.resolve_on(OLSpecId::V0),
        InputResolution::Rejected { .. }
    ));
}

#[test]
fn task_routing_survives_service_restart_and_changes_to_current_asm_predicate() {
    let fixture = EpochFixture::new();
    fixture.put_asm_checkpoint(CheckpointProgram::test_predicate_key(), None);
    assert_eq!(
        checkpoint_task_spec(&fixture.storage, fixture.task).unwrap(),
        OLSpecId::V1,
    );
    fixture.put_asm_checkpoint(PredicateKey::never_accept(), None);
    assert_eq!(
        checkpoint_task_spec(&fixture.storage, fixture.task).unwrap(),
        OLSpecId::V1,
    );

    // Recreate all storage managers to discard in-memory state. A restarted router
    // derives the same assignment solely from the persisted epoch start.
    let storage = Arc::new(
        create_node_storage(Arc::clone(fixture.storage.db()), test_runtime_handle()).unwrap(),
    );
    assert_eq!(
        checkpoint_task_spec(&storage, fixture.task).unwrap(),
        OLSpecId::V1
    );
    let restarted = CheckpointSpec::new(
        Arc::clone(&storage),
        OLRuntimeParams::test_default(),
        OLSpecId::V1,
    );
    assert!(matches!(
        test_runtime_handle()
            .block_on(restarted.resolve_input(&fixture.task))
            .unwrap(),
        InputResolution::Ready(_)
    ));
    let admission = CheckpointAdmission::new(storage, loaded_predicates(), OLSpecId::V1);
    assert!(matches!(
        test_runtime_handle()
            .block_on(admission.check(&fixture.task))
            .unwrap(),
        AdmissionDecision::Admit
    ));
}

#[test]
fn missing_exact_start_state_waits_without_using_a_newer_local_state() {
    let fixture = EpochFixture::new();
    let previous_terminal = fixture.parent.header().compute_block_commitment();
    fixture
        .storage
        .ol_state()
        .del_toplevel_ol_state_blocking(previous_terminal)
        .unwrap();
    fixture
        .storage
        .ol_state()
        .put_toplevel_ol_state_blocking(
            fixture.terminal.header().compute_block_commitment(),
            create_test_container_with_staged(2),
        )
        .unwrap();

    assert!(matches!(
        checkpoint_task_spec(&fixture.storage, fixture.task),
        Err(ProverError::EpochStartStateNotFound { commitment }) if commitment == previous_terminal
    ));
    assert!(matches!(fixture.resolve(), InputResolution::Blocked { .. }));

    fixture
        .storage
        .ol_state()
        .put_toplevel_ol_state_blocking(previous_terminal, fixture.start_state.clone())
        .unwrap();
    assert_eq!(
        checkpoint_task_spec(&fixture.storage, fixture.task).unwrap(),
        OLSpecId::V1
    );
    assert!(matches!(fixture.resolve(), InputResolution::Ready(_)));
}

#[test]
fn routing_rejects_a_task_that_does_not_match_the_canonical_epoch() {
    let fixture = EpochFixture::new();
    let stale = CheckpointTask(EpochCommitment::new(
        fixture.task.0.epoch(),
        fixture.task.0.last_slot(),
        OLBlockId::from(Buf32::from([9; 32])),
    ));
    assert!(matches!(
        checkpoint_task_spec(&fixture.storage, stale),
        Err(ProverError::StaleTaskCommitment { task, canonical, .. })
            if task == stale.0 && canonical == fixture.task.0
    ));
}

#[test]
fn admission_rejects_stale_task_before_waiting_for_missing_required_artifact() {
    let fixture = EpochFixture::new();
    let stale = CheckpointTask(EpochCommitment::new(
        fixture.task.0.epoch(),
        fixture.task.0.last_slot(),
        OLBlockId::from(Buf32::from([9; 32])),
    ));
    let admission = CheckpointAdmission::new(
        Arc::clone(&fixture.storage),
        CheckpointArtifactRegistry::<()>::empty().to_predicates(),
        OLSpecId::V1,
    );
    let error = test_runtime_handle()
        .block_on(admission.check(&stale))
        .unwrap_err();
    assert_eq!(error.action(), FailureAction::Permanent);
}

#[test]
fn admission_leaves_missing_epoch_metadata_to_witness_readiness() {
    let fixture = EpochFixture::new();
    fixture
        .storage
        .ol_checkpoint()
        .del_epoch_summary_blocking(fixture.task.0)
        .unwrap();
    let empty = CheckpointArtifactRegistry::<()>::empty().to_predicates();
    assert!(matches!(fixture.admission(empty), AdmissionDecision::Admit));
    assert!(matches!(fixture.resolve(), InputResolution::Blocked { .. }));
}

fn checkpoint_prover(
    fixture: &EpochFixture,
    store: VersionedTaskStore,
    predicates: LoadedCheckpointPredicates,
) -> Arc<Prover<CheckpointSpec>> {
    let runtime_params = OLRuntimeParams::test_default();
    let registry = native_checkpoint_registry(runtime_params);
    let (spec, host) = registry.into_hosts().next().unwrap();
    assert_eq!(spec, OLSpecId::V1);
    Arc::new(
        ProverBuilder::new(CheckpointSpec::new(
            Arc::clone(&fixture.storage),
            runtime_params,
            OLSpecId::V1,
        ))
        .task_store(store)
        .task_admission(CheckpointAdmission::new(
            Arc::clone(&fixture.storage),
            predicates,
            OLSpecId::V1,
        ))
        .receipt_store(InMemoryReceiptStore::new())
        .native(host),
    )
}

fn wait_for_configuration(prover: &Prover<CheckpointSpec>, task: &CheckpointTask) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let record = prover
            .task_store()
            .get(&task.to_key_bytes())
            .unwrap()
            .unwrap();
        let status = record.status();
        // Wait for both writes made by configuration parking before overriding
        // its retry deadline; otherwise the worker could overwrite our test deadline.
        if matches!(status, TaskStatus::Blocked { .. }) && record.retry_after_secs().is_some() {
            return;
        }
        assert!(
            !status.is_terminal(),
            "task terminated before admission parked it: {status:?}"
        );
        assert!(
            Instant::now() < deadline,
            "admission did not park the task: {status:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn assert_persisted_task_admission(recovered: bool) {
    let fixture = EpochFixture::new();
    let own_store =
        VersionedTaskStore::new(Arc::clone(fixture.storage.prover_tasks()), OLSpecId::V1);
    // The other namespace is a storage fixture only; no V0 proving service is created.
    let foreign_store =
        VersionedTaskStore::new(Arc::clone(fixture.storage.prover_tasks()), OLSpecId::V0);
    let key = fixture.task.to_key_bytes();
    foreign_store
        .insert(TaskRecord::new(key.clone(), TaskStatus::Pending))
        .unwrap();
    foreign_store.set_metadata(&key, vec![9, 8, 7]).unwrap();
    if recovered {
        own_store
            .insert(TaskRecord::new(key.clone(), TaskStatus::Pending))
            .unwrap();
        own_store.set_metadata(&key, vec![1, 2, 3]).unwrap();
    }
    let prover = checkpoint_prover(
        &fixture,
        own_store.clone(),
        CheckpointArtifactRegistry::<()>::empty().to_predicates(),
    );
    if recovered {
        test_runtime_handle().block_on(prover.tick());
    } else {
        test_runtime_handle()
            .block_on(prover.submit(fixture.task))
            .unwrap();
    }
    wait_for_configuration(&prover, &fixture.task);
    let parked = own_store.get(&key).unwrap().unwrap();
    assert_eq!(parked.status().counts(), AttemptCounts::default());
    if recovered {
        assert_eq!(parked.metadata(), Some(&[1, 2, 3][..]));
    }
    assert_eq!(
        foreign_store.get(&key).unwrap().unwrap().status(),
        &TaskStatus::Pending
    );

    // The operator supplies the missing artifact and restarts the service. The task
    // keeps its spec-scoped storage identity and saved remote-job metadata.
    drop(prover);
    let prover = checkpoint_prover(&fixture, own_store.clone(), loaded_predicates());
    own_store.set_retry_after(&key, 0).unwrap();
    let results = test_runtime_handle().block_on(async {
        prover.tick().await;
        timeout(
            Duration::from_secs(10),
            prover.wait_for_tasks(&[fixture.task]),
        )
        .await
        .unwrap()
        .unwrap()
    });
    assert!(results[0].is_completed());
    assert_eq!(
        own_store.get(&key).unwrap().unwrap().status(),
        &TaskStatus::Completed
    );
    let foreign = foreign_store.get(&key).unwrap().unwrap();
    assert_eq!(foreign.status(), &TaskStatus::Pending);
    assert_eq!(foreign.metadata(), Some(&[9, 8, 7][..]));
}

#[test]
fn fresh_checkpoint_task_observes_admission_before_proving() {
    assert_persisted_task_admission(false);
}

#[test]
fn recovered_checkpoint_task_observes_admission_in_its_own_store() {
    assert_persisted_task_admission(true);
}
