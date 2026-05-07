//! Tests covering the paths that route misplaced funds into limbo via
//! [`crate::account_processing::handle_misplaced_funds`].
//!
//! Sites covered here:
//! - A: `process_message` to a non-existent ledger account
//! - B: `process_transfer` to `BRIDGE_GATEWAY_ACCT_ID`
//! - C: `process_transfer` to a non-existent ledger account
//! - E: bridge-gateway message that parses as `MsgRef` but is not a withdrawal
//! - F: bridge-gateway withdrawal message with a bad amount
//! - G: manifest deposit log with a destination that does not decode as a `DepositDescriptor`
//! - H: manifest deposit log referencing an account serial that does not exist
//!
//! Sites D (bridge-gateway message with un-parseable payload) is exercised by
//! existing tests in `tests/multi_operations.rs`, which now also assert the
//! limbo balance grew.

use strata_acct_types::{BitcoinAmount, MsgPayload};
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_logs::DepositLog;
use strata_codec::{VarVec, encode_to_vec};
use strata_identifiers::{AccountSerial, Buf32, SubjectId, SubjectIdBytes, WtxidsRoot};
use strata_ledger_types::{IAccountState, IStateAccessor};
use strata_msg_fmt::{Msg, OwnedMsg};
use strata_ol_bridge_types::DepositDescriptor;
use strata_ol_msg_types::{DEFAULT_OPERATOR_FEE, WITHDRAWAL_MSG_TYPE_ID, WithdrawalMsgData};

use crate::{
    BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL, account_processing,
    assembly::BlockComponents,
    context::{BasicExecContext, BlockInfo},
    output::ExecOutputBuffer,
    test_utils::*,
};

/// Site A: `process_message` to a non-existent ledger account.
///
/// In practice the SAU validator (`verify_effects_safe`) rejects unknown
/// non-special destinations before this code runs, so we exercise the
/// branch directly through the internal entry point.
#[test]
fn limbo_message_to_nonexistent_account() {
    let mut state = create_test_genesis_state();
    let nonexistent = test_account_id(TEST_NONEXISTENT_ID);
    let limbo_before = state.limbo_funds();

    let block_info = BlockInfo::new(1, 0, 0);
    let outputs = ExecOutputBuffer::new_empty();
    let context = BasicExecContext::new(block_info, &outputs);

    let value = BitcoinAmount::from_sat(2_500_000);
    let payload = MsgPayload::new(value, vec![]);

    account_processing::process_message(
        &mut state,
        BRIDGE_GATEWAY_ACCT_ID,
        nonexistent,
        payload,
        &context,
    )
    .expect("process_message should not error");

    let limbo_after = state.limbo_funds();
    assert_eq!(
        limbo_after.to_sat() - limbo_before.to_sat(),
        2_500_000,
        "Message to non-existent account should sweep value into limbo"
    );
}

/// Site C: `process_transfer` to a non-existent ledger account.
///
/// Same dispatch consideration as site A — direct call to the internal
/// entry point since the SAU validator rejects this in normal flow.
#[test]
fn limbo_transfer_to_nonexistent_account() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let nonexistent = test_account_id(TEST_NONEXISTENT_ID);
    let limbo_before = state.limbo_funds();

    let block_info = BlockInfo::new(1, 0, 0);
    let outputs = ExecOutputBuffer::new_empty();
    let context = BasicExecContext::new(block_info, &outputs);

    let value = BitcoinAmount::from_sat(4_000_000);
    account_processing::process_transfer(&mut state, snark_id, nonexistent, value, &context)
        .expect("process_transfer should not error");

    let limbo_after = state.limbo_funds();
    assert_eq!(
        limbo_after.to_sat() - limbo_before.to_sat(),
        4_000_000,
        "Transfer to non-existent account should sweep value into limbo"
    );
}

/// Site B: `process_transfer` to `BRIDGE_GATEWAY_ACCT_ID`.
///
/// Drives the path through a SAU transaction, since `BRIDGE_GATEWAY_ACCT_ID`
/// is special and passes `verify_effects_safe` without an existence check.
#[test]
fn limbo_transfer_to_bridge_gateway() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);
    let limbo_before = state.limbo_funds();

    let snark_state = state
        .get_account_state(snark_id)
        .unwrap()
        .unwrap()
        .as_snark_account()
        .unwrap()
        .clone();
    let tx = SnarkUpdateBuilder::from_snark_state(snark_state)
        .with_transfer(BRIDGE_GATEWAY_ACCT_ID, 7_000_000)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    execute_tx_in_block(&mut state, genesis_block.header(), tx, 1, 0)
        .expect("transfer to bridge gateway should succeed");

    let limbo_after = state.limbo_funds();
    assert_eq!(
        limbo_after.to_sat() - limbo_before.to_sat(),
        7_000_000,
        "Transfer to bridge gateway should sweep value into limbo"
    );

    let snark_acct = state.get_account_state(snark_id).unwrap().unwrap();
    assert_eq!(
        snark_acct.balance().to_sat(),
        100_000_000 - 7_000_000,
        "Sender should have been debited by the transfer amount"
    );
}

