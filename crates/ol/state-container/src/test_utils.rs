//! Test fixtures and proptest strategies for OL state containers.

use proptest::prelude::*;
use strata_ol_state_types::OLSpecId;
use strata_ol_state_types_v1::OLStateV1;
use strata_ol_state_types_v1::test_utils::{create_test_genesis_state, ol_state_strategy};

use crate::{OLStateContainer, OLStateSeries};

/// Wraps a V1 chainstate in a container with both spec versions at
/// [`OLSpecId::GENESIS`].
pub fn genesis_container(chainstate: OLStateV1) -> OLStateContainer {
    OLStateContainer::new(
        OLSpecId::GENESIS,
        OLSpecId::GENESIS.into(),
        OLStateSeries::V1(chainstate),
    )
}

/// Creates a container over the test genesis chainstate with both spec
/// versions at [`OLSpecId::GENESIS`].
pub fn create_test_genesis_container() -> OLStateContainer {
    genesis_container(create_test_genesis_state())
}

/// Creates a container over the test genesis chainstate with the current spec
/// at [`OLSpecId::GENESIS`] and `staged_spec_version` staged.
///
/// A staged version that differs from the current one makes any path that
/// drops or defaults the versions commit to a different root.
pub fn create_test_container_with_staged(staged_spec_version: u32) -> OLStateContainer {
    OLStateContainer::new(
        OLSpecId::GENESIS,
        staged_spec_version,
        OLStateSeries::V1(create_test_genesis_state()),
    )
}

/// Strategy for containers over arbitrary V1 chainstates.
///
/// The staged version is either the current spec or an arbitrary raw value,
/// so it often differs from the current version and often names no known
/// spec.
pub fn ol_state_container_strategy() -> impl Strategy<Value = OLStateContainer> {
    let staged = prop_oneof![Just(u32::from(OLSpecId::V1)), any::<u32>()];
    (ol_state_strategy(), staged).prop_map(|(state, staged_spec_version)| {
        OLStateContainer::new(OLSpecId::V1, staged_spec_version, OLStateSeries::V1(state))
    })
}
