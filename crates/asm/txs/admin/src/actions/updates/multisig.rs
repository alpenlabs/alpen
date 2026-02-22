use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_params::Role;
use strata_crypto::threshold_signature::ThresholdConfigUpdate;

use crate::{actions::Sighash, constants::AdminTxType};

/// An update to a threshold configuration for a specific role:
/// - adds new members
/// - removes old members
/// - updates the threshold
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct MultisigUpdate {
    config: ThresholdConfigUpdate,
    role: Role,
}

impl MultisigUpdate {
    /// Create a `MultisigUpdate` with given config and role.
    pub fn new(config: ThresholdConfigUpdate, role: Role) -> Self {
        Self { config, role }
    }

    /// Borrow the threshold config update.
    pub fn config(&self) -> &ThresholdConfigUpdate {
        &self.config
    }

    /// Get the role this update applies to.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Consume and return the inner config and role.
    pub fn into_inner(self) -> (ThresholdConfigUpdate, Role) {
        (self.config, self.role)
    }
}

impl Sighash for MultisigUpdate {
    fn tx_type(&self) -> AdminTxType {
        match self.role {
            Role::StrataAdministrator => AdminTxType::StrataAdminMultisigUpdate,
            Role::StrataSequencerManager => AdminTxType::StrataSeqManagerMultisigUpdate,
        }
    }

    /// Returns `len(add) ‖ add[0] ‖ … ‖ add[n] ‖ len(rem) ‖ rem[0] ‖ … ‖ rem[m] ‖ threshold`
    /// where lengths are big-endian `u32` and members are 33-byte compressed public keys.
    ///
    /// Only the config is included because the role is already covered by the
    /// [`AdminTxType`] returned from [`tx_type`](Self::tx_type).
    fn sighash_payload(&self) -> Vec<u8> {
        let add = self.config.add_members();
        let rem = self.config.remove_members();
        let mut buf = Vec::with_capacity(4 + add.len() * 33 + 4 + rem.len() * 33 + 1);
        buf.extend_from_slice(&(add.len() as u32).to_be_bytes());
        for member in add {
            buf.extend_from_slice(&member.serialize());
        }
        buf.extend_from_slice(&(rem.len() as u32).to_be_bytes());
        for member in rem {
            buf.extend_from_slice(&member.serialize());
        }
        buf.push(self.config.new_threshold().get());
        buf
    }
}
