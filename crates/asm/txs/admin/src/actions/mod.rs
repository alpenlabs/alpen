use arbitrary::{Arbitrary, Unstructured};
use strata_l1_txfmt::TagData;

mod cancel;
mod sighash;
pub mod updates;

pub use cancel::CancelAction;
pub use sighash::Sighash;
pub use updates::UpdateAction;

pub use crate::MultisigAction;
use crate::constants::{ADMINISTRATION_SUBPROTOCOL_ID, AdminTxType};

pub type UpdateId = u32;

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

impl<'a> Arbitrary<'a> for MultisigAction {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        if bool::arbitrary(u)? {
            Ok(Self::Cancel(CancelAction::arbitrary(u)?))
        } else {
            Ok(Self::Update(UpdateAction::arbitrary(u)?))
        }
    }
}
