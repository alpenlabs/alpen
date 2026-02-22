use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::buf::Buf32;

use crate::{actions::Sighash, constants::AdminTxType};

/// An update to the Bridge Operator Set:
/// - removes the specified `remove_members`
/// - adds the specified `add_members`
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct OperatorSetUpdate {
    add_members: Vec<Buf32>,
    remove_members: Vec<Buf32>,
}

impl OperatorSetUpdate {
    /// Creates a new `OperatorSetUpdate`.
    pub fn new(add_members: Vec<Buf32>, remove_members: Vec<Buf32>) -> Self {
        Self {
            add_members,
            remove_members,
        }
    }

    /// Borrow the list of operator public keys to add.
    pub fn add_members(&self) -> &[Buf32] {
        &self.add_members
    }

    /// Borrow the list of operator public keys to remove.
    pub fn remove_members(&self) -> &[Buf32] {
        &self.remove_members
    }

    /// Consume and return the inner vectors `(add_members, remove_members)`.
    pub fn into_inner(self) -> (Vec<Buf32>, Vec<Buf32>) {
        (self.add_members, self.remove_members)
    }
}

impl Sighash for OperatorSetUpdate {
    fn tx_type(&self) -> AdminTxType {
        AdminTxType::OperatorUpdate
    }

    /// Returns `len(add) ‖ add[0] ‖ … ‖ add[n] ‖ len(rem) ‖ rem[0] ‖ … ‖ rem[m]`
    /// where lengths are encoded as big-endian `u32`.
    fn sighash_payload(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(
            4 + self.add_members.len() * 32 + 4 + self.remove_members.len() * 32,
        );
        buf.extend_from_slice(&(self.add_members.len() as u32).to_be_bytes());
        for member in &self.add_members {
            buf.extend_from_slice(&member.0);
        }
        buf.extend_from_slice(&(self.remove_members.len() as u32).to_be_bytes());
        for member in &self.remove_members {
            buf.extend_from_slice(&member.0);
        }
        buf
    }
}
