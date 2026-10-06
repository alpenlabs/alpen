//! Manifest-processing coverage for checkpoint predicate boundaries.

use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_logs::{CheckpointPredicateEnacted, constants::AsmLogTypeId};
use strata_ledger_types::IStateAccessor;
use strata_predicate::PredicateKey;

use crate::{
    ExecError, has_checkpoint_predicate_enactment, process_asm_manifest, process_block_manifests,
    test_utils::{FixtureAsmManifestBuilder, OLStfFixture},
};

fn boundary_manifest(height: u32, log_count: usize) -> AsmManifest {
    let log = AsmLogEntry::from_log(&CheckpointPredicateEnacted::new(
        PredicateKey::always_accept(),
    ))
    .expect("enactment log encodes");
    FixtureAsmManifestBuilder::new_at_height(height)
        .with_logs(vec![log; log_count])
        .build()
}

#[test]
fn ordinary_manifests_do_not_create_boundaries() {
    assert!(
        !has_checkpoint_predicate_enactment(&FixtureAsmManifestBuilder::new_at_height(1).build())
            .unwrap()
    );
    assert!(has_checkpoint_predicate_enactment(&boundary_manifest(1, 1)).unwrap());
}

#[test]
fn duplicate_enactments_are_rejected_before_buffering() {
    let fixture = OLStfFixture::builder().execute_genesis();
    let mut state = fixture.state().clone();
    let initial_root = state.compute_state_root().unwrap();
    let manifest = boundary_manifest(1, 2);

    assert!(matches!(
        has_checkpoint_predicate_enactment(&manifest),
        Err(ExecError::DuplicateCheckpointPredicateEnactment { height: 1 })
    ));
    assert!(matches!(
        process_asm_manifest(&mut state, &manifest),
        Err(ExecError::DuplicateCheckpointPredicateEnactment { height: 1 })
    ));
    assert_eq!(state.compute_state_root().unwrap(), initial_root);

    let manifests = [
        FixtureAsmManifestBuilder::new_at_height(1).build(),
        boundary_manifest(2, 2),
    ];
    assert!(matches!(
        process_block_manifests(&mut state, &manifests),
        Err(ExecError::DuplicateCheckpointPredicateEnactment { height: 2 })
    ));
    assert_eq!(state.compute_state_root().unwrap(), initial_root);
}

#[test]
fn batch_buffering_rejects_manifests_after_enactment_before_mutation() {
    let fixture = OLStfFixture::builder().execute_genesis();
    let mut state = fixture.state().clone();
    let initial_root = state.compute_state_root().unwrap();
    let manifests = [
        FixtureAsmManifestBuilder::new_at_height(1).build(),
        boundary_manifest(2, 1),
        FixtureAsmManifestBuilder::new_at_height(3).build(),
    ];
    assert!(matches!(
        process_block_manifests(&mut state, &manifests),
        Err(ExecError::CheckpointPredicateBoundaryNotLast { height: 2 })
    ));
    assert_eq!(state.compute_state_root().unwrap(), initial_root);
}

#[test]
fn single_and_batch_buffering_produce_the_same_state_and_signal() {
    let fixture = OLStfFixture::builder().execute_genesis();
    let mut single_state = fixture.state().clone();
    let mut batch_state = single_state.clone();
    let manifests = [
        FixtureAsmManifestBuilder::new_at_height(1).build(),
        boundary_manifest(2, 1),
    ];
    let ordinary = process_asm_manifest(&mut single_state, &manifests[0]).unwrap();
    assert!(!ordinary.checkpoint_predicate_enacted());
    assert_eq!(single_state.last_l1_height(), 1);
    let single = process_asm_manifest(&mut single_state, &manifests[1]).unwrap();
    let batch = process_block_manifests(&mut batch_state, &manifests).unwrap();
    assert!(single.checkpoint_predicate_enacted());
    assert_eq!(single, batch);
    assert_eq!(single_state.last_l1_height(), 2);
    assert_eq!(single_state.pending_asm_logs_len(), 1);
    assert_eq!(
        single_state.compute_state_root().unwrap(),
        batch_state.compute_state_root().unwrap()
    );
}

#[test]
fn single_manifest_processing_rejects_a_height_gap() {
    let fixture = OLStfFixture::builder().execute_genesis();
    let mut state = fixture.state().clone();
    let initial_root = state.compute_state_root().unwrap();
    assert!(matches!(
        process_asm_manifest(&mut state, &boundary_manifest(2, 1)),
        Err(ExecError::AsmManifestHeightMismatch {
            expected: 1,
            actual: 2,
            index: 0
        })
    ));
    assert_eq!(state.compute_state_root().unwrap(), initial_root);
}

#[test]
fn malformed_enactments_are_rejected_before_buffering() {
    for body in [vec![], vec![0xff]] {
        let log = AsmLogEntry::from_msg(AsmLogTypeId::CheckpointPredicateEnacted.into(), body)
            .expect("valid message framing");
        let fixture = OLStfFixture::builder().execute_genesis();
        let mut state = fixture.state().clone();
        let initial_root = state.compute_state_root().unwrap();
        let manifest = FixtureAsmManifestBuilder::new_at_height(1)
            .with_log(log)
            .build();

        assert!(matches!(
            process_asm_manifest(&mut state, &manifest),
            Err(ExecError::MalformedCheckpointPredicateEnactment { height: 1 })
        ));
        assert_eq!(state.compute_state_root().unwrap(), initial_root);
    }
}

#[test]
fn unregistered_predicate_id_still_creates_boundary() {
    let predicate = PredicateKey {
        id: 250,
        condition: Vec::new().try_into().unwrap(),
    };
    let log = AsmLogEntry::from_log(&CheckpointPredicateEnacted::new(predicate))
        .expect("unregistered predicate ID encodes");
    let manifest = FixtureAsmManifestBuilder::new_at_height(1)
        .with_log(log)
        .build();
    let fixture = OLStfFixture::builder().execute_genesis();
    let mut state = fixture.state().clone();
    let outcome = process_asm_manifest(&mut state, &manifest).unwrap();
    assert!(outcome.checkpoint_predicate_enacted());
}
