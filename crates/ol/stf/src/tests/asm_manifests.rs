//! Tests for ASM manifest processing in the OL STF.

use strata_acct_types::{AccountSerial, BitcoinAmount};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::{CheckpointTipUpdate, constants::AsmLogTypeId};
use strata_asm_proto_checkpoint_types::CheckpointTip;
use strata_identifiers::{
    Buf32, EpochCommitment, L1Height, OLBlockCommitment, OLBlockId, SubjectId,
};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_msg_fmt::MAX_TYPE;
use strata_ol_chain_types_new::MAX_SEALING_MANIFEST_COUNT;

use crate::{assembly::BlockComponents, context::BlockInfo, errors::ExecError, test_utils::*};

const GENESIS_MANIFEST_SENTINEL_COUNT: u64 = 1;

fn assert_manifest_height_mismatch(
    err: ExecError,
    expected: L1Height,
    actual: L1Height,
    index: usize,
) {
    match err.into_base() {
        ExecError::AsmManifestHeightMismatch {
            expected: got_expected,
            actual: got_actual,
            index: got_index,
        } => assert_eq!(
            (got_expected, got_actual, got_index),
            (expected, actual, index)
        ),
        err => panic!("expected AsmManifestHeightMismatch, got {err:?}"),
    }
}

#[test]
fn test_manifest_processing_rejects_height_gap() {
    let mut state = make_genesis_state();
    let expected_height = state.last_l1_height() + 1;
    let actual_height = state.last_l1_height() + 2;
    let asm_manifest = FixtureAsmManifestBuilder::new_at_height(actual_height).build();

    let result = execute_block(
        &mut state,
        &BlockInfo::new_genesis(1_000_000),
        None,
        BlockComponents::new_manifests(vec![asm_manifest]).as_terminal(),
    );

    match result {
        Err(err) => assert_manifest_height_mismatch(err, expected_height, actual_height, 0),
        Ok(_) => panic!("manifest with non-contiguous height should fail"),
    }

    assert_eq!(state.last_l1_height(), 0);
    assert_eq!(
        state.l1_block_refs_mmr().num_entries(),
        GENESIS_MANIFEST_SENTINEL_COUNT
    );
}

#[test]
fn test_manifest_processing_rejects_current_height_manifest() {
    let mut fixture = OLStfFixture::builder()
        .with_genesis_manifest(make_empty_manifest(1, 1))
        .execute_genesis();
    let actual_height = fixture.state().last_l1_height();
    let expected_height = actual_height + 1;

    let err = fixture
        .child_block()
        .with_manifest(FixtureAsmManifestBuilder::new_at_height(actual_height).build())
        .execute_err();

    assert_manifest_height_mismatch(err, expected_height, actual_height, 0);
    assert_eq!(fixture.state().last_l1_height(), actual_height);
}

#[test]
fn test_manifest_processing_rejects_older_height_manifest() {
    let mut fixture = OLStfFixture::builder()
        .with_genesis_manifests([make_empty_manifest(1, 1), make_empty_manifest(2, 2)])
        .execute_genesis();
    let expected_height = fixture.state().last_l1_height() + 1;
    let actual_height = fixture.state().last_l1_height() - 1;

    let err = fixture
        .child_block()
        .with_manifest(FixtureAsmManifestBuilder::new_at_height(actual_height).build())
        .execute_err();

    assert_manifest_height_mismatch(err, expected_height, actual_height, 0);
    assert_eq!(fixture.state().last_l1_height(), expected_height - 1);
}

#[test]
fn test_manifest_processing_accepts_empty_manifest_container() {
    let fixture = OLStfFixture::builder().execute_genesis();
    let state = fixture.state();

    assert_eq!(state.cur_epoch(), 1);
    assert_eq!(state.last_l1_height(), 0);
    assert_eq!(
        state.l1_block_refs_mmr().num_entries(),
        GENESIS_MANIFEST_SENTINEL_COUNT
    );
}

