use arbitrary::{Arbitrary, Unstructured};

use super::Sighash;
pub use crate::CancelAction;
use crate::{actions::UpdateId, constants::AdminTxType};

impl CancelAction {
    pub fn new(id: UpdateId) -> Self {
        CancelAction { target_id: id }
    }

    pub fn target_id(&self) -> &UpdateId {
        &self.target_id
    }
}

impl Sighash for CancelAction {
    fn tx_type(&self) -> AdminTxType {
        AdminTxType::Cancel
    }

    fn sighash_payload(&self) -> Vec<u8> {
        self.target_id.to_be_bytes().to_vec()
    }
}

impl<'a> Arbitrary<'a> for CancelAction {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::new(UpdateId::arbitrary(u)?))
    }
}
