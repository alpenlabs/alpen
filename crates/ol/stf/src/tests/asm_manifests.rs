//! Tests for ASM manifest processing in the OL STF.

use strata_acct_types::BitcoinAmount;
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_logs::{CheckpointTipUpdate, DepositLog, constants::DEPOSIT_LOG_TYPE_ID};
use strata_checkpoint_types_ssz::CheckpointTip;
use strata_identifiers::{
    AccountSerial, Buf32, EpochCommitment, OLBlockCommitment, OLBlockId, SubjectId, SubjectIdBytes,
    WtxidsRoot,
};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};
use strata_msg_fmt::MAX_TYPE;
use strata_ol_bridge_types::DepositDescriptor;
use strata_ol_chain_types_new::MAX_SEALING_MANIFEST_COUNT;

use crate::{assembly::BlockComponents, context::BlockInfo, errors::ExecError, test_utils::*};

#[test]
fn test_manifest_processing_rejects_height_gap() {
    let mut state = create_test_genesis_state();
    let asm_manifest = create_test_asm_manifest_with_l1_height(state.last_l1_height() + 2, vec![]);

    let result = execute_block(
        &mut state,
        &BlockInfo::new_genesis(1_000_000),
        None,
        BlockComponents::new_manifests(vec![asm_manifest]),
    );

    match result {
        Err(e) => assert!(
            matches!(e.into_base(), ExecError::ChainIntegrity),
            "Expected ChainIntegrity"
        ),
        Ok(_) => panic!("manifest with non-contiguous height should fail"),
    }

    assert_eq!(state.last_l1_height(), 0);
    assert_eq!(state.asm_manifests_mmr().num_entries(), 0);
}

#[test]
fn test_manifest_processing_accepts_empty_manifest_container() {
    let mut state = create_test_genesis_state();

    execute_block(
        &mut state,
        &BlockInfo::new_genesis(1_000_000),
        None,
        BlockComponents::new_manifests(vec![]),
    )
    .expect("empty terminal manifest container should execute");

    assert_eq!(state.cur_epoch(), 1);
    assert_eq!(state.last_l1_height(), 0);
    assert_eq!(state.asm_manifests_mmr().num_entries(), 0);
}

#[test]
fn test_manifest_processing_accepts_max_manifest_count() {
    let mut state = create_test_genesis_state();
    let asm_manifests: Vec<_> = (1..=MAX_SEALING_MANIFEST_COUNT)
        .map(|height| create_test_asm_manifest_with_l1_height(height as u32, vec![]))
        .collect();

    execute_block(
        &mut state,
        &BlockInfo::new_genesis(1_000_000),
        None,
        BlockComponents::new_manifests(asm_manifests),
    )
    .expect("max manifest count should execute");

    assert_eq!(state.cur_epoch(), 1);
    assert_eq!(state.last_l1_height(), MAX_SEALING_MANIFEST_COUNT as u32);
    assert_eq!(
        state.asm_manifests_mmr().num_entries(),
        MAX_SEALING_MANIFEST_COUNT
    );
}

#[test]
fn test_manifest_processing_skips_unknown_and_unparseable_logs() {
    let mut state = create_test_genesis_state();
    let unknown_type_log =
        AsmLogEntry::from_msg(MAX_TYPE, vec![1, 2, 3]).expect("unknown-type log should encode");
    let raw_log = AsmLogEntry::from_raw(vec![0xff]).expect("raw log should fit");
    let asm_manifest = create_test_asm_manifest_with_l1_height(1, vec![unknown_type_log, raw_log]);

    let output = execute_block_with_outputs(
        &mut state,
        &BlockInfo::new_genesis(1_000_000),
        None,
        BlockComponents::new_manifests(vec![asm_manifest]),
    )
    .expect("unknown and unparseable logs should be skipped");

    assert_eq!(output.outputs().logs().len(), 0);
    assert_eq!(state.cur_epoch(), 1);
    assert_eq!(state.last_l1_height(), 1);
    assert_eq!(state.asm_manifests_mmr().num_entries(), 1);
}

#[test]
fn test_manifest_processing_skips_unknown_serial_deposit() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    create_snark_account_with_balance(&mut state, snark_id, 0);

    let deposit_log = deposit_log_for_serial(AccountSerial::new(9_999), 100_000_000);
    let asm_manifest = create_test_asm_manifest_with_l1_height(1, vec![deposit_log]);

    execute_block(
        &mut state,
        &BlockInfo::new_genesis(1_000_000),
        None,
        BlockComponents::new_manifests(vec![asm_manifest]),
    )
    .expect("deposit to unknown serial should be skipped");

    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(ol_account_state.balance(), BitcoinAmount::from_sat(0));
    assert_eq!(snark_account_state.inbox_mmr().num_entries(), 0);
    assert_eq!(state.last_l1_height(), 1);
}

#[test]
fn test_manifest_processing_updates_checkpoint_tip() {
    let mut state = create_test_genesis_state();
    let l2_commitment = OLBlockCommitment::new(42, OLBlockId::from(Buf32::from([0xab; 32])));
    let tip = CheckpointTip::new(7, 100, l2_commitment);
    let update = CheckpointTipUpdate::new(tip);
    let log = AsmLogEntry::from_log(&update).expect("checkpoint tip update should encode");
    let asm_manifest = create_test_asm_manifest_with_l1_height(1, vec![log]);

    execute_block(
        &mut state,
        &BlockInfo::new_genesis(1_000_000),
        None,
        BlockComponents::new_manifests(vec![asm_manifest]),
    )
    .expect("checkpoint tip update should execute");

    assert_eq!(
        state.asm_recorded_epoch(),
        &EpochCommitment::from_terminal(7, l2_commitment)
    );
    assert_eq!(state.last_l1_height(), 1);
}

#[test]
fn test_manifest_processing_skips_malformed_deposit_log() {
    let mut state = create_test_genesis_state();
    let malformed_deposit_log =
        AsmLogEntry::from_msg(DEPOSIT_LOG_TYPE_ID, vec![0xff]).expect("deposit log should encode");
    let asm_manifest = create_test_asm_manifest_with_l1_height(1, vec![malformed_deposit_log]);

    execute_block(
        &mut state,
        &BlockInfo::new_genesis(1_000_000),
        None,
        BlockComponents::new_manifests(vec![asm_manifest]),
    )
    .expect("malformed deposit log should be skipped");

    assert_eq!(state.cur_epoch(), 1);
    assert_eq!(state.last_l1_height(), 1);
    assert_eq!(state.asm_manifests_mmr().num_entries(), 1);
}

fn create_test_asm_manifest_with_l1_height(l1_height: u32, logs: Vec<AsmLogEntry>) -> AsmManifest {
    AsmManifest::new(
        l1_height,
        test_l1_block_id(l1_height),
        WtxidsRoot::from(Buf32::from([l1_height as u8; 32])),
        logs,
    )
    .expect("test manifest should be valid")
}

fn deposit_log_for_serial(account_serial: AccountSerial, amount: u64) -> AsmLogEntry {
    let subject = SubjectId::from([42u8; 32]);
    let subject_bytes =
        SubjectIdBytes::try_new(subject.inner().to_vec()).expect("valid subject bytes");
    let descriptor =
        DepositDescriptor::new(account_serial, subject_bytes).expect("valid deposit descriptor");
    let deposit = DepositLog::new(descriptor.encode_to_varvec(), amount);
    AsmLogEntry::from_log(&deposit).expect("deposit log should encode")
}