#[test]
fn test_manifest_processing_accepts_max_manifest_count() {
    let asm_manifests: Vec<_> = (1..=MAX_SEALING_MANIFEST_COUNT)
        .map(|height| FixtureAsmManifestBuilder::new_at_height(height as u32).build())
        .collect();

    let fixture = OLStfFixture::builder()
        .with_genesis_manifests(asm_manifests)
        .execute_genesis();
    let state = fixture.state();

    assert_eq!(state.cur_epoch(), 1);
    assert_eq!(state.last_l1_height(), MAX_SEALING_MANIFEST_COUNT as u32);
    assert_eq!(
        state.l1_block_refs_mmr().num_entries(),
        GENESIS_MANIFEST_SENTINEL_COUNT + MAX_SEALING_MANIFEST_COUNT
    );
}

#[test]
fn test_manifest_processing_skips_unknown_and_unparsable_logs() {
    let unknown_type_log =
        AsmLogEntry::from_msg(MAX_TYPE, vec![1, 2, 3]).expect("unknown-type log should encode");
    let raw_log = AsmLogEntry::from_raw(vec![0xff]).expect("raw log should fit");
    let asm_manifest = FixtureAsmManifestBuilder::new_at_height(1)
        .with_logs([unknown_type_log, raw_log])
        .build();

    let output = OLStfFixture::builder()
        .with_genesis_manifest(asm_manifest)
        .execute_genesis_with_outputs();
    let state = output.fixture().state();

    assert_eq!(output.log_count(), 0);
    assert_eq!(state.cur_epoch(), 1);
    assert_eq!(state.last_l1_height(), 1);
    assert_eq!(
        state.l1_block_refs_mmr().num_entries(),
        GENESIS_MANIFEST_SENTINEL_COUNT + 1
    );
}

#[test]
fn test_manifest_processing_skips_unknown_serial_deposit() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let deposit_log = make_deposit_log_for_account(
        AccountSerial::new(9_999),
        SubjectId::from([42u8; 32]),
        BitcoinAmount::from_sat(100_000_000),
    );
    let asm_manifest = FixtureAsmManifestBuilder::new_at_height(1)
        .with_log(deposit_log)
        .build();

    let fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(0))
        })
        .with_genesis_manifest(asm_manifest)
        .execute_genesis();
    let state = fixture.state();

    let (ol_account_state, account_state) = state.expect_snark_account_state(snark_acct_id);
    assert_eq!(ol_account_state.balance(), BitcoinAmount::from_sat(0));
    assert_eq!(account_state.inbox_mmr().num_entries(), 0);
    assert_eq!(state.limbo_funds(), BitcoinAmount::from_sat(100_000_000));
    assert_eq!(state.last_l1_height(), 1);
}

#[test]
fn test_manifest_processing_updates_checkpoint_tip() {
    let l2_commitment = OLBlockCommitment::new(42, OLBlockId::from(Buf32::from([0xab; 32])));
    let tip = CheckpointTip::new(7, 100, l2_commitment);
    let update = CheckpointTipUpdate::new(tip);
    let log = AsmLogEntry::from_log(&update).expect("checkpoint tip update should encode");
    let asm_manifest = FixtureAsmManifestBuilder::new_at_height(1)
        .with_log(log)
        .build();

    let fixture = OLStfFixture::builder()
        .with_genesis_manifest(asm_manifest)
        .execute_genesis();
    let state = fixture.state();

    assert_eq!(
        state.asm_recorded_epoch(),
        &EpochCommitment::from_terminal(7, l2_commitment)
    );
    assert_eq!(state.last_l1_height(), 1);
}

#[test]
fn test_manifest_processing_skips_malformed_deposit_log() {
    let malformed_deposit_log = AsmLogEntry::from_msg(AsmLogTypeId::Deposit.into(), vec![0xff])
        .expect("deposit log should encode");
    let asm_manifest = FixtureAsmManifestBuilder::new_at_height(1)
        .with_log(malformed_deposit_log)
        .build();

    let fixture = OLStfFixture::builder()
        .with_genesis_manifest(asm_manifest)
        .execute_genesis();
    let state = fixture.state();

    assert_eq!(state.cur_epoch(), 1);
    assert_eq!(state.last_l1_height(), 1);
    assert_eq!(
        state.l1_block_refs_mmr().num_entries(),
        GENESIS_MANIFEST_SENTINEL_COUNT + 1
    );
    assert_eq!(state.limbo_funds(), BitcoinAmount::zero());
}
