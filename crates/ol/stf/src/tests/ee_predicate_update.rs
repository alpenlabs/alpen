//! Tests for the [`EePredicateKeyUpdate`] log handler in manifest processing.

use ssz::Encode;
use strata_acct_types::{ADMIN_MSG_ACCT_ID, MessageEntry, MsgPayload};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::EePredicateKeyUpdate;
use strata_identifiers::AccountSerial;
use strata_ledger_types::{IAccountState, ISnarkAccountState};
use strata_msg_fmt::{Msg, OwnedMsg};
use strata_ol_msg_types::PREDICATE_UPDATE_MSG_TYPE_ID;
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
        "rotation must not apply until the account declares it in an update"
    );
    assert_eq!(
        account_state.inbox_mmr().num_entries(),
        1,
        "rotation should land in the account inbox so the EE observes it in \
         its inbox ordering and declares it in a later update"
    );
}

#[test]
fn declared_rotation_activates_on_update() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let initial_vk = make_marker_predicate(b"initial");
    let new_vk = make_marker_predicate(b"rotated");

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_update_vk(initial_vk.clone())
                .with_state_root(make_state_root(1))
        })
        .execute_genesis();

    let declared = new_vk.clone();
    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_new_predicate(declared)
                .with_state_root(make_state_root(2))
        })
        .execute();

    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(
        account_state.update_vk(),
        &new_vk,
        "declared rotation must activate with the update"
    );
}

/// Consuming the queued rotation message must not rotate the key by itself:
/// the OL only applies the update's own declaration. Matching the queued key
/// is the EE's policy, enforced by its guest, not by the OL.
#[test]
fn consuming_rotation_message_without_declaration_keeps_key() {
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

    let mut fixture = genesis
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_update_vk(initial_vk.clone())
                .with_state_root(make_state_root(1))
        })
        .with_genesis_manifest(manifest)
        .execute_genesis();

    // Reconstruct the queued message (mirrors the manifest processing path,
    // which stamps it with the processing epoch) to prove its consumption.
    let msg = OwnedMsg::new(PREDICATE_UPDATE_MSG_TYPE_ID, new_vk.as_ssz_bytes())
        .expect("predicate update message type id is in bounds");
    let payload = MsgPayload::from_bytes_valueless(msg.to_vec())
        .expect("predicate key fits in message payload");
    let rotation_msg = MessageEntry::new(ADMIN_MSG_ACCT_ID, 0, payload);
    let mut inbox_tracker = InboxMmrTracker::new();
    let proof = inbox_tracker.add_message(&rotation_msg);

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_processed_messages(vec![rotation_msg], vec![proof])
                .with_state_root(make_state_root(2))
        })
        .execute();

    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(
        account_state.next_inbox_msg_idx(),
        1,
        "the rotation message must have been consumed"
    );
    assert_eq!(
        account_state.update_vk(),
        &initial_vk,
        "consumption alone must not rotate the key"
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
