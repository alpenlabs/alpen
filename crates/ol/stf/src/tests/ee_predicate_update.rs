//! Tests for the [`EePredicateKeyUpdate`] log handler in manifest processing.

use strata_asm_common::AsmLogEntry;
use strata_asm_logs::EePredicateKeyUpdate;
use strata_identifiers::AccountSerial;
use strata_ledger_types::{IAccountState, ISnarkAccountState};
use strata_predicate::{PredicateKey, PredicateTypeId};

use crate::test_utils::*;

/// Builds a non-trivial predicate key with a unique condition payload, used to
/// distinguish "before" and "after" states in tests.
fn make_marker_predicate(marker: &[u8]) -> PredicateKey {
    PredicateKey::new(PredicateTypeId::AlwaysAccept, marker.to_vec())
}

#[test]
fn ee_predicate_key_update_lands_in_target_account_inbox() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let initial_vk = make_marker_predicate(b"initial");
    let new_vk = make_marker_predicate(b"rotated");

    let genesis = OLStfFixture::builder();
    let snark_acct_serial = genesis.next_account_serial();
    let update = EePredicateKeyUpdate::new(snark_acct_serial, new_vk.clone());
    let log_entry = AsmLogEntry::from_log(&update).expect("encode predicate update log");
    let manifest = FixtureAsmManifestBuilder::new_at_height(1)
        .with_variant(1)
        .with_log(log_entry)
        .build();

    let fixture = genesis
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_update_vk(initial_vk.clone())
                .with_state_root(make_state_root(1))
        })
        .with_genesis_manifest(manifest)
        .execute_genesis();

    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(
        account_state.update_vk(),
        &initial_vk,
        "rotation must not apply until the account consumes the inbox message"
    );
    assert_eq!(
        account_state.inbox_mmr().num_entries(),
        1,
        "rotation should land in the account inbox so it activates when \
         consumed and the EE observes it in its inbox ordering"
    );
}

#[test]
fn ee_predicate_key_update_unknown_serial_is_silently_skipped() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let initial_vk = make_marker_predicate(b"initial");
    let new_vk = make_marker_predicate(b"rotated");
    let bogus_serial = AccountSerial::new(9_999);

    let update = EePredicateKeyUpdate::new(bogus_serial, new_vk);
    let log_entry = AsmLogEntry::from_log(&update).expect("encode predicate update log");
    let manifest = FixtureAsmManifestBuilder::new_at_height(1)
        .with_variant(1)
        .with_log(log_entry)
        .build();

    let fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_update_vk(initial_vk.clone())
                .with_state_root(make_state_root(1))
        })
        .with_genesis_manifest(manifest)
        .execute_genesis();

    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(account_state.update_vk(), &initial_vk);
}

#[test]
fn ee_predicate_key_update_targeting_empty_account_is_silently_skipped() {
    let empty_account_id = make_account_id(7);
    let new_vk = make_marker_predicate(b"rotated");

    let genesis = OLStfFixture::builder();
    let empty_serial = genesis.next_account_serial();
    let update = EePredicateKeyUpdate::new(empty_serial, new_vk);
    let log_entry = AsmLogEntry::from_log(&update).expect("encode predicate update log");
    let manifest = FixtureAsmManifestBuilder::new_at_height(1)
        .with_variant(1)
        .with_log(log_entry)
        .build();

    let fixture = genesis
        .with_genesis_empty_account(empty_account_id)
        .with_genesis_manifest(manifest)
        .execute_genesis();

    assert!(
        fixture
            .expect_account(empty_account_id)
            .as_snark_account()
            .is_err(),
        "empty account must remain non-snark"
    );
}
