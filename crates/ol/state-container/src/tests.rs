use strata_acct_types::{AccountId, BitcoinAmount};
use strata_ol_state_types::{NewAccountData, NewAccountTypeState};
use strata_ol_state_types_v1::OLStateV1;
use strata_ol_state_types_v1::test_utils::create_test_genesis_state;

use crate::errors::OLStateDecodeError;
use crate::test_utils::{create_test_container_with_staged, genesis_container};
use crate::{OLStateContainer, OLStateLayout};

#[test]
fn test_cbor_round_trip_keeps_versions() {
    // The staged spec may name no known spec; the state must still load.
    for staged in [1, 2, u32::MAX] {
        let container = create_test_container_with_staged(staged);
        let mut bytes = Vec::new();
        ciborium::into_writer(&container, &mut bytes).unwrap();
        let decoded: OLStateContainer = ciborium::from_reader(bytes.as_slice()).unwrap();

        assert_eq!(decoded.staged_spec_version(), staged);
        assert_eq!(decoded, container);
    }
}

#[test]
fn test_staged_version_changes_state_root() {
    let current = create_test_container_with_staged(1);
    let staged = create_test_container_with_staged(2);
    assert_eq!(
        current.root().chainstate_root(),
        staged.root().chainstate_root()
    );
    assert_ne!(current.compute_state_root(), staged.compute_state_root());
}

#[test]
fn test_decode_rejects_unsupported_cur_spec() {
    for raw in [0, 2, u32::MAX] {
        let mut serde_form = create_test_container_with_staged(1).to_serde();
        serde_form.cur_spec_version = raw;

        let err = OLStateContainer::from_serde(serde_form).unwrap_err();
        assert!(
            matches!(&err, OLStateDecodeError::UnsupportedSpec(unknown) if unknown.raw() == raw),
            "unexpected error for {raw}: {err:?}"
        );
    }
}

#[test]
fn test_decode_rejects_chainstate_root_mismatch() {
    let mut other = create_test_genesis_state();
    other.global.cur_slot += 1;
    let mut serde_form = create_test_container_with_staged(1).to_serde();
    serde_form.chainstate_root = other.compute_chainstate_root();

    let err = OLStateContainer::from_serde(serde_form).unwrap_err();
    assert!(matches!(
        err,
        OLStateDecodeError::ChainstateRootMismatch { committed, .. }
            if committed == other.compute_chainstate_root()
    ));
}

/// Returns the test genesis chainstate with an empty account appended last,
/// so the encoding ends with the empty variant of the account type union,
/// after which the SSZ decoder ignores trailing bytes.
fn chainstate_ending_in_empty_account() -> OLStateV1 {
    let mut state = create_test_genesis_state();
    let serial = state.global_state().get_next_avail_serial();
    state
        .create_new_account(
            AccountId::from([0xff; 32]),
            serial,
            NewAccountData::new(BitcoinAmount::default(), NewAccountTypeState::Empty),
        )
        .unwrap();
    state
}

#[test]
fn test_decode_rejects_non_canonical_chainstate() {
    let container = genesis_container(chainstate_ending_in_empty_account());

    let mut trailing = container.to_serde();
    trailing.chainstate.extend_from_slice(&[0xde, 0xad]);
    let mut truncated = container.to_serde();
    truncated
        .chainstate
        .truncate(truncated.chainstate.len() / 2);

    for (label, serde_form) in [("trailing bytes", trailing), ("truncated", truncated)] {
        let err = OLStateContainer::from_serde(serde_form).unwrap_err();
        assert!(
            matches!(
                err,
                OLStateDecodeError::MalformedChainstate {
                    layout: OLStateLayout::V1,
                    ..
                }
            ),
            "{label}: unexpected error {err:?}"
        );
    }
    assert_eq!(
        OLStateContainer::from_serde(container.to_serde()).unwrap(),
        container
    );
}
