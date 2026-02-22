use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_l1_txfmt::TagData;

mod cancel;
mod sighash;
pub mod updates;

pub use cancel::CancelAction;
pub use sighash::Sighash;
pub use updates::UpdateAction;

use crate::constants::{ADMINISTRATION_SUBPROTOCOL_ID, AdminTxType};

pub type UpdateId = u32;

/// A high‐level multisig operation that participants can propose.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub enum MultisigAction {
    /// Cancel a pending action.
    Cancel(CancelAction),
    /// Propose an update.
    Update(UpdateAction),
}

impl Sighash for MultisigAction {
    fn tx_type(&self) -> AdminTxType {
        match self {
            MultisigAction::Cancel(c) => c.tx_type(),
            MultisigAction::Update(u) => u.tx_type(),
        }
    }

    fn sighash_payload(&self) -> Vec<u8> {
        match self {
            MultisigAction::Cancel(c) => c.sighash_payload(),
            MultisigAction::Update(u) => u.sighash_payload(),
        }
    }
}

impl MultisigAction {
    /// Constructs the SPS-50 [`TagData`] for this action.
    ///
    /// The tag is built from the administration subprotocol ID and the
    /// action's [`AdminTxType`], with no auxiliary data.
    pub fn tag(&self) -> TagData {
        TagData::new(ADMINISTRATION_SUBPROTOCOL_ID, self.tx_type().into(), vec![])
            .expect("empty aux data always fits")
    }
}
