//! Tests for the [`EePredicateKeyUpdate`] log handler in manifest processing.

use strata_acct_types::{ADMIN_MSG_ACCT_ID, MessageEntry};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::EePredicateKeyUpdate;
use strata_identifiers::AccountSerial;
use strata_ol_state_types::{IAccountState, ISnarkAccountState};
use strata_predicate::{PredicateKey, PredicateTypeId};

use crate::{
    errors::ExecError, manifest_processing::build_predicate_update_payload, test_utils::*,
};

/// Builds a non-trivial predicate key with a unique condition payload, used to
/// distinguish "before" and "after" states in tests.
fn make_marker_predicate(marker: &[u8]) -> PredicateKey {
    PredicateKey::try_new(PredicateTypeId::AlwaysAccept, marker.to_vec())
        .expect("predicate condition must fit within the maximum length")
}

/// Builds a predicate key with a type id outside the registry (only 0, 1, 10,
/// and 20 are registered). SSZ decodes the `id` field as a raw byte, so a
/// value like this is reachable from untrusted wire data even though the
/// typed constructor can't produce it directly.
fn make_unregistered_type_predicate() -> PredicateKey {
    PredicateKey {
        id: 250,
        condition: Vec::new()
            .try_into()
            .expect("empty condition fits any bound"),
    }
}

/// Proves the unregistered-id key isn't just a test-only artifact: it survives the
/// exact wire round trip an ASM log takes (`AsmLogEntry::from_log` /
/// `try_into_log`), so `PredicateKey::try_as_buf_ref` really can fail on data that
/// reached `process_ee_predicate_key_update` from L1.
#[test]
fn unregistered_type_predicate_survives_asm_log_round_trip() {
    let update =
        EePredicateKeyUpdate::new(AccountSerial::new(1), make_unregistered_type_predicate());
    let log_entry = AsmLogEntry::from_log(&update).expect("encode predicate update log");
    let decoded: EePredicateKeyUpdate = log_entry
        .try_into_log()
        .expect("decode predicate update log");

    assert_eq!(decoded.new_predicate().id(), 250);
    assert!(
        decoded.new_predicate().try_as_buf_ref().is_err(),
        "decoded key must still carry the unregistered id"
    );
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

/// A SAU is proven valid under the account's *current* predicate, which says
/// nothing about whether the *declared successor* has a registered type. The
/// OL must reject the update rather than install a key that would panic the
/// next time something tries to use it (e.g. serializing the DA diff).
#[test]
fn declared_rotation_rejects_unregistered_predicate_type_id() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let initial_vk = make_marker_predicate(b"initial");

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_update_vk(initial_vk.clone())
                .with_state_root(make_state_root(1))
        })
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_new_predicate(make_unregistered_type_predicate())
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    assert!(
        matches!(
            err.into_base(),
            ExecError::TxStructureCheckFailed("declared predicate key has unregistered type id")
        ),
        "declared rotation with an unregistered type id must be rejected"
    );

    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(
        account_state.update_vk(),
        &initial_vk,
        "rejected rotation must not change the account's predicate"
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
    let payload = build_predicate_update_payload(&new_vk);
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

/// The ASM log's `PredicateKey` is SSZ-decoded straight from L1 data, so its
/// `id` byte is just as unvalidated as a declared rotation's. ASM manifests
/// can't be rejected without halting checkpoint progress, so this must be
/// dropped like the other unusable-target cases above, not panic when the
/// queued message is built.
#[test]
fn ee_predicate_key_update_with_unregistered_type_id_is_silently_skipped() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let initial_vk = make_marker_predicate(b"initial");

    let genesis = OLStfFixture::builder();
    let snark_acct_serial = genesis.next_account_serial();
    let update = EePredicateKeyUpdate::new(snark_acct_serial, make_unregistered_type_predicate());
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
        "unregistered-type rotation must not reach the account"
    );
    assert_eq!(
        account_state.inbox_mmr().num_entries(),
        0,
        "no rotation message should be queued for an unusable predicate key"
    );
}