/// Site E: bridge-gateway message that parses as `MsgRef` but isn't a
/// withdrawal.
#[test]
fn limbo_non_withdrawal_message_to_bridge_gateway() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 200_000_000);
    let limbo_before = state.limbo_funds();

    // A well-formed `OwnedMsg` whose type id is not the withdrawal type id.
    let bogus_type_id: u16 = 0x99;
    assert_ne!(bogus_type_id, WITHDRAWAL_MSG_TYPE_ID);
    let owned = OwnedMsg::new(bogus_type_id, vec![1, 2, 3, 4]).expect("valid OwnedMsg");
    let msg_bytes = owned.to_vec();

    let snark_state = state
        .get_account_state(snark_id)
        .unwrap()
        .unwrap()
        .as_snark_account()
        .unwrap()
        .clone();
    let tx = SnarkUpdateBuilder::from_snark_state(snark_state)
        .with_output_message(BRIDGE_GATEWAY_ACCT_ID, 100_000_000, msg_bytes)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let output = execute_block_with_outputs(
        &mut state,
        &BlockInfo::new(1_001_000, 1, 0),
        Some(genesis_block.header()),
        BlockComponents::new_txs_from_ol_transactions(vec![tx]),
    )
    .expect("non-withdrawal message to bridge gateway should succeed");

    let limbo_after = state.limbo_funds();
    assert_eq!(
        limbo_after.to_sat() - limbo_before.to_sat(),
        100_000_000,
        "Non-withdrawal bridge-gateway message should sweep value into limbo"
    );

    // No withdrawal intent log should have been emitted.
    for log in output.outputs().logs() {
        assert!(
            log.account_serial() != BRIDGE_GATEWAY_ACCT_SERIAL,
            "no bridge-gateway log expected, got {log:?}"
        );
    }
}

/// Site F: bridge-gateway withdrawal message with an amount that is not a
/// multiple of the 100_000_000 sat denomination.
#[test]
fn limbo_bad_withdrawal_amount_to_bridge_gateway() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);
    let limbo_before = state.limbo_funds();

    // A real, decode-able withdrawal body, but the outer payload value is 1
    // sat — not a multiple of 100_000_000.
    let withdrawal_body =
        WithdrawalMsgData::new(DEFAULT_OPERATOR_FEE, b"bc1qexample".to_vec(), u32::MAX)
            .expect("valid withdrawal body");
    let encoded_body = encode_to_vec(&withdrawal_body).expect("encode withdrawal body");
    let owned = OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, encoded_body).expect("valid OwnedMsg");
    let msg_bytes = owned.to_vec();

    let snark_state = state
        .get_account_state(snark_id)
        .unwrap()
        .unwrap()
        .as_snark_account()
        .unwrap()
        .clone();
    let tx = SnarkUpdateBuilder::from_snark_state(snark_state)
        .with_output_message(BRIDGE_GATEWAY_ACCT_ID, 1, msg_bytes)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let output = execute_block_with_outputs(
        &mut state,
        &BlockInfo::new(1_001_000, 1, 0),
        Some(genesis_block.header()),
        BlockComponents::new_txs_from_ol_transactions(vec![tx]),
    )
    .expect("bad-amount withdrawal should not error block");

    let limbo_after = state.limbo_funds();
    assert_eq!(
        limbo_after.to_sat() - limbo_before.to_sat(),
        1,
        "Bad withdrawal amount should sweep value into limbo"
    );

    for log in output.outputs().logs() {
        assert!(
            log.account_serial() != BRIDGE_GATEWAY_ACCT_SERIAL,
            "no withdrawal intent log expected, got {log:?}"
        );
    }
}

/// Site G: manifest deposit log whose destination bytes do not decode as a
/// [`DepositDescriptor`].
#[test]
fn limbo_deposit_with_malformed_descriptor() {
    let mut state = create_test_genesis_state();
    let limbo_before = state.limbo_funds();
    let deposit_amount = 75_000_000u64;

    // Empty destination — `DepositDescriptor::decode_from_slice` returns
    // `EmptyDescriptor` for this.
    let bogus_destination = VarVec::from_vec(Vec::<u8>::new()).expect("varvec");
    let deposit_log_data = DepositLog::new(bogus_destination, deposit_amount);
    let deposit_log = AsmLogEntry::from_log(&deposit_log_data).expect("log entry");

    let manifest = AsmManifest::new(
        1,
        test_l1_block_id(1),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![deposit_log],
    )
    .expect("valid manifest");

    execute_block(
        &mut state,
        &BlockInfo::new_genesis(1_000_000),
        None,
        BlockComponents::new_manifests(vec![manifest]),
    )
    .expect("genesis manifest with bad descriptor should still execute");

    let limbo_after = state.limbo_funds();
    assert_eq!(
        limbo_after.to_sat() - limbo_before.to_sat(),
        deposit_amount,
        "Deposit with malformed descriptor should sweep amount into limbo"
    );
}

/// Site H: manifest deposit log referencing an account serial that does not
/// exist in state.
#[test]
fn limbo_deposit_to_unknown_account_serial() {
    let mut state = create_test_genesis_state();
    let limbo_before = state.limbo_funds();
    let deposit_amount = 50_000_000u64;

    // No accounts have been created, so any non-reserved serial is unknown.
    let unknown_serial = AccountSerial::new(12345);
    let subject =
        SubjectIdBytes::try_new(SubjectId::from([7u8; 32]).inner().to_vec()).expect("subject");
    let descriptor = DepositDescriptor::new(unknown_serial, subject).expect("valid descriptor");
    let destination = descriptor.encode_to_varvec();
    let deposit_log_data = DepositLog::new(destination, deposit_amount);
    let deposit_log = AsmLogEntry::from_log(&deposit_log_data).expect("log entry");

    let manifest = AsmManifest::new(
        1,
        test_l1_block_id(1),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![deposit_log],
    )
    .expect("valid manifest");

    execute_block(
        &mut state,
        &BlockInfo::new_genesis(1_000_000),
        None,
        BlockComponents::new_manifests(vec![manifest]),
    )
    .expect("genesis manifest with unknown serial should still execute");

    let limbo_after = state.limbo_funds();
    assert_eq!(
        limbo_after.to_sat() - limbo_before.to_sat(),
        deposit_amount,
        "Deposit for unknown account serial should sweep amount into limbo"
    );
}
