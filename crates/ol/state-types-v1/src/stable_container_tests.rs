use ssz::{Decode, DecodeError, Encode};
use ssz_types::{BitVector, Optional, VariableList};
use strata_acct_types::{AccountId, BitcoinAmount, SYSTEM_RESERVED_ACCTS};
use strata_identifiers::{AccountSerial, Buf32, Slot};
use strata_ol_state_types::OLSpecId;
use strata_predicate::PredicateKey;
use tree_hash::{Sha256Hasher, TreeHash};

use crate::ssz_generated::ssz::state::{
    EpochalStateV1Ssz, GlobalStateV1Ssz, IntraepochStateV1Ssz, OLSnarkAccountStateV1Ssz,
    ProtocolStateV1Ssz,
};
use crate::test_utils::create_test_genesis_state;
use crate::{
    EpochalStateV1, GlobalStateV1, IntraepochStateV1, MAX_LEDGER_ACCOUNTS, OLAccountStateV1,
    OLAccountTypeStateV1, OLSnarkAccountStateV1, OLStateV1, ProtocolStateV1, WriteBatch,
};

// Keeps nested wire fields permissive so tests can construct malformed encodings
// that the public domain states deliberately cannot represent.
#[derive(ssz_derive::Encode, ssz_derive::Decode)]
#[ssz(struct_behaviour = "stable_container", max_fields = 32)]
struct TestStateSsz {
    epoch: Optional<EpochalStateV1Ssz>,
    global: Optional<GlobalStateV1Ssz>,
    intraepoch: Optional<IntraepochStateV1Ssz>,
    ledger: Optional<TestLedgerSsz>,
    protocol_state: Optional<ProtocolStateV1Ssz>,
}

#[derive(ssz_derive::Encode, ssz_derive::Decode)]
struct TestLedgerSsz {
    accounts: VariableList<TestAccountEntrySsz, { MAX_LEDGER_ACCOUNTS as usize }>,
}

#[derive(ssz_derive::Encode, ssz_derive::Decode)]
struct TestAccountEntrySsz {
    id: AccountId,
    state: TestAccountSsz,
}

#[derive(ssz_derive::Encode, ssz_derive::Decode)]
#[ssz(struct_behaviour = "stable_container", max_fields = 8)]
struct TestAccountSsz {
    serial: Optional<AccountSerial>,
    balance: Optional<BitcoinAmount>,
    state: Optional<TestAccountTypeSsz>,
}

#[derive(ssz_derive::Encode, ssz_derive::Decode)]
#[ssz(enum_behaviour = "union")]
enum TestAccountTypeSsz {
    Empty,
    Snark(OLSnarkAccountStateV1Ssz),
}

fn fixture_snark_mut(account: &mut TestAccountSsz) -> &mut OLSnarkAccountStateV1Ssz {
    match fixture_field_mut(&mut account.state) {
        TestAccountTypeSsz::Snark(snark) => snark,
        TestAccountTypeSsz::Empty => panic!("fixture uses a snark account"),
    }
}

// Only the permissive test wire representation can contain absent nested fields.
fn fixture_field_mut<T>(field: &mut Optional<T>) -> &mut T {
    match field {
        Optional::Some(value) => value,
        Optional::None => panic!("genesis fixture contains every nested state"),
    }
}

#[test]
fn test_genesis_stable_fields_have_fixed_positions_and_capacity() {
    let state = create_test_genesis_state();
    assert_eq!(&state.as_ssz_bytes()[..4], &[0b0001_1111, 0, 0, 0]);
    assert_eq!(&state.epoch_state().as_ssz_bytes()[..2], &[0b0001_1111, 0]);
    assert_eq!(&state.global_state().as_ssz_bytes()[..2], &[0b0000_0111, 0]);
    assert_eq!(state.intraepoch_state().as_ssz_bytes()[0], 1);
    assert_eq!(state.active_version(), OLSpecId::V1);
    assert_eq!(state.expected_version(), OLSpecId::V1);
    assert_eq!(
        OLStateV1::from_ssz_bytes(&state.as_ssz_bytes()).unwrap(),
        state
    );
}

