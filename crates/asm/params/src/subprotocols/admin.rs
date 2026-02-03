#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};
use strata_crypto::threshold_signature::ThresholdConfig;

/// Parameters for the admnistration subprotocol, containing ThresholdConfig for each role.
///
/// Design choice: Uses individual named fields rather than `Vec<(Role, ThresholdConfig)>`
/// to ensure structural completeness - the compiler guarantees all 4 config fields are
/// provided when constructing this struct. However, it does NOT prevent logical errors
/// like using the same config for multiple roles or mismatched role-field assignments.
/// The benefit is avoiding missing fields at compile-time rather than runtime validation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct AdministrationSubprotoParams {
    /// ThresholdConfig for [StrataAdministrator](Role::StrataAdministrator).
    pub strata_administrator: ThresholdConfig,

    /// ThresholdConfig for [StrataSequencerManager](Role::StrataSequencerManager).
    pub strata_sequencer_manager: ThresholdConfig,

    /// The confirmation depth (CD) setting: after an update transaction receives this many
    /// confirmations, the update is enacted automatically. During this confirmation period,
    /// the update can still be cancelled by submitting a cancel transaction.
    pub confirmation_depth: u32,
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
