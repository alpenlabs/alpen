//! Consensus coverage for checkpoint predicate handover boundaries.

use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_logs::CheckpointPredicateEnacted;
use strata_asm_logs::constants::AsmLogTypeId;
use strata_identifiers::{Buf32, Buf64};
use strata_ol_chain_types_v1::{
    BlockFlagsV1, OLAsmManifestContainerV1, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1,
    OLTxSegmentV1, SignedOLBlockHeaderV1,
};
use strata_ol_da_types_v1::{OLDaPayloadV1, OLDaSchemeV1, OLStateDiffV1};
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_types::IStateAccessor;
use strata_predicate::PredicateKey;

use crate::test_utils::{FixtureAsmManifestBuilder, OLStfFixture};
use crate::{
    BlockInfo, EpochInfo, ExecError, apply_da_epoch, execute_block_batch_predrain,
    has_checkpoint_predicate_enactment, process_asm_manifest, process_block_manifests,
    verify_block, verify_block_structure,
};

/// Builds a test-only manifest with the requested number of enactment logs.
///
/// `AlwaysAccept` supplies a valid log payload; these tests exercise epoch boundaries,
/// without verifying checkpoint proofs under that predicate.
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
fn boundary_requires_terminal_and_rejects_following_manifests() {
    for terminal in [false, true] {
        let mut fixture = OLStfFixture::builder().execute_genesis();
        let block = fixture
            .child_block()
            .with_manifest(boundary_manifest(1, 1))
            .with_manifest(FixtureAsmManifestBuilder::new_at_height(2).build());
        let err = if terminal {
            block.terminal().execute_err()
        } else {
            block.execute_err()
        };
        assert!(matches!(
            err.into_base(),
            ExecError::CheckpointPredicateBoundaryNotLast { height: 1 }
        ));
    }
    let mut fixture = OLStfFixture::builder().execute_genesis();
    let err = fixture
        .child_block()
        .with_manifest(boundary_manifest(1, 1))
        .execute_err();
    assert!(matches!(
        err.into_base(),
        ExecError::CheckpointPredicateBoundaryNonterminal { height: 1 }
    ));
}

#[test]
fn structure_verification_checks_boundary_placement() {
    for terminal in [false, true] {
        for following_manifest in [false, true] {
            let mut manifests = vec![boundary_manifest(1, 1)];
            if following_manifest {
                manifests.push(FixtureAsmManifestBuilder::new_at_height(2).build());
            }
            let body = OLBlockBodyV1::new(
                OLTxSegmentV1::new(vec![]).unwrap(),
                Some(OLAsmManifestContainerV1::new(manifests).unwrap()),
            );
            let mut flags = BlockFlagsV1::zero();
            flags.set_is_terminal(terminal);
            let header = OLBlockHeaderV1::new(
                1_001_000,
                flags,
                1,
                1,
                Buf32::zero().into(),
                body.compute_hash_commitment(),
                Buf32::zero(),
                Buf32::zero(),
            );
            let result = verify_block_structure(&header, &body);
            if following_manifest {
                assert!(matches!(
                    result,
                    Err(ExecError::CheckpointPredicateBoundaryNotLast { height: 1 })
                ));
            } else if !terminal {
                assert!(matches!(
                    result,
                    Err(ExecError::CheckpointPredicateBoundaryNonterminal { height: 1 })
                ));
            } else {
                result.unwrap();
            }
        }
    }
}

#[test]
fn terminal_boundary_drains_once_and_next_epoch_can_continue() {
    let mut fixture = OLStfFixture::builder().execute_genesis();
    fixture
        .child_block()
        .with_manifest(FixtureAsmManifestBuilder::new_at_height(1).build())
        .execute();
    let mut verifier_state = fixture.state().clone();
    let parent = fixture.parent_header().clone();
    let outcome = fixture
        .child_block()
        .with_manifest(boundary_manifest(2, 1))
        .terminal()
        .execute();
    let block = outcome.completed_block();
    verify_block(
        &mut verifier_state,
        block.header(),
        Some(&parent),
        block.body(),
        &OLRuntimeParams::test_default(),
    )
    .unwrap();
    assert_eq!(
        verifier_state.compute_state_root().unwrap(),
        fixture.state().compute_state_root().unwrap()
    );
    assert_eq!(fixture.state().cur_epoch(), 2);
    assert_eq!(fixture.state().last_l1_height(), 2);
    assert_eq!(fixture.state().pending_asm_logs_len(), 0);
    fixture
        .child_block()
        .with_manifest(FixtureAsmManifestBuilder::new_at_height(3).build())
        .execute();
    assert_eq!(fixture.state().cur_epoch(), 2);
    assert_eq!(fixture.state().last_l1_height(), 3);
}

