//! Committed versions reserved for future OL upgrades.

use ssz::DecodeError;
use ssz_types::Optional;
use ssz_types::view::ToOwnedSsz;
use strata_identifiers::{SszDelegate, impl_ssz_via_delegate};
use strata_ol_state_types::OLSpecId;

use crate::required_fields::require_present;
use crate::ssz_generated::ssz::state::ProtocolStateV1Ssz;

/// Committed OL versions with both required identifiers present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolStateV1 {
    active_version: OLSpecId,
    expected_version: OLSpecId,
}

impl SszDelegate for ProtocolStateV1 {
    type Delegate = ProtocolStateV1Ssz;

    fn into_delegate(self) -> Self::Delegate {
        ProtocolStateV1Ssz {
            active_version: Optional::Some(self.active_version),
            expected_version: Optional::Some(self.expected_version),
        }
    }

    fn from_delegate(delegate: Self::Delegate) -> Result<Self, DecodeError> {
        Ok(Self {
            active_version: require_present(
                delegate.active_version,
                "protocol_state.active_version",
            )?,
            expected_version: require_present(
                delegate.expected_version,
                "protocol_state.expected_version",
            )?,
        })
    }
}

impl_ssz_via_delegate!(ProtocolStateV1);

impl ToOwnedSsz<ProtocolStateV1> for ProtocolStateV1 {
    fn to_owned(&self) -> ProtocolStateV1 {
        self.clone()
    }
}

impl ProtocolStateV1 {
    /// Creates the protocol state used at genesis.
    pub fn genesis() -> Self {
        Self {
            active_version: OLSpecId::V1,
            expected_version: OLSpecId::V1,
        }
    }

    /// Returns the rules version represented by the committed state.
    pub fn active_version(&self) -> OLSpecId {
        self.active_version
    }

    /// Returns the expected rules version; remains V1 until upgrade handling exists.
    pub fn expected_version(&self) -> OLSpecId {
        self.expected_version
    }
}
