//! Exercises checkpoint routing, witness validation, and task recovery.

use std::{sync::Arc, time::Duration};

use strata_asm_checkpoint_types::{CheckpointInitConfig, PendingPredicateTransition};
use strata_asm_common::{SectionState, SectionStateExt};
use strata_asm_proto_checkpoint::{CheckpointState, CheckpointSubprotocol};
use strata_checkpoint_types::EpochSummary;
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_db_tests::asm_tests::make_test_asm_state;
use strata_db_types::asm::AsmExecOutput;
use strata_identifiers::{
    Buf32, Buf64, EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
};
use strata_ol_chain_types_v1::{OLBlockV1, SignedOLBlockHeaderV1};
use strata_ol_checkpoint::ProofNotify;
use strata_ol_checkpoint_artifacts::{LoadedCheckpointPredicates, native_checkpoint_registry};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_container::{OLStateContainer, test_utils::create_test_container_with_staged};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{IStateAccessor, IStateAccessorMut, OLSpecId, OLSpecVersions};
use strata_ol_stf::{BlockComponents, BlockContext, BlockInfo, construct_block};
use strata_ol_stf_v1::test_utils::{
    EPOCH_RUNNER_GENESIS_TIMESTAMP, EPOCH_RUNNER_SLOT_TIMESTAMP_STEP, make_empty_manifest,
    make_genesis_state, tamper_state_root, to_ol_block,
};
use strata_paas::{InputResolution, ProofSpec, Prover, TaskRecord, TaskStatus, TaskStore};
use strata_predicate::PredicateKey;
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use strata_storage::{NodeStorage, VersionedTaskStore, create_node_storage, test_runtime_handle};
use tokio::time::timeout;

use super::{
    build_checkpoint_provers,
    errors::ProverError,
    spec::{CheckpointSpec, CheckpointTask, checkpoint_task_spec},
};

struct EpochFixture {
    storage: Arc<NodeStorage>,
    parent: OLBlockV1,
    terminal: OLBlockV1,
    start_state: OLStateContainer,
    terminal_state: OLStateContainer,
    task: CheckpointTask,
}

impl EpochFixture {
    fn new() -> Self {
        Self::with_start_spec(OLSpecId::V1)
    }

    fn with_start_spec(start_spec: OLSpecId) -> Self {
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
        let mut parent = to_ol_block(genesis.completed_block());
        if start_spec == OLSpecId::V0 {
            // Represents the promoted V0 terminal without executing a V0 block.
            state.set_spec_versions(OLSpecVersions::uniform(OLSpecId::V0));
            parent = OLBlockV1::new(
                SignedOLBlockHeaderV1::new(
                    tamper_state_root(parent.header(), state.compute_state_root().unwrap()),
                    Buf64::zero(),
                ),
                parent.body().clone(),
            );
        }
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
        let terminal_state = state.into_container();
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
            .ol_state()
            .put_toplevel_ol_state_blocking(
                terminal.header().compute_block_commitment(),
                terminal_state.clone(),
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
            terminal_state,
            task: CheckpointTask(summary.get_epoch_commitment()),
        }
    }