#[test]
fn verification_and_da_rebuild_reject_nonterminal_boundary() {
    let fixture = OLStfFixture::builder().execute_genesis();
    let body = OLBlockBodyV1::new(
        OLTxSegmentV1::new(vec![]).unwrap(),
        Some(OLAsmManifestContainerV1::new(vec![boundary_manifest(1, 1)]).unwrap()),
    );
    let header = OLBlockHeaderV1::new(
        1_001_000,
        BlockFlagsV1::zero(),
        1,
        1,
        fixture.parent_header().compute_blkid(),
        body.compute_hash_commitment(),
        Buf32::zero(),
        Buf32::zero(),
    );
    let runtime_params = OLRuntimeParams::test_default();
    let mut state = fixture.state().clone();
    let initial_root = state.compute_state_root().unwrap();
    let err = verify_block(
        &mut state,
        &header,
        Some(fixture.parent_header()),
        &body,
        &runtime_params,
    )
    .unwrap_err();
    assert!(matches!(
        err.into_base(),
        ExecError::CheckpointPredicateBoundaryNonterminal { height: 1 }
    ));
    assert_eq!(state.compute_state_root().unwrap(), initial_root);
    let block = OLBlockV1::new(SignedOLBlockHeaderV1::new(header, Buf64::zero()), body);
    let err = execute_block_batch_predrain(
        &mut fixture.state().clone(),
        &[block],
        fixture.parent_header(),
        &runtime_params,
    )
    .unwrap_err();
    assert!(matches!(
        err.into_base(),
        ExecError::CheckpointPredicateBoundaryNonterminal { height: 1 }
    ));
}

