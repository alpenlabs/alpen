use std::num::NonZero;

#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};
use ssz::{Decode, DecodeError, Encode};
use ssz_derive::{Decode as DeriveDecode, Encode as DeriveEncode};
use strata_crypto::threshold_signature::ThresholdConfig;

/// Initialization configuration for the administration subprotocol, containing [`ThresholdConfig`]
/// for each role.
///
/// Design choice: Uses individual named fields rather than `Vec<(Role, ThresholdConfig)>`
/// to ensure structural completeness - the compiler guarantees all config fields are
/// provided when constructing this struct. However, it does NOT prevent logical errors
/// like using the same config for multiple roles or mismatched role-field assignments.
/// The benefit is avoiding missing fields at compile-time rather than runtime validation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct AdministrationInitConfig {
    /// ThresholdConfig for [StrataAdministrator](Role::StrataAdministrator).
    pub strata_administrator: ThresholdConfig,

    /// ThresholdConfig for [StrataSequencerManager](Role::StrataSequencerManager).
    pub strata_sequencer_manager: ThresholdConfig,

    /// The confirmation depth (CD) setting, in Bitcoin blocks: after an update transaction
    /// receives this many confirmations, the update is enacted automatically. During this
    /// confirmation period, the update can still be cancelled by submitting a cancel transaction.
    pub confirmation_depth: u16,

    /// Maximum allowed gap between consecutive sequence numbers for a given authority.
    ///
    /// A payload with `seqno > last_seqno + max_seqno_gap` is rejected. This prevents
    /// excessively large jumps in sequence numbers while still allowing non-sequential usage.
    pub max_seqno_gap: NonZero<u8>,
}

/// SSZ-friendly representation of [`AdministrationInitConfig`].
#[derive(DeriveEncode, DeriveDecode)]
struct AdministrationInitConfigSsz {
    /// ThresholdConfig for [StrataAdministrator](Role::StrataAdministrator).
    strata_administrator: ThresholdConfig,

    /// ThresholdConfig for [StrataSequencerManager](Role::StrataSequencerManager).
    strata_sequencer_manager: ThresholdConfig,

    /// The confirmation depth (CD) setting, in Bitcoin blocks: after an update transaction
    /// receives this many confirmations, the update is enacted automatically. During this
    /// confirmation period, the update can still be cancelled by submitting a cancel transaction.
    confirmation_depth: u16,

    /// Maximum allowed gap between consecutive sequence numbers for a given authority.
    ///
    /// A payload with `seqno > last_seqno + max_seqno_gap` is rejected. This prevents
    /// excessively large jumps in sequence numbers while still allowing non-sequential usage.
    max_seqno_gap: u8,
}

/// Roles with authority in the administration subprotocol.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
#[repr(u8)]
pub enum Role {
    /// The multisig authority that has exclusive ability to:
    /// 1. update (add/remove) bridge signers
    /// 2. update (add/remove) bridge operators
    /// 3. update the definition of what is considered a valid bridge deposit address for:
    ///    - registering deposit UTXOs
    ///    - accepting and minting bridge deposits
    ///    - assigning registered UTXOs to withdrawal requests
    /// 4. update the verifying key for the OL STF
    StrataAdministrator,

    /// The multisig authority that has exclusive ability to change the canonical
    /// public key of the default orchestration layer sequencer.
    StrataSequencerManager,
}

impl Encode for Role {
    fn is_ssz_fixed_len() -> bool {
        <u8 as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <u8 as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        (*self as u8).ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        (*self as u8).ssz_bytes_len()
    }
}

impl Encode for AdministrationInitConfig {
    fn is_ssz_fixed_len() -> bool {
        <AdministrationInitConfigSsz as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <AdministrationInitConfigSsz as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        AdministrationInitConfigSsz {
            strata_administrator: self.strata_administrator.clone(),
            strata_sequencer_manager: self.strata_sequencer_manager.clone(),
            confirmation_depth: self.confirmation_depth,
            max_seqno_gap: self.max_seqno_gap.get(),
        }
        .ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        AdministrationInitConfigSsz {
            strata_administrator: self.strata_administrator.clone(),
            strata_sequencer_manager: self.strata_sequencer_manager.clone(),
            confirmation_depth: self.confirmation_depth,
            max_seqno_gap: self.max_seqno_gap.get(),
        }
        .ssz_bytes_len()
    }
}

impl Decode for AdministrationInitConfig {
    fn is_ssz_fixed_len() -> bool {
        <AdministrationInitConfigSsz as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <AdministrationInitConfigSsz as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let value = AdministrationInitConfigSsz::from_ssz_bytes(bytes)?;
        let max_seqno_gap = NonZero::new(value.max_seqno_gap)
            .ok_or_else(|| DecodeError::BytesInvalid("max_seqno_gap cannot be zero".into()))?;

        Ok(Self {
            strata_administrator: value.strata_administrator,
            strata_sequencer_manager: value.strata_sequencer_manager,
            confirmation_depth: value.confirmation_depth,
            max_seqno_gap,
        })
    }
}

impl Decode for Role {
    fn is_ssz_fixed_len() -> bool {
        <u8 as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <u8 as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        match u8::from_ssz_bytes(bytes)? {
            0 => Ok(Self::StrataAdministrator),
            1 => Ok(Self::StrataSequencerManager),
            value => Err(DecodeError::BytesInvalid(format!(
                "invalid role discriminant: {value}"
            ))),
        }
    }
}

impl AdministrationInitConfig {
    pub fn new(
        strata_administrator: ThresholdConfig,
        strata_sequencer_manager: ThresholdConfig,
        confirmation_depth: u16,
        max_seqno_gap: NonZero<u8>,
    ) -> Self {
        Self {
            strata_administrator,
            strata_sequencer_manager,
            confirmation_depth,
            max_seqno_gap,
        }
    }

    pub fn get_config(&self, role: Role) -> &ThresholdConfig {
        match role {
            Role::StrataAdministrator => &self.strata_administrator,
            Role::StrataSequencerManager => &self.strata_sequencer_manager,
        }
    }

    pub fn get_all_authorities(self) -> Vec<(Role, ThresholdConfig)> {
        vec![
            (Role::StrataAdministrator, self.strata_administrator),
            (Role::StrataSequencerManager, self.strata_sequencer_manager),
        ]
    }
}