#[test]
fn test_public_state_decode_rejects_each_missing_v1_field() {
    use ssz::view::DecodeView;

    type RemoveField = fn(&mut TestStateSsz);
    let cases: &[(&str, RemoveField)] = &[
        ("epoch", |state| state.epoch = Optional::None),
        ("global", |state| state.global = Optional::None),
        ("intraepoch", |state| state.intraepoch = Optional::None),
        ("ledger", |state| state.ledger = Optional::None),
        ("protocol_state", |state| {
            state.protocol_state = Optional::None
        }),
        ("epoch.total_ledger_funds", |state| {
            fixture_field_mut(&mut state.epoch).total_ledger_funds = Optional::None
        }),
        ("epoch.cur_epoch", |state| {
            fixture_field_mut(&mut state.epoch).cur_epoch = Optional::None
        }),
        ("epoch.last_l1_block", |state| {
            fixture_field_mut(&mut state.epoch).last_l1_block = Optional::None
        }),
        ("epoch.checkpointed_epoch", |state| {
            fixture_field_mut(&mut state.epoch).checkpointed_epoch = Optional::None
        }),
        ("epoch.l1_block_refs_mmr", |state| {
            fixture_field_mut(&mut state.epoch).l1_block_refs_mmr = Optional::None
        }),
        ("global.cur_slot", |state| {
            fixture_field_mut(&mut state.global).cur_slot = Optional::None
        }),
        ("global.next_avail_serial", |state| {
            fixture_field_mut(&mut state.global).next_avail_serial = Optional::None
        }),
        ("global.limbo_funds_sats", |state| {
            fixture_field_mut(&mut state.global).limbo_funds_sats = Optional::None
        }),
        ("intraepoch.pending_asm_logs", |state| {
            fixture_field_mut(&mut state.intraepoch).pending_asm_logs = Optional::None
        }),
        ("protocol_state.active_version", |state| {
            fixture_field_mut(&mut state.protocol_state).active_version = Optional::None
        }),
        ("protocol_state.expected_version", |state| {
            fixture_field_mut(&mut state.protocol_state).expected_version = Optional::None
        }),
    ];

    let valid_bytes = create_test_genesis_state().as_ssz_bytes();
    for (field, remove) in cases {
        let mut state = TestStateSsz::from_ssz_bytes(&valid_bytes).unwrap();
        assert_eq!(state.as_ssz_bytes(), valid_bytes);
        remove(&mut state);
        let bytes = state.as_ssz_bytes();
        let expected = Err(DecodeError::BytesInvalid(format!(
            "missing required OL state field: {field}"
        )));
        assert_eq!(<OLStateV1 as Decode>::from_ssz_bytes(&bytes), expected);
        assert_eq!(<OLStateV1 as DecodeView>::from_ssz_bytes(&bytes), expected);
    }
}

#[test]
fn test_account_fields_are_required_through_nested_public_decoding() {
    use ssz::view::DecodeView;

    let snark = OLSnarkAccountStateV1::new_fresh(PredicateKey::always_accept(), Buf32::zero());
    let account = OLAccountStateV1::new(
        AccountSerial::new(SYSTEM_RESERVED_ACCTS),
        BitcoinAmount::try_from(0).unwrap(),
        OLAccountTypeStateV1::Snark(snark.clone()),
    );
    assert_eq!(account.as_ssz_bytes()[0], 0b0000_0111);
    assert_eq!(&snark.as_ssz_bytes()[..2], &[0b0000_1111, 0]);
    let mut state = create_test_genesis_state();
    state
        .ledger_mut()
        .create_account(AccountId::from([1; 32]), account)
        .unwrap();
    let valid_bytes = state.as_ssz_bytes();
    assert_eq!(
        <OLStateV1 as Decode>::from_ssz_bytes(&valid_bytes).unwrap(),
        state
    );
    assert_eq!(
        <OLStateV1 as DecodeView>::from_ssz_bytes(&valid_bytes).unwrap(),
        state
    );

    type RemoveField = fn(&mut TestAccountSsz);
    let cases: &[(&str, RemoveField)] = &[
        ("account.serial", |account| account.serial = Optional::None),
        ("account.balance", |account| {
            account.balance = Optional::None
        }),
        ("account.state", |account| account.state = Optional::None),
        ("snark_account.update_vk", |account| {
            fixture_snark_mut(account).update_vk = Optional::None
        }),
        ("snark_account.seqno", |account| {
            fixture_snark_mut(account).seqno = Optional::None
        }),
        ("snark_account.proof_state", |account| {
            fixture_snark_mut(account).proof_state = Optional::None
        }),
        ("snark_account.inbox_mmr", |account| {
            fixture_snark_mut(account).inbox_mmr = Optional::None
        }),
    ];

    for (field, remove) in cases {
        let mut wire = TestStateSsz::from_ssz_bytes(&valid_bytes).unwrap();
        assert_eq!(wire.as_ssz_bytes(), valid_bytes);
        let account = &mut fixture_field_mut(&mut wire.ledger).accounts[0].state;
        remove(account);
        let expected =
            DecodeError::BytesInvalid(format!("missing required OL state field: {field}"));
        let account_bytes = account.as_ssz_bytes();
        assert_eq!(
            <OLAccountStateV1 as Decode>::from_ssz_bytes(&account_bytes),
            Err(expected.clone())
        );
        assert_eq!(
            <OLAccountStateV1 as DecodeView>::from_ssz_bytes(&account_bytes),
            Err(expected.clone())
        );
        if field.starts_with("snark_account.") {
            let snark_bytes = fixture_snark_mut(account).as_ssz_bytes();
            assert_eq!(
                <OLSnarkAccountStateV1 as Decode>::from_ssz_bytes(&snark_bytes),
                Err(expected.clone())
            );
            assert_eq!(
                <OLSnarkAccountStateV1 as DecodeView>::from_ssz_bytes(&snark_bytes),
                Err(expected.clone())
            );
        }
        let bytes = wire.as_ssz_bytes();
        assert_eq!(
            <OLStateV1 as Decode>::from_ssz_bytes(&bytes),
            Err(expected.clone())
        );
        assert_eq!(
            <OLStateV1 as DecodeView>::from_ssz_bytes(&bytes),
            Err(expected)
        );
    }
}