#[test]
fn da_replay_rejects_straddling_epoch_and_accepts_boundary_at_end() {
    let fixture = OLStfFixture::builder().execute_genesis();
    let epoch = EpochInfo::new(
        BlockInfo::new(1_001_000, 1, 1),
        fixture.parent_header().compute_block_commitment(),
    );
    let manifests = [
        boundary_manifest(1, 1),
        FixtureAsmManifestBuilder::new_at_height(2).build(),
    ];
    let mut state = fixture.state().clone();
    let initial_root = state.compute_state_root().unwrap();
    let err = apply_da_epoch::<_, OLDaSchemeV1>(
        &mut state,
        &epoch,
        OLDaPayloadV1::new(OLStateDiffV1::default()),
        &manifests,
        &OLRuntimeParams::test_default(),
    )
    .unwrap_err();
    assert!(matches!(
        err.into_base(),
        ExecError::CheckpointPredicateBoundaryNotLast { height: 1 }
    ));
    assert_eq!(state.compute_state_root().unwrap(), initial_root);
    apply_da_epoch::<_, OLDaSchemeV1>(
        &mut state,
        &epoch,
        OLDaPayloadV1::new(OLStateDiffV1::default()),
        &manifests[..1],
        &OLRuntimeParams::test_default(),
    )
    .unwrap();
    assert_eq!(state.cur_epoch(), 2);
    assert_eq!(state.last_l1_height(), 1);
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

    // Check the whole batch before buffering even the ordinary prefix.
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
    assert_eq!(ordinary.checkpoint_enactment_height(), None);
    assert_eq!(single_state.last_l1_height(), 1);
    let single = process_asm_manifest(&mut single_state, &manifests[1]).unwrap();
    let batch = process_block_manifests(&mut batch_state, &manifests).unwrap();
    assert_eq!(single.checkpoint_enactment_height(), Some(2));
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
fn block_verification_and_da_replay_reject_duplicate_enactments() {
    let fixture = OLStfFixture::builder().execute_genesis();
    let manifests = vec![boundary_manifest(1, 2)];
    let body = OLBlockBodyV1::new(
        OLTxSegmentV1::new(vec![]).unwrap(),
        Some(OLAsmManifestContainerV1::new(manifests.clone()).unwrap()),
    );
    let mut flags = BlockFlagsV1::zero();
    flags.set_is_terminal(true);
    let header = OLBlockHeaderV1::new(
        1_001_000,
        flags,
        1,
        1,
        fixture.parent_header().compute_blkid(),
        body.compute_hash_commitment(),
        Buf32::zero(),
        Buf32::zero(),
    );
    let mut state = fixture.state().clone();
    let initial_root = state.compute_state_root().unwrap();
    let runtime_params = OLRuntimeParams::test_default();
    let err = verify_block(
        &mut state,
        &header,
        Some(fixture.parent_header()),
        &body,
        &runtime_params,
    )
    .unwrap_err();
    assert!(matches!(
        err.into_base(),
        ExecError::DuplicateCheckpointPredicateEnactment { height: 1 }
    ));
    assert_eq!(state.compute_state_root().unwrap(), initial_root);

    let epoch = EpochInfo::new(
        BlockInfo::new(1_001_000, 1, 1),
        fixture.parent_header().compute_block_commitment(),
    );
    let err = apply_da_epoch::<_, OLDaSchemeV1>(
        &mut state,
        &epoch,
        OLDaPayloadV1::new(OLStateDiffV1::default()),
        &manifests,
        &runtime_params,
    )
    .unwrap_err();
    assert!(matches!(
        err.into_base(),
        ExecError::DuplicateCheckpointPredicateEnactment { height: 1 }
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
            .with_log(log.clone())
            .build();
        assert!(matches!(
            process_asm_manifest(&mut state, &manifest),
            Err(ExecError::MalformedCheckpointPredicateEnactment { height: 1 })
        ));
        assert_eq!(state.compute_state_root().unwrap(), initial_root);

        let manifests = [
            FixtureAsmManifestBuilder::new_at_height(1).build(),
            FixtureAsmManifestBuilder::new_at_height(2)
                .with_log(log)
                .build(),
            FixtureAsmManifestBuilder::new_at_height(3).build(),
        ];
        assert!(matches!(
            process_block_manifests(&mut state, &manifests),
            Err(ExecError::MalformedCheckpointPredicateEnactment { height: 2 })
        ));
        assert_eq!(state.compute_state_root().unwrap(), initial_root);
    }
}

#[test]
fn malformed_enactment_cannot_hide_boundary_in_nonterminal_block() {
    let log = AsmLogEntry::from_msg(AsmLogTypeId::CheckpointPredicateEnacted.into(), vec![])
        .expect("valid message framing");
    let mut fixture = OLStfFixture::builder().execute_genesis();
    let initial_root = fixture.state().compute_state_root().unwrap();
    let err = fixture
        .child_block()
        .with_manifest(
            FixtureAsmManifestBuilder::new_at_height(1)
                .with_log(log)
                .build(),
        )
        .with_manifest(FixtureAsmManifestBuilder::new_at_height(2).build())
        .execute_err();
    assert!(matches!(
        err.into_base(),
        ExecError::MalformedCheckpointPredicateEnactment { height: 1 }
    ));
    assert_eq!(fixture.state().compute_state_root().unwrap(), initial_root);
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
    assert_eq!(outcome.checkpoint_enactment_height(), Some(1));
}

#[test]
fn block_verification_and_da_replay_reject_malformed_enactment_before_mutation() {
    let fixture = OLStfFixture::builder().execute_genesis();
    let log = AsmLogEntry::from_msg(AsmLogTypeId::CheckpointPredicateEnacted.into(), vec![])
        .expect("valid message framing");
    let manifests = vec![
        FixtureAsmManifestBuilder::new_at_height(1).build(),
        FixtureAsmManifestBuilder::new_at_height(2)
            .with_log(log)
            .build(),
        FixtureAsmManifestBuilder::new_at_height(3).build(),
    ];
    let body = OLBlockBodyV1::new(
        OLTxSegmentV1::new(vec![]).unwrap(),
        Some(OLAsmManifestContainerV1::new(manifests.clone()).unwrap()),
    );
    let mut flags = BlockFlagsV1::zero();
    flags.set_is_terminal(true);
    let header = OLBlockHeaderV1::new(
        1_001_000,
        flags,
        1,
        1,
        fixture.parent_header().compute_blkid(),
        body.compute_hash_commitment(),
        Buf32::zero(),
        Buf32::zero(),
    );
    let mut state = fixture.state().clone();
    let initial_root = state.compute_state_root().unwrap();
    let runtime_params = OLRuntimeParams::test_default();
    let err = verify_block(
        &mut state,
        &header,
        Some(fixture.parent_header()),
        &body,
        &runtime_params,
    )
    .unwrap_err();
    assert!(matches!(
        err.into_base(),
        ExecError::MalformedCheckpointPredicateEnactment { height: 2 }
    ));
    assert_eq!(state.compute_state_root().unwrap(), initial_root);

    let epoch = EpochInfo::new(
        BlockInfo::new(1_001_000, 1, 1),
        fixture.parent_header().compute_block_commitment(),
    );
    let err = apply_da_epoch::<_, OLDaSchemeV1>(
        &mut state,
        &epoch,
        OLDaPayloadV1::new(OLStateDiffV1::default()),
        &manifests,
        &runtime_params,
    )
    .unwrap_err();
    assert!(matches!(
        err.into_base(),
        ExecError::MalformedCheckpointPredicateEnactment { height: 2 }
    ));
    assert_eq!(state.compute_state_root().unwrap(), initial_root);
}