    fn resolve(&self) -> InputResolution<CheckpointProverInput> {
        self.resolve_on(OLSpecId::V1)
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

    /// Changes the persisted terminal snapshot for routing tests, without proving it.
    fn set_terminal_versions(&mut self, versions: OLSpecVersions) {
        let mut state = MemoryStateBaseLayer::from_container(self.terminal_state.clone());
        state.set_spec_versions(versions);
        self.terminal_state = state.into_container();
        self.terminal = OLBlockV1::new(
            SignedOLBlockHeaderV1::new(
                tamper_state_root(
                    self.terminal.header(),
                    self.terminal_state.compute_state_root(),
                ),
                Buf64::zero(),
            ),
            self.terminal.body().clone(),
        );
        let old_summary = self
            .storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(self.task.0)
            .unwrap()
            .unwrap();
        let summary = EpochSummary::new(
            old_summary.epoch(),
            self.terminal.header().compute_block_commitment(),
            *old_summary.prev_terminal(),
            *old_summary.new_l1(),
            *self.terminal.header().state_root(),
        );
        self.storage
            .ol_checkpoint()
            .del_epoch_summary_blocking(self.task.0)
            .unwrap();
        self.storage
            .ol_block()
            .put_block_data_blocking(self.terminal.clone())
            .unwrap();
        self.storage
            .ol_state()
            .put_toplevel_ol_state_blocking(*summary.terminal(), self.terminal_state.clone())
            .unwrap();
        self.storage
            .ol_checkpoint()
            .insert_epoch_summary_blocking(summary)
            .unwrap();
        self.task = CheckpointTask(summary.get_epoch_commitment());
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
fn terminal_current_spec_selects_the_program_even_when_another_spec_is_staged() {
    for staged in [0, 2] {
        let mut fixture = EpochFixture::new();
        fixture.set_terminal_versions(OLSpecVersions::new(OLSpecId::V1, staged).unwrap());
        assert_eq!(
            checkpoint_task_spec(&fixture.storage, fixture.task).unwrap(),
            OLSpecId::V1
        );
    }
}

#[test]
fn v0_epoch_is_not_routed_to_the_v1_prover() {
    let mut fixture = EpochFixture::with_start_spec(OLSpecId::V0);
    fixture.set_terminal_versions(OLSpecVersions::uniform(OLSpecId::V0));
    assert_eq!(
        checkpoint_task_spec(&fixture.storage, fixture.task).unwrap(),
        OLSpecId::V0
    );
    fixture.discard_epoch_blocks();
    assert!(
        matches!(fixture.resolve(), InputResolution::Rejected { reason }
        if reason.contains("requires V0") && reason.contains("routed to V1"))
    );
}

#[test]
fn exact_epoch_terminal_selects_v1_even_when_newer_local_state_stages_unknown_spec() {
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
    let InputResolution::Ready(input) = fixture.resolve() else {
        panic!("ASM lag must not block an epoch whose artifact is loaded");
    };
    CheckpointProgram::execute(&input, OLSpecId::V1, OLRuntimeParams::test_default()).unwrap();
}

#[test]
fn first_v1_checkpoint_after_v0_anchor_uses_v1_after_restart() {
    let fixture = EpochFixture::with_start_spec(OLSpecId::V0);
    fixture.put_asm_checkpoint(
        PredicateKey::never_accept(),
        Some(CheckpointProgram::test_predicate_key()),
    );
    let restarted = Arc::new(
        create_node_storage(Arc::clone(fixture.storage.db()), test_runtime_handle()).unwrap(),
    );
    assert_eq!(
        checkpoint_task_spec(&restarted, fixture.task).unwrap(),
        OLSpecId::V1
    );
    let spec = CheckpointSpec::new(restarted, OLRuntimeParams::test_default(), OLSpecId::V1);
    let InputResolution::Ready(input) = test_runtime_handle()
        .block_on(spec.resolve_input(&fixture.task))
        .unwrap()
    else {
        panic!("the first V1 epoch must resolve from its persisted V0 parent");
    };
    assert_eq!(input.start_state.cur_spec(), OLSpecId::V0);
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
    // reads the same assignment from the persisted epoch terminal state.
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
}

#[test]
fn missing_exact_terminal_state_waits_without_using_the_parent_or_newer_state() {
    let fixture = EpochFixture::new();
    let terminal = fixture.task.0.to_block_commitment();
    fixture
        .storage
        .ol_state()
        .del_toplevel_ol_state_blocking(terminal)
        .unwrap();
    let ahead = OLBlockCommitment::new(20, OLBlockId::from(Buf32::from([4; 32])));
    fixture
        .storage
        .ol_state()
        .put_toplevel_ol_state_blocking(ahead, create_test_container_with_staged(2))
        .unwrap();

    assert!(
        matches!(checkpoint_task_spec(&fixture.storage, fixture.task),
        Err(ProverError::EpochTerminalStateNotFound { commitment }) if commitment == terminal)
    );
    assert!(matches!(fixture.resolve(), InputResolution::Blocked { .. }));

    fixture
        .storage
        .ol_state()
        .put_toplevel_ol_state_blocking(terminal, fixture.terminal_state.clone())
        .unwrap();
    assert_eq!(
        checkpoint_task_spec(&fixture.storage, fixture.task).unwrap(),
        OLSpecId::V1
    );
    assert!(matches!(fixture.resolve(), InputResolution::Ready(_)));
}

#[test]
fn missing_start_state_blocks_witness_assembly_without_changing_task_routing() {
    let fixture = EpochFixture::new();
    let previous_terminal = fixture.parent.header().compute_block_commitment();
    fixture
        .storage
        .ol_state()
        .del_toplevel_ol_state_blocking(previous_terminal)
        .unwrap();
    assert_eq!(
        checkpoint_task_spec(&fixture.storage, fixture.task).unwrap(),
        OLSpecId::V1
    );
    assert!(matches!(fixture.resolve(), InputResolution::Blocked { .. }));
    fixture
        .storage
        .ol_state()
        .put_toplevel_ol_state_blocking(previous_terminal, fixture.start_state.clone())
        .unwrap();
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
fn input_resolution_rejects_stale_task_before_assembling_a_witness() {
    let fixture = EpochFixture::new();
    let stale = CheckpointTask(EpochCommitment::new(
        fixture.task.0.epoch(),
        fixture.task.0.last_slot(),
        OLBlockId::from(Buf32::from([9; 32])),
    ));
    fixture.discard_epoch_blocks();
    let spec = CheckpointSpec::new(
        Arc::clone(&fixture.storage),
        OLRuntimeParams::test_default(),
        OLSpecId::V1,
    );
    let resolution = test_runtime_handle()
        .block_on(spec.resolve_input(&stale))
        .unwrap();
    assert!(matches!(resolution, InputResolution::Rejected { reason }
        if reason.contains("stale checkpoint task")));
}

#[test]
fn input_resolution_waits_for_missing_epoch_metadata() {
    let fixture = EpochFixture::new();
    fixture
        .storage
        .ol_checkpoint()
        .del_epoch_summary_blocking(fixture.task.0)
        .unwrap();
    assert!(matches!(fixture.resolve(), InputResolution::Blocked { .. }));
}

fn checkpoint_prover(fixture: &EpochFixture) -> Arc<Prover<CheckpointSpec>> {
    let runtime_params = OLRuntimeParams::test_default();
    let (mut provers, _) = build_checkpoint_provers(
        native_checkpoint_registry(runtime_params),
        &fixture.storage,
        runtime_params,
        &Arc::new(ProofNotify::new()),
        |builder, host| builder.native(host),
    );
    assert_eq!(provers.len(), 1);
    Arc::new(provers.remove(&OLSpecId::V1).unwrap())
}

fn assert_persisted_task_proving(recovered: bool) {
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
    let prover = checkpoint_prover(&fixture);
    let results = test_runtime_handle().block_on(async {
        if recovered {
            prover.tick().await;
        } else {
            prover.submit(fixture.task).await.unwrap();
        }
        timeout(
            Duration::from_secs(10),
            prover.wait_for_tasks(&[fixture.task]),
        )
        .await
        .unwrap()
        .unwrap()
    });
    assert!(results[0].is_completed());
    assert!(
        fixture
            .storage
            .checkpoint_proof()
            .get_proof(&fixture.task.0)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        own_store.get(&key).unwrap().unwrap().status(),
        &TaskStatus::Completed
    );
    let foreign = foreign_store.get(&key).unwrap().unwrap();
    assert_eq!(foreign.status(), &TaskStatus::Pending);
    assert_eq!(foreign.metadata(), Some(&[9, 8, 7][..]));
}

#[test]
fn fresh_checkpoint_task_proves_in_its_own_store() {
    assert_persisted_task_proving(false);
}

#[test]
fn recovered_checkpoint_task_proves_in_its_own_store() {
    assert_persisted_task_proving(true);
}