#[test]
fn test_public_state_decode_rejects_unknown_protocol_version() {
    let state = create_test_genesis_state();
    let mut bytes = state.as_ssz_bytes();
    // Protocol state is the last top-level field; expected_version is its final byte.
    *bytes.last_mut().expect("nonempty state encoding") = u8::MAX;
    assert!(OLStateV1::from_ssz_bytes(&bytes).is_err());
}

#[test]
fn test_intraepoch_reset_and_batch_application_preserve_protocol_state() {
    let mut state = create_test_genesis_state();
    state.intraepoch_state_mut().reset();
    state.apply_write_batch(WriteBatch::default()).unwrap();
    assert_eq!(state.active_version(), OLSpecId::V1);
    assert_eq!(state.expected_version(), OLSpecId::V1);
    assert_eq!(state.intraepoch_state().as_ssz_bytes(), vec![1, 4, 0, 0, 0]);
    assert_eq!(
        OLStateV1::from_ssz_bytes(&state.as_ssz_bytes()).unwrap(),
        state
    );
}

// Represents one possible additive extension without changing the production schema.
#[derive(ssz_derive::Encode, ssz_derive::Decode, tree_hash_derive::TreeHash)]
#[ssz(struct_behaviour = "stable_container", max_fields = 16)]
#[tree_hash(struct_behaviour = "stable_container", max_fields = 16)]
struct ExtendedGlobalStateSsz {
    cur_slot: Optional<Slot>,
    next_avail_serial: Optional<u64>,
    limbo_funds_sats: Optional<u64>,
    future_counter: Optional<u64>,
}

#[test]
fn test_absent_extension_preserves_global_state_encoding_and_root() {
    let state = create_test_genesis_state();
    let global = state.global_state();
    let bytes = global.as_ssz_bytes();
    let wire = GlobalStateV1Ssz::from_ssz_bytes(&bytes).unwrap();
    let mut extended = ExtendedGlobalStateSsz {
        cur_slot: wire.cur_slot,
        next_avail_serial: wire.next_avail_serial,
        limbo_funds_sats: wire.limbo_funds_sats,
        future_counter: Optional::None,
    };
    let root = global.tree_hash_root::<Sha256Hasher>();
    assert_eq!(extended.as_ssz_bytes(), bytes);
    assert_eq!(extended.tree_hash_root::<Sha256Hasher>(), root);
    let historical = ExtendedGlobalStateSsz::from_ssz_bytes(&bytes).unwrap();
    assert_eq!(historical.future_counter, Optional::None);
    assert_eq!(historical.tree_hash_root::<Sha256Hasher>(), root);

    extended.future_counter = Optional::Some(0);
    assert_ne!(extended.tree_hash_root::<Sha256Hasher>(), root);
    assert!(GlobalStateV1::from_ssz_bytes(&extended.as_ssz_bytes()).is_err());
}

#[test]
fn test_nested_public_decoders_reject_absent_fields() {
    assert!(EpochalStateV1::from_ssz_bytes(&[0, 0]).is_err());
    assert!(GlobalStateV1::from_ssz_bytes(&[0, 0]).is_err());
    assert!(IntraepochStateV1::from_ssz_bytes(&[0]).is_err());
    assert!(ProtocolStateV1::from_ssz_bytes(&[0]).is_err());
}
