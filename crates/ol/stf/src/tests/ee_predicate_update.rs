//! Tests for the [`EePredicateKeyUpdate`] log handler in manifest processing.

use strata_acct_types::{BitcoinAmount, Hash};
use strata_asm_common::{AsmLogEntry, AsmManifest};
use strata_asm_logs::EePredicateKeyUpdate;
use strata_identifiers::{AccountSerial, Buf32, WtxidsRoot};
use strata_ledger_types::{
    IAccountState, ISnarkAccountState, IStateAccessor, IStateAccessorMut, NewAccountData,
};
use strata_predicate::{PredicateKey, PredicateTypeId};

use crate::{assembly::BlockComponents, context::BlockInfo, test_utils::*};

/// Builds a non-trivial predicate key with a unique condition payload, used to
/// distinguish "before" and "after" states in tests.
fn make_marker_predicate(marker: &[u8]) -> PredicateKey {
    PredicateKey::new(PredicateTypeId::AlwaysAccept, marker.to_vec())
}

/// Helper that builds a genesis-height ASM manifest carrying a single log.
fn manifest_with_log(log: AsmLogEntry) -> AsmManifest {
    AsmManifest::new(
        1, // genesis manifest height when last_l1_height is 0
        test_l1_block_id(1),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![log],
    )
    .unwrap()
}

#[test]
fn ee_predicate_key_update_applies_to_target_snark_account() {
    let mut state = create_test_genesis_state();

    // Create a snark account with an initial predicate key.
    let snark_account_id = get_test_snark_account_id();
    let initial_vk = make_marker_predicate(b"initial");
    let new_acct_data = NewAccountData::new_snark(
        BitcoinAmount::zero(),
        initial_vk.clone(),
        Buf32::new([1; 32]),
    );
    let snark_serial = state
        .create_new_account(snark_account_id, new_acct_data)
        .expect("create snark account");

    // Build a manifest containing an EePredicateKeyUpdate log for the snark
    // account, switching to a new (distinguishable) predicate key.
    let new_vk = make_marker_predicate(b"rotated");
    let update = EePredicateKeyUpdate::new(snark_serial, new_vk.clone());
    let log_entry = AsmLogEntry::from_log(&update).expect("encode predicate update log");
    let manifest = manifest_with_log(log_entry);

    // Execute a terminal genesis block carrying the manifest.
    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let components = BlockComponents::new_manifests(vec![manifest]);
    execute_block(&mut state, &genesis_info, None, components).expect("genesis with manifest");

    // The snark account's update_vk should now be the new key.
    let acct = state
        .get_account_state(snark_account_id)
        .expect("read account state")
        .expect("account exists");
    let snark = acct.as_snark_account().expect("snark account state");
    assert_eq!(
        snark.update_vk(),
        &new_vk,
        "predicate key should be rotated"
    );
    assert_ne!(
        snark.update_vk(),
        &initial_vk,
        "predicate key must differ from initial"
    );
}

#[test]
fn ee_predicate_key_update_unknown_serial_is_silently_skipped() {
    let mut state = create_test_genesis_state();

    // Create a snark account so the state isn't completely empty.
    let snark_account_id = get_test_snark_account_id();
    let initial_vk = make_marker_predicate(b"initial");
    let new_acct_data = NewAccountData::new_snark(
        BitcoinAmount::zero(),
        initial_vk.clone(),
        Hash::from([1u8; 32]),
    );
    state
        .create_new_account(snark_account_id, new_acct_data)
        .expect("create snark account");

    // Target a serial that does not resolve to any account.
    let bogus_serial = AccountSerial::new(9_999);
    let new_vk = make_marker_predicate(b"rotated");
    let update = EePredicateKeyUpdate::new(bogus_serial, new_vk);
    let log_entry = AsmLogEntry::from_log(&update).expect("encode predicate update log");
    let manifest = manifest_with_log(log_entry);

    // Execution should succeed even though the target serial is unknown.
    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let components = BlockComponents::new_manifests(vec![manifest]);
    execute_block(&mut state, &genesis_info, None, components)
        .expect("genesis must succeed even with unknown serial");

    // The existing snark account's predicate key must be unchanged.
    let acct = state
        .get_account_state(snark_account_id)
        .expect("read account state")
        .expect("account exists");
    let snark = acct.as_snark_account().expect("snark account state");
    assert_eq!(snark.update_vk(), &initial_vk);
}

#[test]
fn ee_predicate_key_update_targeting_empty_account_is_silently_skipped() {
    let mut state = create_test_genesis_state();

    // Create a non-snark (empty) account.
    let empty_account_id = test_account_id(7);
    let empty_serial = create_empty_account(&mut state, empty_account_id);

    // Build the update log targeting the empty account's serial.
    let new_vk = make_marker_predicate(b"rotated");
    let update = EePredicateKeyUpdate::new(empty_serial, new_vk);
    let log_entry = AsmLogEntry::from_log(&update).expect("encode predicate update log");
    let manifest = manifest_with_log(log_entry);

    // Execution should succeed even though the target is not a snark account.
    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let components = BlockComponents::new_manifests(vec![manifest]);
    execute_block(&mut state, &genesis_info, None, components)
        .expect("genesis must succeed when target is non-snark");

    // The empty account is still empty.
    let acct = state
        .get_account_state(empty_account_id)
        .expect("read account state")
        .expect("account exists");
    assert!(
        acct.as_snark_account().is_err(),
        "empty account must remain non-snark"
    );
}
